//! Read-only access to a git repository for comparing the workspace with a
//! branch, a commit, or a snapshot of an earlier moment.
//!
//! The repository is untrusted: its config may name filter drivers, hooks, or
//! an fsmonitor command. Every call here avoids running any of them, so files
//! are hashed and read as raw bytes and line-ending differences are left to
//! the line diff.
use crate::files;
use anyhow::{Context, Result, anyhow, bail, ensure};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    ffi::OsStr,
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime},
};

/// Checking out a whole working tree can take minutes on a large repository.
const WORKTREE_TIMEOUT: Duration = Duration::from_secs(600);

/// A hung git (a lock, a network filesystem) must not leave the view loading forever.
const GIT_TIMEOUT: Duration = Duration::from_secs(10);

/// A file modified this recently might change again within the timestamp's
/// resolution, so its hash is not reused.
const SETTLED: Duration = Duration::from_secs(2);

/// Inherited from a hook or wrapper script, these would point every call at another repository.
const REPOSITORY_ENV: [&str; 5] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_COMMON_DIR",
    "GIT_OBJECT_DIRECTORY",
];

#[derive(Clone, Debug)]
pub struct Repo {
    root: PathBuf,
    /// Object ids of files hashed before, by path, size and modification time.
    hashed: Arc<Mutex<HashMap<Vec<u8>, Hashed>>>,
}

#[derive(Debug)]
struct Hashed {
    size: u64,
    modified: SystemTime,
    id: String,
}

impl PartialEq for Repo {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root
    }
}

impl Eq for Repo {}

/// Something the workspace can be compared with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rev {
    Commit(String),
    /// A tree from [`Repo::snapshot`], which no commit or branch refers to.
    Tree(String),
}

/// Where the current branch left another branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Base {
    /// As found, such as "develop" or "origin/develop".
    pub branch: String,
    pub commit: String,
}

/// The author of each line of a committed file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blame {
    /// The file as committed.
    pub text: String,
    /// Author email per line of `text`.
    pub authors: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileChange {
    pub path: PathBuf,
    pub status: FileStatus,
    pub added: usize,
    pub removed: usize,
    /// Line counts are zero for binary files.
    pub binary: bool,
}

impl Rev {
    pub fn id(&self) -> &str {
        match self {
            Rev::Commit(id) | Rev::Tree(id) => id,
        }
    }

    /// Revisions are passed as arguments, so one must never read as an option.
    fn object(&self) -> Result<&str> {
        let id = self.id();
        ensure!(
            !id.is_empty() && !id.starts_with('-'),
            "Invalid git revision {id:?}"
        );
        Ok(id)
    }
}

impl Repo {
    /// `Ok(None)` when `path` is not inside a git working tree.
    pub fn discover(path: &Path) -> Result<Option<Repo>> {
        let path = plain_path(path);
        let directory = if path.is_dir() {
            path.as_path()
        } else {
            path.parent().context("Path has no parent directory")?
        };
        let inside = git(directory, &["rev-parse", "--is-inside-work-tree"])?;
        if !inside.status.success() || line(&inside.stdout) != b"true" {
            return Ok(None);
        }
        let top = succeeded(git(directory, &["rev-parse", "--show-toplevel"])?)?;
        let top = os_str(line(&top)).context("Repository path is not valid UTF-8")?;
        // Match the canonical paths the workspace uses, with native separators.
        let root = Path::new(top)
            .canonicalize()
            .with_context(|| format!("Cannot open repository {}", top.display()))?;
        Ok(Some(Repo {
            root: plain_path(&root),
            hashed: Arc::default(),
        }))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `None` until the first commit.
    pub fn head(&self) -> Result<Option<String>> {
        self.resolve("HEAD")
    }

    /// The merge base with `preferred` when given, otherwise as [`Self::default_base`].
    pub fn base(&self, preferred: Option<&str>) -> Result<Option<Base>> {
        match preferred.map(str::trim).filter(|b| !b.is_empty()) {
            Some(branch) => {
                ensure!(!branch.starts_with('-'), "Invalid branch name {branch:?}");
                // A misspelt setting falls back to the usual branches.
                match self.merge_base(branch)? {
                    Some(base) => Ok(Some(base)),
                    None => self.default_base(),
                }
            }
            None => self.default_base(),
        }
    }

    /// The merge base with the first of develop, main, or master that exists,
    /// preferring a local branch to its origin counterpart.
    pub fn default_base(&self) -> Result<Option<Base>> {
        for branch in ["develop", "main", "master"] {
            if let Some(base) = self.merge_base(branch)? {
                return Ok(Some(base));
            }
        }
        Ok(None)
    }

    /// The merge base of HEAD with `branch`, or with `origin/<branch>` when
    /// there is no local branch. `None` without commits or such a branch.
    pub fn merge_base(&self, branch: &str) -> Result<Option<Base>> {
        let Some(head) = self.head()? else {
            return Ok(None);
        };
        let candidates = [
            (branch.to_owned(), format!("refs/heads/{branch}")),
            (
                format!("origin/{branch}"),
                format!("refs/remotes/origin/{branch}"),
            ),
        ];
        for (name, reference) in candidates {
            let Some(tip) = self.resolve(&reference)? else {
                continue;
            };
            let output = git(&self.root, &["merge-base", &head, &tip])?;
            // Unrelated histories have no merge base.
            if output.status.code() == Some(1) {
                continue;
            }
            let commit = String::from_utf8_lossy(line(&succeeded(output)?)).into_owned();
            return Ok(Some(Base {
                branch: name,
                commit,
            }));
        }
        Ok(None)
    }

    /// Records the working tree as it is on disk as a tree object: tracked
    /// files, plus untracked files that are not ignored. The user's index and
    /// files are left alone, and only new objects are written.
    pub fn snapshot(&self) -> Result<String> {
        self.snapshot_once().or_else(|_| {
            // A remembered object may have been pruned since; hash everything again.
            self.hashed
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
            self.snapshot_once()
        })
    }

    fn snapshot_once(&self) -> Result<String> {
        let staged = self.staged()?;
        let untracked = self.git(&["ls-files", "-z", "--others", "--exclude-standard"])?;
        let paths: BTreeSet<&[u8]> = staged
            .keys()
            .map(Vec::as_slice)
            .chain(fields(&untracked))
            .collect();
        let now = SystemTime::now();
        let mut entries = Vec::new();
        let mut unhashed = Vec::new();
        {
            let hashed = self.hashed.lock().unwrap_or_else(|e| e.into_inner());
            for path in paths {
                let Some(full) = native_path(&self.root, path) else {
                    continue;
                };
                let staged_entry = staged.get(path);
                let Some(Entry { mode, id, stamp }) = entry(&full, staged_entry) else {
                    continue;
                };
                // Large untracked files (builds, data) cannot be compared,
                // and hashing them would copy them into the object store.
                if staged_entry.is_none()
                    && stamp.as_ref().is_some_and(|s| s.0 > files::MAX_FILE_BYTES)
                {
                    continue;
                }
                let known = stamp.as_ref().and_then(|(size, modified)| {
                    hashed
                        .get(path)
                        .filter(|h| h.size == *size && h.modified == *modified)
                        .map(|h| h.id.clone())
                });
                let id = id.or(known);
                if id.is_none() {
                    unhashed.push((path, stamp));
                }
                entries.push((mode, id, path));
            }
        }
        let paths: Vec<&[u8]> = unhashed.iter().map(|(path, _)| *path).collect();
        let hashes = self.hash(&paths)?;
        {
            let mut hashed = self.hashed.lock().unwrap_or_else(|e| e.into_inner());
            for ((path, stamp), id) in unhashed.iter().zip(&hashes) {
                let settled = stamp.as_ref().is_some_and(|(_, modified)| {
                    now.duration_since(*modified)
                        .is_ok_and(|age| age >= SETTLED)
                });
                if let (Some((size, modified)), true) = (stamp, settled) {
                    hashed.insert(
                        path.to_vec(),
                        Hashed {
                            size: *size,
                            modified: *modified,
                            id: id.clone(),
                        },
                    );
                }
            }
        }
        let mut hashes = hashes.into_iter();
        let mut info = Vec::new();
        for (mode, id, path) in entries {
            let id = match id {
                Some(id) => id,
                None => hashes
                    .next()
                    .context("git hashed fewer files than expected")?,
            };
            info.extend_from_slice(format!("{mode} {id}\t").as_bytes());
            info.extend_from_slice(path);
            info.push(0);
        }
        self.write_tree(info)
    }

    /// The file at `rev`, or `None` if it does not exist there. `path` may be
    /// absolute inside the repository or relative to its root.
    pub fn read(&self, rev: &Rev, path: &Path) -> Result<Option<String>> {
        let id = rev.object()?;
        let object = format!("{id}:{}", self.relative(path)?);
        // Asking for the size first avoids loading a huge blob only to reject it.
        let size = git(&self.root, &["cat-file", "-s", &object])?;
        if !size.status.success() {
            ensure!(
                git(&self.root, &["cat-file", "-e", id])?.status.success(),
                "Revision {id} is no longer in this repository"
            );
            return Ok(None);
        }
        let size: u64 = String::from_utf8_lossy(line(&size.stdout))
            .parse()
            .context("Unexpected git cat-file output")?;
        ensure!(
            size <= files::MAX_FILE_BYTES,
            "The prototype supports files up to 4 MiB"
        );
        files::decode_text(self.git(&["cat-file", "blob", &object])?).map(Some)
    }

    /// Every file that differs between `base` and the working tree, by path.
    pub fn changes(&self, base: &Rev) -> Result<Vec<FileChange>> {
        let current = Rev::Tree(self.snapshot()?);
        self.changes_between(base, &current)
    }

    /// Every file that differs between two revisions, by path.
    pub fn changes_between(&self, base: &Rev, current: &Rev) -> Result<Vec<FileChange>> {
        let base = base.object()?;
        let current = current.object()?;
        let output = self.git(&[
            "diff-tree",
            "-r",
            "-z",
            "--no-renames",
            "--no-ext-diff",
            "--no-textconv",
            "--ignore-cr-at-eol",
            "--raw",
            "--numstat",
            base,
            current,
        ])?;
        let mut statuses = BTreeMap::new();
        let mut counts = BTreeMap::new();
        let mut records = fields(&output);
        while let Some(record) = records.next() {
            if let Some(raw) = record.strip_prefix(b":") {
                let path = records.next().context("Unexpected git diff output")?;
                let status = match raw.last() {
                    Some(b'A') => FileStatus::Added,
                    Some(b'D') => FileStatus::Deleted,
                    _ => FileStatus::Modified,
                };
                statuses.insert(path, status);
            } else {
                let mut parts = record.splitn(3, |&byte| byte == b'\t');
                let (Some(added), Some(removed), Some(path)) =
                    (parts.next(), parts.next(), parts.next())
                else {
                    bail!("Unexpected git diff output");
                };
                // Binary files report "-" for both counts.
                counts.insert(path, (count(added), count(removed)));
            }
        }
        let mut changes: Vec<FileChange> = statuses
            .into_iter()
            .filter_map(|(path, status)| {
                // Git counts no lines for a file whose only difference is
                // line endings; the line diff ignores those too, so it is
                // not a change.
                let (added, removed) = match counts.get(path).copied() {
                    Some((Some(0), Some(0))) if status == FileStatus::Modified => return None,
                    Some(counts) => counts,
                    None if status == FileStatus::Modified => return None,
                    None => (Some(0), Some(0)),
                };
                Some(FileChange {
                    path: native_path(&self.root, path)?,
                    status,
                    added: added.unwrap_or(0),
                    removed: removed.unwrap_or(0),
                    binary: added.is_none() || removed.is_none(),
                })
            })
            .collect();
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(changes)
    }

    /// Checks out HEAD as a new branch in a new working tree at `path`, which
    /// must not exist yet. Uncommitted changes stay where they are.
    pub fn add_worktree(&self, path: &Path, branch: &str) -> Result<()> {
        ensure!(!path.exists(), "{} already exists", path.display());
        ensure!(
            !branch.starts_with('-') && !branch.is_empty(),
            "Invalid branch name {branch:?}"
        );
        // A checkout runs the content filters a repository's own config
        // names; filters from the user's global config are theirs to trust.
        let local = git(
            &self.root,
            &[
                "config",
                "--local",
                "--name-only",
                "--get-regexp",
                r"^filter\.",
            ],
        )?;
        ensure!(
            local.stdout.trim_ascii().is_empty(),
            "This repository's own config defines content filters; create the worktree from a terminal"
        );
        let target = plain_path(path);
        let target = target
            .to_str()
            .context("Worktree path is not valid UTF-8")?;
        // A large checkout takes far longer than a query.
        let added = run_for(
            command(
                &self.root,
                &["worktree", "add", "-q", "-b", branch, "--", target, "HEAD"],
            ),
            None,
            WORKTREE_TIMEOUT,
            "git",
        )
        .and_then(succeeded);
        if let Err(err) = added {
            // Leave nothing half made behind.
            let _ = git(&self.root, &["worktree", "remove", "--force", "--", target]);
            let _ = git(&self.root, &["worktree", "prune"]);
            let _ = git(&self.root, &["branch", "-D", "--", branch]);
            let _ = fs::remove_dir_all(path);
            return Err(err);
        }
        Ok(())
    }

    /// The email commits made here are recorded with, if set.
    pub fn user_email(&self) -> Result<Option<String>> {
        let output = git(&self.root, &["config", "user.email"])?;
        Ok(output
            .status
            .success()
            .then(|| String::from_utf8_lossy(line(&output.stdout)).into_owned())
            .filter(|email| !email.is_empty()))
    }

    /// Who last changed each line of `path` as committed at HEAD. Only
    /// committed content is read, so no filters run on the working tree.
    /// `None` when HEAD does not have the file.
    pub fn blame(&self, path: &Path) -> Result<Option<Blame>> {
        let Some(head) = self.head()? else {
            return Ok(None);
        };
        let head = Rev::Commit(head);
        let Some(text) = self.read(&head, path)? else {
            return Ok(None);
        };
        let relative = self.relative(path)?;
        let output = self.git(&[
            "blame",
            "--porcelain",
            "--no-textconv",
            head.object()?,
            "--",
            &relative,
        ])?;
        let mut emails: BTreeMap<&[u8], String> = BTreeMap::new();
        let mut authors = Vec::new();
        let mut commit: &[u8] = b"";
        for record in output.split(|&byte| byte == b'\n') {
            if record.first() == Some(&b'\t') {
                authors.push(emails.get(commit).cloned().unwrap_or_default());
            } else if let Some(mail) = record.strip_prefix(b"author-mail ") {
                let mail = String::from_utf8_lossy(mail);
                let mail = mail.trim().trim_start_matches('<').trim_end_matches('>');
                emails.insert(commit, mail.to_owned());
            } else if let Some(id) = record.split(|&byte| byte == b' ').next()
                && id.len() >= 40
                && id.iter().all(u8::is_ascii_hexdigit)
            {
                commit = id;
            }
        }
        Ok(Some(Blame { text, authors }))
    }

    fn git(&self, args: &[&str]) -> Result<Vec<u8>> {
        succeeded(git(&self.root, args)?)
    }

    fn resolve(&self, name: &str) -> Result<Option<String>> {
        let commit = format!("{name}^{{commit}}");
        let output = git(&self.root, &["rev-parse", "--verify", "--quiet", &commit])?;
        Ok(output
            .status
            .success()
            .then(|| String::from_utf8_lossy(line(&output.stdout)).into_owned()))
    }

    /// The staged version of a file and its mode, if the index has it.
    pub fn staged_text(&self, path: &Path) -> Result<Option<(String, String)>> {
        let relative = self.relative(path)?;
        let output = self.git(&["ls-files", "-z", "--stage", "--", &relative])?;
        let Some(record) = fields(&output).next() else {
            return Ok(None);
        };
        let meta = String::from_utf8_lossy(record);
        let mut meta = meta.split([' ', '\t']);
        let (Some(mode), Some(id)) = (meta.next(), meta.next()) else {
            bail!("Unexpected git ls-files output");
        };
        let text = files::decode_text(self.git(&["cat-file", "blob", id])?)?;
        Ok(Some((mode.to_owned(), text)))
    }

    /// Stages `text` as the file's content, leaving the working tree alone.
    /// Files that git transforms when staging (filters such as LFS, or
    /// working-tree encodings) are refused: their staged form is not the text.
    pub fn stage_text(&self, path: &Path, mode: &str, text: &str) -> Result<()> {
        let relative = self.relative(path)?;
        ensure!(
            matches!(mode, "100644" | "100755"),
            "Only regular files can be staged here"
        );
        let attributes = self.git(&[
            "check-attr",
            "filter",
            "working-tree-encoding",
            "--",
            &relative,
        ])?;
        let transformed = String::from_utf8_lossy(&attributes)
            .lines()
            .any(|line| !line.ends_with(": unspecified"));
        ensure!(
            !transformed,
            "Git transforms this file when staging; stage it from the terminal"
        );
        let id = succeeded(run(
            command(
                &self.root,
                &["hash-object", "-w", "--no-filters", "--stdin"],
            ),
            Some(text.as_bytes().to_vec()),
        )?)?;
        let id = String::from_utf8_lossy(line(&id)).into_owned();
        let entry = format!("{mode},{id},{relative}");
        self.git(&["update-index", "--cacheinfo", &entry])?;
        Ok(())
    }

    /// Index entries by path, so files git cannot hash from disk keep their recorded version.
    fn staged(&self) -> Result<BTreeMap<Vec<u8>, Staged>> {
        let output = self.git(&["ls-files", "-z", "--stage"])?;
        let mut staged = BTreeMap::new();
        for record in fields(&output) {
            // "<mode> <id> <stage>\t<path>"
            let tab = record
                .iter()
                .position(|&byte| byte == b'\t')
                .context("Unexpected git ls-files output")?;
            let meta = String::from_utf8_lossy(&record[..tab]);
            let mut meta = meta.split(' ');
            let (Some(mode), Some(id)) = (meta.next(), meta.next()) else {
                bail!("Unexpected git ls-files output");
            };
            staged.insert(
                record[tab + 1..].to_vec(),
                Staged {
                    mode: mode.to_owned(),
                    id: id.to_owned(),
                },
            );
        }
        Ok(staged)
    }

    /// Hashes all files in one process. `--no-filters` stores the bytes on disk
    /// instead of running the repository's clean filters.
    fn hash(&self, paths: &[&[u8]]) -> Result<Vec<String>> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let mut input = Vec::new();
        for path in paths {
            stdin_path(path, &mut input);
        }
        let output = succeeded(run(
            command(
                &self.root,
                &["hash-object", "-w", "--no-filters", "--stdin-paths"],
            ),
            Some(input),
        )?)?;
        let hashes: Vec<String> = String::from_utf8_lossy(&output)
            .lines()
            .map(str::to_owned)
            .collect();
        ensure!(
            hashes.len() == paths.len(),
            "git hashed {} of {} files",
            hashes.len(),
            paths.len()
        );
        Ok(hashes)
    }

    /// Builds the tree in a throwaway index so the user's staging area is untouched.
    fn write_tree(&self, info: Vec<u8>) -> Result<String> {
        let directory = tempfile::tempdir().context("Could not create a temporary index")?;
        let index = directory.path().join("index");
        // A split index would leave shared index files in the repository.
        let mut update = command(
            &self.root,
            &[
                "-c",
                "core.splitIndex=false",
                "update-index",
                "-z",
                "--index-info",
            ],
        );
        update.env("GIT_INDEX_FILE", &index);
        succeeded(run(update, Some(info))?)?;
        let mut write = command(&self.root, &["write-tree"]);
        write.env("GIT_INDEX_FILE", &index);
        let tree = succeeded(run(write, None)?)?;
        Ok(String::from_utf8_lossy(line(&tree)).into_owned())
    }

    /// A forward-slash path relative to the root, as git names files.
    fn relative(&self, path: &Path) -> Result<String> {
        let path = plain_path(path);
        let relative = if path.is_absolute() {
            self.strip_root(&path)?
        } else {
            path
        };
        let mut parts = Vec::new();
        for component in relative.components() {
            match component {
                Component::Normal(part) => parts.push(
                    part.to_str()
                        .context("Git comparison needs UTF-8 file names")?,
                ),
                Component::CurDir => {}
                _ => bail!("{} is outside the repository", relative.display()),
            }
        }
        ensure!(!parts.is_empty(), "Not a file in the repository");
        Ok(parts.join("/"))
    }

    fn strip_root(&self, path: &Path) -> Result<PathBuf> {
        if let Ok(relative) = path.strip_prefix(&self.root) {
            return Ok(relative.to_path_buf());
        }
        // Another spelling of the same place, such as different case or a
        // symlinked directory. The file itself may have been deleted.
        let outside = || anyhow!("{} is outside the repository", path.display());
        let parent = path.parent().and_then(|parent| parent.canonicalize().ok());
        let (Some(parent), Some(name)) = (parent, path.file_name()) else {
            return Err(outside());
        };
        plain_path(&parent)
            .join(name)
            .strip_prefix(&self.root)
            .map(Path::to_path_buf)
            .map_err(|_| outside())
    }
}

struct Staged {
    mode: String,
    id: String,
}

/// One snapshot path: its mode, its object unless git must hash it, and for
/// regular files the size and modification time.
struct Entry {
    mode: String,
    id: Option<String>,
    stamp: Option<(u64, SystemTime)>,
}

/// Deleted files and anything else without a usable version are skipped.
fn entry(full: &Path, staged: Option<&Staged>) -> Option<Entry> {
    let metadata = fs::symlink_metadata(full).ok()?;
    let kind = metadata.file_type();
    if kind.is_file() {
        return Some(Entry {
            mode: file_mode(&metadata, staged).to_owned(),
            id: None,
            stamp: metadata.modified().ok().map(|m| (metadata.len(), m)),
        });
    }
    // Hashing would follow a symlink or fail on a submodule, so keep what the
    // index records for them rather than reporting them deleted.
    let staged = staged?;
    let kept = (kind.is_symlink() && staged.mode == "120000")
        || (kind.is_dir() && staged.mode == "160000");
    kept.then(|| Entry {
        mode: staged.mode.clone(),
        id: Some(staged.id.clone()),
        stamp: None,
    })
}

#[cfg(unix)]
fn file_mode(metadata: &fs::Metadata, _staged: Option<&Staged>) -> &'static str {
    use std::os::unix::fs::PermissionsExt;
    if metadata.permissions().mode() & 0o100 != 0 {
        "100755"
    } else {
        "100644"
    }
}

/// Windows has no executable bit, and checks out symlinks as plain files
/// unless enabled, so keep what the index records as git itself does.
#[cfg(not(unix))]
fn file_mode(_metadata: &fs::Metadata, staged: Option<&Staged>) -> &'static str {
    match staged.map(|staged| staged.mode.as_str()) {
        Some("100755") => "100755",
        Some("120000") => "120000",
        _ => "100644",
    }
}

/// `--stdin-paths` reads lines and unquotes C-style quoted ones, so names
/// containing line breaks, or starting with a quote, must be quoted.
fn stdin_path(path: &[u8], input: &mut Vec<u8>) {
    let plain = !path.starts_with(b"\"") && !path.iter().any(|&b| b == b'\n' || b == b'\r');
    if plain {
        input.extend_from_slice(path);
    } else {
        input.push(b'"');
        for &byte in path {
            match byte {
                b'"' | b'\\' => input.extend_from_slice(&[b'\\', byte]),
                b'\n' => input.extend_from_slice(b"\\n"),
                b'\r' => input.extend_from_slice(b"\\r"),
                _ => input.push(byte),
            }
        }
        input.push(b'"');
    }
    input.push(b'\n');
}

fn count(field: &[u8]) -> Option<usize> {
    std::str::from_utf8(field).ok()?.parse().ok()
}

fn fields(output: &[u8]) -> impl Iterator<Item = &[u8]> {
    output
        .split(|&byte| byte == 0)
        .filter(|field| !field.is_empty())
}

fn line(output: &[u8]) -> &[u8] {
    let output = output.strip_suffix(b"\n").unwrap_or(output);
    output.strip_suffix(b"\r").unwrap_or(output)
}

/// Git prints paths relative to the root, separated by `/`.
fn native_path(root: &Path, path: &[u8]) -> Option<PathBuf> {
    let mut full = root.to_path_buf();
    for part in path.split(|&byte| byte == b'/') {
        full.push(os_str(part)?);
    }
    Some(full)
}

#[cfg(unix)]
fn os_str(bytes: &[u8]) -> Option<&OsStr> {
    use std::os::unix::ffi::OsStrExt;
    Some(OsStr::from_bytes(bytes))
}

/// Git for Windows reads and prints paths as UTF-8.
#[cfg(not(unix))]
fn os_str(bytes: &[u8]) -> Option<&OsStr> {
    std::str::from_utf8(bytes).ok().map(OsStr::new)
}

fn succeeded(output: Output) -> Result<Vec<u8>> {
    ensure!(
        output.status.success(),
        "git: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(output.stdout)
}

pub(crate) fn git(directory: &Path, args: &[&str]) -> Result<Output> {
    run(command(directory, args), None)
}

/// Never lets the repository's config start a program: fsmonitor is off and
/// hooks are looked up where none can exist. Filters and diff drivers are
/// avoided per command with raw reads and `--no-filters`/`--no-textconv`.
fn command(directory: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .args([
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            // A partial clone would fetch missing objects, running whatever
            // transport command the repository's config names.
            "-c",
            "protocol.allow=never",
        ])
        .env("GIT_NO_LAZY_FETCH", "1")
        .arg("-C")
        .arg(directory)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in REPOSITORY_ENV {
        command.env_remove(name);
    }
    #[cfg(windows)]
    {
        // Avoid flashing a console window from the GUI process.
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

fn run(command: Command, input: Option<Vec<u8>>) -> Result<Output> {
    run_for(command, input, GIT_TIMEOUT, "git")
}

/// Runs a program to completion, stopping it after `timeout`.
pub(crate) fn run_for(
    mut command: Command,
    input: Option<Vec<u8>>,
    timeout: Duration,
    name: &str,
) -> Result<Output> {
    command.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let mut child = command
        .spawn()
        .with_context(|| format!("Could not run {name}. Is it installed?"))?;
    // Feed and drain the pipes on threads so a full pipe cannot stall the wait.
    let stdin = child.stdin.take();
    let writer = input.map(|bytes| {
        thread::spawn(move || {
            if let Some(mut stdin) = stdin {
                // A failed write shows up as git's exit status.
                let _ = stdin.write_all(&bytes);
            }
        })
    });
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let stdout = drain(child.stdout.take().map(|p| Box::new(p) as _));
    let stderr = drain(child.stderr.take().map(|p| Box::new(p) as _));
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "{name} did not respond within {} seconds",
                timeout.as_secs()
            );
        }
        thread::sleep(Duration::from_millis(10));
    };
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    Ok(Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

/// Git cannot use the `\\?\` paths that `canonicalize` returns on Windows.
pub(crate) fn plain_path(path: &Path) -> PathBuf {
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

    const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

    fn run(root: &Path, args: &[&str]) -> String {
        let output = git(root, args).unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn init() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = plain_path(&dir.path().canonicalize().unwrap());
        run(&root, &["init", "-q", "-b", "main"]);
        run(&root, &["config", "core.autocrlf", "false"]);
        (dir, root)
    }

    fn commit(root: &Path, message: &str) -> String {
        run(root, &["add", "-A"]);
        run(
            root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.com",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-qm",
                message,
            ],
        );
        run(root, &["rev-parse", "HEAD"])
    }

    fn write(root: &Path, path: &str, text: &str) {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn open(root: &Path) -> Repo {
        Repo::discover(root).unwrap().unwrap()
    }

    #[test]
    fn discover_finds_the_root_only_inside_a_work_tree() {
        let (_dir, root) = init();
        write(&root, "src/a.rs", "");
        assert_eq!(open(&root.join("src/a.rs")).root(), root);
        assert_eq!(open(&root.join("src")).root(), root);
        assert_eq!(Repo::discover(&root.join(".git")).unwrap(), None);
        let outside = tempfile::tempdir().unwrap();
        // The temp directory could sit inside a repository on a developer machine.
        if !git(outside.path(), &["rev-parse"])
            .unwrap()
            .status
            .success()
        {
            assert_eq!(Repo::discover(outside.path()).unwrap(), None);
        }
    }

    #[test]
    fn a_repository_without_commits_has_no_head_or_base() {
        let (_dir, root) = init();
        let repo = open(&root);
        assert_eq!(repo.head().unwrap(), None);
        assert_eq!(repo.default_base().unwrap(), None);
        assert_eq!(repo.snapshot().unwrap(), EMPTY_TREE);
    }

    #[test]
    fn default_base_prefers_develop_and_returns_the_merge_base() {
        let (_dir, root) = init();
        write(&root, "a.txt", "a\n");
        let on_main = commit(&root, "main");
        run(&root, &["checkout", "-q", "-b", "develop"]);
        write(&root, "b.txt", "b\n");
        let on_develop = commit(&root, "develop");
        run(&root, &["checkout", "-q", "-b", "feature"]);
        write(&root, "c.txt", "c\n");
        commit(&root, "feature");
        let repo = open(&root);
        let base = |branch: &str, commit: &str| {
            Some(Base {
                branch: branch.into(),
                commit: commit.into(),
            })
        };
        assert_eq!(repo.default_base().unwrap(), base("develop", &on_develop));
        assert_eq!(repo.merge_base("main").unwrap(), base("main", &on_main));
        assert_eq!(repo.merge_base("missing").unwrap(), None);
        // Without a local develop, its origin counterpart still wins over main.
        run(
            &root,
            &["update-ref", "refs/remotes/origin/develop", &on_develop],
        );
        run(&root, &["branch", "-q", "-D", "develop"]);
        assert_eq!(
            repo.default_base().unwrap(),
            base("origin/develop", &on_develop)
        );
    }

    #[test]
    fn snapshot_records_the_working_tree_without_touching_the_index() {
        let (_dir, root) = init();
        write(&root, ".gitignore", "*.log\n");
        write(&root, "kept.txt", "kept\n");
        write(&root, "edited.txt", "old\n");
        write(&root, "gone.txt", "gone\n");
        let head = commit(&root, "init");
        write(&root, "edited.txt", "new\n");
        fs::remove_file(root.join("gone.txt")).unwrap();
        write(&root, "new.txt", "new\n");
        write(&root, "dir/inner.txt", "inner\n");
        write(&root, "debug.log", "ignored\n");
        write(&root, "staged.txt", "staged\n");
        run(&root, &["add", "staged.txt"]);
        let status = || run(&root, &["--no-optional-locks", "status", "--porcelain"]);
        let listing = || {
            let mut names: Vec<_> = fs::read_dir(root.join(".git"))
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            names.sort();
            names
        };
        let (status_before, index_before, listing_before) = (
            status(),
            fs::read(root.join(".git/index")).unwrap(),
            listing(),
        );

        let repo = open(&root);
        let tree = repo.snapshot().unwrap();

        assert_eq!(status(), status_before);
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index_before);
        assert_eq!(listing(), listing_before);
        assert_eq!(
            run(&root, &["ls-tree", "-r", "--name-only", &tree]),
            ".gitignore\ndir/inner.txt\nedited.txt\nkept.txt\nnew.txt\nstaged.txt"
        );
        let snapshot = Rev::Tree(tree);
        let read = |rev: &Rev, path: &Path| repo.read(rev, path).unwrap();
        assert_eq!(read(&snapshot, &root.join("edited.txt")).unwrap(), "new\n");
        assert_eq!(
            read(&snapshot, Path::new("dir/inner.txt")).unwrap(),
            "inner\n"
        );
        assert_eq!(read(&snapshot, &root.join("gone.txt")), None);
        assert_eq!(
            read(&Rev::Commit(head), &root.join("edited.txt")).unwrap(),
            "old\n"
        );
        assert!(repo.read(&snapshot, &root.join("../outside.txt")).is_err());
    }

    #[test]
    fn snapshot_reuses_settled_hashes_and_skips_large_untracked_files() {
        let (_dir, root) = init();
        write(&root, "a.txt", "one\n");
        commit(&root, "base");
        let old = std::time::SystemTime::now() - Duration::from_secs(60);
        fs::File::options()
            .write(true)
            .open(root.join("a.txt"))
            .unwrap()
            .set_modified(old)
            .unwrap();
        let big = "x".repeat(files::MAX_FILE_BYTES as usize + 1);
        write(&root, "big.bin", &big);
        let repo = open(&root);
        let first = repo.snapshot().unwrap();
        assert_eq!(repo.hashed.lock().unwrap().len(), 1);
        assert_eq!(repo.snapshot().unwrap(), first);
        assert_eq!(repo.changes(&Rev::Tree(first.clone())).unwrap(), []);
        let listed = succeeded(git(&root, &["ls-tree", "--name-only", &first]).unwrap()).unwrap();
        assert_eq!(String::from_utf8_lossy(&listed).trim(), "a.txt");
        // A changed file has a new size or time, so it is hashed again.
        write(&root, "a.txt", "two!\n");
        assert_ne!(repo.snapshot().unwrap(), first);
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_keeps_the_executable_bit() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, root) = init();
        write(&root, "run.sh", "#!/bin/sh\n");
        fs::set_permissions(root.join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
        let tree = open(&root).snapshot().unwrap();
        assert!(run(&root, &["ls-tree", &tree, "run.sh"]).starts_with("100755 "));
    }

    #[test]
    fn changes_report_status_and_line_counts_against_a_base() {
        let (_dir, root) = init();
        write(&root, "edit.txt", "a\nb\nc\n");
        write(&root, "remove.txt", "x\ny\n");
        write(&root, "same.txt", "same\n");
        let base = Rev::Commit(commit(&root, "base"));
        write(&root, "edit.txt", "a\nB\nc\nd\n");
        fs::remove_file(root.join("remove.txt")).unwrap();
        write(&root, "new.txt", "1\n2\n3\n");
        fs::write(root.join("bin.dat"), [0, 1, 2, 0]).unwrap();
        let change = |path: &str, status, added, removed, binary| FileChange {
            path: root.join(path),
            status,
            added,
            removed,
            binary,
        };
        let repo = open(&root);
        assert_eq!(
            repo.changes(&base).unwrap(),
            [
                change("bin.dat", FileStatus::Added, 0, 0, true),
                change("edit.txt", FileStatus::Modified, 2, 1, false),
                change("new.txt", FileStatus::Added, 3, 0, false),
                change("remove.txt", FileStatus::Deleted, 0, 2, false),
            ]
        );
        let now = Rev::Tree(repo.snapshot().unwrap());
        assert_eq!(repo.changes(&now).unwrap(), []);
    }

    #[test]
    fn blame_names_the_author_of_each_committed_line() {
        let (_dir, root) = init();
        write(&root, "a.txt", "one\ntwo\n");
        commit(&root, "base");
        write(&root, "a.txt", "one\nTWO\nthree\n");
        let ok = |args: &[&str]| succeeded(git(&root, args).unwrap()).unwrap();
        ok(&["add", "-A"]);
        ok(&[
            "-c",
            "user.email=other@example.com",
            "-c",
            "user.name=Other",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "theirs",
        ]);
        write(&root, "a.txt", "uncommitted\n");
        run(&root, &["config", "user.email", "test@example.com"]);
        let repo = open(&root);
        let blame = repo.blame(&root.join("a.txt")).unwrap().unwrap();
        assert_eq!(blame.text, "one\nTWO\nthree\n");
        assert_eq!(
            repo.user_email().unwrap().as_deref(),
            Some("test@example.com")
        );
        assert_eq!(
            blame.authors,
            ["test@example.com", "other@example.com", "other@example.com"]
        );
        assert_eq!(repo.blame(&root.join("missing.txt")).unwrap(), None);
    }

    #[test]
    fn worktrees_start_from_head_on_a_new_branch() {
        let (_dir, root) = init();
        write(
            &root, "a.txt", "one
",
        );
        commit(&root, "base");
        write(
            &root,
            "a.txt",
            "uncommitted
",
        );
        let elsewhere = tempfile::tempdir().unwrap();
        let tree = elsewhere.path().join("pi-1");
        let repo = open(&root);
        repo.add_worktree(&tree, "pi/1").unwrap();
        assert_eq!(
            fs::read_to_string(tree.join("a.txt")).unwrap(),
            "one
"
        );
        assert_eq!(run(&tree, &["branch", "--show-current"]), "pi/1");
        assert!(repo.add_worktree(&tree, "pi/2").is_err());
        assert!(
            repo.add_worktree(&elsewhere.path().join("x"), "-x")
                .is_err()
        );
        run(&root, &["config", "filter.evil.smudge", "evil"]);
        let refused = elsewhere.path().join("pi-3");
        assert!(repo.add_worktree(&refused, "pi/3").is_err());
        assert!(!refused.exists());
    }

    #[test]
    fn staging_text_changes_only_the_index() {
        let (_dir, root) = init();
        write(&root, "a.txt", "one\ntwo\n");
        commit(&root, "base");
        write(&root, "a.txt", "one\nTWO\nthree\n");
        let repo = open(&root);
        let path = root.join("a.txt");
        let (mode, text) = repo.staged_text(&path).unwrap().unwrap();
        assert_eq!((mode.as_str(), text.as_str()), ("100644", "one\ntwo\n"));
        repo.stage_text(&path, &mode, "one\nTWO\n").unwrap();
        assert_eq!(repo.staged_text(&path).unwrap().unwrap().1, "one\nTWO\n");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "one\nTWO\nthree\n",
            "the working tree is untouched"
        );
        assert_eq!(repo.staged_text(&root.join("new.txt")).unwrap(), None);
        write(&root, ".gitattributes", "*.lfs filter=lfs\n");
        write(&root, "b.lfs", "x\n");
        run(&root, &["add", "b.lfs", ".gitattributes"]);
        assert!(
            repo.stage_text(&root.join("b.lfs"), "100644", "y\n")
                .is_err()
        );
    }

    #[test]
    fn changes_between_compares_two_snapshots() {
        let (_dir, root) = init();
        write(&root, "a.txt", "one\n");
        commit(&root, "base");
        let repo = open(&root);
        let before = Rev::Tree(repo.snapshot().unwrap());
        write(&root, "a.txt", "one\ntwo\n");
        write(&root, "b.txt", "new\n");
        let after = Rev::Tree(repo.snapshot().unwrap());
        let changes = repo.changes_between(&before, &after).unwrap();
        let summary: Vec<_> = changes
            .iter()
            .map(|c| (c.path.file_name().unwrap().to_owned(), c.status, c.added))
            .collect();
        assert_eq!(
            summary,
            [
                ("a.txt".into(), FileStatus::Modified, 1),
                ("b.txt".into(), FileStatus::Added, 1)
            ]
        );
    }

    #[test]
    fn line_ending_only_differences_are_not_changes() {
        let (_dir, root) = init();
        write(&root, "a.txt", "one\ntwo\n");
        let base = Rev::Commit(commit(&root, "base"));
        write(&root, "a.txt", "one\r\ntwo\r\n");
        assert_eq!(open(&root).changes(&base).unwrap(), []);
        write(&root, "a.txt", "one\r\nTWO\r\n");
        let changes = open(&root).changes(&base).unwrap();
        assert_eq!((changes[0].added, changes[0].removed), (1, 1));
    }

    #[test]
    fn repository_filter_drivers_never_run() {
        let (_dir, root) = init();
        write(&root, ".gitattributes", "* filter=evil\n");
        write(&root, "a.txt", "text\n");
        let head = Rev::Commit(commit(&root, "init"));
        run(&root, &["config", "filter.evil.clean", "touch ran-clean"]);
        run(&root, &["config", "filter.evil.smudge", "touch ran-smudge"]);
        write(&root, "a.txt", "edited\n");
        write(&root, "b.txt", "new\n");

        let repo = open(&root);
        let snapshot = Rev::Tree(repo.snapshot().unwrap());
        assert_eq!(
            repo.read(&head, Path::new("a.txt")).unwrap().unwrap(),
            "text\n"
        );
        assert_eq!(
            repo.read(&snapshot, Path::new("a.txt")).unwrap().unwrap(),
            "edited\n"
        );
        assert_eq!(repo.changes(&head).unwrap().len(), 2);
        assert!(!root.join("ran-clean").exists());
        assert!(!root.join("ran-smudge").exists());
        // The filter is live: an ordinary `git add` runs it.
        run(&root, &["add", "a.txt"]);
        assert!(root.join("ran-clean").exists());
    }

    #[test]
    fn unusual_names_are_quoted_for_stdin_paths() {
        let quoted = |path: &[u8]| {
            let mut input = Vec::new();
            stdin_path(path, &mut input);
            input
        };
        assert_eq!(quoted(b"dir/a b\\c"), b"dir/a b\\c\n");
        assert_eq!(quoted(b"a\nb\r"), b"\"a\\nb\\r\"\n");
        assert_eq!(quoted(b"\"q\\"), b"\"\\\"q\\\\\"\n");
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
