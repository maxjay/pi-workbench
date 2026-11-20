//! Reviewing a pull request. All of it comes through dova. What is read of
//! a pull request is asked of Azure DevOps with `dova api`, any REST call,
//! signed in: each thing once, and what does not change between two reads is
//! kept. What changed in a file, and writing a comment or a description, are
//! dova's own commands.

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use protocol::{
    ChangedFile, CheckRun, Comment, DiffKind, DiffLine, FileChange, Place, Policy, PullRequest,
    PullTo, Review, Reviewer, Step, Thread, WorkItemRef,
};
use serde_json::{Value, json};

use crate::dova::Dova;
use crate::error::Error;
use crate::git::run;
use crate::places::Found;
use crate::{files, git, pulls};

/// Asked of dova for one file, this many lines around each change is all of the file.
const WHOLE_FILE: u64 = 1_000_000;
/// A file that changes more lines than this comes without them: it would be most of the
/// answer. Asking for the whole file gets them. dova has the same rule for a whole diff.
const LARGE: usize = 10_000;
/// No more of a step's log than its last lines.
const LOG_LINES: usize = 5_000;
/// What the agent is told is no longer than this: it is one argument on its command line.
const BRIEF: usize = 60_000;
/// How the name of a pull request's folder ends when it is not a checkout: no clone of the
/// repo was here to make one from.
const PLAIN: &str = "-quick";
/// A pull request's files and work items are asked for again after this long, even when it
/// was not pushed to.
const WHOLE: Duration = Duration::from_secs(600);
/// A pull request that was read to find where it is counts as read now for this long.
const JUST: Duration = Duration::from_secs(5);
/// A push's changed files are asked for this many at a time.
const PAGE: u64 = 2000;
/// Work items are asked about this many at a time: Azure DevOps takes no more.
const ITEMS: usize = 200;
/// What was read this lately is used again where the agent's folder is made and the agent is
/// told about the pull request, in place of reading it once more.
const FRESH: Duration = Duration::from_secs(60);
/// git gets this long to fetch a pull request's commits.
const FETCH: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct Reviews {
    /// Who you are to each organization, by its place.
    me: Mutex<HashMap<String, String>>,
    /// The organization each folder that was asked from is cloned from, as its place.
    orgs: Mutex<HashMap<String, String>>,
    /// Each pull request as Azure DevOps gave it when it was looked for, until it is read.
    seen: Mutex<HashMap<(String, u64), (Instant, Value)>>,
    /// What Azure DevOps said each pull request is in, by the place asked from and number.
    known: Mutex<HashMap<(String, u64), Known>>,
    /// The project of each pipeline run that was listed, by the run's id.
    runs: Mutex<HashMap<u64, String>>,
    /// Each pull request as it was last read, with the parts that are not asked for each time.
    held: Mutex<HashMap<(String, u64), Held>>,
    /// Each pull request's pipeline runs as they were last listed, and when.
    checked: Mutex<HashMap<(String, u64), Checked>>,
    /// What changed in each pull request's files, as far as it was asked for.
    diffs: Mutex<HashMap<(String, u64), Kept>>,
    /// The checkouts that were kept at the last sweep though their pull request ended.
    left: Mutex<Vec<String>>,
}

/// Where a pull request is, asked once. A pull request is found by its number in its
/// organization, so the place it is asked from can be a clone of any repo there, or the
/// organization alone. Everything after that names its own project and repo.
#[derive(Clone)]
struct Known {
    /// Its place in Azure DevOps' REST paths.
    at: String,
    /// Its project's id: the paths of its builds and policies start with it.
    project_id: String,
    project: String,
    repo: String,
    /// Its address on the web. dova reads the organization, project and repo from it, so
    /// dova is given this and not the bare number.
    link: Option<String>,
    /// Where its repo is cloned from, over https and over ssh.
    https: Option<String>,
    ssh: Option<String>,
    /// The branch it merges from, as a ref. It is the same for as long as the pull request is.
    source: String,
}

/// A pull request's pipeline runs, and when they were listed.
type Checked = (Instant, Vec<CheckRun>);

/// A pull request as it was last read.
struct Held {
    lasting: Lasting,
    /// Its description as it was written, with no picture put in place.
    written: String,
    /// What was made of it all, and when.
    review: Review,
    read: Instant,
}

/// What is kept of a pull request from one read to the next. What changes while it is open
/// is asked each time: itself, its threads, its policies. Its files change only when it is
/// pushed to or aimed at another branch, which `mark` holds, so they are kept until then.
#[derive(Clone)]
struct Lasting {
    /// When the files and the work items were asked for.
    asked: Instant,
    mark: String,
    /// Its latest push, by its number.
    push: u64,
    /// What that push changes: the commits it is between and its files' names, as dova's
    /// diff has them.
    diff: Value,
    work_items: Vec<WorkItemRef>,
    /// Each text that shows a picture, with the picture where its address was.
    pictured: HashMap<String, String>,
}

impl Known {
    /// What dova is given to name the pull request.
    fn link(&self, pr: u64) -> String {
        self.link.clone().unwrap_or_else(|| pr.to_string())
    }
}

/// The folders a pull request's checkout has to do with.
pub struct Folders<'a> {
    /// The workbench's own folder: checkouts are in `pulls` there.
    pub own: &'a Path,
    /// Where checkouts were made before: `reviews` in wbd's data.
    pub data: &'a Path,
    /// The clones in your repos folder.
    pub found: &'a [Found],
}

/// A pull request's checkout, as the list of them in the workbench's own folder has it.
#[derive(Clone, PartialEq, serde::Deserialize, serde::Serialize)]
struct Checkout {
    folder: String,
    /// The clone it was made from, and is a worktree of.
    clone: String,
    pr: u64,
}

/// The checkouts on the list in `own`.
fn checkouts(own: &Path) -> Vec<Checkout> {
    std::fs::read_to_string(own.join("pulls.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn keep_checkouts(own: &Path, list: &[Checkout]) {
    let text = serde_json::to_string_pretty(list).expect("checkouts serialize");
    if std::fs::create_dir_all(own).is_ok() {
        let _ = files::write_whole(&own.join("pulls.json"), &text, None);
    }
}

/// The files of a pull request that were asked for, each as dova gave it, by its path. They
/// are of one push: after another, none of them is kept.
#[derive(Default)]
struct Kept {
    base: Value,
    head: Value,
    files: HashMap<String, Value>,
}

impl Kept {
    /// Keeps nothing of another push than the one dova now answers about.
    fn of(&mut self, diff: &Value) {
        if self.head != diff["head"] || self.base != diff["base"] {
            *self = Kept {
                base: diff["base"].clone(),
                head: diff["head"].clone(),
                files: HashMap::new(),
            };
        }
    }
}

impl Reviews {
    /// A pull request to review.
    pub async fn open(&self, dova: &Dova, dir: &str, pr: u64) -> Result<Review, Error> {
        let known = self.known(dova, dir, pr).await?;
        let at = &known.at;
        let key = (dir.to_string(), pr);
        let before = {
            let held = self.held.lock().expect("not poisoned");
            held.get(&key).map(|held| held.lasting.clone())
        };
        // What changes while a pull request is open is asked each time: itself, its threads
        // and its policies. Three requests. The policies are asked for by the project's own
        // id, and not being able to ask counts as not met. The threads are asked for as they
        // are after its latest push, so the first time they wait until that is known.
        let evaluations = format!(
            "/{0}/_apis/policy/evaluations?artifactId=vstfs:///CodeReview/CodeReviewId/{0}/{pr}&api-version=7.1-preview.1",
            known.project_id
        );
        let placed = before.as_ref().map(|kept| kept.push);
        let (raw, policies, me, threads) = tokio::join!(
            self.raw(dova, dir, pr),
            get(dova, dir, evaluations),
            self.me(dova, dir),
            async {
                match placed {
                    Some(push) => Some(get(dova, dir, threads_of(at, push)).await),
                    None => None,
                }
            },
        );
        let (raw, me, mut threads) = (raw?, me?, threads);

        // Its pushes, its files and its work items are asked for only when it was pushed to
        // or aimed at another branch, or they are old.
        let mark = mark(&raw);
        let same = before
            .as_ref()
            .filter(|kept| kept.mark == mark && kept.asked.elapsed() < WHOLE);
        let (asked, push, diff, work_items) = match same {
            Some(kept) => (
                kept.asked,
                kept.push,
                kept.diff.clone(),
                kept.work_items.clone(),
            ),
            None => {
                let pushed = async {
                    let pushes = format!("{at}/iterations?api-version=7.1");
                    let (push, mut diff) =
                        latest(&get(dova, dir, pushes).await?).ok_or_else(|| {
                            Error::Dova(format!("pull request {pr} has nothing pushed to it"))
                        })?;
                    // Threads that were placed for an earlier push are asked for again.
                    let moved = async {
                        match placed == Some(push) {
                            true => None,
                            false => Some(get(dova, dir, threads_of(at, push)).await),
                        }
                    };
                    let (files, moved) = tokio::join!(changes(dova, dir, at, push), moved);
                    diff["files"] = files?;
                    Ok::<_, Error>((push, diff, moved))
                };
                let (pushed, items) = tokio::join!(pushed, work_items(dova, dir, at));
                let (push, diff, moved) = pushed?;
                threads = moved.or(threads);
                (Instant::now(), push, diff, items?)
            }
        };
        let Some(threads) = threads else {
            return Err(Error::Dova(format!(
                "pull request {pr}'s threads were not asked for"
            )));
        };
        let threads = threads?;

        // A text that shows a picture is asked for with the picture once, and kept with it:
        // each picture is a request of its own.
        let written = text(&raw["description"]);
        let kept = before.map(|kept| kept.pictured).unwrap_or_default();
        let lacks = |text: &str| pictured(text) && !kept.contains_key(text);
        let said = words(&threads);
        let (described, spoken) = tokio::join!(
            async {
                match lacks(&written) {
                    true => get_pictured(dova, dir, format!("{at}?api-version=7.1"))
                        .await
                        .ok(),
                    false => None,
                }
            },
            async {
                match said.values().any(|text| lacks(text)) {
                    true => get_pictured(dova, dir, threads_of(at, push)).await.ok(),
                    false => None,
                }
            },
        );
        let spoken = spoken.as_ref().map(words);
        let mut shown: HashMap<String, String> = HashMap::new();
        if pictured(&written) {
            let with = described.map(|raw| text(&raw["description"]));
            if let Some(with) = with.or_else(|| kept.get(&written).cloned()) {
                shown.insert(written.clone(), with);
            }
        }
        for (ids, plain) in said.iter().filter(|(_, plain)| pictured(plain)) {
            let with = spoken.as_ref().and_then(|spoken| spoken.get(ids));
            let with = with.map(|with| with.to_string());
            if let Some(with) = with.or_else(|| kept.get(*plain).cloned()) {
                shown.insert(plain.to_string(), with);
            }
        }

        let changed: HashSet<&str> = diff["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|file| file["path"].as_str())
            .collect();
        let reviewers = raw["reviewers"].as_array().into_iter().flatten();
        let vote = reviewers
            .clone()
            .find(|reviewer| reviewer["id"].as_str() == Some(me.as_str()))
            .map_or(0, vote_of);
        let branch = |name: &str| {
            text(&raw[name])
                .trim_start_matches("refs/heads/")
                .to_string()
        };
        let review = Review {
            pr: PullRequest {
                id: pr,
                title: text(&raw["title"]),
                status: raw["status"].as_str().unwrap_or("unknown").to_string(),
                is_draft: raw["isDraft"].as_bool() == Some(true),
                source_branch: branch("sourceRefName"),
                target_branch: branch("targetRefName"),
                project: known.project.clone(),
                repo: known.repo.clone(),
                url: known.link.clone().unwrap_or_default(),
            },
            author: text(&raw["createdBy"]["displayName"]),
            description: shown.get(&written).cloned().unwrap_or(written.clone()),
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
            work_items: work_items.clone(),
            threads: threads["value"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|raw| thread(raw, &changed, &shown))
                .collect(),
            files: self.listed(dir, pr, &diff),
            base: text(&diff["base"]),
            head: text(&diff["head"]),
        };
        let lasting = Lasting {
            asked,
            mark,
            push,
            diff,
            work_items,
            pictured: shown,
        };
        let held = Held {
            lasting,
            written,
            review: review.clone(),
            read: Instant::now(),
        };
        self.held.lock().expect("not poisoned").insert(key, held);
        Ok(review)
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
        let known = self.known(dova, dir, pr).await?;
        let branches = [known.source.clone(), format!("refs/pull/{pr}/merge")];
        let mut runs: Vec<CheckRun> = Vec::new();
        for branch in branches {
            let path = format!(
                "/{}/_apis/build/builds?branchName={}&$top=10&queryOrder=queueTimeDescending&api-version=7.1",
                known.project_id,
                encoded(&branch)
            );
            let listed = get(dova, dir, path).await?;
            runs.extend(listed["value"].as_array().into_iter().flatten().map(check));
        }
        runs.sort_by_key(|run| std::cmp::Reverse(run.id));
        // A run is asked about later by its id alone, and its path starts with its project.
        let mut projects = self.runs.lock().expect("not poisoned");
        for run in &runs {
            projects.insert(run.id, known.project_id.clone());
        }
        drop(projects);
        self.checked
            .lock()
            .expect("not poisoned")
            .insert((dir.to_string(), pr), (Instant::now(), runs.clone()));
        Ok(runs)
    }

    /// A pull request's project and repo, by their names.
    async fn named(&self, dova: &Dova, dir: &str, pr: u64) -> Result<(String, String), Error> {
        let known = self.known(dova, dir, pr).await?;
        Ok((known.project, known.repo))
    }

    /// A pull request as it was read a moment ago, or read now.
    async fn reviewed(&self, dova: &Dova, dir: &str, pr: u64) -> Result<Review, Error> {
        let key = (dir.to_string(), pr);
        let review = {
            let held = self.held.lock().expect("not poisoned");
            let lately = held.get(&key).filter(|held| held.read.elapsed() < FRESH);
            lately.map(|held| held.review.clone())
        };
        match review {
            Some(review) => Ok(review),
            None => self.open(dova, dir, pr).await,
        }
    }

    /// A pull request's pipeline runs as they were listed a moment ago, or listed now. None
    /// when they can't be listed.
    async fn checked(&self, dova: &Dova, dir: &str, pr: u64) -> Vec<CheckRun> {
        let key = (dir.to_string(), pr);
        let runs = {
            let checked = self.checked.lock().expect("not poisoned");
            let lately = checked.get(&key).filter(|(when, _)| when.elapsed() < FRESH);
            lately.map(|(_, runs)| runs.clone())
        };
        match runs {
            Some(runs) => runs,
            None => self.checks(dova, dir, pr).await.unwrap_or_default(),
        }
    }

    /// What dova is given to name a pull request: its address, which says where it is. Its
    /// bare number when the address is not known, which works from a clone in its organization.
    pub async fn handle(&self, dova: &Dova, dir: &str, pr: u64) -> String {
        match self.known(dova, dir, pr).await {
            Ok(known) => known.link(pr),
            Err(_) => pr.to_string(),
        }
    }

    /// The start of a pipeline run's REST path: its project, where a listing said it.
    fn run_at(&self, run: u64) -> String {
        match self.runs.lock().expect("not poisoned").get(&run) {
            Some(project) => format!("/{project}/_apis/build/builds/{run}"),
            None => format!("_apis/build/builds/{run}"),
        }
    }

    /// A run's jobs, each followed by its steps, and what the pipeline said
    /// went wrong outside any of them.
    pub async fn steps(
        &self,
        dova: &Dova,
        dir: &str,
        run: u64,
    ) -> Result<(Vec<Step>, Vec<String>), Error> {
        let path = format!("{}/timeline?api-version=7.1", self.run_at(run));
        Ok(read_timeline(&get(dova, dir, path).await?))
    }

    /// What one step of a run printed: the end of it, without each line's time.
    pub async fn step_log(
        &self,
        dova: &Dova,
        dir: &str,
        run: u64,
        log: u64,
    ) -> Result<String, Error> {
        let path = format!("{}/logs/{log}?api-version=7.1", self.run_at(run));
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
    pub async fn rerun(&self, dova: &Dova, dir: &str, run: u64) -> Result<(), Error> {
        let path = format!("{}?retry=true&api-version=7.1", self.run_at(run));
        send(dova, dir, "PATCH", path, &json!({})).await
    }

    /// Where the agent's folder for a pull request is, or will be made, and whether it is a
    /// checkout: whether a clone of the repo is here to make one from. Asking makes nothing.
    pub async fn place(
        &self,
        dova: &Dova,
        at: &Folders<'_>,
        dir: &str,
        pr: u64,
    ) -> Result<(PathBuf, bool), Error> {
        let (project, repo) = self.named(dova, dir, pr).await?;
        let clone = clone_of(dir, &project, &repo, at.found).await;
        Ok(folder(at, clone.as_deref(), &repo, pr).await)
    }

    /// Makes the agent's folder for a pull request if it is not there, and gives it with
    /// what to tell the agent about the pull request.
    ///
    /// With a clone of the repo, the folder is a worktree of it at the pull request's last
    /// commit, so the agent has all the code. The clone is the folder asked from when it is
    /// that repo's, or else the repo's clone in your repos folder. Without one, `clone_it`
    /// clones the repo into `repos` first. Otherwise the folder holds nothing, and the agent
    /// reads what the pull request changes through dova.
    pub async fn session(
        &self,
        dova: &Dova,
        at: &Folders<'_>,
        repos: &Path,
        dir: &str,
        pr: u64,
        clone_it: bool,
    ) -> Result<(String, String), Error> {
        let review = self.reviewed(dova, dir, pr).await?;
        let known = self.known(dova, dir, pr).await?;
        let mut clone = clone_of(dir, &known.project, &known.repo, at.found).await;
        if clone.is_none() && clone_it {
            clone = Some(clone_into(repos, &known, at.found).await?);
        }
        let (own, _) = folder(at, clone.as_deref(), &known.repo, pr).await;
        let made = match &clone {
            Some(clone) => {
                checkout(clone, &own, &review.pr.source_branch, &review.head).await?;
                clone.display().to_string()
            }
            None => {
                std::fs::create_dir_all(&own).map_err(|err| Error::io(&own, err))?;
                // Where it is asked about later, to remove it when the pull request ends.
                dir.to_string()
            }
        };
        let made = Checkout {
            folder: own.display().to_string(),
            clone: made,
            pr,
        };
        let mut list = checkouts(at.own);
        if !list.contains(&made) {
            list.retain(|known| known.folder != made.folder);
            list.push(made);
            keep_checkouts(at.own, &list);
        }
        let runs = self.checked(dova, dir, pr).await;
        let link = clone.is_none().then(|| known.link(pr));
        Ok((
            own.display().to_string(),
            brief(&review, &runs, link.as_deref()),
        ))
    }

    /// Removes the checkouts whose pull request was completed or abandoned. One that holds
    /// work that was not committed, or commits that are not in the pull request, is kept.
    pub async fn sweep(&self, dova: &Dova, own: &Path, data: &Path) {
        let mut list = checkouts(own);
        let listed = list.clone();
        // The ones made before there was a list are put on it.
        for folder in std::fs::read_dir(data.join("reviews"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let folder = folder.path();
            let known = list.iter().any(|one| Path::new(&one.folder) == folder);
            if known || !folder.join(".git").is_file() {
                continue;
            }
            let name = folder.file_name().unwrap_or_default().to_string_lossy();
            let pr = name.rsplit('-').next().and_then(|n| n.parse().ok());
            let common = ["rev-parse", "--path-format=absolute", "--git-common-dir"];
            let clone = run(&folder, &common, None).await.ok().and_then(|out| {
                let git = PathBuf::from(String::from_utf8_lossy(&out).trim());
                git.parent().map(|clone| clone.display().to_string())
            });
            if let (Some(pr), Some(clone)) = (pr, clone) {
                list.push(Checkout {
                    folder: folder.display().to_string(),
                    clone,
                    pr,
                });
            }
        }

        let mut left = Vec::new();
        for one in list.clone() {
            let (folder, clone) = (Path::new(&one.folder), Path::new(&one.clone));
            let forget = |list: &mut Vec<Checkout>| list.retain(|known| *known != one);
            // A folder made without a clone holds nothing of the repo: only what the agent left.
            let plain = one.folder.ends_with(PLAIN);
            if plain && !folder.is_dir() {
                forget(&mut list);
                continue;
            }
            // One that was deleted by hand is taken off git's list and off this one.
            if !plain && !folder.join(".git").exists() {
                let _ = run(clone, &["worktree", "prune"], None).await;
                forget(&mut list);
                continue;
            }
            let path = format!("/_apis/git/pullrequests/{}?api-version=7.1", one.pr);
            // Not being able to ask is no reason to remove anything.
            let Ok(raw) = dova.call("api", &one.clone, json!({ "path": path })).await else {
                continue;
            };
            if !matches!(raw["status"].as_str(), Some("completed" | "abandoned")) {
                continue;
            }
            if plain {
                // Only a folder the workbench made itself, in its own place, is deleted.
                if folder.starts_with(own.join("pulls")) && std::fs::remove_dir_all(folder).is_ok()
                {
                    forget(&mut list);
                }
                continue;
            }
            let last = text(&raw["lastMergeSourceCommit"]["commitId"]);
            let clean = run(folder, &["status", "--porcelain"], None)
                .await
                .is_ok_and(|changes| changes.is_empty());
            // Every commit in it was in the pull request, or is on some branch of the remote.
            let in_pull = ["merge-base", "--is-ancestor", "HEAD", &last];
            let pushed = run(folder, &in_pull, None).await.is_ok()
                || run(folder, &["branch", "-r", "--contains", "HEAD"], None)
                    .await
                    .is_ok_and(|branches| !branches.is_empty());
            let remove = ["worktree", "remove", &one.folder];
            if clean && pushed && run(clone, &remove, None).await.is_ok() {
                forget(&mut list);
            } else {
                left.push(one.folder.clone());
            }
        }
        if list != listed {
            keep_checkouts(own, &list);
        }
        *self.left.lock().expect("not poisoned") = left;
    }

    /// Every pull request checkout there is, and the ones the last sweep kept though their
    /// pull request ended.
    pub fn kept(&self, own: &Path) -> (Vec<String>, Vec<String>) {
        let all = checkouts(own).into_iter().map(|one| one.folder).collect();
        (all, self.left.lock().expect("not poisoned").clone())
    }

    /// A pull request's description as it was written: with its pictures put in place, it
    /// could not be written back. What was read a moment ago is used.
    pub async fn description(&self, dova: &Dova, dir: &str, pr: u64) -> Result<String, Error> {
        let lately = {
            let held = self.held.lock().expect("not poisoned");
            let lately = held
                .get(&(dir.to_string(), pr))
                .filter(|held| held.read.elapsed() < FRESH);
            lately.map(|held| held.written.clone())
        };
        match lately {
            Some(written) => Ok(written),
            None => Ok(text(&self.raw(dova, dir, pr).await?["description"])),
        }
    }

    /// A pull request's files, from dova's list of their names. The ones that were asked
    /// for since its last push come with what changed in them.
    fn listed(&self, dir: &str, pr: u64, names: &Value) -> Vec<ChangedFile> {
        let mut diffs = self.diffs.lock().expect("not poisoned");
        let kept = diffs.entry((dir.to_string(), pr)).or_default();
        kept.of(names);
        let head = text(&names["head"]);
        let files = names["files"].as_array().into_iter().flatten();
        files
            .map(|named| match kept.files.get(&text(&named["path"])) {
                Some(file) => changed(file, &head, true),
                None => changed(named, &head, false),
            })
            .collect()
    }

    /// What changed in some of a pull request's files: one call to dova for them all. It is
    /// kept, so the pull request comes with it from then on, until its next push.
    pub async fn files(
        &self,
        dova: &Dova,
        dir: &str,
        pr: u64,
        paths: &[String],
    ) -> Result<Vec<ChangedFile>, Error> {
        let named: Vec<String> = paths.iter().map(|path| format!("/{path}")).collect();
        let asked = json!({ "pr": self.handle(dova, dir, pr).await, "files": named });
        let mut diff = dova.call("pr.diff", dir, asked).await?;
        let head = text(&diff["head"]);
        let mut diffs = self.diffs.lock().expect("not poisoned");
        let kept = diffs.entry((dir.to_string(), pr)).or_default();
        kept.of(&diff);
        let files = diff["files"].as_array_mut().into_iter().flatten();
        Ok(files
            .map(|file| {
                // Asked for by name, a file comes with its whole text before and after. Only
                // its changes are kept.
                if let Some(file) = file.as_object_mut() {
                    file.remove("old");
                    file.remove("new");
                }
                let changed = changed(file, &head, true);
                kept.files.insert(text(&file["path"]), file.take());
                changed
            })
            .collect())
    }

    /// Where a pull request is in Azure DevOps' REST paths. A pull request
    /// is asked for by number in its project, and written to in its repo.
    async fn at(&self, dova: &Dova, dir: &str, pr: u64) -> Result<String, Error> {
        Ok(self.known(dova, dir, pr).await?.at)
    }

    /// Where a pull request is: asked of its organization by number, once.
    async fn known(&self, dova: &Dova, dir: &str, pr: u64) -> Result<Known, Error> {
        let key = (dir.to_string(), pr);
        if let Some(known) = self.known.lock().expect("not poisoned").get(&key) {
            return Ok(known.clone());
        }
        // A path that starts with a slash is asked of the organization, not of a project.
        let path = format!("/_apis/git/pullrequests/{pr}?api-version=7.1");
        let raw = get(dova, dir, path).await?;
        let repo = &raw["repository"];
        let (Some(id), Some(project_id)) = (repo["id"].as_str(), repo["project"]["id"].as_str())
        else {
            return Err(Error::Dova(format!(
                "Azure DevOps didn't say which repo pull request {pr} is in"
            )));
        };
        let web = repo["webUrl"].as_str();
        let known = Known {
            at: format!("/{project_id}/_apis/git/repositories/{id}/pullRequests/{pr}"),
            project_id: project_id.to_string(),
            project: text(&repo["project"]["name"]),
            repo: text(&repo["name"]),
            link: web.map(|web| format!("{web}/pullrequest/{pr}")),
            https: repo["remoteUrl"].as_str().map(str::to_string),
            ssh: repo["sshUrl"].as_str().map(str::to_string),
            source: text(&raw["sourceRefName"]),
        };
        self.known
            .lock()
            .expect("not poisoned")
            .insert(key.clone(), known.clone());
        // It was read whole to find it. Reading it right after uses this.
        self.seen
            .lock()
            .expect("not poisoned")
            .insert(key, (Instant::now(), raw));
        Ok(known)
    }

    /// A pull request as Azure DevOps has it now: asked of its organization by its number.
    /// When it was read a moment ago to find where it is, that is used, once.
    async fn raw(&self, dova: &Dova, dir: &str, pr: u64) -> Result<Value, Error> {
        let seen = self
            .seen
            .lock()
            .expect("not poisoned")
            .remove(&(dir.to_string(), pr));
        match seen {
            Some((when, raw)) if when.elapsed() < JUST => Ok(raw),
            _ => {
                get(
                    dova,
                    dir,
                    format!("/_apis/git/pullrequests/{pr}?api-version=7.1"),
                )
                .await
            }
        }
    }

    /// Who you are to the organization `dir` is in. Asked once for each organization.
    pub async fn me(&self, dova: &Dova, dir: &str) -> Result<String, Error> {
        let org = self.org(dir).await;
        if let Some(known) = self.me.lock().expect("not poisoned").get(&org) {
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
            .insert(org, id.clone());
        Ok(id)
    }

    /// The organization a place is in, as its own place: `@name`. A folder's is read from
    /// where it is cloned from, once. The folder itself when that is not Azure DevOps.
    async fn org(&self, dir: &str) -> String {
        if dir.starts_with('@') {
            return dir.to_lowercase();
        }
        if let Some(org) = self.orgs.lock().expect("not poisoned").get(dir) {
            return org.clone();
        }
        let remote = git::origin(Path::new(dir)).await;
        let org = match remote.as_deref().and_then(pulls::azure) {
            Some((project, _)) => pulls::whole(&project).to_lowercase(),
            None => dir.to_string(),
        };
        self.orgs
            .lock()
            .expect("not poisoned")
            .insert(dir.to_string(), org.clone());
        org
    }
}

/// Where the agent's folder for a pull request is, and whether it is a checkout. With a
/// clone of the repo it is a checkout, in `pulls` in the workbench's own folder. Without
/// one it is a folder beside those, named apart, so a checkout can still be made later.
async fn folder(at: &Folders<'_>, clone: Option<&Path>, repo: &str, pr: u64) -> (PathBuf, bool) {
    let pulls = at.own.join("pulls");
    let own = pulls.join(format!("{}-{pr}", slug(repo)));
    // One made before checkouts were kept in the workbench's own folder is still used:
    // the conversations in it are tied to where it is.
    if let Some(clone) = clone {
        let origin = git::origin(clone).await.unwrap_or_default();
        let old = at
            .data
            .join("reviews")
            .join(format!("{}-{pr}", slug(&origin)));
        if old.join(".git").exists() {
            return (old, true);
        }
    }
    match clone.is_some() || own.join(".git").exists() {
        true => (own, true),
        false => (pulls.join(format!("{}-{pr}{PLAIN}", slug(repo))), false),
    }
}

/// Clones a pull request's repo into your repos folder, the way your other clones from
/// Azure Repos were cloned: over ssh if one of them was, or else over https. So git signs in
/// the way that already works for you.
async fn clone_into(repos: &Path, known: &Known, found: &[Found]) -> Result<PathBuf, Error> {
    let target = repos.join(&known.repo);
    if target.exists() {
        return Err(Error::Conflict(format!(
            "{} is there already and is not a clone of the {} repo. Move it, or clone the repo yourself under another name.",
            target.display(),
            known.repo
        )));
    }
    let over_ssh = found.iter().any(|clone| {
        pulls::azure(&clone.remote).is_some()
            && (clone.remote.starts_with("git@") || clone.remote.starts_with("ssh://"))
    });
    let (first, second) = match over_ssh {
        true => (&known.ssh, &known.https),
        false => (&known.https, &known.ssh),
    };
    let url = first.as_ref().or(second.as_ref()).ok_or_else(|| {
        Error::Dova(format!(
            "Azure DevOps didn't say where the {} repo is cloned from",
            known.repo
        ))
    })?;
    std::fs::create_dir_all(repos).map_err(|err| Error::io(repos, err))?;
    let to = target.display().to_string();
    // A large repo takes minutes, so this is not given up on.
    run(repos, &["clone", "--quiet", url, &to], None)
        .await
        .map_err(|why| Error::Io(format!("git couldn't clone {}: {why}", known.repo)))?;
    Ok(target)
}

/// What a pull request's files depend on: its last commit and the branch it is aimed at.
/// While this is as it was, its pushes and its files are not asked for again.
fn mark(raw: &Value) -> String {
    format!(
        "{} {}",
        raw["lastMergeSourceCommit"]["commitId"], raw["targetRefName"]
    )
}

/// Where a pull request's threads are asked for: each as it is after `push`, compared with
/// the target. Asked plainly, Azure DevOps gives the lines a thread was written on, which
/// later pushes move or remove.
fn threads_of(at: &str, push: u64) -> String {
    format!("{at}/threads?$iteration={push}&$baseIteration=0&api-version=7.1")
}

/// A pull request's latest push: its number, and the commits its diff is between, named as
/// dova's diff names them.
fn latest(pushes: &Value) -> Option<(u64, Value)> {
    let latest = pushes["value"].as_array()?.last()?;
    let commit = |name: &str| text(&latest[name]["commitId"]);
    let between = json!({ "base": commit("commonRefCommit"), "head": commit("sourceRefCommit") });
    Some((latest["id"].as_u64()?, between))
}

/// Every file a push of a pull request changes, against where its branch left the target:
/// its name and how it changed, as dova's diff lists them.
async fn changes(dova: &Dova, dir: &str, at: &str, push: u64) -> Result<Value, Error> {
    let (mut files, mut skip) = (Vec::new(), 0);
    loop {
        let path = format!(
            "{at}/iterations/{push}/changes?$compareTo=0&$top={PAGE}&$skip={skip}&api-version=7.1"
        );
        let page = get(dova, dir, path).await?;
        let entries = page["changeEntries"].as_array().into_iter().flatten();
        files.extend(entries.filter_map(change));
        match page["nextSkip"].as_u64() {
            Some(next) if next > skip => skip = next,
            _ => return Ok(Value::Array(files)),
        }
    }
}

/// One file of a push's changes. None for what is not a file.
fn change(raw: &Value) -> Option<Value> {
    let item = &raw["item"];
    if item["isFolder"] == true || item["gitObjectType"] == "tree" {
        return None;
    }
    let kinds = raw["changeType"].as_str().unwrap_or_default();
    // Only a file that moved, or went, has its old path spelled out.
    let (path, moved) = (item["path"].as_str(), raw["originalPath"].as_str());
    Some(if kinds.contains("delete") {
        json!({ "path": moved.or(path), "change": "deleted" })
    } else if kinds.contains("add") {
        json!({ "path": path, "change": "added" })
    } else if kinds.contains("rename") {
        json!({ "path": path, "oldPath": moved.or(path), "change": "renamed" })
    } else {
        json!({ "path": path, "change": "edited" })
    })
}

/// The work items linked to a pull request, each with its title, type and state. One
/// request when there are none, two otherwise.
async fn work_items(dova: &Dova, dir: &str, at: &str) -> Result<Vec<WorkItemRef>, Error> {
    let linked = get(dova, dir, format!("{at}/workitems?api-version=7.1")).await?;
    let mut ids: Vec<u64> = Vec::new();
    for item in linked["value"].as_array().into_iter().flatten() {
        let id = &item["id"];
        let id = id.as_u64().or_else(|| id.as_str()?.parse().ok());
        ids.extend(id.filter(|id| !ids.contains(id)));
    }
    let mut items = Vec::new();
    for some in ids.chunks(ITEMS) {
        let some: Vec<String> = some.iter().map(u64::to_string).collect();
        // A path that starts with a slash is asked of the organization, not of a project.
        let path = format!(
            "/_apis/wit/workitems?ids={}&fields=System.Title,System.WorkItemType,System.State&errorPolicy=omit&api-version=7.1",
            some.join(",")
        );
        let found = get(dova, dir, path).await?;
        // One that can't be read comes as nothing in its place.
        let found = found["value"].as_array().into_iter().flatten();
        items.extend(found.filter_map(|item| {
            let field = |name: &str| item["fields"][name].as_str().map(str::to_string);
            Some(WorkItemRef {
                id: item["id"].as_u64()?,
                title: field("System.Title"),
                kind: field("System.WorkItemType"),
                state: field("System.State"),
            })
        }));
    }
    Ok(items)
}

/// Whether a text may show a picture. Only such a text is asked for with its pictures.
fn pictured(text: &str) -> bool {
    text.contains("![") || text.contains("<img")
}

/// What each comment of some threads says, by its thread's id and its own.
fn words(threads: &Value) -> HashMap<(u64, u64), &str> {
    let mut said = HashMap::new();
    for thread in threads["value"].as_array().into_iter().flatten() {
        for comment in thread["comments"].as_array().into_iter().flatten() {
            let ids = (thread["id"].as_u64(), comment["id"].as_u64());
            if let ((Some(of), Some(id)), Some(text)) = (ids, comment["content"].as_str()) {
                said.insert((of, id), text);
            }
        }
    }
    said
}

/// The clone a pull request's checkout is made from: the folder asked from when it is that
/// repo's, or else a clone of the repo found in your repos folder.
async fn clone_of(dir: &str, project: &str, repo: &str, found: &[Found]) -> Option<PathBuf> {
    let same = |remote: &str| {
        pulls::azure(remote).is_some_and(|(of, name)| {
            name.eq_ignore_ascii_case(repo)
                && pulls::project_name(&of).eq_ignore_ascii_case(project)
        })
    };
    if git::origin(Path::new(dir))
        .await
        .as_deref()
        .is_some_and(same)
    {
        return Some(PathBuf::from(dir));
    }
    let clone = found.iter().find(|clone| same(&clone.remote))?;
    Some(PathBuf::from(&clone.path))
}

/// One changed file's diff with every line of the file.
pub async fn diff(dova: &Dova, dir: &str, pr: &str, path: &str) -> Result<Vec<DiffLine>, Error> {
    let params = json!({ "pr": pr, "files": [format!("/{path}")], "context": WHOLE_FILE });
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
    pr: &str,
    words: &str,
    place: Option<Place>,
) -> Result<(), Error> {
    let mut params = json!({ "pr": pr, "text": words });
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
    pr: &str,
    title: Option<String>,
    description: Option<String>,
) -> Result<(), Error> {
    let mut params = json!({ "idOrUrl": pr });
    if let Some(title) = title {
        params["title"] = json!(title);
    }
    if let Some(description) = description {
        params["body"] = json!(description);
    }
    dova.call("pr.edit", dir, params).await.map(|_| ())
}

async fn get(dova: &Dova, dir: &str, path: String) -> Result<Value, Error> {
    dova.call("api", dir, json!({ "path": path })).await
}

/// The same, with each picture that a text of the answer shows put where its address was.
/// Each picture is a request more, so this is asked only for a text that was not seen yet.
async fn get_pictured(dova: &Dova, dir: &str, path: String) -> Result<Value, Error> {
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

/// Where a thread is, from what Azure DevOps says of it when asked as of the latest push.
/// None for one about the whole pull request. `changed` is the files that push changes.
fn place(context: &Value, changed: &HashSet<&str>) -> Option<Place> {
    let file = context["filePath"].as_str()?;
    // On the pull request's side of the file when it is on both. On neither when it is
    // about the whole file.
    let side = ["right", "left"]
        .into_iter()
        .find(|side| context[format!("{side}FileStart")]["line"].is_u64());
    // Line 0 is where Azure DevOps puts a thread whose lines a later push removed.
    let line = |end: &str| {
        let line = context[format!("{}File{end}", side?)]["line"].as_u64();
        line.filter(|line| *line > 0).map(|line| line as u32)
    };
    // A thread on a file the pull request does not change, as it is now, is on no line of
    // it: the file was deleted again, or put back as it was. Azure DevOps keeps such a
    // thread where it was written.
    let still = changed.contains(file);
    let from = line("Start").filter(|_| still);
    Some(Place {
        file: file.trim_start_matches('/').to_string(),
        from,
        line: from.map(|from| line("End").filter(|last| *last >= from).unwrap_or(from)),
        old: side == Some("left"),
        outdated: !still || (side.is_some() && from.is_none()),
    })
}

/// A thread people wrote in, placed where it is after the latest push. None for one Azure
/// DevOps wrote itself, such as "added a reviewer", and for a deleted one. `shown` has the
/// texts that show a picture, each with the picture in place.
fn thread(raw: &Value, changed: &HashSet<&str>, shown: &HashMap<String, String>) -> Option<Thread> {
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
        .map(|comment| {
            let said = text(&comment["content"]);
            Comment {
                id: comment["id"].as_u64().unwrap_or_default(),
                author: comment["author"]["displayName"]
                    .as_str()
                    .map(str::to_string),
                published_at: comment["publishedDate"].as_str().map(str::to_string),
                text: shown.get(&said).cloned().unwrap_or(said),
            }
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
        place: place(&raw["threadContext"], changed),
        comments,
    })
}

/// A changed file, from dova's diff. Not `loaded`, it is from the list of names alone.
fn changed(file: &Value, head: &str, loaded: bool) -> ChangedFile {
    let mut lines = lines(file);
    let count = |key: &str| file[key].as_u64().unwrap_or_default() as u32;
    let mut hasher = DefaultHasher::new();
    match lines.is_empty() {
        // Nothing to tell one change from another by: any push counts as one.
        true => head.hash(&mut hasher),
        false => lines
            .iter()
            .for_each(|line| (line.kind as u8, &line.text).hash(&mut hasher)),
    }
    let large = lines.len() > LARGE || file["large"].as_bool() == Some(true);
    if large {
        lines.clear();
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
        // A file that was not asked for yet has no mark: what is in it is not known.
        mark: match loaded {
            true => format!("{:016x}", hasher.finish()),
            false => String::new(),
        },
        binary: file["binary"].as_bool() == Some(true),
        large,
        lines,
        loaded,
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

/// What the agent is told about a pull request before anything you write. With `link`, the
/// pull request's address, the agent has no checkout and is told to read through dova.
fn brief(review: &Review, runs: &[CheckRun], link: Option<&str>) -> String {
    let pr = &review.pr;
    let short = |commit: &str| commit.get(..9).unwrap_or(commit).to_string();
    let clip = |words: &str, most: usize| match words.char_indices().nth(most) {
        Some((cut, _)) => format!("{} […]", &words[..cut]),
        None => words.to_string(),
    };
    let code = match link {
        Some(link) => format!(
            "The repository is not cloned on this machine, so this folder holds no code. \
             Read the code through dova, which is installed, and name the pull request by its address. \
             `dova pr diff {link}` shows what changed. \
             `dova pr diff {link} --files <path>...` gives those files whole, before and after, as JSON. \
             `dova pr view {link}` and `dova pr thread {link} <thread>` read the pull request as it is now. \
             You cannot read a file the pull request does not change. When an answer needs one, say so: \
             the person can clone the repository from the Workbench for a thorough review.\n"
        ),
        None => format!(
            "This folder is a separate checkout of the pull request's branch, at {head}, not on any branch. \
             The whole change is `git diff {base} {head}`. \
             To push a fix to the pull request: commit here, then `git push origin HEAD:refs/heads/{source}`.\n\
             dova is installed. `dova pr view {id}`, `dova pr diff {id}` and `dova pr thread {id} <thread>` read the pull request as it is now, \
             and `dova pipeline log <run>` reads why a run failed.\n",
            id = pr.id,
            source = pr.source_branch,
            head = short(&review.head),
            base = short(&review.base),
        ),
    };
    let mut out = format!(
        "You are helping someone review pull request #{id} \"{title}\" in the {repo} repository on Azure DevOps.\n\
         Opened by {author}. It merges {source} into {target}. It is {state}.\n\n\
         {code}\
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
    fn a_file_comes_by_its_name_alone_until_it_is_asked_for_and_then_until_the_next_push() {
        let names = |head: &str| {
            json!({ "base": "b", "head": head, "files": [
                { "path": "/a.txt", "oldPath": null, "change": "edited" },
                { "path": "/b.txt", "oldPath": null, "change": "added" },
            ] })
        };
        let reviews = Reviews::default();
        let listed = reviews.listed("dir", 7, &names("one"));
        assert_eq!(
            (listed[1].path.as_str(), listed[1].change, listed[1].loaded),
            ("b.txt", FileChange::Added, false)
        );
        assert!(listed[1].mark.is_empty() && listed[1].lines.is_empty());

        let asked = json!({ "path": "/a.txt", "change": "edited", "added": 1, "removed": 0,
            "hunks": [{ "lines": [{ "kind": "added", "old": null, "new": 1, "text": "x" }] }] });
        let mut diffs = reviews.diffs.lock().unwrap();
        let kept = diffs.get_mut(&("dir".to_string(), 7)).unwrap();
        kept.files.insert("/a.txt".into(), asked);
        drop(diffs);

        let listed = reviews.listed("dir", 7, &names("one"));
        assert_eq!(
            listed.iter().map(|file| file.loaded).collect::<Vec<_>>(),
            [true, false]
        );
        assert_eq!((listed[0].added, listed[0].lines.len()), (1, 1));
        let pushed = reviews.listed("dir", 7, &names("two"));
        assert!(pushed.iter().all(|file| !file.loaded));
    }

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
        let read = changed(&file, "abc", true);
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
        assert_eq!(read.mark, changed(&file, "another push", true).mark);
        let mut other = file.clone();
        other["hunks"][1]["lines"][0]["text"] = json!("changed again");
        assert_ne!(read.mark, changed(&other, "abc", true).mark);

        let renamed = json!({ "path": "/docs/to.md", "oldPath": "/docs/from.md", "change": "renamed", "binary": true });
        let read = changed(&renamed, "abc", true);
        assert_eq!(
            (read.old_path.as_deref(), read.change, read.binary),
            (Some("docs/from.md"), FileChange::Renamed, true)
        );
    }

    #[test]
    fn keeps_the_threads_people_wrote_and_places_them() {
        let (changed, shown) = (HashSet::from(["/src/auth.ts"]), HashMap::new());
        let system = json!({ "id": 19, "comments": [{ "id": 1, "commentType": "system", "content": "added a reviewer" }] });
        assert_eq!(thread(&system, &changed, &shown), None);

        let on = |file: &str, side: &str, from: u64, to: u64| json!({ "filePath": file, format!("{side}FileStart"): { "line": from, "offset": 1 }, format!("{side}FileEnd"): { "line": to, "offset": 9 } });
        let mut raw = json!({
            "id": 4, "status": "active", "threadContext": on("/src/auth.ts", "right", 42, 44),
            "comments": [{ "id": 1, "commentType": "text", "content": "Why? ![shot](a)", "author": { "displayName": "Sam Lee" }, "publishedDate": "2026-11-14T19:47:13Z" }],
        });
        let read = thread(&raw, &changed, &shown).unwrap();
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
        // A text that was read with its picture is shown with it.
        let shown = HashMap::from([(
            "Why? ![shot](a)".to_string(),
            "Why? ![shot](data:x)".to_string(),
        )]);
        let read = thread(&raw, &changed, &shown).unwrap();
        assert_eq!(read.comments[0].text, "Why? ![shot](data:x)");

        // A later push removed its lines: Azure DevOps puts it on line 0.
        let gone = place(&on("/src/auth.ts", "left", 0, 0), &changed).unwrap();
        assert!(gone.old && gone.outdated && gone.line.is_none());
        // The pull request no longer changes its file.
        let unchanged = place(&on("/a.rs", "right", 3, 3), &changed).unwrap();
        assert!(unchanged.outdated && unchanged.from.is_none());
        // About the whole file, and about the whole pull request.
        let whole = place(&json!({ "filePath": "/src/auth.ts" }), &changed).unwrap();
        assert!(!whole.outdated && whole.line.is_none());
        raw["threadContext"] = Value::Null;
        assert_eq!(thread(&raw, &changed, &shown).unwrap().place, None);
    }

    #[test]
    fn reads_a_push_and_what_it_changes() {
        let pushes = json!({ "value": [
            { "id": 1, "commonRefCommit": { "commitId": "a" }, "sourceRefCommit": { "commitId": "b" } },
            { "id": 2, "commonRefCommit": { "commitId": "a" }, "sourceRefCommit": { "commitId": "c" } },
        ] });
        assert_eq!(
            latest(&pushes),
            Some((2, json!({ "base": "a", "head": "c" })))
        );
        assert_eq!(latest(&json!({ "value": [] })), None);

        let entry = |kind: &str, path: &str, from: Option<&str>| json!({ "changeType": kind, "originalPath": from, "item": { "path": path, "gitObjectType": "blob" } });
        assert_eq!(
            change(&json!({ "changeType": "add", "item": { "path": "/src", "isFolder": true } })),
            None
        );
        assert_eq!(
            change(&entry("add", "/a", None)),
            Some(json!({ "path": "/a", "change": "added" }))
        );
        assert_eq!(
            change(&entry("edit", "/a", None)),
            Some(json!({ "path": "/a", "change": "edited" }))
        );
        assert_eq!(
            change(&entry("edit, rename", "/b", Some("/a"))),
            Some(json!({ "path": "/b", "oldPath": "/a", "change": "renamed" }))
        );
        assert_eq!(
            change(&entry("delete", "/a", None)),
            Some(json!({ "path": "/a", "change": "deleted" }))
        );

        let raw = |commit: &str, target: &str| json!({ "lastMergeSourceCommit": { "commitId": commit }, "targetRefName": target, "title": "x" });
        assert_eq!(
            mark(&raw("c", "refs/heads/main")),
            mark(&raw("c", "refs/heads/main"))
        );
        assert_ne!(
            mark(&raw("c", "refs/heads/main")),
            mark(&raw("d", "refs/heads/main"))
        );
        assert_ne!(
            mark(&raw("c", "refs/heads/main")),
            mark(&raw("c", "refs/heads/next"))
        );
        assert!(pictured("see ![a](b)") && pictured("<img src=x>") && !pictured("plain"));
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
