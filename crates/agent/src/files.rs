//! Files opened for viewing: text as text, images as images, and anything
//! else described rather than sent. Files saved from the editor, unless
//! something else changed them first. And the files in a folder, for `@`.

use std::io::Read;
use std::path::Path;
use std::time::UNIX_EPOCH;

use base64::Engine;
use protocol::{FileContent, FileView};

/// More than this of a text file isn't sent.
const TEXT_LIMIT: usize = 512 * 1024;
/// Images larger than this aren't sent.
const IMAGE_LIMIT: u64 = 16 * 1024 * 1024;

/// No more paths than this are listed.
const LIST_LIMIT: usize = 5000;

/// Folders a file search skips, as nobody means a file in them.
const SKIPPED: [&str; 6] = [
    "node_modules",
    "target",
    "__pycache__",
    "venv",
    "dist",
    "build",
];

/// The files under `dir`, as paths relative to it, sorted, skipping hidden
/// files and folders and ones like `node_modules`. True when there were
/// more than [`LIST_LIMIT`].
pub fn list(dir: &Path) -> (Vec<String>, bool) {
    let mut paths = Vec::new();
    let mut folders = vec![dir.to_path_buf()];
    let mut truncated = false;
    while let Some(folder) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                if !SKIPPED.contains(&name.as_ref()) {
                    folders.push(path);
                }
            } else if paths.len() >= LIST_LIMIT {
                truncated = true;
                break;
            } else if let Ok(relative) = path.strip_prefix(dir) {
                paths.push(relative.display().to_string());
            }
        }
        if truncated {
            break;
        }
    }
    paths.sort();
    (paths, truncated)
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
    let view = |content| FileView {
        path: path.display().to_string(),
        content,
        modified: modified(&meta),
    };
    if let Some(mime_type) = image_type(path) {
        if meta.len() > IMAGE_LIMIT {
            return Ok(view(FileContent::Binary { size: meta.len() }));
        }
        let data = base64::engine::general_purpose::STANDARD.encode(std::fs::read(path)?);
        return Ok(view(FileContent::Image {
            mime_type: mime_type.into(),
            data,
        }));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(TEXT_LIMIT as u64)
        .read_to_end(&mut bytes)?;
    if bytes.contains(&0) {
        return Ok(view(FileContent::Binary { size: meta.len() }));
    }
    Ok(view(FileContent::Text {
        text: String::from_utf8_lossy(&bytes).into_owned(),
        truncated: meta.len() > TEXT_LIMIT as u64,
    }))
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
    // Written beside it and moved into place, so a crash never leaves half
    // a file. A link is written through, to what it points at.
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    let temporary = target.with_file_name(format!(".{name}.wb-save"));
    std::fs::write(&temporary, text).map_err(Unsaved::Io)?;
    if let Some(permissions) = permissions {
        let _ = std::fs::set_permissions(&temporary, permissions);
    }
    std::fs::rename(&temporary, &target).map_err(|err| {
        let _ = std::fs::remove_file(&temporary);
        Unsaved::Io(err)
    })?;
    Ok(std::fs::metadata(&target).map_or(0, |meta| modified(&meta)))
}

fn image_type(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_a_folder_but_not_whats_hidden_or_built() {
        let dir = tempfile::tempdir().unwrap();
        for file in [
            "plan.md",
            "notes/a.md",
            ".git/HEAD",
            "node_modules/x/i.js",
            ".env",
        ] {
            let path = dir.path().join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "x").unwrap();
        }
        assert_eq!(
            list(dir.path()),
            (vec!["notes/a.md".into(), "plan.md".into()], false)
        );
    }

    #[test]
    fn reads_text_images_and_binaries() {
        let dir = tempfile::tempdir().unwrap();
        let note = dir.path().join("note.md");
        std::fs::write(&note, "# Hi\n").unwrap();
        assert!(
            matches!(read(&note).unwrap().content, FileContent::Text { text, truncated: false } if text == "# Hi\n")
        );

        let shot = dir.path().join("shot.PNG");
        std::fs::write(&shot, [0x89, b'P', b'N', b'G']).unwrap();
        assert!(
            matches!(read(&shot).unwrap().content, FileContent::Image { mime_type, .. } if mime_type == "image/png")
        );

        let blob = dir.path().join("blob.bin");
        std::fs::write(&blob, [1, 0, 2]).unwrap();
        assert!(matches!(
            read(&blob).unwrap().content,
            FileContent::Binary { size: 3 }
        ));

        assert_eq!(
            read(&dir.path().join("gone")).unwrap().content,
            FileContent::Missing
        );
    }

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

        // Pi wrote it meanwhile: the editor's copy is out of date.
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
