//! Pull requests, as dova reports them from Azure DevOps. The field names
//! are dova's own, so what it prints is read straight into these.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PullRequest {
    #[ts(type = "number")]
    pub id: u64,
    pub title: String,
    /// `active`, `completed` or `abandoned`.
    pub status: String,
    pub is_draft: bool,
    pub source_branch: String,
    pub target_branch: String,
    pub project: String,
    pub repo: String,
    /// Its page on Azure DevOps.
    pub url: String,
}

/// A pipeline run. `result` is nothing while it runs.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PipelineRun {
    #[ts(type = "number")]
    pub id: u64,
    pub name: String,
    pub status: String,
    /// `succeeded`, `failed`, `canceled`, or nothing yet.
    pub result: Option<String>,
    pub queued_at: Option<String>,
    pub url: String,
}

/// A review thread, by its last comment.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ThreadSummary {
    #[ts(type = "number")]
    pub id: u64,
    pub status: String,
    /// True while it still wants an answer.
    pub unresolved: bool,
    /// The line it is on. None for a thread about the whole pull request.
    pub location: Option<Location>,
    pub comment_count: u32,
    pub last_author: Option<String>,
    pub last_comment: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct Location {
    pub file: String,
    pub line: Option<u32>,
}

/// One of your pull requests, as Home lists it.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PullRow {
    #[serde(flatten)]
    pub pr: PullRequest,
    /// Its latest check, if it has one.
    pub check: Option<PipelineRun>,
    pub open_threads: u32,
    /// Why the check failed, in a line, when it did.
    #[serde(default)]
    pub failed: Option<String>,
    /// The first thread that still wants an answer.
    #[serde(default)]
    pub thread: Option<ThreadSummary>,
    /// The workspace that has this pull request's repo, if one does.
    #[serde(default)]
    pub workspace: Option<String>,
    /// Who opened it.
    #[serde(default)]
    pub author: Option<String>,
    /// Set when you or a team of yours were asked to review it: how you stand on it.
    #[serde(default)]
    pub review: Option<Reviewing>,
}

/// How you stand on a pull request you were asked to review.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct Reviewing {
    /// Your vote, in dova's word: `none`, `approved`, `approved with suggestions`,
    /// `waiting for author`, `rejected`.
    pub vote: String,
    /// The team of yours that was asked, when it was a team and not you by name.
    pub team: Option<String>,
}

/// A folder Home's list of pull requests was made from, and what was done with it.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct Asked {
    /// The workspace's name.
    pub name: String,
    /// Where it is cloned from, when it has a remote. Without any sign-in that was written in it.
    pub remote: Option<String>,
    /// The project its pull requests were asked for in. None when its remote is not on
    /// Azure Repos: it was passed over.
    pub project: Option<String>,
}

/// A work item a pull request is for.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct WorkItemRef {
    #[ts(type = "number")]
    pub id: u64,
    pub title: Option<String>,
    /// `Bug`, `Product Backlog Item` and so on.
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub state: Option<String>,
}

/// A branch and its pull request, as the workspace shows them: what dova's
/// `status` says. `pr` is None for a branch that has none yet.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PullView {
    /// The branch the folder has checked out. None on a detached HEAD.
    pub branch: Option<String>,
    pub pr: Option<PullRequest>,
    pub work_items: Vec<WorkItemRef>,
    /// The latest runs, the newest first.
    pub runs: Vec<PipelineRun>,
    /// Open threads first.
    pub threads: Vec<ThreadSummary>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Comment {
    #[ts(type = "number")]
    pub id: u64,
    pub author: Option<String>,
    pub published_at: Option<String>,
    pub text: String,
}

/// What a pull request from a branch would hold: the commits it is ahead
/// of its base by, the newest first.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct PullBefore {
    /// The branch it would go into, such as `origin/main`.
    pub base: String,
    pub commits: Vec<Commit>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct Commit {
    pub sha: String,
    pub subject: String,
}
