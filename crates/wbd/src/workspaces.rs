//! The workspaces: folders opened in the workbench, kept in
//! `workspaces.json` in wbd's data folder. Nothing is written into the
//! folders themselves.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use protocol::{Folder, Workspace};

use crate::error::Error;
use crate::files;

pub struct Workspaces {
    file: PathBuf,
    list: Mutex<Vec<Workspace>>,
}

impl Workspaces {
    pub fn new(data_dir: &Path) -> Self {
        let file = data_dir.join("workspaces.json");
        let list = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Workspaces {
            file,
            list: Mutex::new(list),
        }
    }

    /// The last opened first.
    pub fn all(&self) -> Vec<Workspace> {
        let mut list = self.list.lock().expect("not poisoned").clone();
        list.sort_by_key(|workspace| std::cmp::Reverse(workspace.opened));
        list
    }

    /// Opens a folder as a workspace, or opens it again.
    pub fn open(&self, path: &str) -> Result<Workspace, Error> {
        let path = expand(path);
        if !path.is_dir() {
            return Err(Error::NotFound(format!("no folder at {}", path.display())));
        }
        let path = path.canonicalize().map_err(|err| Error::io(&path, err))?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_millis() as u64);
        self.change(|list| {
            let path = path.display().to_string();
            if let Some(workspace) = list.iter_mut().find(|workspace| workspace.path == path) {
                workspace.opened = now;
                return workspace.clone();
            }
            let workspace = Workspace {
                id: new_id(list, now),
                name: name_of(Path::new(&path)),
                path,
                opened: now,
            };
            list.push(workspace.clone());
            workspace
        })
    }

    pub fn forget(&self, id: &str) -> Result<(), Error> {
        self.change(|list| list.retain(|workspace| workspace.id != id))
    }

    fn change<R>(&self, edit: impl FnOnce(&mut Vec<Workspace>) -> R) -> Result<R, Error> {
        let mut list = self.list.lock().expect("not poisoned");
        let result = edit(&mut list);
        let text = serde_json::to_string_pretty(&*list).expect("workspaces serialize");
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir).map_err(|err| Error::io(dir, err))?;
        }
        files::write_whole(&self.file, &text, None).map_err(|err| Error::io(&self.file, err))?;
        Ok(result)
    }
}

/// The folders in `dir`, hidden ones left out, sorted by name, with the
/// branch of any that is a git repository. Also `dir` itself, resolved.
pub fn folders(dir: &str) -> Result<(String, Vec<Folder>), Error> {
    let dir = expand(dir);
    let dir = dir.canonicalize().map_err(|err| Error::io(&dir, err))?;
    let entries = std::fs::read_dir(&dir).map_err(|err| Error::io(&dir, err))?;
    let mut folders: Vec<Folder> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.path())
        .filter(|path| !name_of(path).starts_with('.'))
        .map(|path| Folder {
            name: name_of(&path),
            branch: branch(&path),
            path: path.display().to_string(),
        })
        .collect();
    folders.sort_by_key(|folder| folder.name.to_lowercase());
    Ok((dir.display().to_string(), folders))
}

/// A folder's own name.
fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// `~` and `~/x` are in your home folder.
fn expand(path: &str) -> PathBuf {
    let home = || std::env::var_os("HOME").map(PathBuf::from);
    match path.strip_prefix('~') {
        Some("") => home().unwrap_or_else(|| path.into()),
        Some(rest) if rest.starts_with('/') => {
            home().map_or_else(|| path.into(), |home| home.join(&rest[1..]))
        }
        _ => path.into(),
    }
}

/// The branch a folder has checked out, read from `.git/HEAD` without
/// running git. None when the folder isn't the top of a repository.
fn branch(dir: &Path) -> Option<String> {
    let head = std::fs::read_to_string(dir.join(".git/HEAD")).ok()?;
    Some(match head.trim().strip_prefix("ref: refs/heads/") {
        Some(branch) => branch.to_string(),
        None => "detached".to_string(),
    })
}

/// A short id no workspace has yet: the time it was made, in base 36.
fn new_id(list: &[Workspace], now: u64) -> String {
    let mut n = now;
    loop {
        let id = base36(n);
        if list.iter().all(|workspace| workspace.id != id) {
            return id;
        }
        n += 1;
    }
}

fn base36(mut n: u64) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    loop {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).expect("digits are ASCII")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_remembers_and_forgets() {
        let data = tempfile::tempdir().unwrap();
        let code = tempfile::tempdir().unwrap();
        std::fs::create_dir(code.path().join("web")).unwrap();
        std::fs::create_dir(code.path().join("api")).unwrap();

        let workspaces = Workspaces::new(data.path());
        let web = workspaces
            .open(&code.path().join("web").display().to_string())
            .unwrap();
        assert_eq!(web.name, "web");
        let api = workspaces
            .open(&code.path().join("api").display().to_string())
            .unwrap();
        // Opening one again doesn't list it twice.
        let again = workspaces.open(&web.path).unwrap();
        assert_eq!(again.id, web.id);

        // A new wbd reads the same list back.
        let reread = Workspaces::new(data.path());
        assert_eq!(reread.all().len(), 2);

        reread.forget(&api.id).unwrap();
        assert_eq!(reread.all().len(), 1);
        assert!(matches!(
            reread.open("/no/such/folder"),
            Err(Error::NotFound(_))
        ));
    }

    #[test]
    fn lists_folders_with_their_branch() {
        let code = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(code.path().join("web/.git")).unwrap();
        std::fs::write(
            code.path().join("web/.git/HEAD"),
            "ref: refs/heads/fix/login\n",
        )
        .unwrap();
        std::fs::create_dir(code.path().join("Notes")).unwrap();
        std::fs::create_dir(code.path().join(".cache")).unwrap();
        std::fs::write(code.path().join("file.txt"), "x").unwrap();

        let (_, folders) = folders(&code.path().display().to_string()).unwrap();
        let seen: Vec<_> = folders
            .iter()
            .map(|folder| (folder.name.as_str(), folder.branch.as_deref()))
            .collect();
        assert_eq!(seen, [("Notes", None), ("web", Some("fix/login"))]);
    }

    #[test]
    fn ids_are_short_and_differ() {
        assert_eq!(base36(0), "0");
        assert_eq!(base36(36), "10");
        let taken = [Workspace {
            id: base36(5),
            path: String::new(),
            name: String::new(),
            opened: 0,
        }];
        assert_eq!(new_id(&taken, 5), base36(6));
    }
}
