//! Codespaces: folders in WSL opened in the workbench, with their files,
//! git, terminals and sessions with Pi. wb-agent keeps the list; the folder
//! itself is left as it is.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::SessionSummary;

/// A folder opened as a codespace.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Codespace {
    pub id: String,
    /// Absolute.
    pub path: String,
    /// The folder's own name.
    pub name: String,
    pub pinned: bool,
    /// When it was last opened, in milliseconds since 1970.
    #[ts(type = "number")]
    pub opened: u64,
}

/// A codespace as Home shows it: where its git stands, what runs in its
/// terminals, and its sessions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CodespaceView {
    pub space: Codespace,
    /// None when the folder isn't in a git repository, or is gone.
    pub git: Option<GitSummary>,
    /// False once the folder can't be found.
    pub exists: bool,
    /// What runs in its terminals, by name.
    pub running: Vec<String>,
    /// How many sessions ran in it, and how many wait on you now.
    pub sessions: u32,
    pub need_you: u32,
    /// The newest few, newest first.
    pub latest: Vec<SessionSummary>,
}

/// A repository's state in short: its branch, and what has changed since
/// the last commit.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitSummary {
    /// The branch, or the commit when none is checked out.
    pub branch: String,
    pub files: u32,
    pub added: u32,
    pub removed: u32,
}

/// Where a repository stands, file by file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitStatus {
    pub summary: GitSummary,
    /// The repository's top folder, absolute.
    pub root: String,
    /// Changed files, by path relative to the folder asked about.
    pub files: Vec<GitFile>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GitFile {
    pub path: String,
    pub mark: GitMark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum GitMark {
    Modified,
    Added,
    Deleted,
    Renamed,
    /// Not tracked yet.
    Untracked,
    /// Both sides changed it in a merge.
    Conflicted,
}

/// A folder inside the one asked about, for opening as a codespace.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Folder {
    pub name: String,
    /// Absolute.
    pub path: String,
    /// The branch, when the folder is a git repository's top.
    pub branch: Option<String>,
    /// Already a codespace.
    pub codespace: bool,
}

/// A terminal in a codespace's folder: a shell of its own, which you and Pi
/// both type in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Terminal {
    pub id: String,
    /// Where it started, absolute.
    pub cwd: String,
    /// What runs in it now, or the shell's name when nothing does.
    pub title: String,
    /// Something other than the shell is running.
    pub running: bool,
    /// The shell has exited.
    pub exited: bool,
}
