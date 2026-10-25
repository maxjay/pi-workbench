//! Pi's own files, in its agent directory. The workbench edits the same
//! files `pi` reads in a terminal. wb-agent keeps only what Pi has no
//! place for (scratchpad folders, which are done, its question tool) in a
//! folder of its own.

use std::path::{Path, PathBuf};

use protocol::{ConfigFile, ConfigText};

use crate::error::Error;

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// Where Pi keeps its files: `PI_CODING_AGENT_DIR`, or `~/.pi/agent`.
pub fn agent_dir() -> PathBuf {
    match std::env::var_os("PI_CODING_AGENT_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => home().join(".pi").join("agent"),
    }
}

/// wb-agent's own files, which Pi doesn't read: `WB_DATA_DIR`, or
/// `~/.local/share/pi-workbench`.
pub fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("WB_DATA_DIR") {
        return PathBuf::from(dir);
    }
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local").join("share"));
    data.join("pi-workbench")
}

/// How long a session's Pi may sit idle before wb-agent stops it:
/// `WB_IDLE_SECS`, or 30 minutes.
pub fn idle_after() -> std::time::Duration {
    let secs = std::env::var("WB_IDLE_SECS")
        .ok()
        .and_then(|secs| secs.parse().ok())
        .unwrap_or(30 * 60);
    std::time::Duration::from_secs(secs)
}

/// Where scratchpads get their folders: `WB_SCRATCH_DIR`, or `scratchpads`
/// in [`data_dir`].
pub fn scratch_dir() -> PathBuf {
    match std::env::var_os("WB_SCRATCH_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => data_dir().join("scratchpads"),
    }
}

pub fn read(dir: &Path, file: ConfigFile) -> Result<ConfigText, Error> {
    let path = dir.join(file.file_name());
    let (exists, text) = match std::fs::read_to_string(&path) {
        Ok(text) => (true, text),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => (false, String::new()),
        Err(err) => return Err(Error::io(&path, err)),
    };
    Ok(ConfigText {
        file,
        path: path.display().to_string(),
        exists,
        text,
    })
}

/// Writes the whole file at once: to a temporary file beside it, then a
/// rename, so `pi` never reads half of one.
pub fn write(dir: &Path, file: ConfigFile, text: &str) -> Result<(), Error> {
    std::fs::create_dir_all(dir).map_err(|err| Error::io(dir, err))?;
    let path = dir.join(file.file_name());
    let temp = dir.join(format!(".{}.wb-tmp", file.file_name()));
    std::fs::write(&temp, text).map_err(|err| Error::io(&temp, err))?;
    std::fs::rename(&temp, &path).map_err(|err| Error::io(&path, err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let text = read(dir.path(), ConfigFile::Agents).unwrap();
        assert!(!text.exists);
        assert_eq!(text.text, "");
    }

    #[test]
    fn writes_then_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let agent = dir.path().join("agent");
        write(&agent, ConfigFile::Agents, "# How I work\n").unwrap();
        let text = read(&agent, ConfigFile::Agents).unwrap();
        assert!(text.exists);
        assert_eq!(text.text, "# How I work\n");
        assert!(text.path.ends_with("AGENTS.md"));
    }
}
