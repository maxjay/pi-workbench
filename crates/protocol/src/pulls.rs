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
}
