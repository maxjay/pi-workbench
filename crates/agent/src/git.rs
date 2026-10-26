//! Git, asked through the `git` command: where a repository stands, and a
//! file as the last commit has it. A folder outside git, or no `git` at
//! all, is not an error: there's just nothing to say.

use std::path::{Path, PathBuf};

use protocol::{GitFile, GitMark, GitStatus, GitSummary};
use tokio::process::Command;

/// Runs git in `dir`, giving what it printed if it succeeded.
async fn git(dir: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    output.status.success().then_some(output.stdout)
}

/// Where the repository `dir` is in stands, with changed files relative to
/// `dir`. None outside git.
pub async fn status(dir: &Path) -> Option<GitStatus> {
    let top = git(dir, &["rev-parse", "--show-toplevel"]).await?;
    let root = PathBuf::from(String::from_utf8_lossy(&top).trim());
    let listed = git(
        dir,
        &[
            "status",
            "--porcelain=v1",
            "--branch",
            "--untracked-files=all",
            "-z",
            "--",
            ".",
        ],
    )
    .await?;
    let (branch, changed) = parse_status(&String::from_utf8_lossy(&listed));
    let (added, removed) = match git(dir, &["diff", "--numstat", "HEAD", "--", "."]).await {
        Some(numstat) => parse_numstat(&String::from_utf8_lossy(&numstat)),
        // No commit yet: everything is new, and nothing is counted.
        None => (0, 0),
    };
    // git names files from the repository's top; the folder may be inside it.
    let inside = dir
        .canonicalize()
        .ok()
        .and_then(|dir| {
            dir.strip_prefix(root.canonicalize().ok()?)
                .ok()
                .map(Path::to_path_buf)
        })
        .unwrap_or_default();
    let files: Vec<GitFile> = changed
        .into_iter()
        .filter_map(|(path, mark)| {
            let path = Path::new(&path).strip_prefix(&inside).ok()?;
            Some(GitFile {
                path: path.display().to_string(),
                mark,
            })
        })
        .collect();
    Some(GitStatus {
        summary: GitSummary {
            branch,
            files: files.len() as u32,
            added,
            removed,
        },
        root: root.display().to_string(),
        files,
    })
}

/// A file as the last commit has it. None when it isn't in it, or isn't text.
pub async fn original(path: &Path) -> Option<String> {
    let dir = path.parent()?;
    let name = path.file_name()?.to_str()?;
    let bytes = git(dir, &["show", &format!("HEAD:./{name}")]).await?;
    if bytes.contains(&0) {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// The branch from `status --branch`'s first entry, and each changed file
/// with its mark. Entries end in NUL; a rename's old name follows it.
fn parse_status(text: &str) -> (String, Vec<(String, GitMark)>) {
    let mut entries = text.split('\0').filter(|entry| !entry.is_empty());
    let mut branch = String::new();
    let mut files = Vec::new();
    while let Some(entry) = entries.next() {
        if let Some(head) = entry.strip_prefix("## ") {
            branch = branch_name(head);
            continue;
        }
        if entry.len() < 4 {
            continue;
        }
        let (code, path) = entry.split_at(3);
        let (x, y) = (code.as_bytes()[0], code.as_bytes()[1]);
        let mark = match (x, y) {
            (b'?', _) => GitMark::Untracked,
            (b'U', _) | (_, b'U') | (b'A', b'A') | (b'D', b'D') => GitMark::Conflicted,
            (b'R', _) | (b'C', _) => {
                entries.next();
                GitMark::Renamed
            }
            (b'A', _) => GitMark::Added,
            (b'D', _) | (_, b'D') => GitMark::Deleted,
            _ => GitMark::Modified,
        };
        files.push((path.to_string(), mark));
    }
    (branch, files)
}

/// `main...origin/main [ahead 1]` is on main; `HEAD (no branch)` is on no
/// branch; `No commits yet on main` is on main too.
fn branch_name(head: &str) -> String {
    let head = head.strip_prefix("No commits yet on ").unwrap_or(head);
    let head = head.split(" [").next().unwrap_or(head);
    match head.split_once("...") {
        Some((branch, _)) => branch.to_string(),
        None if head.starts_with("HEAD") => "detached".to_string(),
        None => head.to_string(),
    }
}

/// Lines added and removed, from `diff --numstat`. Binary files count none.
fn parse_numstat(text: &str) -> (u32, u32) {
    text.lines().fold((0, 0), |(added, removed), line| {
        let mut parts = line.split('\t');
        let count = |part: Option<&str>| part.and_then(|n| n.parse::<u32>().ok()).unwrap_or(0);
        (added + count(parts.next()), removed + count(parts.next()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_branch_and_the_files() {
        let text = "## fix/guest...origin/fix/guest [ahead 2]\0 M src/Order.java\0?? notes.md\0R  new.rs\0old.rs\0A  added.rs\0 D gone.rs\0UU both.rs\0";
        let (branch, files) = parse_status(text);
        assert_eq!(branch, "fix/guest");
        let marks: Vec<_> = files.iter().map(|(p, m)| (p.as_str(), *m)).collect();
        assert_eq!(
            marks,
            [
                ("src/Order.java", GitMark::Modified),
                ("notes.md", GitMark::Untracked),
                ("new.rs", GitMark::Renamed),
                ("added.rs", GitMark::Added),
                ("gone.rs", GitMark::Deleted),
                ("both.rs", GitMark::Conflicted),
            ]
        );
    }

    #[test]
    fn names_odd_branches() {
        assert_eq!(branch_name("main"), "main");
        assert_eq!(branch_name("No commits yet on main"), "main");
        assert_eq!(branch_name("HEAD (no branch)"), "detached");
    }

    #[test]
    fn counts_lines() {
        assert_eq!(
            parse_numstat("3\t1\ta.rs\n-\t-\tlogo.png\n10\t0\tb.rs\n"),
            (13, 1)
        );
    }

    #[tokio::test]
    async fn asks_a_real_repository() {
        if std::process::Command::new("git")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .unwrap()
        };
        run(&["init", "-q", "-b", "main"]);
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.txt"), "one\n").unwrap();
        run(&["add", "."]);
        run(&[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-qm",
            "first",
        ]);
        std::fs::write(dir.path().join("src/a.txt"), "one\ntwo\n").unwrap();
        std::fs::write(dir.path().join("src/b.txt"), "new\n").unwrap();

        let status = status(&dir.path().join("src")).await.unwrap();
        assert_eq!(status.summary.branch, "main");
        assert_eq!((status.summary.added, status.summary.removed), (1, 0));
        let files: Vec<_> = status
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.mark))
            .collect();
        assert_eq!(
            files,
            [("a.txt", GitMark::Modified), ("b.txt", GitMark::Untracked)]
        );
        assert_eq!(
            original(&dir.path().join("src/a.txt")).await.as_deref(),
            Some("one\n")
        );
        assert_eq!(original(&dir.path().join("src/b.txt")).await, None);
        let outside = tempfile::tempdir().unwrap();
        assert_eq!(super::status(outside.path()).await, None);
    }
}
