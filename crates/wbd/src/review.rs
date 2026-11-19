//! Reviewing a pull request. All of it comes through dova: the pull
//! request, its diff and comments on its lines from dova's own commands,
//! and what dova has no command for yet (votes, completing, a run's steps)
//! from `dova api`, any Azure DevOps REST call, signed in.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use protocol::{
    ChangedFile, CheckRun, Comment, DiffKind, DiffLine, FileChange, Place, Policy, PullRequest,
    PullTo, Review, Reviewer, Step, Thread, WorkItemRef,
};
use serde_json::{Value, json};
use tokio::process::Command;

use crate::dova::Dova;
use crate::error::Error;
use crate::{git, pulls};

/// Asked of dova for one file, this many lines around each change is all of the file.
const WHOLE_FILE: u64 = 1_000_000;
/// No more of a step's log than its last lines.
const LOG_LINES: usize = 5_000;
/// What the agent is told is no longer than this: it is one argument on its command line.
const BRIEF: usize = 60_000;
/// git gets this long to fetch a pull request's commits.
const FETCH: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct Reviews {
    /// Who you are to Azure DevOps, by the folder asked from.
    me: Mutex<HashMap<String, String>>,
    /// Each pull request's place in Azure DevOps' REST paths, by folder and number.
    paths: Mutex<HashMap<(String, u64), String>>,
    /// Each pull request's whole diff, as dova last gave it.
    diffs: Mutex<HashMap<(String, u64), Value>>,
}

impl Reviews {
    /// A pull request to review.
    pub async fn open(&self, dova: &Dova, dir: &str, pr: u64) -> Result<Review, Error> {
        let id = json!({ "idOrUrl": pr.to_string(), "inlineImages": true });
        let at = self.at(dova, dir, pr).await?;
        let (view, raw, threads, diff, me) = tokio::join!(
            dova.call("pr.view", dir, id),
            get(dova, dir, format!("{at}?api-version=7.1")),
            get(dova, dir, format!("{at}/threads?api-version=7.1")),
            self.whole_diff(dova, dir, pr),
            self.me(dova, dir),
        );
        let (view, raw, threads, diff, me) = (view?, raw?, threads?, diff?, me?);

        // dova says where each thread is after the latest push. Azure DevOps,
        // asked plainly, says where it was written.
        let places: HashMap<u64, Option<Place>> = view["threads"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|thread| Some((thread["id"].as_u64()?, place(&thread["location"]))))
            .collect();
        let reviewers = raw["reviewers"].as_array().into_iter().flatten();
        let vote = reviewers
            .clone()
            .find(|reviewer| reviewer["id"].as_str() == Some(me.as_str()))
            .map_or(0, vote_of);
        let head = text(&diff["head"]);
        // The policies are asked for by the project's own id. Not being able to ask counts as not met.
        let policies = get(
            dova,
            dir,
            format!(
                "_apis/policy/evaluations?artifactId=vstfs:///CodeReview/CodeReviewId/{}/{pr}&api-version=7.1-preview.1",
                text(&raw["repository"]["project"]["id"])
            ),
        )
        .await;
        Ok(Review {
            pr: shape::<PullRequest>(view.clone())?,
            author: text(&view["author"]),
            description: text(&view["description"]),
            merge: raw["mergeStatus"].as_str().map(str::to_string),
            auto_complete: raw["autoCompleteSetBy"]["id"].is_string(),
            ready: policies.as_ref().is_ok_and(met),
            mine: raw["createdBy"]["id"].as_str() == Some(me.as_str()),
            policies: policies.as_ref().map(listed).unwrap_or_default(),
            reviewers: reviewers
                .map(|reviewer| Reviewer {
                    name: text(&reviewer["displayName"]),
                    vote: vote_of(reviewer),
                    required: reviewer["isRequired"].as_bool() == Some(true),
                })
                .collect(),
            vote,
            work_items: shape::<Vec<WorkItemRef>>(view["workItems"].clone())?,
            threads: threads["value"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|raw| thread(raw, &places))
                .collect(),
            files: diff["files"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|file| changed(file, &head))
                .collect(),
            base: text(&diff["base"]),
            head,
        })
    }

    pub async fn vote(&self, dova: &Dova, dir: &str, pr: u64, vote: i8) -> Result<(), Error> {
        let (at, me) = (self.at(dova, dir, pr).await?, self.me(dova, dir).await?);
        let path = format!("{at}/reviewers/{me}?api-version=7.1");
        send(dova, dir, "PUT", path, &json!({ "vote": vote })).await
    }

    /// Completes, abandons or reopens a pull request, or moves it in or out of draft.
    pub async fn set(&self, dova: &Dova, dir: &str, pr: u64, to: PullTo) -> Result<(), Error> {
        let at = format!("{}?api-version=7.1", self.at(dova, dir, pr).await?);
        let how = json!({ "mergeStrategy": "squash", "deleteSourceBranch": true, "transitionWorkItems": true });
        let body = match to {
            PullTo::Complete => {
                // Azure DevOps completes it only if told which commit you last saw.
                let raw = get(dova, dir, at.clone()).await?;
                json!({
                    "status": "completed",
                    "lastMergeSourceCommit": { "commitId": raw["lastMergeSourceCommit"]["commitId"] },
                    "completionOptions": how,
                })
            }
            PullTo::AutoComplete => {
                json!({ "autoCompleteSetBy": { "id": self.me(dova, dir).await? }, "completionOptions": how })
            }
            PullTo::CancelAutoComplete => {
                json!({ "autoCompleteSetBy": { "id": "00000000-0000-0000-0000-000000000000" } })
            }
            PullTo::Draft => json!({ "isDraft": true }),
            PullTo::Ready => json!({ "isDraft": false }),
            PullTo::Abandon => json!({ "status": "abandoned" }),
            PullTo::Reactivate => json!({ "status": "active" }),
        };
        send(dova, dir, "PATCH", at, &body).await
    }

    /// The pipeline runs on a pull request's branch and on its merge, the newest first.
    pub async fn checks(&self, dova: &Dova, dir: &str, pr: u64) -> Result<Vec<CheckRun>, Error> {
        let at = self.at(dova, dir, pr).await?;
        let raw = get(dova, dir, format!("{at}?api-version=7.1")).await?;
        let branches = [text(&raw["sourceRefName"]), format!("refs/pull/{pr}/merge")];
        let mut runs = Vec::new();
        for branch in branches {
            let path = format!(
                "_apis/build/builds?branchName={}&$top=10&queryOrder=queueTimeDescending&api-version=7.1",
                encoded(&branch)
            );
            let listed = get(dova, dir, path).await?;
            runs.extend(listed["value"].as_array().into_iter().flatten().map(check));
        }
        runs.sort_by_key(|run| std::cmp::Reverse(run.id));
        Ok(runs)
    }

    /// Gets a pull request's own folder ready for the agent, and what to
    /// tell the agent about the pull request.
    pub async fn session(
        &self,
        dova: &Dova,
        data: &Path,
        dir: &str,
        pr: u64,
    ) -> Result<(String, String), Error> {
        let review = self.open(dova, dir, pr).await?;
        let folder = Path::new(dir);
        // The agent works on the code, so the folder asked from must be that repo's.
        let origin = git::origin(folder).await.unwrap_or_default();
        let here = pulls::azure(&origin).map(|(_, repo)| repo);
        if !here.is_some_and(|repo| repo.eq_ignore_ascii_case(&review.pr.repo)) {
            return Err(Error::NotFound(format!(
                "The {} repo has no folder open here. Open it to work on this pull request with the agent.",
                review.pr.repo
            )));
        }
        let own = data.join("reviews").join(format!("{}-{pr}", slug(&origin)));
        checkout(folder, &own, &review.pr.source_branch, &review.head).await?;
        let runs = self.checks(dova, dir, pr).await.unwrap_or_default();
        Ok((own.display().to_string(), brief(&review, &runs)))
    }

    /// A pull request's description as it was written.
    pub async fn description(&self, dova: &Dova, dir: &str, pr: u64) -> Result<String, Error> {
        let at = self.at(dova, dir, pr).await?;
        // Asked plainly: with its pictures put in place, it could not be written back.
        let path = format!("{at}?api-version=7.1");
        let raw = dova.call("api", dir, json!({ "path": path })).await?;
        Ok(text(&raw["description"]))
    }

    /// A pull request's whole diff. It only changes with a push, so it is
    /// kept until the pull request ends at another commit.
    async fn whole_diff(&self, dova: &Dova, dir: &str, pr: u64) -> Result<Value, Error> {
        let key = (dir.to_string(), pr);
        // Which files changed says which push this is, and costs one request.
        let asked = json!({ "pr": pr.to_string(), "names": true });
        let names = dova.call("pr.diff", dir, asked).await?;
        let kept = self.diffs.lock().expect("not poisoned").get(&key).cloned();
        let same = |kept: &Value| kept["head"] == names["head"] && kept["base"] == names["base"];
        if let Some(kept) = kept.filter(same) {
            return Ok(kept);
        }
        let diff = dova
            .call("pr.diff", dir, json!({ "pr": pr.to_string() }))
            .await?;
        self.diffs
            .lock()
            .expect("not poisoned")
            .insert(key, diff.clone());
        Ok(diff)
    }

    /// Where a pull request is in Azure DevOps' REST paths. A pull request
    /// is asked for by number in its project, and written to in its repo.
    async fn at(&self, dova: &Dova, dir: &str, pr: u64) -> Result<String, Error> {
        let key = (dir.to_string(), pr);
        if let Some(known) = self.paths.lock().expect("not poisoned").get(&key) {
            return Ok(known.clone());
        }
        let raw = get(
            dova,
            dir,
            format!("_apis/git/pullrequests/{pr}?api-version=7.1"),
        )
        .await?;
        let repo = raw["repository"]["id"].as_str().ok_or_else(|| {
            Error::Dova(format!(
                "Azure DevOps didn't say which repo pull request {pr} is in"
            ))
        })?;
        let at = format!("_apis/git/repositories/{repo}/pullRequests/{pr}");
        self.paths
            .lock()
            .expect("not poisoned")
            .insert(key, at.clone());
        Ok(at)
    }

    /// Who you are to the organization `dir` is cloned from. Asked once.
    pub async fn me(&self, dova: &Dova, dir: &str) -> Result<String, Error> {
        if let Some(known) = self.me.lock().expect("not poisoned").get(dir) {
            return Ok(known.clone());
        }
        let path = "/_apis/connectionData?api-version=7.1-preview.1".to_string();
        let who = get(dova, dir, path).await?;
        let id = who["authenticatedUser"]["id"]
            .as_str()
            .ok_or_else(|| Error::Dova("Azure DevOps didn't say who you are".into()))?
            .to_string();
        self.me
            .lock()
            .expect("not poisoned")
            .insert(dir.to_string(), id.clone());
        Ok(id)
    }
}

/// One changed file's diff with every line of the file.
pub async fn diff(dova: &Dova, dir: &str, pr: u64, path: &str) -> Result<Vec<DiffLine>, Error> {
    let params =
        json!({ "pr": pr.to_string(), "files": [format!("/{path}")], "context": WHOLE_FILE });
    let diff = dova.call("pr.diff", dir, params).await?;
    let file = diff["files"]
        .get(0)
        .ok_or_else(|| Error::NotFound(format!("pull request {pr} doesn't change {path}")))?;
    Ok(lines(file))
}

/// Starts a thread: on lines of a file, or on the whole pull request.
pub async fn comment(
    dova: &Dova,
    dir: &str,
    pr: u64,
    words: &str,
    place: Option<Place>,
) -> Result<(), Error> {
    let mut params = json!({ "pr": pr.to_string(), "text": words });
    if let Some(place) = place {
        let (Some(from), Some(line)) = (place.from, place.line) else {
            return Err(Error::NotFound(
                "a comment on a file needs its lines".into(),
            ));
        };
        params["at"] = json!(match from == line {
            true => format!("{}:{line}", place.file),
            false => format!("{}:{from}-{line}", place.file),
        });
        params["old"] = json!(place.old);
    }
    dova.call("pr.comment", dir, params).await.map(|_| ())
}

/// A run's jobs, each followed by its steps, and what the pipeline said
/// went wrong outside any of them.
pub async fn steps(dova: &Dova, dir: &str, run: u64) -> Result<(Vec<Step>, Vec<String>), Error> {
    let path = format!("_apis/build/builds/{run}/timeline?api-version=7.1");
    Ok(read_timeline(&get(dova, dir, path).await?))
}

/// What one step of a run printed: the end of it, without each line's time.
pub async fn step_log(dova: &Dova, dir: &str, run: u64, log: u64) -> Result<String, Error> {
    let path = format!("_apis/build/builds/{run}/logs/{log}?api-version=7.1");
    let printed = get(dova, dir, path).await?;
    let lines: Vec<&str> = printed["value"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(untimed)
        .collect();
    Ok(lines[lines.len().saturating_sub(LOG_LINES)..].join("\n"))
}

/// Runs a run's failed jobs again.
pub async fn rerun(dova: &Dova, dir: &str, run: u64) -> Result<(), Error> {
    let path = format!("_apis/build/builds/{run}?retry=true&api-version=7.1");
    send(dova, dir, "PATCH", path, &json!({})).await
}

/// A REST call through dova. A picture kept on Azure DevOps that the
/// answer links to comes back as the picture itself.
/// Keeps a picture in a file under `data`, so a description can name it. Gives the file's path.
pub fn keep_picture(data: &Path, name: &str, base64: &str) -> Result<String, Error> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64)
        .map_err(|err| Error::Io(format!("The picture did not arrive whole: {err}")))?;
    // Only the ending is kept, and only if it is plain: the name came from outside.
    let ending = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("png")
        .to_ascii_lowercase();
    let folder = data.join("pictures");
    std::fs::create_dir_all(&folder).map_err(|err| Error::io(&folder, err))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_millis());
    let path = folder.join(format!("picture-{stamp}.{ending}"));
    std::fs::write(&path, bytes).map_err(|err| Error::io(&path, err))?;
    Ok(path.display().to_string())
}

/// Changes a pull request's title, its description, or both.
pub async fn edit(
    dova: &Dova,
    dir: &str,
    pr: u64,
    title: Option<String>,
    description: Option<String>,
) -> Result<(), Error> {
    let mut params = json!({ "idOrUrl": pr.to_string() });
    if let Some(title) = title {
        params["title"] = json!(title);
    }
    if let Some(description) = description {
        params["body"] = json!(description);
    }
    dova.call("pr.edit", dir, params).await.map(|_| ())
}

async fn get(dova: &Dova, dir: &str, path: String) -> Result<Value, Error> {
    dova.call("api", dir, json!({ "path": path, "inlineImages": true }))
        .await
}

async fn send(
    dova: &Dova,
    dir: &str,
    method: &str,
    path: String,
    body: &Value,
) -> Result<(), Error> {
    let params = json!({ "path": path, "method": method, "body": body.to_string() });
    dova.call("api", dir, params).await.map(|_| ())
}

fn shape<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, Error> {
    serde_json::from_value(value)
        .map_err(|err| Error::Dova(format!("couldn't read what dova said: {err}")))
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

/// Whether every policy on a pull request is met, the optional ones too: left to complete by
/// itself, it waits for those as well. With no policy, it is.
fn met(evaluations: &Value) -> bool {
    let all = evaluations["value"].as_array().into_iter().flatten();
    all.filter(|policy| policy["configuration"]["isEnabled"].as_bool() != Some(false))
        .all(|policy| {
            matches!(
                policy["status"].as_str(),
                Some("approved" | "notApplicable")
            )
        })
}

/// The policies on a pull request that are switched on, each with where it stands.
fn listed(evaluations: &Value) -> Vec<Policy> {
    let all = evaluations["value"].as_array().into_iter().flatten();
    all.filter(|policy| policy["configuration"]["isEnabled"].as_bool() != Some(false))
        .map(|policy| {
            let rule = &policy["configuration"];
            // A build policy is known by the name it was given, when it has one.
            let named = rule["settings"]["displayName"]
                .as_str()
                .filter(|name| !name.is_empty());
            Policy {
                name: named.map_or_else(|| text(&rule["type"]["displayName"]), str::to_string),
                status: text(&policy["status"]),
                blocking: rule["isBlocking"].as_bool() == Some(true),
            }
        })
        .collect()
}

fn vote_of(reviewer: &Value) -> i8 {
    reviewer["vote"].as_i64().unwrap_or_default() as i8
}

/// Where a thread is, from dova's `location`. None for one about the whole pull request.
fn place(location: &Value) -> Option<Place> {
    let line = |key: &str| location[key].as_u64().map(|line| line as u32);
    Some(Place {
        file: location["file"]
            .as_str()?
            .trim_start_matches('/')
            .to_string(),
        from: line("line"),
        line: line("endLine").or(line("line")),
        old: location["side"].as_str() == Some("old"),
        outdated: location["outdated"].as_bool() == Some(true),
    })
}

/// A thread people wrote in, placed where dova says it is now. None for
/// one Azure DevOps wrote itself, such as "added a reviewer", and for a deleted one.
fn thread(raw: &Value, places: &HashMap<u64, Option<Place>>) -> Option<Thread> {
    if raw["isDeleted"].as_bool() == Some(true) {
        return None;
    }
    let comments: Vec<Comment> = raw["comments"]
        .as_array()?
        .iter()
        .filter(|comment| {
            comment["isDeleted"].as_bool() != Some(true)
                && comment["commentType"].as_str() != Some("system")
        })
        .map(|comment| Comment {
            id: comment["id"].as_u64().unwrap_or_default(),
            author: comment["author"]["displayName"]
                .as_str()
                .map(str::to_string),
            published_at: comment["publishedDate"].as_str().map(str::to_string),
            text: text(&comment["content"]),
        })
        .collect();
    if comments.is_empty() {
        return None;
    }
    let id = raw["id"].as_u64()?;
    // A thread with no status is a plain comment: there is nothing to resolve.
    let status = raw["status"].as_str().unwrap_or("closed");
    Some(Thread {
        id,
        status: status.to_string(),
        unresolved: matches!(status, "active" | "pending"),
        place: places.get(&id).cloned().flatten(),
        comments,
    })
}

/// A changed file, from dova's diff.
fn changed(file: &Value, head: &str) -> ChangedFile {
    let lines = lines(file);
    let count = |key: &str| file[key].as_u64().unwrap_or_default() as u32;
    let mut hasher = DefaultHasher::new();
    match lines.is_empty() {
        // Nothing to tell one change from another by: any push counts as one.
        true => head.hash(&mut hasher),
        false => lines
            .iter()
            .for_each(|line| (line.kind as u8, &line.text).hash(&mut hasher)),
    }
    ChangedFile {
        path: text(&file["path"]).trim_start_matches('/').to_string(),
        old_path: file["oldPath"]
            .as_str()
            .map(|path| path.trim_start_matches('/').to_string()),
        change: match file["change"].as_str() {
            Some("added") => FileChange::Added,
            Some("deleted") => FileChange::Deleted,
            Some("renamed") => FileChange::Renamed,
            _ => FileChange::Edited,
        },
        added: count("added"),
        removed: count("removed"),
        mark: format!("{:016x}", hasher.finish()),
        binary: file["binary"].as_bool() == Some(true),
        large: file["large"].as_bool() == Some(true),
        lines,
    }
}

/// A file's hunks as one list of lines. Where lines are left out between
/// two hunks, the numbers jump.
fn lines(file: &Value) -> Vec<DiffLine> {
    let hunks = file["hunks"].as_array().into_iter().flatten();
    hunks
        .flat_map(|hunk| hunk["lines"].as_array().into_iter().flatten())
        .map(|line| DiffLine {
            kind: match line["kind"].as_str() {
                Some("added") => DiffKind::Added,
                Some("removed") => DiffKind::Removed,
                _ => DiffKind::Context,
            },
            old: line["old"].as_u64().map(|n| n as u32),
            new: line["new"].as_u64().map(|n| n as u32),
            text: text(&line["text"]),
        })
        .collect()
}

fn check(run: &Value) -> CheckRun {
    let time = |key: &str| run[key].as_str().map(str::to_string);
    CheckRun {
        id: run["id"].as_u64().unwrap_or_default(),
        name: text(&run["definition"]["name"]),
        number: text(&run["buildNumber"]),
        status: text(&run["status"]),
        result: time("result"),
        commit: text(&run["sourceVersion"]),
        queued_at: time("queueTime"),
        started_at: time("startTime"),
        finished_at: time("finishTime"),
        url: text(&run["_links"]["web"]["href"]),
    }
}

/// A run's timeline as jobs, each followed by its steps in the order they ran.
fn read_timeline(timeline: &Value) -> (Vec<Step>, Vec<String>) {
    let records: Vec<&Value> = timeline["records"]
        .as_array()
        .into_iter()
        .flatten()
        .collect();
    let errors = |record: &Value| -> Vec<String> {
        record["issues"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|issue| issue["type"].as_str() == Some("error"))
            .filter_map(|issue| issue["message"].as_str().map(str::to_string))
            .collect()
    };
    let step = |record: &Value, job: bool| Step {
        name: text(&record["name"]),
        job,
        state: text(&record["state"]),
        result: record["result"].as_str().map(str::to_string),
        started_at: record["startTime"].as_str().map(str::to_string),
        finished_at: record["finishTime"].as_str().map(str::to_string),
        log: record["log"]["id"].as_u64(),
        errors: errors(record),
    };
    let of = |kind: &'static str| {
        let mut found: Vec<&Value> = records
            .iter()
            .copied()
            .filter(|record| record["type"].as_str() == Some(kind))
            .collect();
        found.sort_by_key(|record| record["order"].as_u64().unwrap_or(u64::MAX));
        found
    };

    let mut steps = Vec::new();
    for job in of("Job") {
        steps.push(step(job, true));
        let mine = of("Task")
            .into_iter()
            .filter(|task| task["parentId"] == job["id"]);
        steps.extend(mine.map(|task| step(task, false)));
    }
    // What stopped the run before a job could start is said on a stage or a phase.
    let outside = records
        .iter()
        .filter(|record| !matches!(record["type"].as_str(), Some("Job" | "Task")))
        .flat_map(|record| errors(record))
        .collect();
    (steps, outside)
}

/// A line of log without the time Azure Pipelines puts in front of it.
fn untimed(line: &str) -> &str {
    let stamped = line.len() > 29 && line.as_bytes()[10] == b'T' && line.as_bytes()[28] == b' ';
    if stamped { &line[29..] } else { line }
}

/// A value as it goes in a URL's query.
fn encoded(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// A folder's name made from a repo's address.
fn slug(origin: &str) -> String {
    let named: String = origin
        .trim()
        .split("://")
        .last()
        .unwrap_or_default()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    named.trim_matches('-').to_string()
}

/// Runs git in `dir`, giving what it printed, or why it failed.
async fn run(dir: &Path, args: &[&str], patience: Option<Duration>) -> Result<Vec<u8>, String> {
    let going = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = match patience {
        Some(patience) => tokio::time::timeout(patience, going)
            .await
            .map_err(|_| "git took too long".to_string())?,
        None => going.await,
    }
    .map_err(|err| format!("couldn't run git: {err}"))?;
    match output.status.success() {
        true => Ok(output.stdout),
        false => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
    }
}

/// Puts a pull request's last commit in a folder of its own, beside the
/// repo's folder and not in place of it. A folder that has changes of its
/// own, the agent's or yours, is left as it is.
async fn checkout(repo: &Path, own: &Path, branch: &str, head: &str) -> Result<(), Error> {
    let named = head.len() >= 7 && head.bytes().all(|byte| byte.is_ascii_hexdigit());
    if !named {
        return Err(Error::Dova(
            "dova didn't say which commit the pull request ends at".into(),
        ));
    }
    // The repo's folder is given the commit first, if it doesn't have it.
    let commit = format!("{head}^{{commit}}");
    if run(repo, &["cat-file", "-e", &commit], None).await.is_err() {
        let from = format!("+refs/heads/{branch}:refs/remotes/origin/{branch}");
        let fetch = ["fetch", "--quiet", "--no-tags", "origin", &from];
        run(repo, &fetch, Some(FETCH)).await.map_err(|why| {
            Error::Io(format!(
                "git couldn't fetch the pull request's branch: {why}"
            ))
        })?;
    }
    let there = own.display().to_string();
    if !own.join(".git").exists() {
        if let Some(parent) = own.parent() {
            std::fs::create_dir_all(parent).map_err(|err| Error::io(parent, err))?;
        }
        // A checkout that was deleted by hand is still on git's list until pruned.
        let _ = run(repo, &["worktree", "prune"], None).await;
        let add = ["worktree", "add", "--quiet", "--detach", &there, head];
        return run(repo, &add, None).await.map(|_| ()).map_err(|why| {
            Error::Io(format!(
                "couldn't check the pull request out to {there}: {why}"
            ))
        });
    }
    let changes = run(own, &["status", "--porcelain"], None).await;
    if changes.is_ok_and(|changes| changes.is_empty()) {
        let _ = run(own, &["checkout", "--quiet", "--detach", head], None).await;
    }
    Ok(())
}

/// What the agent is told about a pull request before anything you write.
fn brief(review: &Review, runs: &[CheckRun]) -> String {
    let pr = &review.pr;
    let short = |commit: &str| commit.get(..9).unwrap_or(commit).to_string();
    let clip = |words: &str, most: usize| match words.char_indices().nth(most) {
        Some((cut, _)) => format!("{} […]", &words[..cut]),
        None => words.to_string(),
    };
    let mut out = format!(
        "You are helping someone review pull request #{id} \"{title}\" in the {repo} repository on Azure DevOps.\n\
         Opened by {author}. It merges {source} into {target}. It is {state}.\n\n\
         This folder is a separate checkout of the pull request's branch, at {head}, not on any branch. \
         The whole change is `git diff {base} {head}`. \
         To push a fix to the pull request: commit here, then `git push origin HEAD:refs/heads/{source}`.\n\
         dova is installed. `dova pr view {id}`, `dova pr diff {id}` and `dova pr thread {id} <thread>` read the pull request as it is now, \
         and `dova pipeline log <run>` reads why a run failed.\n\
         Do not post comments, vote, or push unless you are asked to: the person posts from the Workbench.\n\
         When you are asked about lines of a file, answer about those lines. \
         When you are asked to draft a comment or a reply, answer with only the text to post.\n\
         A message may start with an `<about>` block. It says what the person is looking at as they ask: \
         lines of a file, a review thread, or a pipeline run. It is not part of what they wrote.\n",
        id = pr.id,
        title = pr.title,
        repo = pr.repo,
        author = review.author,
        source = pr.source_branch,
        target = pr.target_branch,
        state = if pr.is_draft {
            "a draft"
        } else {
            pr.status.as_str()
        },
        head = short(&review.head),
        base = short(&review.base),
    );
    if !review.description.trim().is_empty() {
        out += &format!("\n## Description\n{}\n", clip(&review.description, 4_000));
    }
    if !review.work_items.is_empty() {
        out += "\n## Work items\n";
        for item in &review.work_items {
            let kind = item.kind.as_deref().unwrap_or("Work item");
            let (title, state) = (
                item.title.as_deref().unwrap_or(""),
                item.state.as_deref().unwrap_or(""),
            );
            out += &format!("- #{} {kind}: {title} ({state})\n", item.id);
        }
    }
    out += "\n## Changed files\n";
    for file in &review.files {
        out += &format!("- {} (+{} -{})\n", file.path, file.added, file.removed);
    }
    if !review.threads.is_empty() {
        out += "\n## Review threads\n";
        for thread in &review.threads {
            let state = if thread.unresolved {
                "open"
            } else {
                "resolved"
            };
            out += &format!(
                "- Thread #{} ({state}) {}\n",
                thread.id,
                wherabouts(thread.place.as_ref())
            );
            for comment in &thread.comments {
                let author = comment.author.as_deref().unwrap_or("Someone");
                out += &format!(
                    "  - {author}: {}\n",
                    clip(&comment.text, 1_500).replace('\n', " ")
                );
            }
        }
    }
    if !runs.is_empty() {
        out += "\n## Pipeline runs, the newest first\n";
        for run in runs.iter().take(5) {
            let result = run.result.as_deref().unwrap_or(&run.status);
            out += &format!(
                "- {} {}: {result} (run {}, on {})\n",
                run.name,
                run.number,
                run.id,
                short(&run.commit)
            );
        }
    }
    clip(&out, BRIEF)
}

/// Where a thread is, in words.
fn wherabouts(place: Option<&Place>) -> String {
    let Some(place) = place else {
        return "on the whole pull request".into();
    };
    let side = if place.old { " of the old file" } else { "" };
    match (place.from, place.line) {
        (Some(from), Some(line)) if from != line => {
            format!("on {}:{from}-{line}{side}", place.file)
        }
        (_, Some(line)) => format!("on {}:{line}{side}", place.file),
        _ if place.outdated => format!("on {}, on lines a later push removed", place.file),
        _ => format!("on the file {}", place.file),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_dovas_diff_of_a_file() {
        let file = json!({
            "path": "/src/auth.ts", "oldPath": null, "change": "edited", "binary": false, "added": 2, "removed": 1,
            "hunks": [
                { "oldStart": 3, "lines": [
                    { "kind": "context", "old": 3, "new": 3, "text": "same" },
                    { "kind": "removed", "old": 4, "new": null, "text": "old" },
                    { "kind": "added", "old": null, "new": 4, "text": "new" },
                ] },
                { "oldStart": 40, "lines": [{ "kind": "added", "old": null, "new": 41, "text": "more" }] },
            ],
        });
        let read = changed(&file, "abc");
        assert_eq!(
            (read.path.as_str(), read.change, read.added, read.removed),
            ("src/auth.ts", FileChange::Edited, 2, 1)
        );
        let seen: Vec<_> = read
            .lines
            .iter()
            .map(|l| (l.kind, l.old, l.new, l.text.as_str()))
            .collect();
        assert_eq!(
            seen,
            [
                (DiffKind::Context, Some(3), Some(3), "same"),
                (DiffKind::Removed, Some(4), None, "old"),
                (DiffKind::Added, None, Some(4), "new"),
                (DiffKind::Added, None, Some(41), "more"),
            ]
        );
        // The same change has the same mark, and a different one a different mark.
        assert_eq!(read.mark, changed(&file, "another push").mark);
        let mut other = file.clone();
        other["hunks"][1]["lines"][0]["text"] = json!("changed again");
        assert_ne!(read.mark, changed(&other, "abc").mark);

        let renamed = json!({ "path": "/docs/to.md", "oldPath": "/docs/from.md", "change": "renamed", "binary": true });
        let read = changed(&renamed, "abc");
        assert_eq!(
            (read.old_path.as_deref(), read.change, read.binary),
            (Some("docs/from.md"), FileChange::Renamed, true)
        );
    }

    #[test]
    fn keeps_the_threads_people_wrote_and_places_them() {
        let none = HashMap::new();
        let system = json!({ "id": 19, "comments": [{ "id": 1, "commentType": "system", "content": "added a reviewer" }] });
        assert_eq!(thread(&system, &none), None);

        let raw = json!({
            "id": 4, "status": "active",
            "comments": [{ "id": 1, "commentType": "text", "content": "Why?", "author": { "displayName": "Sam Lee" }, "publishedDate": "2026-11-14T19:47:13Z" }],
        });
        let at = place(
            &json!({ "file": "/src/auth.ts", "line": 42, "endLine": 44, "side": "new", "outdated": false }),
        );
        let places = HashMap::from([(4, at)]);
        let read = thread(&raw, &places).unwrap();
        assert!(read.unresolved);
        assert_eq!(
            read.place,
            Some(Place {
                file: "src/auth.ts".into(),
                from: Some(42),
                line: Some(44),
                old: false,
                outdated: false
            })
        );
        assert_eq!(read.comments[0].author.as_deref(), Some("Sam Lee"));
        assert_eq!(wherabouts(read.place.as_ref()), "on src/auth.ts:42-44");

        let gone = place(&json!({ "file": "/a.rs", "line": null, "endLine": null, "side": "old", "outdated": true })).unwrap();
        assert!(gone.old && gone.outdated && gone.line.is_none());
        assert_eq!(place(&Value::Null), None);
        assert_eq!(thread(&raw, &none).unwrap().place, None);
    }

    #[test]
    fn reads_a_timeline_as_jobs_and_their_steps() {
        let timeline = json!({ "records": [
            { "id": "t2", "parentId": "j1", "type": "Task", "name": "npm test", "order": 2, "state": "completed", "result": "failed",
              "log": { "id": 9 }, "issues": [{ "type": "error", "message": "exit code 1" }, { "type": "warning", "message": "slow" }] },
            { "id": "j1", "parentId": "p1", "type": "Job", "name": "Test", "order": 1, "state": "completed", "result": "failed" },
            { "id": "t1", "parentId": "j1", "type": "Task", "name": "Checkout", "order": 1, "state": "completed", "result": "succeeded" },
            { "id": "p1", "type": "Phase", "name": "Test", "issues": [{ "type": "error", "message": "No agent found" }] },
        ] });
        let (steps, errors) = read_timeline(&timeline);
        let seen: Vec<_> = steps
            .iter()
            .map(|s| (s.name.as_str(), s.job, s.log))
            .collect();
        assert_eq!(
            seen,
            [
                ("Test", true, None),
                ("Checkout", false, None),
                ("npm test", false, Some(9))
            ]
        );
        assert_eq!(steps[2].errors, ["exit code 1"]);
        assert_eq!(errors, ["No agent found"]);
    }

    #[test]
    fn lists_the_policies_that_are_on() {
        let evaluations = json!({ "value": [
            { "configuration": { "isBlocking": true, "isEnabled": true, "type": { "displayName": "Build" }, "settings": { "displayName": "CI" } }, "status": "rejected" },
            { "configuration": { "isBlocking": false, "isEnabled": true, "type": { "displayName": "Comment requirements" }, "settings": {} }, "status": "approved" },
            { "configuration": { "isBlocking": true, "isEnabled": false, "type": { "displayName": "Work item linking" } }, "status": "queued" }
        ] });
        let policies = listed(&evaluations);
        assert_eq!(policies.len(), 2);
        assert_eq!(
            (
                policies[0].name.as_str(),
                policies[0].status.as_str(),
                policies[0].blocking
            ),
            ("CI", "rejected", true)
        );
        assert_eq!(
            (policies[1].name.as_str(), policies[1].blocking),
            ("Comment requirements", false)
        );
    }

    #[test]
    fn knows_when_the_policies_are_met() {
        let policy = |enabled: bool, status: &str| json!({ "configuration": { "isBlocking": false, "isEnabled": enabled }, "status": status });
        assert!(met(&json!({ "value": [] })));
        assert!(met(
            &json!({ "value": [policy(true, "approved"), policy(false, "rejected")] })
        ));
        assert!(!met(
            &json!({ "value": [policy(true, "approved"), policy(true, "queued")] })
        ));
        assert!(!met(&json!({ "value": [policy(true, "rejected")] })));
    }

    #[test]
    fn small_things() {
        assert_eq!(
            untimed("2026-11-19T01:41:14.0109789Z ##[section]Starting: Job"),
            "##[section]Starting: Job"
        );
        assert_eq!(untimed("plain"), "plain");
        assert_eq!(encoded("refs/heads/fix/a b#1"), "refs/heads/fix/a%20b%231");
        assert_eq!(
            slug("https://dev.azure.com/contoso/Shop/_git/web\n"),
            "dev-azure-com-contoso-Shop--git-web"
        );
    }
}
