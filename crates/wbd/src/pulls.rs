//! Your pull requests, for Home: asked from dova, once for each Azure DevOps
//! project your workspaces are cloned from, with the reason under each one
//! that needs you.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use protocol::{
    PipelineRun, PullBefore, PullRequest, PullRow, PullView, ThreadSummary, WorkItemRef, Workspace,
};
use serde_json::{Value, json};

use crate::dova::Dova;
use crate::error::Error;
use crate::git;

#[derive(Default)]
pub struct Pulls {
    /// Why each failed run failed, by run id. A finished run doesn't change.
    failed: Mutex<HashMap<u64, String>>,
}

/// A workspace that is cloned from Azure Repos.
struct Clone {
    workspace: Workspace,
    /// Its remote without the repo's name: the same for every repo in a project.
    project: String,
    repo: String,
    branch: Option<String>,
}

impl Pulls {
    /// The pull requests you opened, across every project your workspaces are in.
    pub async fn list(
        &self,
        dova: &Dova,
        workspaces: Vec<Workspace>,
    ) -> Result<Vec<PullRow>, Error> {
        let mut clones = Vec::new();
        for workspace in workspaces.into_iter().filter(|w| !w.scratch) {
            let dir = std::path::Path::new(&workspace.path);
            let Some((project, repo)) = git::origin(dir).await.as_deref().and_then(azure) else {
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

        let mut rows: Vec<PullRow> = Vec::new();
        let mut asked = HashSet::new();
        for clone in &clones {
            // dova lists a whole project at once, so each project is asked once.
            if !asked.insert(clone.project.to_lowercase()) {
                continue;
            }
            let cwd = &clone.workspace.path;
            let listed = dova.call("pr.list", cwd, json!({})).await?;
            let listed: Vec<PullRow> = serde_json::from_value(listed)
                .map_err(|err| Error::Dova(format!("couldn't read dova's list: {err}")))?;
            for mut row in listed {
                row.workspace = home(&clones, &clone.project, &row);
                if row
                    .check
                    .as_ref()
                    .is_some_and(|run| run.result.as_deref() == Some("failed"))
                {
                    row.failed = self.why(dova, cwd, &row).await;
                }
                if row.open_threads > 0 {
                    row.thread = first_open(dova, cwd, row.pr.id).await;
                }
                rows.push(row);
            }
        }
        Ok(rows)
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

/// The workspace that has a pull request's repo: the one on its branch, if
/// there are several.
fn home(clones: &[Clone], project: &str, row: &PullRow) -> Option<String> {
    let mut here = clones.iter().filter(|clone| {
        clone.project.eq_ignore_ascii_case(project) && clone.repo.eq_ignore_ascii_case(&row.pr.repo)
    });
    let first = here.clone().next()?;
    let on_branch =
        here.find(|clone| clone.branch.as_deref() == Some(row.pr.source_branch.as_str()));
    Some(on_branch.unwrap_or(first).workspace.id.clone())
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
        };
        let project = "https://dev.azure.com/contoso/Shop/_git";
        assert_eq!(
            home(&clones, project, &row("web", "fix/login")).as_deref(),
            Some("b")
        );
        assert_eq!(
            home(&clones, project, &row("web", "other")).as_deref(),
            Some("a")
        );
        assert_eq!(home(&clones, project, &row("docs", "main")), None);
    }
}
