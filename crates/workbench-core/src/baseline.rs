//! Sources for the text a diff compares against.
//!
//! Implement [`BaselineSource`] to compare with something other than the last
//! commit, such as the index, another branch, or a snapshot taken before an
//! agent edited the file.
use crate::files;
use anyhow::{Context, Result, ensure};
use std::{
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

pub trait BaselineSource: Send + Sync {
    /// Short name shown above the baseline side, such as "HEAD".
    fn label(&self) -> String;

    /// Runs off the UI thread. `Ok(None)` means the file is new, so every line is added.
    fn load(&self, path: &Path) -> Result<Option<String>>;
}

/// The committed version of the file, converted as `git checkout` would.
pub struct GitHead;

impl BaselineSource for GitHead {
    fn label(&self) -> String {
        "HEAD".into()
    }

    fn load(&self, path: &Path) -> Result<Option<String>> {
        let path = plain_path(path);
        let directory = path.parent().context("File has no parent directory")?;
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .context("Git comparison needs a UTF-8 file name")?;
        ensure!(
            git(directory, &["rev-parse", "--is-inside-work-tree"])?
                .status
                .success(),
            "Not in a git repository"
        );
        let object = format!("HEAD:./{name}");
        // Also covers repositories without any commits.
        if !git(directory, &["cat-file", "-e", &object])?
            .status
            .success()
        {
            return Ok(None);
        }
        // --filters applies line-ending and smudge conversion to match the working copy.
        let output = git(directory, &["cat-file", "--filters", &object])?;
        ensure!(
            output.status.success(),
            "git: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        files::decode_text(output.stdout).map(Some)
    }
}

fn git(directory: &Path, args: &[&str]) -> Result<Output> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(directory)
        .args(args)
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        // Avoid flashing a console window from the GUI process.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
        .output()
        .context("Could not run git. Is it installed?")
}

/// Git cannot use the `\\?\` paths that `canonicalize` returns on Windows.
fn plain_path(path: &Path) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{unc}"))
    } else if let Some(local) = text.strip_prefix(r"\\?\") {
        PathBuf::from(local)
    } else {
        path.to_path_buf()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn verbatim_windows_paths_are_simplified() {
        assert_eq!(
            plain_path(Path::new(r"\\?\C:\work\a.rs")),
            PathBuf::from(r"C:\work\a.rs")
        );
        assert_eq!(
            plain_path(Path::new(r"\\?\UNC\server\share\a.rs")),
            PathBuf::from(r"\\server\share\a.rs")
        );
        assert_eq!(
            plain_path(Path::new("/tmp/a.rs")),
            PathBuf::from("/tmp/a.rs")
        );
    }
}
