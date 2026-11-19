//! Tickets: Azure DevOps work items, through dova. Listing, reading,
//! changing and commenting are dova's own commands. What a ticket is
//! linked to and the states its type has come from `dova api`.

use std::collections::HashSet;
use std::path::Path;

use protocol::{Comment, Section, Ticket, TicketRow, TicketState, WorkItemRef, Workspace};
use serde_json::{Value, json};
use tokio::process::Command;

use crate::dova::Dova;
use crate::error::Error;
use crate::review::Reviews;
use crate::{git, pulls};

/// A ticket in one of these states is finished with, whatever its type calls them.
const OVER: [&str; 3] = ["Closed", "Done", "Removed"];

/// Your tickets that are not finished, or with `words` every ticket whose
/// title has them, across every project your workspaces are cloned from.
pub async fn list(
    dova: &Dova,
    workspaces: Vec<Workspace>,
    words: &str,
) -> Result<Vec<TicketRow>, Error> {
    let mut rows = Vec::new();
    let mut asked = HashSet::new();
    for workspace in workspaces.into_iter().filter(|w| !w.scratch) {
        let origin = git::origin(Path::new(&workspace.path)).await;
        let Some((project, _)) = origin.as_deref().and_then(pulls::azure) else {
            continue;
        };
        // dova searches a whole project at once, so each project is asked once.
        if !asked.insert(project.to_lowercase()) {
            continue;
        }
        let params = match words.trim() {
            "" => json!({ "assignee": "me" }),
            words => json!({ "query": words }),
        };
        let found = dova.call("wi.search", &workspace.path, params).await?;
        let items: Vec<WorkItemRef> = shape(found["items"].clone())?;
        rows.extend(
            items
                .into_iter()
                .filter(|item| !words.trim().is_empty() || !over(item.state.as_deref()))
                .map(|item| TicketRow {
                    item,
                    workspace: workspace.id.clone(),
                }),
        );
    }
    Ok(rows)
}

/// A ticket, with what it is linked to and the states it can move to.
pub async fn open(dova: &Dova, reviews: &Reviews, dir: &str, id: u64) -> Result<Ticket, Error> {
    let asked = json!({ "idOrUrl": id.to_string(), "full": true, "inlineImages": true });
    let (view, raw, me) = tokio::join!(
        dova.call("wi.view", dir, asked),
        get(
            dova,
            dir,
            format!("_apis/wit/workitems/{id}?$expand=relations&api-version=7.1")
        ),
        reviews.me(dova, dir),
    );
    let (view, raw, me) = (view?, raw?, me?);
    let text = |key: &str| view[key].as_str().map(str::to_string);
    let (kind, project) = (
        text("type").unwrap_or_default(),
        text("project").unwrap_or_default(),
    );
    // A type's states are asked for in its own project, which need not be the folder's.
    let states = get(
        dova,
        dir,
        format!(
            "/{}/_apis/wit/workitemtypes/{}/states?api-version=7.1",
            segment(&project),
            segment(&kind)
        ),
    )
    .await
    .unwrap_or_default();
    let (pulls, branches) = linked(&raw);
    Ok(Ticket {
        id,
        title: text("title").unwrap_or_default(),
        state: text("state").unwrap_or_default(),
        assigned_to: text("assignedTo"),
        mine: raw["fields"]["System.AssignedTo"]["id"].as_str() == Some(me.as_str()),
        iteration: text("iterationPath"),
        points: view["points"].as_f64(),
        body: shape::<Vec<Section>>(view["body"].clone())?,
        url: text("url").unwrap_or_default(),
        parent: shape::<Option<WorkItemRef>>(view["parent"].clone())?,
        children: shape::<Vec<WorkItemRef>>(view["children"].clone())?,
        comments: shape::<Vec<Comment>>(view["comments"].clone())?,
        pulls,
        branches,
        states: states["value"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|state| {
                Some(TicketState {
                    name: state["name"].as_str()?.to_string(),
                    category: state["category"].as_str().unwrap_or_default().to_string(),
                })
            })
            .collect(),
        kind,
        project,
    })
}

/// Moves a ticket to a state, or assigns it to you or to nobody.
pub async fn set(
    dova: &Dova,
    dir: &str,
    id: u64,
    state: Option<&str>,
    mine: Option<bool>,
) -> Result<(), Error> {
    let mut params = json!({ "idOrUrl": id.to_string() });
    if let Some(state) = state {
        params["state"] = json!(state);
    }
    if let Some(mine) = mine {
        params["assignee"] = json!(if mine { "me" } else { "none" });
    }
    dova.call("wi.edit", dir, params).await.map(|_| ())
}

pub async fn comment(dova: &Dova, dir: &str, id: u64, words: &str) -> Result<(), Error> {
    let params = json!({ "idOrUrl": id.to_string(), "text": words });
    dova.call("wi.comment", dir, params).await.map(|_| ())
}

/// Files a ticket in the project `dir` is cloned from. Its number is the answer.
pub async fn create(dova: &Dova, dir: &str, title: &str, kind: &str) -> Result<u64, Error> {
    let made = dova
        .call("wi.create", dir, json!({ "title": title, "type": kind }))
        .await?;
    made["id"]
        .as_u64()
        .ok_or_else(|| Error::Dova("dova didn't say which ticket it filed".into()))
}

/// Starts work on a ticket in the folder `dir`: a new branch from the
/// repo's main one, linked to the ticket, and the ticket assigned to you.
/// The branch's name is the answer. A folder with changes that are not
/// committed is left alone.
pub async fn start(dova: &Dova, reviews: &Reviews, dir: &str, id: u64) -> Result<String, Error> {
    let ticket = open(dova, reviews, dir, id).await?;
    let folder = Path::new(dir);
    let changes = run(folder, &["status", "--porcelain"])
        .await
        .map_err(Error::Io)?;
    if !changes.trim().is_empty() {
        return Err(Error::Conflict(
            "This folder has changes that are not committed. Commit them or put them aside first."
                .into(),
        ));
    }
    let main = run(
        folder,
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
    )
    .await
    .map_or("origin/main".to_string(), |head| head.trim().to_string());
    // Start from where the main branch is now, if it can be asked.
    let name = main.strip_prefix("origin/").unwrap_or(&main).to_string();
    let _ = run(folder, &["fetch", "--quiet", "--no-tags", "origin", &name]).await;
    let branch = branch_name(&ticket.kind, id, &ticket.title);
    run(
        folder,
        &["switch", "--quiet", "--no-track", "-c", &branch, &main],
    )
    .await
    .map_err(|why| Error::Io(format!("couldn't make the branch {branch}: {why}")))?;
    dova.call("link", dir, json!({ "ids": [id.to_string()] }))
        .await?;
    if !ticket.mine {
        set(dova, dir, id, None, Some(true)).await?;
    }
    Ok(branch)
}

/// What the agent is told about a ticket before anything you write.
pub async fn brief(dova: &Dova, reviews: &Reviews, dir: &str, id: u64) -> Result<String, Error> {
    let ticket = open(dova, reviews, dir, id).await?;
    let mut out = format!(
        "You are helping someone with ticket #{id} \"{title}\", a {kind} in the {project} project on Azure DevOps. It is {state}.\n\
         This folder is the repo the ticket is about, as the person has it checked out. Read the code before you answer about it.\n\
         dova is installed. `dova wi view {id}` reads the ticket as it is now, `dova wi search` finds others, and `dova status` says where this branch stands.\n\
         Do not change the ticket, comment on it, or edit files unless you are asked to: the person does that from the Workbench.\n\
         When you are asked to draft a comment, answer with only the text to post.\n",
        title = ticket.title,
        kind = ticket.kind,
        project = ticket.project,
        state = ticket.state,
    );
    for section in &ticket.body {
        out += &format!("\n## {}\n{}\n", section.label, clip(&section.text, 6_000));
    }
    if let Some(parent) = &ticket.parent {
        out += &format!(
            "\n## Parent\n- #{} {}\n",
            parent.id,
            parent.title.as_deref().unwrap_or("")
        );
    }
    if !ticket.children.is_empty() {
        out += "\n## Children\n";
        for child in &ticket.children {
            let (title, state) = (
                child.title.as_deref().unwrap_or(""),
                child.state.as_deref().unwrap_or(""),
            );
            out += &format!("- #{} {title} ({state})\n", child.id);
        }
    }
    if !ticket.pulls.is_empty() || !ticket.branches.is_empty() {
        out += "\n## Linked work\n";
        for pull in &ticket.pulls {
            out += &format!("- pull request #{pull} (`dova pr view {pull}`)\n");
        }
        for branch in &ticket.branches {
            out += &format!("- branch {branch}\n");
        }
    }
    if !ticket.comments.is_empty() {
        out += "\n## Discussion\n";
        for comment in &ticket.comments {
            let author = comment.author.as_deref().unwrap_or("Someone");
            out += &format!(
                "- {author}: {}\n",
                clip(&comment.text, 1_500).replace('\n', " ")
            );
        }
    }
    Ok(clip(&out, 60_000))
}

async fn get(dova: &Dova, dir: &str, path: String) -> Result<Value, Error> {
    dova.call("api", dir, json!({ "path": path })).await
}

fn shape<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, Error> {
    serde_json::from_value(value)
        .map_err(|err| Error::Dova(format!("couldn't read what dova said: {err}")))
}

fn over(state: Option<&str>) -> bool {
    state.is_some_and(|state| OVER.contains(&state))
}

fn clip(words: &str, most: usize) -> String {
    match words.char_indices().nth(most) {
        Some((cut, _)) => format!("{} […]", &words[..cut]),
        None => words.to_string(),
    }
}

/// A name as one part of a URL's path.
fn segment(name: &str) -> String {
    name.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// The pull requests and branches a work item is linked to, from its relations.
/// Azure DevOps names them as `vstfs:///Git/PullRequestId/<project>%2F<repo>%2F<number>`
/// and `vstfs:///Git/Ref/<project>%2F<repo>%2FGB<branch>`.
fn linked(raw: &Value) -> (Vec<u64>, Vec<String>) {
    let (mut pulls, mut branches) = (Vec::new(), Vec::new());
    for relation in raw["relations"].as_array().into_iter().flatten() {
        let Some(url) = relation["url"].as_str() else {
            continue;
        };
        // The name is three parts joined by an encoded slash, and a branch has slashes of its own.
        let name = url
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .replace("%2f", "%2F");
        let Some(last) = name.splitn(3, "%2F").nth(2) else {
            continue;
        };
        if url.contains("/Git/PullRequestId/") {
            pulls.extend(last.parse::<u64>().ok());
        } else if url.contains("/Git/Ref/") {
            branches.extend(
                last.strip_prefix("GB")
                    .map(|branch| branch.replace("%2F", "/")),
            );
        }
    }
    pulls.sort_unstable();
    (pulls, branches)
}

/// `fix/5219-null-check-missing` for a bug, `feature/…` for anything else:
/// the ticket's number and the first words of its title.
fn branch_name(kind: &str, id: u64, title: &str) -> String {
    let words: Vec<String> = title
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(4)
        .map(str::to_lowercase)
        .collect();
    let prefix = if kind.eq_ignore_ascii_case("bug") {
        "fix"
    } else {
        "feature"
    };
    match words.is_empty() {
        true => format!("{prefix}/{id}"),
        false => format!("{prefix}/{id}-{}", words.join("-")),
    }
}

/// Runs git in `dir`, giving what it printed, or why it failed.
async fn run(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .map_err(|err| format!("couldn't run git: {err}"))?;
    match output.status.success() {
        true => Ok(String::from_utf8_lossy(&output.stdout).to_string()),
        false => Err(String::from_utf8_lossy(&output.stderr).trim().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_a_branch_after_its_ticket() {
        assert_eq!(
            branch_name("Bug", 5219, "Null check missing on empty session token"),
            "fix/5219-null-check-missing-on"
        );
        assert_eq!(
            branch_name("Task", 6, "Add retry to login"),
            "feature/6-add-retry-to-login"
        );
        assert_eq!(branch_name("Task", 7, "¿?"), "feature/7");
    }

    #[test]
    fn reads_what_a_ticket_is_linked_to() {
        let raw = json!({ "relations": [
            { "rel": "ArtifactLink", "url": "vstfs:///Git/PullRequestId/9cae%2f378a%2f8" },
            { "rel": "ArtifactLink", "url": "vstfs:///Git/PullRequestId/9cae%2F378a%2F1" },
            { "rel": "ArtifactLink", "url": "vstfs:///Git/Ref/9cae%2F378a%2FGBfix%2F1-login-redirect" },
            { "rel": "System.LinkTypes.Hierarchy-Reverse", "url": "https://dev.azure.com/o/_apis/wit/workItems/4" },
        ] });
        let (pulls, branches) = linked(&raw);
        assert_eq!(pulls, [1, 8]);
        assert_eq!(branches, ["fix/1-login-redirect"]);
    }

    #[test]
    fn small_things() {
        assert!(over(Some("Done")) && !over(Some("Active")) && !over(None));
        assert_eq!(segment("Product Backlog Item"), "Product%20Backlog%20Item");
    }
}
