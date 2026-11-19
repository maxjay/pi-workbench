//! Where things are: the folder your clones are in, and the folder the
//! workbench keeps its own things in. Both can be changed. The clones are
//! found by looking in the first, never listed by hand. Nothing in a clone
//! is changed by looking.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::{files, workspaces};

/// What was found is used again for this long before the folder is looked in again.
const FRESH: Duration = Duration::from_secs(60);

/// A clone found in the repos folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub path: String,
    /// The address of its remote called origin, as its `.git/config` has it.
    pub remote: String,
}

/// The two folders, as you wrote them. `~` is your home folder.
#[derive(Clone, Deserialize, Serialize)]
struct Written {
    repos: String,
    own: String,
}

impl Default for Written {
    fn default() -> Self {
        Written {
            repos: "~/Developer".into(),
            own: "~/.workbench".into(),
        }
    }
}

pub struct Places {
    file: PathBuf,
    written: Mutex<Written>,
    /// The clones last found, the folder they were found in, and when.
    found: Mutex<Option<(Instant, PathBuf, Vec<Found>)>>,
}

impl Places {
    /// Reads `settings.json` in `data`. Without it, the folders are the usual ones.
    pub fn new(data: &Path) -> Self {
        let file = data.join("settings.json");
        let written = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Places {
            file,
            written: Mutex::new(written),
            found: Mutex::new(None),
        }
    }

    /// The two folders as you wrote them: where your clones are, and the workbench's own.
    pub fn written(&self) -> (String, String) {
        let written = self.written.lock().expect("not poisoned");
        (written.repos.clone(), written.own.clone())
    }

    /// The folder your clones are in.
    pub fn repos(&self) -> PathBuf {
        workspaces::expand(&self.written().0)
    }

    /// The folder the workbench keeps its own things in.
    pub fn own(&self) -> PathBuf {
        workspaces::expand(&self.written().1)
    }

    /// Changes the two folders. One left empty goes back to the usual one.
    pub fn set(&self, repos: &str, own: &str) -> Result<(), Error> {
        let usual = Written::default();
        let or = |given: &str, usual: String| match given.trim() {
            "" => usual,
            given => given.to_string(),
        };
        let next = Written {
            repos: or(repos, usual.repos),
            own: or(own, usual.own),
        };
        let text = serde_json::to_string_pretty(&next).expect("settings serialize");
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir).map_err(|err| Error::io(dir, err))?;
        }
        files::write_whole(&self.file, &text, None).map_err(|err| Error::io(&self.file, err))?;
        *self.written.lock().expect("not poisoned") = next;
        *self.found.lock().expect("not poisoned") = None;
        Ok(())
    }

    /// The clones in your repos folder. It is looked in at most once a minute.
    pub async fn found(&self) -> Vec<Found> {
        let folder = self.repos();
        if let Some((when, of, found)) = &*self.found.lock().expect("not poisoned")
            && when.elapsed() < FRESH
            && *of == folder
        {
            return found.clone();
        }
        // A folder of hundreds of clones on a slow drive takes a while, so it is read off this thread.
        let looked_in = folder.clone();
        let found = tokio::task::spawn_blocking(move || look(&looked_in))
            .await
            .unwrap_or_default();
        *self.found.lock().expect("not poisoned") = Some((Instant::now(), folder, found.clone()));
        found
    }
}

/// The clones in `folder`: each folder there that is one, and for a folder that is not one,
/// the clones directly inside it. Hidden folders are left out, and so is a worktree, which
/// is not a clone but a part of one.
fn look(folder: &Path) -> Vec<Found> {
    let inside = |dir: &Path| -> Vec<PathBuf> {
        let mut folders: Vec<PathBuf> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                let hidden = path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with('.'));
                !hidden && path.is_dir()
            })
            .collect();
        folders.sort();
        folders
    };
    let is_clone = |dir: &Path| dir.join(".git").is_dir();
    let mut found = Vec::new();
    for dir in inside(folder) {
        let clones = match is_clone(&dir) {
            true => vec![dir],
            false => inside(&dir)
                .into_iter()
                .filter(|dir| is_clone(dir))
                .collect(),
        };
        found.extend(clones.iter().filter_map(|dir| clone_at(dir)));
    }
    found
}

/// A clone and where it is cloned from. None for one with no remote called origin.
fn clone_at(dir: &Path) -> Option<Found> {
    let config = std::fs::read_to_string(dir.join(".git/config")).ok()?;
    Some(Found {
        path: dir.display().to_string(),
        remote: origin(&config)?,
    })
}

/// The address of the remote called origin, from the text of a `.git/config`.
fn origin(config: &str) -> Option<String> {
    let mut inside = false;
    for line in config.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line.replace(' ', "") == "[remote\"origin\"]";
        } else if inside
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == "url"
        {
            return Some(value.trim().to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_where_a_clone_is_from() {
        let config = "[core]\n\tbare = false\n[remote \"upstream\"]\n\turl = https://x/other\n\
                      [remote \"origin\"]\n\turl = https://dev.azure.com/contoso/Shop/_git/web\n\
                      \tfetch = +refs/heads/*:refs/remotes/origin/*\n[branch \"main\"]\n\tremote = origin\n";
        assert_eq!(
            origin(config).as_deref(),
            Some("https://dev.azure.com/contoso/Shop/_git/web")
        );
        assert_eq!(origin("[core]\n\tbare = false\n"), None);
    }

    #[test]
    fn finds_the_clones_in_a_folder_and_one_folder_down() {
        let root = std::env::temp_dir().join(format!("wbd-places-{}", std::process::id()));
        let clone = |at: &str, url: &str| {
            let git = root.join(at).join(".git");
            std::fs::create_dir_all(&git).unwrap();
            let config = format!("[remote \"origin\"]\n\turl = {url}\n");
            std::fs::write(git.join("config"), config).unwrap();
        };
        clone("web", "https://dev.azure.com/contoso/Shop/_git/web");
        clone("team/api", "https://dev.azure.com/contoso/Shop/_git/api");
        clone(".hidden", "https://dev.azure.com/contoso/Shop/_git/hidden");
        // A worktree has a file called `.git`, not a folder, and a plain folder has neither.
        std::fs::create_dir_all(root.join("tree")).unwrap();
        std::fs::write(root.join("tree/.git"), "gitdir: elsewhere").unwrap();
        std::fs::create_dir_all(root.join("notes")).unwrap();

        let found: Vec<String> = look(&root).into_iter().map(|one| one.remote).collect();
        std::fs::remove_dir_all(&root).unwrap();
        assert_eq!(
            found,
            [
                "https://dev.azure.com/contoso/Shop/_git/api",
                "https://dev.azure.com/contoso/Shop/_git/web",
            ]
        );
    }
}
