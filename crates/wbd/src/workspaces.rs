//! The workspaces: folders opened in the workbench, kept in
//! `workspaces.json` in wbd's data folder. Nothing is written into the
//! folders themselves.
//!
//! The folders under `~/.pi/workspaces` are workspaces without being
//! opened by hand: each is taken in the first time it is seen there.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use protocol::{Folder, Workspace};

use crate::error::Error;
use crate::files;

pub struct Workspaces {
    file: PathBuf,
    /// Where scratchpads' folders are made.
    scratch: PathBuf,
    list: Mutex<Vec<Workspace>>,
    /// Where Pi's own workspaces are: `~/.pi/workspaces`, or what `WB_PI_WORKSPACES` names.
    pi: Option<PathBuf>,
    /// The folders that were taken in from there, kept in `adopted.json`. One you take off the
    /// list is still in here, so it is not taken in again.
    adopted_file: PathBuf,
    adopted: Mutex<Vec<String>>,
}

impl Workspaces {
    pub fn new(data_dir: &Path) -> Self {
        let pi = std::env::var_os("WB_PI_WORKSPACES")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".pi/workspaces"))
            });
        Self::beside(data_dir, pi)
    }

    /// The store, with `pi` as the folder Pi's own workspaces are in, if there is one.
    fn beside(data_dir: &Path, pi: Option<PathBuf>) -> Self {
        let file = data_dir.join("workspaces.json");
        let list = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let adopted_file = data_dir.join("adopted.json");
        let adopted = std::fs::read_to_string(&adopted_file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Workspaces {
            file,
            scratch: data_dir.join("scratch"),
            list: Mutex::new(list),
            pi,
            adopted_file,
            adopted: Mutex::new(adopted),
        }
    }

    /// The last opened first.
    pub fn all(&self) -> Vec<Workspace> {
        self.adopt();
        let mut list = self.list.lock().expect("not poisoned").clone();
        list.sort_by_key(|workspace| std::cmp::Reverse(workspace.opened));
        list
    }

    /// Makes an empty folder named after what you asked, and opens it as a
    /// workspace marked as a scratchpad.
    pub fn scratch(&self, text: &str) -> Result<Workspace, Error> {
        let name = slug(text);
        let folder = (1..)
            .map(|n| match n {
                1 => self.scratch.join(&name),
                n => self.scratch.join(format!("{name}-{n}")),
            })
            .find(|folder| !folder.exists())
            .expect("some number is free");
        std::fs::create_dir_all(&folder).map_err(|err| Error::io(&folder, err))?;
        self.add(&folder.display().to_string(), true)
    }

    /// Opens a folder as a workspace, or opens it again.
    pub fn open(&self, path: &str) -> Result<Workspace, Error> {
        self.add(path, false)
    }

    fn add(&self, path: &str, scratch: bool) -> Result<Workspace, Error> {
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
                scratch,
            };
            list.push(workspace.clone());
            workspace
        })
    }

    /// Takes in the folders under Pi's workspaces that were not seen before. A folder that is a
    /// repo is one workspace. A folder that only holds repos gives one for each of them.
    fn adopt(&self) {
        let Some(found) = self.pi.as_deref().map(under) else {
            return;
        };
        let fresh: Vec<PathBuf> = {
            let adopted = self.adopted.lock().expect("not poisoned");
            found
                .into_iter()
                .filter(|folder| !adopted.contains(&folder.display().to_string()))
                .collect()
        };
        if fresh.is_empty() {
            return;
        }
        for folder in &fresh {
            // It is listed by when it was last changed, not as if it was opened just now.
            let changed = std::fs::metadata(folder)
                .and_then(|data| data.modified())
                .ok()
                .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |since| since.as_millis() as u64);
            let path = folder.display().to_string();
            let _ = self.change(|list| {
                if list.iter().any(|workspace| workspace.path == path) {
                    return;
                }
                list.push(Workspace {
                    id: new_id(list, changed),
                    name: name_of(folder),
                    path: path.clone(),
                    opened: changed,
                    scratch: false,
                });
            });
        }
        let mut adopted = self.adopted.lock().expect("not poisoned");
        adopted.extend(fresh.iter().map(|folder| folder.display().to_string()));
        if let Ok(text) = serde_json::to_string_pretty(&*adopted) {
            let _ = files::write_whole(&self.adopted_file, &text, None);
        }
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

/// The folders to take in from Pi's workspaces: each folder there, or the repos inside it when
/// it is not a repo itself and holds some. Hidden folders are left out.
fn under(dir: &Path) -> Vec<PathBuf> {
    let inside = |dir: &Path| -> Vec<PathBuf> {
        let mut folders: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir() && !name_of(path).starts_with('.'))
            .filter_map(|path| path.canonicalize().ok())
            .collect();
        folders.sort();
        folders
    };
    let mut found = Vec::new();
    for folder in inside(dir) {
        let repos: Vec<PathBuf> = match folder.join(".git").exists() {
            true => Vec::new(),
            false => inside(&folder)
                .into_iter()
                .filter(|inner| inner.join(".git").exists())
                .collect(),
        };
        match repos.is_empty() {
            true => found.push(folder),
            false => found.extend(repos),
        }
    }
    found
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

/// The branch a folder has checked out, read from its `.git` without running git. None
/// when the folder isn't the top of a repository. In a worktree `.git` is a file that says
/// where the rest is.
pub fn branch(dir: &Path) -> Option<String> {
    let git = dir.join(".git");
    let head = match std::fs::read_to_string(&git) {
        Ok(points) => {
            let elsewhere = points.trim().strip_prefix("gitdir:")?.trim();
            std::fs::read_to_string(dir.join(elsewhere).join("HEAD")).ok()?
        }
        Err(_) => std::fs::read_to_string(git.join("HEAD")).ok()?,
    };
    Some(match head.trim().strip_prefix("ref: refs/heads/") {
        Some(branch) => branch.to_string(),
        None => "detached".to_string(),
    })
}

/// A folder name from what you asked: its first few words, in letters,
/// digits and dashes.
fn slug(text: &str) -> String {
    let words: Vec<String> = text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(5)
        .map(str::to_ascii_lowercase)
        .collect();
    match words.is_empty() {
        true => "scratch".to_string(),
        false => words.join("-"),
    }
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
    fn takes_in_pis_workspaces_once() {
        let data = tempfile::tempdir().unwrap();
        let pi = tempfile::tempdir().unwrap();
        // A repo, a folder that only holds repos, a plain folder, and a hidden one.
        std::fs::create_dir_all(pi.path().join("web/.git")).unwrap();
        std::fs::create_dir_all(pi.path().join("shop/api/.git")).unwrap();
        std::fs::create_dir_all(pi.path().join("shop/site/.git")).unwrap();
        std::fs::create_dir_all(pi.path().join("notes")).unwrap();
        std::fs::create_dir_all(pi.path().join(".cache")).unwrap();

        let workspaces = Workspaces::beside(data.path(), Some(pi.path().to_path_buf()));
        let mut names: Vec<String> = workspaces.all().into_iter().map(|w| w.name).collect();
        names.sort();
        assert_eq!(names, ["api", "notes", "site", "web"]);

        // One taken off the list stays off, in this wbd and in the next.
        let web = workspaces
            .all()
            .into_iter()
            .find(|w| w.name == "web")
            .unwrap();
        workspaces.forget(&web.id).unwrap();
        assert_eq!(workspaces.all().len(), 3);
        let next = Workspaces::beside(data.path(), Some(pi.path().to_path_buf()));
        assert_eq!(next.all().len(), 3);

        // A folder that comes later is taken in then.
        std::fs::create_dir_all(pi.path().join("later/.git")).unwrap();
        assert_eq!(next.all().len(), 4);
    }

    #[test]
    fn opens_remembers_and_forgets() {
        let data = tempfile::tempdir().unwrap();
        let code = tempfile::tempdir().unwrap();
        std::fs::create_dir(code.path().join("web")).unwrap();
        std::fs::create_dir(code.path().join("api")).unwrap();

        let workspaces = Workspaces::beside(data.path(), None);
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
        let reread = Workspaces::beside(data.path(), None);
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
    fn a_scratchpad_gets_a_folder_named_after_what_you_asked() {
        let data = tempfile::tempdir().unwrap();
        let workspaces = Workspaces::beside(data.path(), None);
        let first = workspaces
            .scratch("Why does checkout crash on run?")
            .unwrap();
        assert_eq!(first.name, "why-does-checkout-crash-on");
        assert!(first.scratch && Path::new(&first.path).is_dir());
        // The same question again gets a folder of its own.
        let second = workspaces
            .scratch("Why does checkout crash on run?")
            .unwrap();
        assert_eq!(second.name, "why-does-checkout-crash-on-2");
        assert_eq!(slug("¿?"), "scratch");
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
            scratch: false,
        }];
        assert_eq!(new_id(&taken, 5), base36(6));
    }
}
