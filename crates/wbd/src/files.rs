//! Files: one folder read for the tree, every file listed to find one by
//! name, a file read for the editor, and saved from it unless something else
//! changed it first.

use std::io::Read;
use std::path::Path;
use std::time::UNIX_EPOCH;

use protocol::{FileContent, FileView};

/// More than this of a text file isn't sent.
const TEXT_LIMIT: usize = 512 * 1024;

/// No more paths than this are listed.
const LIST_LIMIT: usize = 100_000;

/// Folders the walk of a folder outside git skips, as nobody means a file in them.
const SKIPPED: [&str; 7] = [
    ".git",
    "node_modules",
    "target",
    "__pycache__",
    "venv",
    "dist",
    "build",
];

/// What `dir` holds directly, as names: its folders and its files, each
/// sorted without regard to case. A link to a folder is a folder. Only `.git`
/// is left out.
pub fn entries(dir: &Path) -> std::io::Result<(Vec<String>, Vec<String>)> {
    let (mut folders, mut files) = (Vec::new(), Vec::new());
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let kind = entry.file_type()?;
        // Only a link needs a second look, at what it points to.
        if kind.is_dir() || (kind.is_symlink() && entry.path().is_dir()) {
            folders.push(name);
        } else {
            files.push(name);
        }
    }
    for names in [&mut folders, &mut files] {
        // Sorted twice so that `A` and `a` always come in the same order.
        names.sort();
        names.sort_by_cached_key(|name| name.to_lowercase());
    }
    Ok((folders, files))
}

/// Every file under `dir`, as paths relative to it, sorted. In a git
/// repository these are the files git tracks or would track, so only what
/// the repository ignores is left out. Elsewhere the folder is walked. True
/// when there were more than [`LIST_LIMIT`].
pub async fn list(dir: &Path) -> (Vec<String>, bool) {
    let mut paths = match crate::git::files(dir).await {
        Some(paths) => paths,
        None => {
            let dir = dir.to_path_buf();
            let walking = tokio::task::spawn_blocking(move || walk(&dir));
            walking.await.unwrap_or_default()
        }
    };
    paths.sort();
    // git names a file in a conflict once for each side.
    paths.dedup();
    let truncated = paths.len() > LIST_LIMIT;
    paths.truncate(LIST_LIMIT);
    (paths, truncated)
}

/// The files under `dir`, skipping only the [`SKIPPED`] folders. Stops at
/// one more than [`LIST_LIMIT`].
fn walk(dir: &Path) -> Vec<String> {
    let mut paths = Vec::new();
    let mut folders = vec![dir.to_path_buf()];
    while let Some(folder) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                if !SKIPPED.contains(&entry.file_name().to_string_lossy().as_ref()) {
                    folders.push(path);
                }
            } else if let Ok(relative) = path.strip_prefix(dir) {
                paths.push(relative.display().to_string());
                if paths.len() > LIST_LIMIT {
                    return paths;
                }
            }
        }
    }
    paths
}

pub fn read(path: &Path) -> std::io::Result<FileView> {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileView {
                path: path.display().to_string(),
                content: FileContent::Missing,
                modified: 0,
            });
        }
        Err(err) => return Err(err),
    };
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(TEXT_LIMIT as u64)
        .read_to_end(&mut bytes)?;
    let content = if bytes.contains(&0) {
        FileContent::Binary { size: meta.len() }
    } else {
        FileContent::Text {
            text: String::from_utf8_lossy(&bytes).into_owned(),
            truncated: meta.len() > TEXT_LIMIT as u64,
        }
    };
    Ok(FileView {
        path: path.display().to_string(),
        content,
        modified: modified(&meta),
    })
}

fn modified(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_millis() as u64)
}

/// When each file last changed; 0 for one that's gone.
pub fn stat(paths: &[String]) -> Vec<u64> {
    paths
        .iter()
        .map(|path| std::fs::metadata(path).map_or(0, |meta| modified(&meta)))
        .collect()
}

/// Why a save didn't happen.
#[derive(Debug)]
pub enum Unsaved {
    /// The file no longer holds what the editor read.
    Changed,
    Io(std::io::Error),
}

/// Saves `text` to `path`, keeping the file's permissions. With `expected`,
/// only if the file still holds that: otherwise something else changed it
/// since it was read, and saving would lose that.
pub fn write(path: &Path, text: &str, expected: Option<&str>) -> Result<u64, Unsaved> {
    if let Some(expected) = expected {
        let now = match std::fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => return Err(Unsaved::Io(err)),
        };
        if now.as_deref() != Some(expected.as_bytes()) {
            return Err(Unsaved::Changed);
        }
    }
    let permissions = std::fs::metadata(path).ok().map(|meta| meta.permissions());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(Unsaved::Io)?;
    }
    write_whole(path, text, permissions).map_err(Unsaved::Io)?;
    Ok(std::fs::metadata(path).map_or(0, |meta| modified(&meta)))
}

/// Writes a file beside itself and moves it into place, so a crash never
/// leaves half a file. A link is written through, to what it points at.
pub fn write_whole(
    path: &Path,
    text: &str,
    permissions: Option<std::fs::Permissions>,
) -> std::io::Result<()> {
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let temporary = target.with_file_name(format!(".{name}.wb-save"));
    std::fs::write(&temporary, text)?;
    if let Some(permissions) = permissions {
        let _ = std::fs::set_permissions(&temporary, permissions);
    }
    std::fs::rename(&temporary, &target).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder with these files in it, each holding `x`.
    fn folder_with(files: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for file in files {
            let path = dir.path().join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "x").unwrap();
        }
        dir
    }

    #[cfg(unix)]
    #[test]
    fn reads_one_folder_and_leaves_out_only_git() {
        let dir = folder_with(&[
            ".git/HEAD",
            ".github/ci.yml",
            ".gitignore",
            "Src/deep/a.rs",
            "Zed.md",
            "about.md",
            "build/out.rs",
            "node_modules/x/i.js",
        ]);
        std::os::unix::fs::symlink(dir.path().join("Src"), dir.path().join("link")).unwrap();
        let names = |names: &[&str]| names.iter().map(|name| name.to_string()).collect();
        assert_eq!(
            entries(dir.path()).unwrap(),
            (
                names(&[".github", "build", "link", "node_modules", "Src"]),
                names(&[".gitignore", "about.md", "Zed.md"])
            )
        );
        assert!(entries(&dir.path().join("gone")).is_err());
    }

    #[test]
    fn walks_a_folder_outside_git() {
        let dir = folder_with(&[
            "plan.md",
            "notes/a.md",
            ".env",
            ".github/ci.yml",
            ".git/HEAD",
            "node_modules/x/i.js",
        ]);
        let mut paths = walk(dir.path());
        paths.sort();
        assert_eq!(paths, [".env", ".github/ci.yml", "notes/a.md", "plan.md"]);
    }

    #[tokio::test]
    async fn lists_what_git_tracks_or_would() {
        let dir = folder_with(&[
            ".github/ci.yml",
            "build/a.rs",
            "inner/a.txt",
            "new.txt",
            "node_modules/x/i.js",
        ]);
        std::fs::write(dir.path().join(".gitignore"), "node_modules/").unwrap();
        // `inner` is a repository of its own, so its files are not this one's.
        let init = |at: &Path| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(at)
                .args(["init", "-q"])
                .status()
                .is_ok_and(|status| status.success())
        };
        if !init(dir.path()) || !init(&dir.path().join("inner")) {
            return;
        }
        let paths = [".github/ci.yml", ".gitignore", "build/a.rs", "new.txt"];
        assert_eq!(
            list(dir.path()).await,
            (paths.map(String::from).to_vec(), false)
        );
    }

    #[test]
    fn reads_text_and_says_what_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let note = dir.path().join("note.md");
        std::fs::write(&note, "# Hi\n").unwrap();
        assert_eq!(
            read(&note).unwrap().content,
            FileContent::Text {
                text: "# Hi\n".into(),
                truncated: false
            }
        );

        let blob = dir.path().join("blob.bin");
        std::fs::write(&blob, [1, 0, 2]).unwrap();
        assert_eq!(
            read(&blob).unwrap().content,
            FileContent::Binary { size: 3 }
        );

        assert_eq!(
            read(&dir.path().join("gone")).unwrap().content,
            FileContent::Missing
        );
    }

    #[cfg(unix)]
    #[test]
    fn saves_unless_something_else_changed_it() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("run.sh");
        std::fs::write(&script, "echo one\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert!(write(&script, "echo two\n", Some("echo one\n")).unwrap() > 0);
        assert_eq!(std::fs::read_to_string(&script).unwrap(), "echo two\n");
        let mode = std::fs::metadata(&script).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);

        // Something else wrote it meanwhile: the editor's copy is out of date.
        assert!(matches!(
            write(&script, "echo three\n", Some("echo one\n")),
            Err(Unsaved::Changed)
        ));
        assert_eq!(std::fs::read_to_string(&script).unwrap(), "echo two\n");

        let new = dir.path().join("src/new.txt");
        write(&new, "hi", None).unwrap();
        assert_eq!(stat(&[new.display().to_string(), "/no/such".into()])[1], 0);
        assert!(
            dir.path()
                .read_dir()
                .unwrap()
                .flatten()
                .all(|e| !e.file_name().to_string_lossy().ends_with(".wb-save"))
        );
    }
}
