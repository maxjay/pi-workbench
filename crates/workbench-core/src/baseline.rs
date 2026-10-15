//! Sources for the text a diff compares against.
//!
//! Implement [`BaselineSource`] to compare with something other than the last
//! commit, such as the index, another branch, or a snapshot taken before an
//! agent edited the file.
use crate::git::{Repo, Rev};
use anyhow::{Context, Result};
use std::path::Path;

pub trait BaselineSource: Send + Sync {
    /// Short name shown above the baseline side, such as "HEAD".
    fn label(&self) -> String;

    /// Runs off the UI thread. `Ok(None)` means the file is new, so every line is added.
    fn load(&self, path: &Path) -> Result<Option<String>>;
}

/// The committed version of the file, read as a raw blob. Running the
/// repository's filter drivers would execute commands from an untrusted
/// checkout, and the line diff already ignores line-ending differences.
pub struct GitHead;

impl BaselineSource for GitHead {
    fn label(&self) -> String {
        "HEAD".into()
    }

    fn load(&self, path: &Path) -> Result<Option<String>> {
        let repo = Repo::discover(path)?.context("Not in a git working tree")?;
        // A repository without commits has nothing to compare against.
        let Some(head) = repo.head()? else {
            return Ok(None);
        };
        repo.read(&Rev::Commit(head), path)
    }
}

/// The file at a fixed revision, such as a branch's merge base or a snapshot
/// taken before an agent turn.
pub struct GitRev {
    pub repo: Repo,
    pub rev: Rev,
    /// Shown above the baseline side, such as "develop" or "Before last turn".
    pub label: String,
}

impl BaselineSource for GitRev {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn load(&self, path: &Path) -> Result<Option<String>> {
        self.repo.read(&self.rev, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::git;
    use std::fs;

    fn run(directory: &Path, args: &[&str]) {
        let output = git(directory, args).unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn git_head_returns_committed_text_and_none_for_new_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        run(&root, &["init", "-q"]);
        run(&root, &["config", "core.autocrlf", "false"]);
        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "committed\n").unwrap();
        run(&root, &["add", "."]);
        run(
            &root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "-qm",
                "init",
            ],
        );
        fs::write(root.join("src/main.rs"), "edited\n").unwrap();
        fs::write(root.join("src/new.rs"), "new\n").unwrap();
        let source = GitHead;
        assert_eq!(
            source.load(&root.join("src/main.rs")).unwrap().as_deref(),
            Some("committed\n")
        );
        assert_eq!(source.load(&root.join("src/new.rs")).unwrap(), None);
    }

    #[test]
    fn git_head_ignores_repository_filter_drivers() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        run(&root, &["init", "-q"]);
        run(&root, &["config", "core.autocrlf", "false"]);
        // A filter that would leave a marker file if git ran it.
        run(
            &root,
            &[
                "config",
                "filter.evil.smudge",
                "git hash-object -w --stdin && touch ran",
            ],
        );
        fs::write(
            root.join(".gitattributes"),
            "*.rs filter=evil
",
        )
        .unwrap();
        fs::write(
            root.join("a.rs"),
            "text
",
        )
        .unwrap();
        run(&root, &["add", "a.rs"]);
        run(
            &root,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t",
                "commit",
                "-qm",
                "c",
            ],
        );
        assert_eq!(
            GitHead.load(&root.join("a.rs")).unwrap().as_deref(),
            Some(
                "text
"
            )
        );
        assert!(!root.join("ran").exists());
    }

    #[test]
    fn git_head_reports_files_outside_a_repository() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap().join("loose.rs");
        fs::write(&path, "").unwrap();
        // The temp directory could sit inside a repository on a developer machine.
        if !git(dir.path(), &["rev-parse"]).unwrap().status.success() {
            assert!(GitHead.load(&path).is_err());
        }
    }

    #[test]
    fn git_rev_loads_files_at_a_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        run(&root, &["init", "-q"]);
        fs::write(root.join("a.rs"), "before\n").unwrap();
        let repo = Repo::discover(&root).unwrap().unwrap();
        let source = GitRev {
            rev: Rev::Tree(repo.snapshot().unwrap()),
            repo,
            label: "Before last turn".into(),
        };
        fs::write(root.join("a.rs"), "after\n").unwrap();
        assert_eq!(source.label(), "Before last turn");
        assert_eq!(
            source.load(&root.join("a.rs")).unwrap().as_deref(),
            Some("before\n")
        );
        assert_eq!(source.load(&root.join("b.rs")).unwrap(), None);
    }
}
