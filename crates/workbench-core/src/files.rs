use anyhow::{Context, Result, bail, ensure};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub directory: bool,
}

/// Lists one level only. Hidden files remain visible, except generated directories.
pub fn list_directory(root: &Path, directory: &Path) -> Result<Vec<Entry>> {
    let directory = within(root, directory)?;
    let mut entries = Vec::new();
    for item in fs::read_dir(directory)? {
        let item = item?;
        let ty = item.file_type()?;
        // Symlink directories may form cycles or leave the workspace.
        if ty.is_symlink() || !(ty.is_dir() || ty.is_file()) {
            continue;
        }
        if ty.is_dir()
            && matches!(
                item.file_name().to_str(),
                Some(".git" | "target" | "node_modules")
            )
        {
            continue;
        }
        entries.push(Entry {
            path: item.path(),
            directory: ty.is_dir(),
        });
    }
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(entries)
}

pub fn within(root: &Path, path: &Path) -> Result<PathBuf> {
    let path = path
        .canonicalize()
        .with_context(|| format!("Cannot open {}", path.display()))?;
    ensure!(path.starts_with(root), "Path is outside this workspace");
    Ok(path)
}

pub fn read_text(path: &Path) -> Result<String> {
    let file = fs::File::open(path)?;
    ensure!(file.metadata()?.is_file(), "Not a regular file");
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_FILE_BYTES,
        "The prototype supports files up to 4 MiB"
    );
    ensure!(!bytes.contains(&0), "Binary files are not supported");
    String::from_utf8(bytes).context("Only UTF-8 text files are supported")
}

/// Replaces the file only if its current contents match the version we opened.
/// Atomic replacement avoids partial writes; this is not a cross-process CAS.
pub fn save_text(path: &Path, expected: &str, text: &str) -> Result<()> {
    ensure!(
        text.len() as u64 <= MAX_FILE_BYTES,
        "The prototype supports files up to 4 MiB"
    );
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.file_type().is_file(),
        "File has been replaced or removed; save cancelled"
    );
    ensure!(!metadata.permissions().readonly(), "File is read-only");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            metadata.nlink() == 1,
            "Hard-linked files require an external editor in this prototype"
        );
    }
    ensure!(
        read_text(path)? == expected,
        "File changed on disk. Save cancelled; your edits remain open. Use Reload to discard them, or copy them elsewhere first."
    );
    let parent = path.parent().context("File has no parent directory")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary
        .as_file()
        .set_permissions(metadata.permissions())?;
    temporary.write_all(text.as_bytes())?;
    temporary.as_file().sync_all()?;
    if read_text(path)? != expected {
        bail!("File changed during save; your edits remain open");
    }
    temporary.persist(path).context("Could not replace file")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_changes_are_never_silently_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.rs");
        fs::write(&path, "original").unwrap();
        let base = read_text(&path).unwrap();
        fs::write(&path, "agent edit").unwrap();
        assert!(save_text(&path, &base, "my edit").is_err());
        assert_eq!(read_text(&path).unwrap(), "agent edit");
    }

    #[test]
    fn save_preserves_crlf_unicode_and_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.rs");
        fs::write(&path, "a\r\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        save_text(&path, "a\r\n", "λ\r\n").unwrap();
        assert_eq!(fs::read(&path).unwrap(), "λ\r\n".as_bytes());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
    }

    #[test]
    fn deleted_files_and_binary_files_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing");
        assert!(save_text(&path, "", "new").is_err());
        assert!(!path.exists());
        fs::write(&path, [1, 0, 2]).unwrap();
        assert!(read_text(&path).is_err());
    }

    #[test]
    fn listing_is_shallow_and_excludes_generated_directories() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("src")).unwrap();
        fs::create_dir(dir.path().join("target")).unwrap();
        fs::write(dir.path().join("src/nested.rs"), "").unwrap();
        fs::write(dir.path().join(".gitignore"), "").unwrap();
        let root = dir.path().canonicalize().unwrap();
        let entries = list_directory(&root, &root).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].directory);
        assert!(entries[1].path.ends_with(".gitignore"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_escape_the_workspace() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("escape")).unwrap();
        assert!(
            within(
                &root.path().canonicalize().unwrap(),
                &root.path().join("escape")
            )
            .is_err()
        );
    }
}
