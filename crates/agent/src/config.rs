//! Pi's own files, in its agent directory. The workbench edits the same
//! files `pi` reads in a terminal; nothing is kept anywhere else.

use std::path::{Path, PathBuf};

use protocol::{ConfigFile, ConfigText};

use crate::error::Error;

/// Where Pi keeps its files: `PI_CODING_AGENT_DIR`, or `~/.pi/agent`.
pub fn agent_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("PI_CODING_AGENT_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".pi").join("agent")
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
