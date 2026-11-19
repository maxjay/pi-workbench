//! A pull request as you review it: who voted, what was said and where,
//! which files changed and how, and the runs that checked it.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{Comment, PullRequest, WorkItemRef};

/// Everything the pull request's tab shows.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Review {
    pub pr: PullRequest,
    pub author: String,
    /// Markdown.
    pub description: String,
    /// Whether it can merge, as Azure DevOps says: `succeeded`, `conflicts`, `queued`.
    pub merge: Option<String>,
    /// It completes by itself once its checks pass.
    pub auto_complete: bool,
    pub reviewers: Vec<Reviewer>,
    /// Your own vote. See [`Reviewer::vote`].
    pub vote: i8,
    pub work_items: Vec<WorkItemRef>,
    /// The oldest first.
    pub threads: Vec<Thread>,
    pub files: Vec<ChangedFile>,
    /// The commit the branch left its target at. The diff starts here.
    pub base: String,
    /// The branch's last commit. The diff ends here.
    pub head: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Reviewer {
    pub name: String,
    /// 10 approved, 5 approved with suggestions, 0 no vote, -5 waiting for
    /// the author, -10 rejected.
    pub vote: i8,
    pub required: bool,
}

/// A review thread, with everything said in it.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Thread {
    #[ts(type = "number")]
    pub id: u64,
    /// Azure DevOps' own word: `active`, `fixed`, `wontFix`, `closed`, `byDesign`, `pending`.
    pub status: String,
    /// True while it still wants an answer.
    pub unresolved: bool,
    /// The file it is on. None for a thread about the whole pull request.
    pub place: Option<Place>,
    pub comments: Vec<Comment>,
}

/// Lines of a file in a pull request, as they are after its latest push.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct Place {
    /// Relative to the repository, with no slash in front.
    pub file: String,
    /// The first line. None when it is on no line: it is about the whole
    /// file, or its lines are gone.
    pub from: Option<u32>,
    /// The last line. A thread is drawn under it.
    pub line: Option<u32>,
    /// The lines are counted in the file as it was before the change.
    pub old: bool,
    /// A later push removed the lines it was on.
    pub outdated: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ChangedFile {
    /// Relative to the repository, with no slash in front.
    pub path: String,
    /// What it was called before, for a renamed file.
    pub old_path: Option<String>,
    pub change: FileChange,
    pub added: u32,
    pub removed: u32,
    /// Names what changed in the file. The same change has the same name,
    /// so a file you marked as viewed stays so until a push changes it.
    pub mark: String,
    /// Not text: there are no lines to show.
    pub binary: bool,
    /// It changes too many lines to send with the rest. Asking for the file gets them.
    pub large: bool,
    /// The changed lines, with a few unchanged ones around each change.
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FileChange {
    Added,
    Edited,
    Deleted,
    Renamed,
}

/// One line of a file's diff. A line that didn't change has both numbers.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct DiffLine {
    pub kind: DiffKind,
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DiffKind {
    Context,
    Added,
    Removed,
}

/// What you can do to a pull request as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PullTo {
    /// Merges it now: squashed, the branch deleted, its work items closed.
    Complete,
    /// Completes it once its checks pass.
    AutoComplete,
    CancelAutoComplete,
    Draft,
    Ready,
    Abandon,
    Reactivate,
}

/// A pipeline run on a pull request's branch.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CheckRun {
    #[ts(type = "number")]
    pub id: u64,
    /// The pipeline's name.
    pub name: String,
    /// The run's own number, such as `20261119.1`.
    pub number: String,
    /// `completed`, `inProgress`, `notStarted`.
    pub status: String,
    /// `succeeded`, `failed`, `canceled`, `partiallySucceeded`, or nothing yet.
    pub result: Option<String>,
    /// The commit it ran on.
    pub commit: String,
    pub queued_at: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub url: String,
}

/// A job of a run, or a step of a job.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Step {
    pub name: String,
    /// True for a job. Its steps follow it.
    pub job: bool,
    /// `completed`, `inProgress`, `pending`.
    pub state: String,
    /// `succeeded`, `failed`, `skipped`, `canceled`, or nothing yet.
    pub result: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    /// Its log's number in the run, when it left one.
    #[ts(type = "number | null")]
    pub log: Option<u64>,
    /// What went wrong in it, in the pipeline's own words.
    pub errors: Vec<String>,
}
