//! The repos in a workspace. A workspace whose folder is not a repo itself can hold several:
//! each is a git worktree of one of your clones, on a branch of its own, in a folder named
//! after the repo. The clone is not changed, apart from knowing one more worktree and branch.

use std::path::Path;
use std::time::Duration;

use protocol::Folder;

use crate::error::Error;
use crate::git::run;
use crate::places::Found;
use crate::workspaces;

/// git gets this long to fetch the main branch before a worktree starts from it.
const FETCH: Duration = Duration::from_secs(60);

/// A repo that was put into a workspace.
pub struct Added {
    pub name: String,
    pub branch: String,
    /// What the branch started from, or nothing when the branch was there already.
    pub from: Option<String>,
}

/// Whether repos can be put into `folder`: it is not a repo, and not inside one.
pub fn takes(folder: &Path) -> bool {
    !folder.ancestors().any(|dir| dir.join(".git").exists())
}

/// The repos directly inside `folder`, each with its branch.
pub fn inside(folder: &Path) -> Vec<Folder> {
    let listed = workspaces::folders(&folder.display().to_string());
    let folders = listed.map_or_else(|_| Vec::new(), |(_, folders)| folders);
    folders
        .into_iter()
        .filter(|inner| inner.branch.is_some())
        .collect()
}

/// The branch a repo put into `folder` goes on when none is named: the one the repos there
/// are on, when they are all on the same one. A workspace is one piece of work, so its repos
/// share a branch name. Without that, the workspace's own name.
pub fn branch_for(folder: &Path) -> String {
    let mut on = inside(folder).into_iter().filter_map(|repo| repo.branch);
    match on.next() {
        Some(first) if first != "detached" && on.all(|next| next == first) => first,
        _ => name_of(folder),
    }
}

/// Gives the work in `folder` another name: the branch each repo there is on is renamed to
/// `new`. Nothing is renamed when one of them can't be: a branch that is pushed keeps its
/// name, and so does everything else then. Gives the repos that were renamed.
pub async fn rename(folder: &Path, new: &str) -> Result<Vec<String>, String> {
    let fits = new
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "-_./".contains(c));
    let valid = run(folder, &["check-ref-format", "--branch", new], None).await;
    if !fits || valid.is_err() {
        return Err(format!(
            "{new} can't be a branch's name. Use lower case letters, digits and dashes."
        ));
    }
    let repos = inside(folder);
    for repo in &repos {
        let (dir, old) = (
            Path::new(&repo.path),
            repo.branch.as_deref().unwrap_or_default(),
        );
        if old == new {
            continue;
        }
        let there = |name: String| async move {
            let asked = ["show-ref", "--verify", "--quiet", &name];
            run(dir, &asked, None).await.is_ok()
        };
        if there(format!("refs/heads/{new}")).await {
            return Err(format!(
                "{} already has a branch called {new}. Pick another name.",
                repo.name
            ));
        }
        if there(format!("refs/remotes/origin/{old}")).await {
            return Err(format!(
                "the branch {old} of {} is pushed, so the work keeps its name",
                repo.name
            ));
        }
    }
    let mut renamed = Vec::new();
    for repo in repos {
        let on = repo.branch.as_deref().unwrap_or_default();
        if on == new || on == "detached" {
            continue;
        }
        run(Path::new(&repo.path), &["branch", "-m", new], None)
            .await
            .map_err(|why| format!("couldn't rename the branch of {}: {why}", repo.name))?;
        renamed.push(repo.name);
    }
    Ok(renamed)
}

/// A folder's own name.
pub fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The clone a name means: the one whose folder is called that, or whose remote ends in it.
/// Upper and lower case are the same. An error says what there is that is close.
pub fn pick<'a>(found: &'a [Found], name: &str) -> Result<&'a Found, String> {
    let wanted = name.trim_end_matches('/').to_lowercase();
    let folder = |one: &Found| name_of(Path::new(&one.path)).to_lowercase();
    let remote = |one: &Found| {
        let end = one.remote.trim_end_matches('/').trim_end_matches(".git");
        end.rsplit('/').next().unwrap_or_default().to_lowercase()
    };
    let by = |named: &dyn Fn(&Found) -> String| found.iter().find(|one| named(one) == wanted);
    if let Some(one) = by(&folder).or_else(|| by(&remote)) {
        return Ok(one);
    }
    let close: Vec<String> = found
        .iter()
        .map(|one| name_of(Path::new(&one.path)))
        .filter(|each| each.to_lowercase().contains(&wanted))
        .take(8)
        .collect();
    Err(match close.is_empty() {
        true => format!("no clone called {name}"),
        false => format!("no clone called {name}. Close to it: {}", close.join(", ")),
    })
}

/// Puts a worktree of `clone` into `folder`, in a folder named after the clone, on `branch`.
/// A branch that is not there yet starts from where the remote's main branch is now.
pub async fn add(clone: &Path, folder: &Path, branch: &str) -> Result<Added, Error> {
    let name = name_of(clone);
    let there = folder.join(&name);
    if there.exists() {
        return Err(Error::Conflict(format!(
            "{name} is already in this workspace"
        )));
    }
    let failed = |why: String| Error::Io(format!("couldn't add {name}: {why}"));
    let text = |out: Vec<u8>| String::from_utf8_lossy(&out).trim().to_string();
    // A worktree that was deleted by hand is still on git's list until pruned.
    let _ = run(clone, &["worktree", "prune"], None).await;
    let at = there.display().to_string();
    let head = format!("refs/heads/{branch}");
    let kept = run(clone, &["show-ref", "--verify", "--quiet", &head], None).await;
    if kept.is_ok() {
        // git refuses a branch that another worktree has checked out, and says where.
        let add = ["worktree", "add", "--quiet", &at, branch];
        run(clone, &add, None).await.map_err(failed)?;
        return Ok(Added {
            name,
            branch: branch.to_string(),
            from: None,
        });
    }
    let asked = ["symbolic-ref", "--short", "refs/remotes/origin/HEAD"];
    let main = run(clone, &asked, None).await.ok().map(text);
    if let Some(short) = main
        .as_deref()
        .and_then(|main| main.strip_prefix("origin/"))
    {
        // Without the network it starts from where the clone last saw the main branch.
        let fetch = ["fetch", "--quiet", "--no-tags", "origin", short];
        let _ = run(clone, &fetch, Some(FETCH)).await;
    }
    let from = main.unwrap_or_else(|| "HEAD".into());
    let add = [
        "worktree",
        "add",
        "--quiet",
        "--no-track",
        "-b",
        branch,
        &at,
        &from,
    ];
    run(clone, &add, None).await.map_err(failed)?;
    Ok(Added {
        name,
        branch: branch.to_string(),
        from: Some(from),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(path: &str, remote: &str) -> Found {
        Found {
            path: path.into(),
            remote: remote.into(),
        }
    }

    #[test]
    fn a_name_means_a_clone_by_its_folder_or_its_remote() {
        let all = [
            found(
                "/code/web",
                "https://dev.azure.com/contoso/Shop/_git/Storefront",
            ),
            found("/code/shop-api", "git@github.com:contoso/api.git"),
        ];
        assert_eq!(pick(&all, "Web").unwrap().path, "/code/web");
        assert_eq!(pick(&all, "storefront").unwrap().path, "/code/web");
        assert_eq!(pick(&all, "api").unwrap().path, "/code/shop-api");
        assert_eq!(
            pick(&all, "shop").unwrap_err(),
            "no clone called shop. Close to it: shop-api"
        );
        assert_eq!(pick(&all, "nope").unwrap_err(), "no clone called nope");
    }

    #[test]
    fn repos_go_into_a_folder_that_is_not_in_one() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("clone/.git")).unwrap();
        std::fs::create_dir_all(root.path().join("clone/src")).unwrap();
        std::fs::create_dir_all(root.path().join("plain")).unwrap();
        assert!(!takes(&root.path().join("clone")));
        assert!(!takes(&root.path().join("clone/src")));
        // The temporary folder itself may be inside a repo on the machine that runs this.
        assert_eq!(takes(&root.path().join("plain")), takes(root.path()));
    }

    fn git(dir: &Path, args: &[&str]) {
        let done = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(done.status.success(), "git {args:?}");
    }

    #[tokio::test]
    async fn a_repo_is_added_as_a_worktree_on_a_branch_of_its_own() {
        let root = tempfile::tempdir().unwrap();
        let (clone, room) = (root.path().join("web"), root.path().join("room"));
        std::fs::create_dir_all(&clone).unwrap();
        std::fs::create_dir_all(&room).unwrap();
        git(&clone, &["init", "--quiet", "-b", "main"]);
        std::fs::write(clone.join("a.txt"), "a").unwrap();
        git(&clone, &["add", "."]);
        let commit = [
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "--quiet",
            "-m",
            "one",
        ];
        git(&clone, &commit);

        let added = add(&clone, &room, "fix-it").await.unwrap();
        assert_eq!(
            (added.name.as_str(), added.branch.as_str()),
            ("web", "fix-it")
        );
        assert!(room.join("web/a.txt").is_file());
        assert_eq!(inside(&room)[0].branch.as_deref(), Some("fix-it"));
        // The next repo here goes on the same branch, where none is named.
        assert_eq!(branch_for(&room), "fix-it");
        // Naming the work renames the branch, and a name git would not take is refused.
        assert_eq!(rename(&room, "login-fix").await.unwrap(), ["web"]);
        assert_eq!(inside(&room)[0].branch.as_deref(), Some("login-fix"));
        assert!(rename(&room, "Not A Name").await.is_err());
        assert!(rename(&room, "main").await.is_err());
        rename(&room, "fix-it").await.unwrap();
        // A workspace with no repo yet names the branch after itself.
        assert_eq!(branch_for(&clone.join(".git")), ".git");
        // The clone is where it was.
        assert_eq!(workspaces::branch(&clone).as_deref(), Some("main"));
        // It is not put in twice.
        assert!(matches!(
            add(&clone, &room, "fix-it").await,
            Err(Error::Conflict(_))
        ));
    }
}
