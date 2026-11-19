//! Workspaces: a folder opened in the workbench, its files, and where its
//! git repository stands.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A folder opened in the workbench. Nothing is written into the folder itself.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Workspace {
    pub id: String,
    /// Absolute, with links resolved.
    pub path: String,
    /// What it is called. For a folder you opened, the folder's own name. For one the
    /// workbench made, the main repo and the name of the work: `web-fix-login`.
    pub name: String,
    /// When it was last opened, in milliseconds since 1970.
    #[ts(type = "number")]
    pub opened: u64,
    /// True for a scratchpad: a folder the workbench made, for something
    /// you asked that is not about a repo.
    #[serde(default)]
    pub scratch: bool,
    /// The repo the work started from, by its folder's name. None while there is no repo in
    /// it, and for a folder you opened.
    #[serde(default)]
    pub main: Option<String>,
    /// The name of the work, in a workspace the workbench made: the branch in every repo in it.
    #[serde(default)]
    pub work: Option<String>,
    /// True once the work was named. Until then its name is the first words you wrote.
    #[serde(default)]
    pub named: bool,
}

/// A workspace as Home lists it.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkspaceView {
    pub workspace: Workspace,
    /// False when the folder was moved or deleted.
    pub exists: bool,
    /// The branch it is on, read from the folder without running git: listing and opening a
    /// workspace never wait for git. None outside git.
    pub branch: Option<String>,
    /// The repos in it, by name, when its folder is not a repo itself: each a folder of its
    /// own directly inside.
    pub repos: Vec<String>,
}

/// Where things are kept, and what the workbench keeps there.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Settings {
    /// The folder your clones are in, as you wrote it. `~` is your home folder.
    pub repos: String,
    /// The folder the workbench keeps its own things in, as you wrote it: a checkout for
    /// each pull request you work on with the agent.
    pub own: String,
    /// How many clones were found in `repos`.
    pub clones: u32,
    /// The checkouts of pull requests there are, each by its folder.
    pub checkouts: Vec<String>,
    /// The ones whose pull request ended that are kept all the same: they hold work that
    /// was not committed or not pushed.
    pub kept: Vec<String>,
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
