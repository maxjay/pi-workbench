//! Your pull requests, for Home: asked from dova, once for each Azure DevOps
//! project your workspaces are cloned from, with the reason under each one
//! that needs you. And the ones you were asked to review, asked from Azure
//! DevOps itself in a few calls, because there can be a great many.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use protocol::{
    Asked, PipelineRun, PullBefore, PullRequest, PullRow, PullView, Reviewing, ThreadSummary,
    WorkItemRef, Workspace,
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
    /// The teams you are in, as id and name, by organization. Asked once: they change rarely.
    teams: Mutex<HashMap<String, Vec<(String, String)>>>,
}

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
    /// The pull requests you opened, in the whole of each organization a workspace is cloned
    /// from: its other projects too. With them, every folder that was looked at and every
    /// project that was asked, so an empty list can say why. With `quick`, the rows without
    /// what is asked for one row at a time.
    pub async fn list(
        &self,
        dova: &Dova,
        reviews: &Reviews,
        workspaces: Vec<Workspace>,
        quick: bool,
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
                    if !quick {
                        let failed = |run: &PipelineRun| run.result.as_deref() == Some("failed");
                        if row.check.as_ref().is_some_and(failed) {
                            row.failed = self.why(dova, cwd, &row).await;
                        }
                        if row.open_threads > 0 {
                            row.thread = first_open(dova, cwd, row.pr.id).await;
                        }
                    }
                    rows.push(row);
                }
                searched.push(project);
            }
        }
        Ok((rows, looked, searched))
    }

    /// The open pull requests that you, or a team you are in, were asked to review, in the
    /// whole of each organization a workspace is cloned from. Asked of Azure DevOps itself:
    /// one call for you and one for each team. dova's own list also asks for each row's
    /// check and threads, which is too much when a team is asked to review a hundred, so
    /// these rows come without them. The ones you opened yourself are left out.
    pub async fn to_review(
        &self,
        dova: &Dova,
        reviews: &Reviews,
        workspaces: Vec<Workspace>,
    ) -> Result<Vec<PullRow>, Error> {
        let (clones, _) = clones(workspaces).await;
        let mut rows: Vec<PullRow> = Vec::new();
        let mut seen = HashSet::new();
        for clone in one_each(&clones) {
            let cwd = &clone.workspace.path;
            let me = reviews.me(dova, cwd).await?;
            let site = site(organization(&clone.project));
            // You by name first, so a row you were asked for both ways says so by name.
            let mut asked = vec![(me.clone(), None)];
            for (id, name) in self.teams(dova, cwd, &site).await {
                asked.push((id, Some(name)));
            }
            for (who, team) in asked {
                let path = format!(
                    "/_apis/git/pullrequests?searchCriteria.reviewerId={who}&searchCriteria.status=active&$top=500&api-version=7.1"
                );
                let found = match dova.call("api", cwd, json!({ "path": path })).await {
                    Ok(found) => found,
                    // You must be answered for. A team that will not list is left out.
                    Err(err) if team.is_none() => return Err(err),
                    Err(_) => continue,
                };
                for pull in found["value"].as_array().into_iter().flatten() {
                    if pull["createdBy"]["id"].as_str() == Some(me.as_str()) {
                        continue;
                    }
                    let Some(mut row) = review_row(pull, &site, &me, team.as_deref()) else {
                        continue;
                    };
                    let key = (pull["repository"]["id"].to_string(), row.pr.id);
                    if !seen.insert(key) {
                        continue;
                    }
                    row.workspace = home(&clones, &row);
                    rows.push(row);
                }
            }
        }
        Ok(rows)
    }

    /// The teams you are in, anywhere in the organization, as id and name. None when the
    /// organization will not say.
    async fn teams(&self, dova: &Dova, cwd: &str, site: &str) -> Vec<(String, String)> {
        if let Some(known) = self.teams.lock().expect("not poisoned").get(site) {
            return known.clone();
        }
        let path = "/_apis/teams?$mine=true&$top=500&api-version=7.1-preview.3";
        let Ok(found) = dova.call("api", cwd, json!({ "path": path })).await else {
            return Vec::new();
        };
        let teams: Vec<(String, String)> = found["value"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|team| {
                Some((
                    team["id"].as_str()?.to_string(),
                    team["name"].as_str()?.to_string(),
                ))
            })
            .collect();
        self.teams
            .lock()
            .expect("not poisoned")
            .insert(site.to_string(), teams.clone());
        teams
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

/// A pull request to review, from Azure DevOps' own list: who opened it and how you stand
/// on it. `team` is the team the search was for, when it was not for you by name.
fn review_row(pull: &Value, site: &str, me: &str, team: Option<&str>) -> Option<PullRow> {
    let id = pull["pullRequestId"].as_u64()?;
    let repo = pull["repository"]["name"].as_str()?;
    let project = pull["repository"]["project"]["name"].as_str()?;
    let branch = |name: &str| {
        let full = pull[name].as_str().unwrap_or_default();
        full.strip_prefix("refs/heads/").unwrap_or(full).to_string()
    };
    let mine = pull["reviewers"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|reviewer| reviewer["id"].as_str() == Some(me));
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
        review: Some(Reviewing {
            vote: vote.into(),
            // Asked by name counts first, whichever search found the row.
            team: team.filter(|_| mine.is_none()).map(str::to_string),
        }),
    })
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

    #[test]
    fn reads_a_row_to_review_from_azures_own_list() {
        let pull = listed(json!([{ "id": "me", "vote": -5 }, { "id": "team", "vote": 0 }]));
        let row = review_row(&pull, "https://dev.azure.com/contoso", "me", None).expect("a row");
        assert_eq!(row.pr.id, 7);
        assert_eq!(row.pr.source_branch, "feature/thing");
        assert_eq!(row.pr.target_branch, "main");
        assert!(row.pr.is_draft);
        assert_eq!(
            row.pr.url,
            "https://dev.azure.com/contoso/Shop%20Front/_git/Web%20Shop/pullrequest/7"
        );
        assert_eq!(row.author.as_deref(), Some("Ann Lee"));
        let review = row.review.expect("how you stand");
        assert_eq!(review.vote, "waiting for author");
        assert_eq!(review.team, None);
    }

    #[test]
    fn says_the_team_only_when_you_were_not_asked_by_name() {
        let by_team = listed(json!([{ "id": "team", "vote": 0 }]));
        let row = review_row(&by_team, "https://x", "me", Some("Payments")).expect("a row");
        let review = row.review.expect("how you stand");
        assert_eq!(review.vote, "none");
        assert_eq!(review.team.as_deref(), Some("Payments"));

        let both = listed(json!([{ "id": "team", "vote": 0 }, { "id": "me", "vote": 10 }]));
        let row = review_row(&both, "https://x", "me", Some("Payments")).expect("a row");
        let review = row.review.expect("how you stand");
        assert_eq!(review.vote, "approved");
        assert_eq!(review.team, None);
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
