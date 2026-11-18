//! Workspaces: a folder opened in the workbench, its files, and where its
//! git repository stands.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A folder opened in the workbench. Nothing is written into the folder itself.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Workspace {
    pub id: String,
    /// Absolute, with links resolved.
    pub path: String,
    /// The folder's own name.
    pub name: String,
    /// When it was last opened, in milliseconds since 1970.
    #[ts(type = "number")]
    pub opened: u64,
}

/// A workspace as Home lists it.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkspaceView {
    pub workspace: Workspace,
    /// False when the folder was moved or deleted.
    pub exists: bool,
    /// None outside git.
    pub git: Option<GitSummary>,
}

/// A folder to pick when opening one.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Folder {
    pub name: String,
    pub path: String,
    /// Its branch, when it is a git repository.
    pub branch: Option<String>,
}

/// A file, as the editor opens it.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileView {
    pub path: String,
    pub content: FileContent,
    /// When it last changed, in milliseconds since 1970. 0 for a missing file.
    #[ts(type = "number")]
    pub modified: u64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum FileContent {
    /// `truncated` when the file is longer than what was sent.
    Text {
        text: String,
        truncated: bool,
    },
    /// Not text. Only its size is sent.
    Binary {
        #[ts(type = "number")]
        size: u64,
    },
    Missing,
}

/// Where a folder's repository stands.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GitStatus {
    pub summary: GitSummary,
    /// The repository's top folder.
    pub root: String,
    /// The changed files, relative to the folder asked about.
    pub files: Vec<GitFile>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GitSummary {
    pub branch: String,
    /// How many files changed.
    pub files: u32,
    /// Lines added and removed since the last commit.
    pub added: u32,
    pub removed: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct GitFile {
    pub path: String,
    pub mark: GitMark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum GitMark {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

/// A terminal in a workspace's folder: a shell of its own.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Terminal {
    pub id: String,
    /// Where it started, absolute.
    pub cwd: String,
    /// What runs in it now, or the shell's name when nothing does.
    pub title: String,
    /// Something other than the shell is running.
    pub running: bool,
}
