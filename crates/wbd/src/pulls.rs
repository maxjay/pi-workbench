//! Your pull requests, for Home: asked from dova, once for each Azure DevOps
//! project your workspaces are cloned from, with the reason under each one
//! that needs you. And the ones of other people that you chose to see, asked
//! from Azure DevOps itself: one call for each repo, project or person.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use protocol::{
    Asked, Person, PipelineRun, PullBefore, PullRequest, PullRow, PullView, RepoName, RepoPick,
    Reviewing, Team, ThreadSummary, WorkItemRef, Workspace,
};
use serde_json::{Value, json};

use crate::dova::Dova;
use crate::error::Error;
use crate::git;
use crate::review::Reviews;

#[derive(Default)]
pub struct Pulls {
    /// Why each failed run failed, by run id. A finished run doesn't change.
    failed: Mutex<HashMap<u64, String>>,
    /// The teams you are in, by organization. Asked once: they change rarely.
    teams: Mutex<HashMap<String, Vec<Mine>>>,
    /// Who is in each of those teams, by the team's id. Asked once.
    members: Mutex<HashMap<String, Vec<Person>>>,
    /// Every repo of an organization, by organization. Asked once: the search reads from it.
    repos: Mutex<HashMap<String, Vec<RepoName>>>,
}

/// A team you are in.
#[derive(Clone)]
struct Mine {
    id: String,
    name: String,
    project: String,
}

/// The most of each kind a search gives back.
const FOUND: usize = 8;

/// A workspace that is cloned from Azure Repos.
struct Clone {
    workspace: Workspace,
    /// Its remote without the repo's name: the same for every repo in a project.
    project: String,
    repo: String,
    branch: Option<String>,
}

/// The workspaces cloned from Azure Repos, and every folder that was looked at.
async fn clones(workspaces: Vec<Workspace>) -> (Vec<Clone>, Vec<Asked>) {
    let mut clones = Vec::new();
    let mut looked = Vec::new();
    for workspace in workspaces.into_iter().filter(|w| !w.scratch) {
        let dir = std::path::Path::new(&workspace.path);
        let remote = git::origin(dir).await;
        let on_azure = remote.as_deref().and_then(azure);
        looked.push(Asked {
            name: workspace.name.clone(),
            remote: remote.as_deref().map(without_sign_in),
            project: on_azure.as_ref().map(|(project, _)| project_name(project)),
        });
        let Some((project, repo)) = on_azure else {
            continue;
        };
        let branch = git::status(dir).await.map(|status| status.summary.branch);
        clones.push(Clone {
            workspace,
            project,
            repo,
            branch,
        });
    }
    (clones, looked)
}

/// One folder for each organization: one is enough to ask about the whole of it.
fn one_each(clones: &[Clone]) -> Vec<&Clone> {
    let mut organizations = HashSet::new();
    clones
        .iter()
        .filter(|clone| organizations.insert(organization(&clone.project).to_lowercase()))
        .collect()
}

impl Pulls {
    /// The pull requests you opened, as Azure DevOps' own list has them: one request to each
    /// organization a workspace is cloned from, whatever number of projects they are in. They
    /// come without a check and threads, which take requests for each one.
    pub async fn mine(
        &self,
        dova: &Dova,
        reviews: &Reviews,
        workspaces: Vec<Workspace>,
    ) -> Result<Vec<PullRow>, Error> {
        let (clones, _) = clones(workspaces).await;
        let mut rows = Vec::new();
        for clone in one_each(&clones) {
            let cwd = &clone.workspace.path;
            let me = reviews.me(dova, cwd).await?;
            let site = site(organization(&clone.project));
            // A path that starts with a slash is asked of the organization, not of the folder's project.
            let path = format!(
                "/_apis/git/pullrequests?searchCriteria.creatorId={me}&searchCriteria.status=active&$top=500&api-version=7.1"
            );
            let found = dova.call("api", cwd, json!({ "path": path })).await?;
            for pull in found["value"].as_array().into_iter().flatten() {
                let Some(mut row) = review_row(pull, &site, &me, &[]) else {
                    continue;
                };
                // The ones you opened are listed as yours, even where you are a reviewer too.
                row.review = None;
                row.workspace = home(&clones, &row);
                rows.push(row);
            }
        }
        Ok(rows)
    }

    /// The pull requests you opened, each with its check, its open threads and what it waits
    /// on, in the whole of each organization a workspace is cloned from: its other projects
    /// too. With them, every folder that was looked at and every project that was asked, so
    /// an empty list can say why. This takes requests for each pull request.
    pub async fn list(
        &self,
        dova: &Dova,
        reviews: &Reviews,
        workspaces: Vec<Workspace>,
    ) -> Result<(Vec<PullRow>, Vec<Asked>, Vec<String>), Error> {
        let (clones, looked) = clones(workspaces).await;
        let mut rows: Vec<PullRow> = Vec::new();
        let mut searched = Vec::new();
        for clone in one_each(&clones) {
            let cwd = &clone.workspace.path;
            // The projects of the folders open here, which must answer, and then every other
            // project that has a pull request of yours, found by asking the organization.
            let mut projects: Vec<(String, bool)> = Vec::new();
            for open in clones.iter().filter(|other| {
                organization(&other.project).eq_ignore_ascii_case(organization(&clone.project))
            }) {
                let name = project_name(&open.project);
                if !projects
                    .iter()
                    .any(|(known, _)| known.eq_ignore_ascii_case(&name))
                {
                    projects.push((name, true));
                }
            }
            for name in elsewhere(dova, reviews, cwd).await {
                if !projects
                    .iter()
                    .any(|(known, _)| known.eq_ignore_ascii_case(&name))
                {
                    projects.push((name, false));
                }
            }

            for (project, must) in projects {
                let asked = json!({ "project": project, "review": false });
                let listed = match dova.call("pr.list", cwd, asked).await {
                    Ok(listed) => listed,
                    Err(err) if must => return Err(err),
                    // A project found by the search that will not list is left out.
                    Err(_) => continue,
                };
                let listed: Vec<PullRow> = serde_json::from_value(listed)
                    .map_err(|err| Error::Dova(format!("couldn't read dova's list: {err}")))?;
                for mut row in listed {
                    if rows.iter().any(|known| known.pr.id == row.pr.id) {
                        continue;
                    }
                    // The ones you opened are listed as yours, even where you are a reviewer too.
                    row.review = None;
                    row.workspace = home(&clones, &row);
                    // Why a check failed and which thread is open are asked for one row at a time.
                    let failed = |run: &PipelineRun| run.result.as_deref() == Some("failed");
                    if row.check.as_ref().is_some_and(failed) {
                        row.failed = self.why(dova, cwd, &row).await;
                    }
                    if row.open_threads > 0 {
                        row.thread = first_open(dova, cwd, row.pr.id).await;
                    }
                    rows.push(row);
                }
                searched.push(project);
            }
        }
        Ok((rows, looked, searched))
    }

    /// The open pull requests of other people that you chose to see: the ones in `repos`,
    /// and the ones opened by `people`, who are given by their accounts. With `both`, and
    /// with repos and people both given, only the ones by one of the people in one of the
    /// repos. One call for each repo, project or person, in each organization a workspace is
    /// cloned from; nothing else is asked for. The rows come without a check and threads.
    /// Each says whether you or a team you are in is a reviewer, and those come first.
    pub async fn chosen(
        &self,
        dova: &Dova,
        reviews: &Reviews,
        workspaces: Vec<Workspace>,
        people: &[String],
        repos: &[RepoPick],
        both: bool,
    ) -> Result<Vec<PullRow>, Error> {
        // By person first: with both, the people's own are asked for and the repos only sift them.
        let narrow = both && !people.is_empty() && !repos.is_empty();
        let mut paths: Vec<String> = people
            .iter()
            .map(|id| format!("/_apis/git/pullrequests?searchCriteria.creatorId={id}&"))
            .collect();
        if !narrow {
            paths.extend(repos.iter().map(|pick| {
                let project = pick.project.replace(' ', "%20");
                match &pick.repo {
                    Some(repo) => {
                        let repo = repo.replace(' ', "%20");
                        format!("/{project}/_apis/git/repositories/{repo}/pullrequests?")
                    }
                    None => format!("/{project}/_apis/git/pullrequests?"),
                }
            }));
        }
        if paths.is_empty() {
            return Ok(Vec::new());
        }

        let (clones, _) = clones(workspaces).await;
        let mut rows: Vec<PullRow> = Vec::new();
        let mut seen = HashSet::new();
        // A repo or person that is not in one organization is in another, so one call that
        // fails is passed over. When none answers, the last reason is given.
        let (mut answered, mut failed) = (false, None);
        for clone in one_each(&clones) {
            let cwd = &clone.workspace.path;
            let me = reviews.me(dova, cwd).await?;
            let site = site(organization(&clone.project));
            let teams = self.teams(dova, cwd, &site).await;
            for path in &paths {
                let path = format!("{path}searchCriteria.status=active&$top=500&api-version=7.1");
                let found = match dova.call("api", cwd, json!({ "path": path })).await {
                    Ok(found) => found,
                    Err(err) => {
                        failed = Some(err);
                        continue;
                    }
                };
                answered = true;
                for pull in found["value"].as_array().into_iter().flatten() {
                    if pull["createdBy"]["id"].as_str() == Some(me.as_str()) {
                        continue;
                    }
                    let Some(mut row) = review_row(pull, &site, &me, &teams) else {
                        continue;
                    };
                    if narrow && !repos.iter().any(|pick| in_repo(pick, &row)) {
                        continue;
                    }
                    let key = (pull["repository"]["id"].to_string(), row.pr.id);
                    if !seen.insert(key) {
                        continue;
                    }
                    row.workspace = home(&clones, &row);
                    rows.push(row);
                }
            }
        }
        if let (false, Some(err)) = (answered, failed) {
            return Err(err);
        }
        // The ones you are a reviewer of first. The sort keeps the rest in the order they came.
        rows.sort_by_key(|row| row.review.is_none());
        Ok(rows)
    }

    /// The teams you are in, anywhere in the organization. None when the organization
    /// will not say.
    async fn teams(&self, dova: &Dova, cwd: &str, site: &str) -> Vec<Mine> {
        if let Some(known) = self.teams.lock().expect("not poisoned").get(site) {
            return known.clone();
        }
        let path = "/_apis/teams?$mine=true&$top=500&api-version=7.1-preview.3";
        let Ok(found) = dova.call("api", cwd, json!({ "path": path })).await else {
            return Vec::new();
        };
        let teams: Vec<Mine> = found["value"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|team| {
                Some(Mine {
                    id: team["id"].as_str()?.to_string(),
                    name: team["name"].as_str()?.to_string(),
                    project: team["projectName"].as_str()?.to_string(),
                })
            })
            .collect();
        self.teams
            .lock()
            .expect("not poisoned")
            .insert(site.to_string(), teams.clone());
        teams
    }

    /// The teams you are in, each with who else is in it. A team whose members the
    /// organization will not give is listed with none.
    pub async fn teams_with_members(
        &self,
        dova: &Dova,
        reviews: &Reviews,
        workspaces: Vec<Workspace>,
    ) -> Result<Vec<Team>, Error> {
        let (clones, _) = clones(workspaces).await;
        let mut teams = Vec::new();
        for clone in one_each(&clones) {
            let cwd = &clone.workspace.path;
            let me = reviews.me(dova, cwd).await?;
            let site = site(organization(&clone.project));
            for team in self.teams(dova, cwd, &site).await {
                let members = self.members(dova, cwd, &team).await;
                teams.push(Team {
                    name: team.name,
                    project: team.project,
                    members: members.into_iter().filter(|one| one.id != me).collect(),
                });
            }
        }
        Ok(teams)
    }

    /// Who is in a team: the people, not the groups inside it.
    async fn members(&self, dova: &Dova, cwd: &str, team: &Mine) -> Vec<Person> {
        if let Some(known) = self.members.lock().expect("not poisoned").get(&team.id) {
            return known.clone();
        }
        let project = team.project.replace(' ', "%20");
        let path = format!(
            "/_apis/projects/{project}/teams/{}/members?$top=500&api-version=7.1",
            team.id
        );
        let Ok(found) = dova.call("api", cwd, json!({ "path": path })).await else {
            return Vec::new();
        };
        let mut members: Vec<Person> = found["value"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|member| &member["identity"])
            .filter(|identity| identity["isContainer"].as_bool() != Some(true))
            .filter_map(|identity| {
                Some(Person {
                    id: identity["id"].as_str()?.to_string(),
                    name: identity["displayName"].as_str()?.to_string(),
                    mail: identity["uniqueName"].as_str().map(str::to_string),
                })
            })
            .collect();
        members.sort_by(|a, b| a.name.cmp(&b.name));
        self.members
            .lock()
            .expect("not poisoned")
            .insert(team.id.clone(), members.clone());
        members
    }

    /// The people, repos and projects that have `text` in their name, in each organization a
    /// workspace is cloned from. A few of each. What an organization will not give is left out.
    pub async fn find(
        &self,
        dova: &Dova,
        workspaces: Vec<Workspace>,
        text: &str,
    ) -> (Vec<Person>, Vec<RepoName>, Vec<String>) {
        let text = text.trim().to_lowercase();
        let (mut people, mut repos, mut projects) = (Vec::new(), Vec::new(), Vec::new());
        if text.is_empty() {
            return (people, repos, projects);
        }
        let (clones, _) = clones(workspaces).await;
        for clone in one_each(&clones) {
            let cwd = &clone.workspace.path;
            let site = site(organization(&clone.project));
            let all = self.repos(dova, cwd, &site).await;
            let (found, of) = matching(&all, &text);
            repos.extend(found);
            for project in of {
                if !projects.contains(&project) {
                    projects.push(project);
                }
            }
            for person in search_people(dova, cwd, &text).await {
                if !people.iter().any(|known: &Person| known.id == person.id) {
                    people.push(person);
                }
            }
        }
        people.truncate(FOUND);
        repos.truncate(FOUND);
        projects.truncate(FOUND);
        (people, repos, projects)
    }

    /// Every repo of an organization that is in use.
    async fn repos(&self, dova: &Dova, cwd: &str, site: &str) -> Vec<RepoName> {
        if let Some(known) = self.repos.lock().expect("not poisoned").get(site) {
            return known.clone();
        }
        let path = "/_apis/git/repositories?api-version=7.1";
        let Ok(found) = dova.call("api", cwd, json!({ "path": path })).await else {
            return Vec::new();
        };
        let repos: Vec<RepoName> = found["value"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|repo| repo["isDisabled"].as_bool() != Some(true))
            .filter_map(|repo| {
                Some(RepoName {
                    project: repo["project"]["name"].as_str()?.to_string(),
                    repo: repo["name"].as_str()?.to_string(),
                })
            })
            .collect();
        self.repos
            .lock()
            .expect("not poisoned")
            .insert(site.to_string(), repos.clone());
        repos
    }

    /// Why a row's check failed, in a line. Asked once for each run.
    async fn why(&self, dova: &Dova, cwd: &str, row: &PullRow) -> Option<String> {
        let run = row.check.as_ref()?.id;
        if let Some(known) = self.failed.lock().expect("not poisoned").get(&run) {
            return Some(known.clone());
        }
        let log = dova
            .call("pipeline.log", cwd, json!({ "run": run.to_string() }))
            .await
            .ok()?;
        let reason = reason(&log)?;
        self.failed
            .lock()
            .expect("not poisoned")
            .insert(run, reason.clone());
        Some(reason)
    }
}

/// A folder's branch and its pull request: dova's `status`. With `pr`, that
/// pull request and the runs on its branch, whatever the folder is on.
pub async fn status(dova: &Dova, dir: &str, pr: Option<u64>) -> Result<PullView, Error> {
    let Some(pr) = pr else {
        return shape(dova.call("status", dir, json!({})).await?);
    };
    let view = dova
        .call("pr.view", dir, json!({ "idOrUrl": pr.to_string() }))
        .await?;
    let request: PullRequest = shape(view.clone())?;
    let runs = dova
        .call(
            "pipeline.list",
            dir,
            json!({ "branch": request.source_branch }),
        )
        .await?;
    Ok(PullView {
        branch: git::status(std::path::Path::new(dir))
            .await
            .map(|status| status.summary.branch),
        work_items: shape::<Vec<WorkItemRef>>(view["workItems"].clone())?,
        threads: shape::<Vec<ThreadSummary>>(view["threads"].clone())?,
        runs: shape::<Vec<PipelineRun>>(runs["runs"].clone())?,
        pr: Some(request),
    })
}

/// Answers a thread. With `resolve`, the same call resolves it.
pub async fn reply(
    dova: &Dova,
    dir: &str,
    pr: u64,
    thread: u64,
    text: &str,
    resolve: bool,
) -> Result<(), Error> {
    let mut params = json!({ "pr": pr.to_string(), "thread": thread.to_string(), "text": text });
    if resolve {
        params["resolve"] = json!("resolved");
    }
    dova.call("pr.reply", dir, params).await.map(|_| ())
}

/// Sets a thread's status: `resolved`, or `active` to open it again.
pub async fn resolve(
    dova: &Dova,
    dir: &str,
    pr: u64,
    thread: u64,
    status: &str,
) -> Result<(), Error> {
    let params = json!({ "pr": pr.to_string(), "thread": thread.to_string(), "status": status });
    dova.call("pr.resolve", dir, params).await.map(|_| ())
}

/// What a pull request from the folder's branch would hold: dova's `summarize`.
pub async fn before(dova: &Dova, dir: &str) -> Result<PullBefore, Error> {
    shape(dova.call("summarize", dir, json!({})).await?)
}

/// Opens a pull request from the folder's branch. dova attaches the work
/// items linked to the branch, and uploads pictures the description names.
pub async fn create(
    dova: &Dova,
    dir: &str,
    title: &str,
    body: &str,
    draft: bool,
) -> Result<PullRequest, Error> {
    let params = json!({ "title": title, "body": body, "draft": draft });
    shape(dova.call("pr.create", dir, params).await?)
}

pub async fn link(dova: &Dova, dir: &str, ids: &[u64]) -> Result<(), Error> {
    let ids: Vec<String> = ids.iter().map(u64::to_string).collect();
    dova.call("link", dir, json!({ "ids": ids }))
        .await
        .map(|_| ())
}

/// What dova returned, read as the shape the page is sent.
fn shape<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, Error> {
    serde_json::from_value(value)
        .map_err(|err| Error::Dova(format!("couldn't read what dova said: {err}")))
}

/// The first thread on a pull request that still wants an answer.
async fn first_open(dova: &Dova, cwd: &str, pr: u64) -> Option<ThreadSummary> {
    let view = dova
        .call("pr.view", cwd, json!({ "idOrUrl": pr.to_string() }))
        .await
        .ok()?;
    let threads: Vec<ThreadSummary> = serde_json::from_value(view["threads"].clone()).ok()?;
    threads.into_iter().find(|thread| thread.unresolved)
}

/// The projects, anywhere in the organization `cwd` is cloned from, that have an open pull
/// request you opened. Nothing when the organization will not say.
async fn elsewhere(dova: &Dova, reviews: &Reviews, cwd: &str) -> Vec<String> {
    let Ok(me) = reviews.me(dova, cwd).await else {
        return Vec::new();
    };
    // A path that starts with a slash is asked of the organization, not of the folder's project.
    let path = format!(
        "/_apis/git/pullrequests?searchCriteria.creatorId={me}&searchCriteria.status=active&$top=500&api-version=7.1"
    );
    let Ok(found) = dova.call("api", cwd, json!({ "path": path })).await else {
        return Vec::new();
    };
    let mut projects: Vec<String> = Vec::new();
    for pull in found["value"].as_array().into_iter().flatten() {
        let name = pull["repository"]["project"]["name"].as_str();
        if let Some(name) = name.filter(|name| !projects.iter().any(|known| known == name)) {
            projects.push(name.to_string());
        }
    }
    projects
}

/// Whether a row is in a repo you chose.
fn in_repo(pick: &RepoPick, row: &PullRow) -> bool {
    pick.project.eq_ignore_ascii_case(&row.pr.project)
        && pick
            .repo
            .as_ref()
            .is_none_or(|repo| repo.eq_ignore_ascii_case(&row.pr.repo))
}

/// Someone else's pull request, from Azure DevOps' own list: who opened it, and how you stand
/// on it when you or one of `teams`, the teams you are in, is a reviewer of it.
fn review_row(pull: &Value, site: &str, me: &str, teams: &[Mine]) -> Option<PullRow> {
    let id = pull["pullRequestId"].as_u64()?;
    let repo = pull["repository"]["name"].as_str()?;
    let project = pull["repository"]["project"]["name"].as_str()?;
    let branch = |name: &str| {
        let full = pull[name].as_str().unwrap_or_default();
        full.strip_prefix("refs/heads/").unwrap_or(full).to_string()
    };
    let reviewer = |id: &str| {
        pull["reviewers"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|reviewer| reviewer["id"].as_str() == Some(id))
    };
    let mine = reviewer(me);
    let team = teams.iter().find(|team| reviewer(&team.id).is_some());
    let vote = match mine.and_then(|reviewer| reviewer["vote"].as_i64()) {
        Some(10) => "approved",
        Some(5) => "approved with suggestions",
        Some(-5) => "waiting for author",
        Some(-10) => "rejected",
        _ => "none",
    };
    let place = format!("{project}/_git/{repo}").replace(' ', "%20");
    Some(PullRow {
        pr: PullRequest {
            id,
            title: pull["title"].as_str().unwrap_or_default().to_string(),
            status: pull["status"].as_str().unwrap_or("active").to_string(),
            is_draft: pull["isDraft"].as_bool().unwrap_or(false),
            source_branch: branch("sourceRefName"),
            target_branch: branch("targetRefName"),
            project: project.to_string(),
            repo: repo.to_string(),
            url: format!("{site}/{place}/pullrequest/{id}"),
        },
        check: None,
        open_threads: 0,
        failed: None,
        thread: None,
        workspace: None,
        author: pull["createdBy"]["displayName"]
            .as_str()
            .map(str::to_string),
        author_id: pull["createdBy"]["id"].as_str().map(str::to_string),
        // Asked by name counts first: the team is said only when it alone was asked.
        review: (mine.is_some() || team.is_some()).then(|| Reviewing {
            vote: vote.into(),
            team: team
                .filter(|_| mine.is_none())
                .map(|team| team.name.clone()),
        }),
    })
}

/// The repos with `text` (in small letters) in their name, and the projects with it in
/// theirs. A repo whose name starts with it comes before one that only has it somewhere.
fn matching(all: &[RepoName], text: &str) -> (Vec<RepoName>, Vec<String>) {
    let mut repos: Vec<&RepoName> = all
        .iter()
        .filter(|one| one.repo.to_lowercase().contains(text))
        .collect();
    repos.sort_by_key(|one| {
        (
            !one.repo.to_lowercase().starts_with(text),
            one.repo.to_lowercase(),
            one.project.to_lowercase(),
        )
    });
    let mut projects: Vec<String> = Vec::new();
    for one in all {
        if one.project.to_lowercase().contains(text) && !projects.contains(&one.project) {
            projects.push(one.project.clone());
        }
    }
    projects.sort_by_key(|name| (!name.to_lowercase().starts_with(text), name.to_lowercase()));
    (
        repos.into_iter().take(FOUND).cloned().collect(),
        projects.into_iter().take(FOUND).collect(),
    )
}

/// The people of the organization `cwd` is cloned from whose name or sign-in has `text`:
/// the search Azure DevOps' own people fields use. Only those who have an account in the
/// organization: nobody else can have opened a pull request there.
async fn search_people(dova: &Dova, cwd: &str, text: &str) -> Vec<Person> {
    let body = json!({
        "query": text,
        "identityTypes": ["user"],
        "operationScopes": ["ims", "source"],
        "options": { "MinResults": 5, "MaxResults": 20 },
        "properties": ["DisplayName", "Mail", "SignInAddress"],
    });
    let asked = json!({
        "path": "/_apis/IdentityPicker/Identities?api-version=7.1-preview.1",
        "method": "POST",
        "body": body.to_string(),
        "contentType": "application/json",
    });
    let Ok(found) = dova.call("api", cwd, asked).await else {
        return Vec::new();
    };
    let results = found["results"].as_array().into_iter().flatten();
    results
        .flat_map(|result| result["identities"].as_array().into_iter().flatten())
        .filter_map(|identity| {
            Some(Person {
                id: identity["localId"].as_str()?.to_string(),
                name: identity["displayName"].as_str()?.to_string(),
                mail: identity["mail"]
                    .as_str()
                    .or(identity["signInAddress"].as_str())
                    .map(str::to_string),
            })
        })
        .collect()
}

/// An organization's own address on the web, from its part of a remote:
/// `https://dev.azure.com/contoso` from that, or from `git@ssh.dev.azure.com:v3/contoso`.
fn site(organization: &str) -> String {
    if organization.starts_with("http") {
        return without_sign_in(organization);
    }
    let name = organization
        .rsplit(['/', ':'])
        .next()
        .unwrap_or(organization);
    format!("https://dev.azure.com/{name}")
}

/// A project's organization, from the project's part of a remote: what is left without its name.
fn organization(project: &str) -> &str {
    let named = project.trim_end_matches("/_git");
    named
        .rsplit_once(['/', ':'])
        .map_or(named, |(organization, _)| organization)
}

/// The workspace that has a pull request's repo: the one on its branch, if
/// there are several.
fn home(clones: &[Clone], row: &PullRow) -> Option<String> {
    let mut here = clones.iter().filter(|clone| {
        project_name(&clone.project).eq_ignore_ascii_case(&row.pr.project)
            && clone.repo.eq_ignore_ascii_case(&row.pr.repo)
    });
    let first = here.clone().next()?;
    let on_branch =
        here.find(|clone| clone.branch.as_deref() == Some(row.pr.source_branch.as_str()));
    Some(on_branch.unwrap_or(first).workspace.id.clone())
}

/// A project as people call it, from its part of a remote: `Shop` from
/// `https://dev.azure.com/contoso/Shop/_git`.
fn project_name(project: &str) -> String {
    let named = project.trim_end_matches("/_git");
    let name = named.rsplit(['/', ':']).next().unwrap_or(named);
    name.replace("%20", " ")
}

/// A remote without the name or token that was written in it to sign in.
fn without_sign_in(remote: &str) -> String {
    match remote.split_once("://") {
        Some((scheme, rest)) => {
            let host_on = rest.split('/').next().unwrap_or_default();
            match host_on.rsplit_once('@') {
                Some((sign_in, _)) => format!("{scheme}://{}", &rest[sign_in.len() + 1..]),
                None => remote.to_string(),
            }
        }
        None => remote.to_string(),
    }
}

/// A remote on Azure Repos, as its project's part and the repo's name.
/// None for a remote anywhere else.
pub fn azure(remote: &str) -> Option<(String, String)> {
    let remote = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    if !remote.contains("dev.azure.com") && !remote.contains("visualstudio.com") {
        return None;
    }
    let (project, repo) = remote.rsplit_once('/')?;
    Some((project.to_string(), repo.to_string()))
}

/// Why a run failed, from its log as dova gives it: what the pipeline
/// itself said went wrong, or else which task failed.
fn reason(log: &Value) -> Option<String> {
    let first = |key: &str| log[key].as_array()?.first()?.as_str().map(str::to_string);
    first("errors")
        .or_else(|| first("failedTasks").map(|task| format!("Failed in the task {task}")))
}

#[cfg(test)]
mod tests {
    use protocol::PullRequest;

    use super::*;

    #[test]
    fn knows_an_azure_remote_from_any_other() {
        assert_eq!(
            azure("https://dev.azure.com/contoso/Shop/_git/web\n"),
            Some((
                "https://dev.azure.com/contoso/Shop/_git".into(),
                "web".into()
            ))
        );
        assert_eq!(
            azure("git@ssh.dev.azure.com:v3/contoso/Shop/web"),
            Some(("git@ssh.dev.azure.com:v3/contoso/Shop".into(), "web".into()))
        );
        assert_eq!(azure("https://github.com/maxjay/dova.git"), None);
    }

    #[test]
    fn names_a_project_and_hides_a_sign_in() {
        assert_eq!(
            organization("https://dev.azure.com/contoso/Shop/_git"),
            "https://dev.azure.com/contoso"
        );
        assert_eq!(
            organization("git@ssh.dev.azure.com:v3/contoso/Shop"),
            "git@ssh.dev.azure.com:v3/contoso"
        );
        assert_eq!(
            project_name("https://dev.azure.com/contoso/Shop/_git"),
            "Shop"
        );
        assert_eq!(
            project_name("git@ssh.dev.azure.com:v3/contoso/Web%20Shop"),
            "Web Shop"
        );
        assert_eq!(
            project_name("https://contoso.visualstudio.com/DefaultCollection/Shop/_git"),
            "Shop"
        );
        assert_eq!(
            without_sign_in("https://max:s3cret@dev.azure.com/contoso/Shop/_git/web"),
            "https://dev.azure.com/contoso/Shop/_git/web"
        );
        assert_eq!(
            without_sign_in("https://contoso@dev.azure.com/contoso/Shop/_git/web"),
            "https://dev.azure.com/contoso/Shop/_git/web"
        );
        assert_eq!(
            without_sign_in("git@github.com:maxjay/dova.git"),
            "git@github.com:maxjay/dova.git"
        );
        assert_eq!(
            without_sign_in("https://github.com/maxjay/dova"),
            "https://github.com/maxjay/dova"
        );
    }

    #[test]
    fn says_why_a_run_failed() {
        let no_agent = json!({ "errors": ["No agent found"], "failedTasks": ["Job"] });
        assert_eq!(reason(&no_agent).as_deref(), Some("No agent found"));
        let tests = json!({ "errors": [], "failedTasks": ["npm test"] });
        assert_eq!(
            reason(&tests).as_deref(),
            Some("Failed in the task npm test")
        );
        assert_eq!(reason(&json!({ "errors": [], "failedTasks": [] })), None);
    }

    #[test]
    fn a_pull_request_belongs_to_the_workspace_on_its_branch() {
        let clone = |id: &str, repo: &str, branch: &str| Clone {
            workspace: Workspace {
                id: id.into(),
                path: String::new(),
                name: String::new(),
                opened: 0,
                scratch: false,
            },
            project: "https://dev.azure.com/contoso/Shop/_git".into(),
            repo: repo.into(),
            branch: Some(branch.into()),
        };
        let clones = [
            clone("a", "web", "main"),
            clone("b", "web", "fix/login"),
            clone("c", "api", "main"),
        ];
        let row = |repo: &str, branch: &str| PullRow {
            pr: PullRequest {
                id: 1,
                title: String::new(),
                status: "active".into(),
                is_draft: false,
                source_branch: branch.into(),
                target_branch: "main".into(),
                project: "Shop".into(),
                repo: repo.into(),
                url: String::new(),
            },
            check: None,
            open_threads: 0,
            failed: None,
            thread: None,
            workspace: None,
            author: None,
            author_id: None,
            review: None,
        };
        assert_eq!(
            home(&clones, &row("web", "fix/login")).as_deref(),
            Some("b")
        );
        assert_eq!(home(&clones, &row("web", "other")).as_deref(), Some("a"));
        assert_eq!(home(&clones, &row("docs", "main")), None);
    }

    /// A row as Azure DevOps lists it, with these reviewers.
    fn listed(reviewers: Value) -> Value {
        json!({
            "pullRequestId": 7,
            "title": "Add a thing",
            "status": "active",
            "isDraft": true,
            "sourceRefName": "refs/heads/feature/thing",
            "targetRefName": "refs/heads/main",
            "repository": { "id": "r1", "name": "Web Shop", "project": { "name": "Shop Front" } },
            "createdBy": { "id": "ann", "displayName": "Ann Lee" },
            "reviewers": reviewers,
        })
    }

    fn payments() -> Vec<Mine> {
        vec![Mine {
            id: "team".into(),
            name: "Payments".into(),
            project: "Shop Front".into(),
        }]
    }

    #[test]
    fn reads_a_row_from_azures_own_list() {
        let pull = listed(json!([{ "id": "me", "vote": -5 }, { "id": "team", "vote": 0 }]));
        let row =
            review_row(&pull, "https://dev.azure.com/contoso", "me", &payments()).expect("a row");
        assert_eq!(row.pr.id, 7);
        assert_eq!(row.pr.source_branch, "feature/thing");
        assert_eq!(row.pr.target_branch, "main");
        assert!(row.pr.is_draft);
        assert_eq!(
            row.pr.url,
            "https://dev.azure.com/contoso/Shop%20Front/_git/Web%20Shop/pullrequest/7"
        );
        assert_eq!(row.author.as_deref(), Some("Ann Lee"));
        assert_eq!(row.author_id.as_deref(), Some("ann"));
        let review = row.review.expect("how you stand");
        assert_eq!(review.vote, "waiting for author");
        assert_eq!(review.team, None);
    }

    #[test]
    fn says_how_you_stand_only_when_you_or_a_team_of_yours_is_a_reviewer() {
        let by_team = listed(json!([{ "id": "team", "vote": 0 }]));
        let row = review_row(&by_team, "https://x", "me", &payments()).expect("a row");
        let review = row.review.expect("how you stand");
        assert_eq!(review.vote, "none");
        assert_eq!(review.team.as_deref(), Some("Payments"));

        let others = listed(json!([{ "id": "someone", "vote": 10 }]));
        let row = review_row(&others, "https://x", "me", &payments()).expect("a row");
        assert_eq!(row.review, None);
    }

    #[test]
    fn knows_a_row_in_a_repo_or_a_project_you_chose() {
        let pull = listed(json!([]));
        let row = review_row(&pull, "https://x", "me", &[]).expect("a row");
        let pick = |project: &str, repo: Option<&str>| RepoPick {
            project: project.into(),
            repo: repo.map(str::to_string),
        };
        assert!(in_repo(&pick("shop front", Some("web shop")), &row));
        assert!(in_repo(&pick("Shop Front", None), &row));
        assert!(!in_repo(&pick("Shop Front", Some("api")), &row));
        assert!(!in_repo(&pick("Platform", None), &row));
    }

    #[test]
    fn knows_an_organizations_address_from_either_kind_of_remote() {
        assert_eq!(
            site("https://someone@dev.azure.com/contoso"),
            "https://dev.azure.com/contoso"
        );
        assert_eq!(
            site("git@ssh.dev.azure.com:v3/contoso"),
            "https://dev.azure.com/contoso"
        );
        assert_eq!(
            site("https://contoso.visualstudio.com"),
            "https://contoso.visualstudio.com"
        );
    }
}
