//! The codespaces: folders opened in the workbench, kept in
//! `codespaces.json` in wb-agent's data folder. Nothing is written into the
//! folders themselves.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use protocol::{Codespace, Folder};
use serde::{Deserialize, Serialize};

use crate::config;
use crate::error::Error;
use crate::sessions::millis;

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    spaces: Vec<Codespace>,
}

pub struct Codespaces {
    path: PathBuf,
    saved: Mutex<Saved>,
}

impl Codespaces {
    pub fn new(data_dir: &Path) -> Self {
        let path = data_dir.join("codespaces.json");
        let saved = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Codespaces {
            path,
            saved: Mutex::new(saved),
        }
    }

    /// Pinned first, then the last opened first.
    pub fn all(&self) -> Vec<Codespace> {
        let mut spaces = self.saved.lock().expect("not poisoned").spaces.clone();
        spaces.sort_by_key(|space| (!space.pinned, std::cmp::Reverse(space.opened)));
        spaces
    }

    /// The codespace a path is in, the innermost when one is inside another.
    pub fn holding(&self, path: &Path) -> Option<Codespace> {
        self.all()
            .into_iter()
            .filter(|space| path.starts_with(&space.path))
            .max_by_key(|space| space.path.len())
    }

    /// Opens a folder as a codespace, or opens it again.
    pub fn open(&self, path: &str) -> Result<Codespace, Error> {
        let path = expand(path);
        if !path.is_dir() {
            return Err(Error::NotFound(format!("no folder at {}", path.display())));
        }
        let path = path.canonicalize().map_err(|err| Error::io(&path, err))?;
        let path = path.display().to_string();
        let now = millis(SystemTime::now());
        let opened = self.change(|saved| {
            if let Some(space) = saved.spaces.iter_mut().find(|space| space.path == path) {
                space.opened = now;
                return space.clone();
            }
            let space = Codespace {
                id: new_id(&saved.spaces, now),
                name: Path::new(&path)
                    .file_name()
                    .map_or_else(|| path.clone(), |name| name.to_string_lossy().into_owned()),
                path: path.clone(),
                pinned: false,
                opened: now,
            };
            saved.spaces.push(space.clone());
            space
        })?;
        Ok(opened)
    }

    pub fn set_pinned(&self, id: &str, pinned: bool) -> Result<(), Error> {
        self.change(|saved| {
            saved
                .spaces
                .iter_mut()
                .find(|space| space.id == id)
                .map(|space| space.pinned = pinned)
                .ok_or_else(|| Error::NotFound(format!("no codespace {id}")))
        })?
    }

    pub fn forget(&self, id: &str) -> Result<(), Error> {
        self.change(|saved| saved.spaces.retain(|space| space.id != id))
    }

    fn change<R>(&self, edit: impl FnOnce(&mut Saved) -> R) -> Result<R, Error> {
        let mut saved = self.saved.lock().expect("not poisoned");
        let result = edit(&mut saved);
        let text = serde_json::to_string_pretty(&*saved).expect("codespaces serialize");
        config::write_file(&self.path, &text)?;
        Ok(result)
    }

    /// The folders in `dir`, hidden ones left out, with the branch of any
    /// that's a repository and whether it's a codespace already.
    pub fn folders(&self, dir: &str) -> Result<(PathBuf, Vec<Folder>), Error> {
        let dir = expand(dir);
        let entries = std::fs::read_dir(&dir).map_err(|err| Error::io(&dir, err))?;
        let spaces = self.all();
        let mut folders: Vec<Folder> = entries
            .flatten()
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    return None;
                }
                let path = entry.path();
                let real = path.canonicalize().unwrap_or_else(|_| path.clone());
                Some(Folder {
                    branch: branch(&path),
                    codespace: spaces.iter().any(|space| Path::new(&space.path) == real),
                    path: path.display().to_string(),
                    name,
                })
            })
            .collect();
        folders.sort_by_key(|folder| folder.name.to_lowercase());
        Ok((dir, folders))
    }
}

/// `~` and `~/…` are home.
pub fn expand(path: &str) -> PathBuf {
    let path = path.trim();
    match path.strip_prefix('~') {
        Some("") => config::home(),
        Some(rest) if rest.starts_with('/') => config::home().join(&rest[1..]),
        _ => PathBuf::from(path),
    }
}

/// A short id no other codespace has.
fn new_id(spaces: &[Codespace], now: u64) -> String {
    let mut n = now;
    loop {
        let id = format!("c{n:x}");
        if !spaces.iter().any(|space| space.id == id) {
            return id;
        }
        n += 1;
    }
}

/// The branch checked out in a repository's top folder, read from
/// `.git/HEAD` without running git. A worktree's `.git` is a file naming
/// where its HEAD is.
pub fn branch(dir: &Path) -> Option<String> {
    let dot = dir.join(".git");
    let git_dir = if dot.is_file() {
        let text = std::fs::read_to_string(&dot).ok()?;
        let target = text.trim().strip_prefix("gitdir:")?.trim();
        dir.join(target)
    } else {
        dot
    };
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    Some(match head.strip_prefix("ref: refs/heads/") {
        Some(branch) => branch.to_string(),
        None => head.chars().take(7).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_pins_and_forgets() {
        let data = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        for name in ["shop", "blog", ".hidden"] {
            std::fs::create_dir(work.path().join(name)).unwrap();
        }
        std::fs::create_dir_all(work.path().join("shop/.git")).unwrap();
        std::fs::write(
            work.path().join("shop/.git/HEAD"),
            "ref: refs/heads/fix/guest\n",
        )
        .unwrap();

        let spaces = Codespaces::new(data.path());
        let shop = spaces
            .open(&work.path().join("shop").display().to_string())
            .unwrap();
        assert_eq!(shop.name, "shop");
        let again = spaces
            .open(&work.path().join("shop").display().to_string())
            .unwrap();
        assert_eq!(again.id, shop.id);
        let blog = spaces
            .open(&work.path().join("blog").display().to_string())
            .unwrap();
        spaces.set_pinned(&shop.id, true).unwrap();
        let order: Vec<_> = spaces.all().into_iter().map(|s| s.name).collect();
        assert_eq!(order, ["shop", "blog"]);

        let inside = work.path().join("shop").canonicalize().unwrap().join("src");
        assert_eq!(spaces.holding(&inside).map(|s| s.id), Some(shop.id.clone()));

        let (_, folders) = spaces.folders(&work.path().display().to_string()).unwrap();
        let seen: Vec<_> = folders
            .iter()
            .map(|f| (f.name.as_str(), f.branch.as_deref(), f.codespace))
            .collect();
        assert_eq!(
            seen,
            [("blog", None, true), ("shop", Some("fix/guest"), true)]
        );

        spaces.forget(&blog.id).unwrap();
        // Kept on disk: a new start reads the same list.
        let reread = Codespaces::new(data.path());
        assert_eq!(reread.all().len(), 1);
        assert!(spaces.open("/no/such/folder").is_err());
    }
}
