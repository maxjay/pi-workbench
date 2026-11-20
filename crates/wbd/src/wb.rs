//! `wb`: the command an agent runs to ask the workbench for something. It is wbd itself under
//! another name, put on the PATH of every agent the workbench starts, so neither Pi nor
//! Claude Code needs anything set up. It speaks to the wbd that started the agent over a
//! socket: one line of JSON each way.
//!
//! `wb repos [words]` lists your clones. `wb add <repo>...` puts a worktree of each into the
//! conversation's codespace, after you said yes to it on a card in the conversation.
//! `wb name <name>` names the work: the codespace, and the branch in every repo in it.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use protocol::{Ask, Codespace, Event};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

use crate::{Ctx, repos};

/// `wb repos` prints no more clones than this at once.
const LISTED: usize = 200;

const USAGE: &str = "wb name <name>       name the work: the codespace, and the branch in every repo here\n\
                     wb repos [words]     the user's clones, or those with any of the words\n\
                     wb add <repo>...     put a worktree of each into this codespace";

/// What an agent in a codespace that takes repos is told about it and about `wb`, added to
/// what it is told about itself.
pub fn note(codespace: Option<&Codespace>, folder: &Path) -> String {
    let here: Vec<String> = repos::inside(folder)
        .into_iter()
        .map(|repo| repo.name)
        .collect();
    let main = codespace.and_then(|codespace| codespace.main.as_deref());
    let mut out = String::from(
        "This folder is a codespace in the Workbench app, for one piece of work. It holds the \
         repositories of the user that the work needs, each in a folder of its own, as a git \
         worktree on the work's branch.\n",
    );
    match here.as_slice() {
        [] => out.push_str("No repository is here yet.\n"),
        names => {
            let listed: Vec<String> = names
                .iter()
                .map(|name| match Some(name.as_str()) == main {
                    true => format!("./{name} (the main one)"),
                    false => format!("./{name}"),
                })
                .collect();
            out.push_str(&format!(
                "Here now: {}. A repository's own instructions for agents (AGENTS.md, CLAUDE.md) \
                 are in its folder: read them before you work in it.\n",
                listed.join(", ")
            ));
        }
    }
    match codespace.filter(|codespace| codespace.named) {
        Some(codespace) => out.push_str(&format!(
            "The work is named {}. That is the branch in every repository here.\n",
            codespace.work.as_deref().unwrap_or(&codespace.name)
        )),
        None => out.push_str(
            "The work has no name yet. As soon as you know what it is, name it, once, with \
             `wb name <name>`: two to four words in lower case with dashes, like \
             fix-login-redirect. It becomes the branch in every repository here and the title \
             the user sees.\n",
        ),
    }
    out.push_str(
        "`wb repos [words]` lists the user's clones. `wb add <repo>...` puts a worktree of each \
         into this folder at ./<repo>, on the work's branch, after the user confirms it in the \
         app: wait for it, the command prints what was added or that the user said no.\n\
         When the work needs a repository that is not in this folder, add it with `wb add`. \
         Do not clone it yourself, and do not change files in the user's own clones. \
         Run a `wb` command by itself, not chained with other commands.",
    );
    out
}

/// The socket wbd listens on for `wb`, and what an agent's environment needs to reach it.
pub struct Door {
    pub listener: UnixListener,
    pub socket: PathBuf,
    pub env: Vec<(String, String)>,
}

/// Makes the socket in `data` and `wb` in `data/bin`. Nothing, where either can't be made:
/// the agents then run without `wb`.
pub fn open(data: &Path) -> Option<Door> {
    std::fs::create_dir_all(data).ok()?;
    // A socket left by a wbd that is gone.
    for entry in std::fs::read_dir(data).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let pid = name
            .strip_prefix("wb.")
            .and_then(|rest| rest.strip_suffix(".sock"));
        if pid.is_some_and(|pid| !Path::new("/proc").join(pid).exists()) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    let socket = data.join(format!("wb.{}.sock", std::process::id()));
    let listener = UnixListener::bind(&socket).ok()?;

    let bin = data.join("bin");
    std::fs::create_dir_all(&bin).ok()?;
    let (wb, wbd) = (bin.join("wb"), std::env::current_exe().ok()?);
    if std::fs::read_link(&wb).ok().as_ref() != Some(&wbd) {
        let _ = std::fs::remove_file(&wb);
        std::os::unix::fs::symlink(&wbd, &wb).ok()?;
    }
    let path = match std::env::var("PATH") {
        Ok(path) => format!("{}:{path}", bin.display()),
        Err(_) => bin.display().to_string(),
    };
    Some(Door {
        listener,
        env: vec![
            ("PATH".into(), path),
            ("WB_SOCKET".into(), socket.display().to_string()),
        ],
        socket,
    })
}

/// Answers each `wb` that connects, for as long as wbd runs.
pub async fn serve(listener: UnixListener, ctx: Arc<Ctx>) {
    while let Ok((stream, _)) = listener.accept().await {
        let ctx = ctx.clone();
        tokio::spawn(async move {
            let (read, mut write) = stream.into_split();
            let mut lines = BufReader::new(read).lines();
            let Ok(Some(line)) = lines.next_line().await else {
                return;
            };
            let asked: Value = serde_json::from_str(&line).unwrap_or_default();
            // The next thing read is the end: `wb` hung up, because the agent ended it.
            let hung_up = async {
                let _ = lines.next_line().await;
            };
            let what = format!(
                "an agent ran wb {} {}",
                asked["ask"].as_str().unwrap_or_default(),
                crate::log::given(&asked, "ask")
            );
            let running = crate::log::begin(protocol::LogKind::Run, what.trim_end(), None);
            let said = match asked["ask"].as_str() {
                Some("repos") => Ok(listed(&ctx, &words(&asked["words"])).await),
                Some("add") => add(&ctx, &asked, hung_up).await,
                Some("name") => name(&ctx, &asked).await,
                _ => Err(USAGE.to_string()),
            };
            running.end(&said);
            let answer = match said {
                Ok(text) => json!({ "ok": true, "text": text }),
                Err(text) => json!({ "ok": false, "text": text }),
            };
            let _ = write.write_all(format!("{answer}\n").as_bytes()).await;
        });
    }
}

fn words(value: &Value) -> Vec<String> {
    let all = value.as_array().into_iter().flatten();
    all.filter_map(|word| word.as_str().map(str::to_string))
        .collect()
}

/// Your clones, or with words those whose folder or remote has any of them, a line each.
async fn listed(ctx: &Ctx, words: &[String]) -> String {
    let found = ctx.places.found().await;
    let (folder, _) = ctx.places.written();
    let matching: Vec<String> = found
        .iter()
        .filter(|one| {
            let text = format!("{} {}", one.path, one.remote).to_lowercase();
            words.is_empty() || words.iter().any(|word| text.contains(&word.to_lowercase()))
        })
        .map(|one| format!("{}\t{}", repos::name_of(Path::new(&one.path)), one.path))
        .collect();
    if matching.is_empty() {
        return match words.is_empty() {
            true => format!("No clones in {folder}."),
            false => format!("No clone in {folder} matches."),
        };
    }
    let mut out = matching[..matching.len().min(LISTED)].join("\n");
    if matching.len() > LISTED {
        let more = matching.len() - LISTED;
        out.push_str(&format!("\n{more} more. Give words to narrow them."));
    }
    out
}

/// `wb name`: names the work, once. The branch in every repo of the codespace is renamed.
async fn name(ctx: &Ctx, asked: &Value) -> Result<String, String> {
    let conversation = asked["conversation"].as_str().unwrap_or_default();
    let Some(cwd) = ctx.conversations.cwd(conversation) else {
        return Err("wb works in a conversation started in the Workbench.".into());
    };
    let folder = Path::new(&cwd);
    let codespace = ctx.codespaces.at(folder);
    let Some(codespace) = codespace.filter(|codespace| codespace.scratch) else {
        return Err("This folder is not a codespace the Workbench made. Its name stays.".into());
    };
    let was = codespace.work.as_deref().unwrap_or(&codespace.name);
    let Some(new) = asked["name"].as_str().filter(|name| !name.is_empty()) else {
        return Ok(format!("The work is called {was}."));
    };
    if codespace.named {
        return Ok(format!(
            "The work already has its name: {was}. Nothing was changed."
        ));
    }
    let renamed = repos::rename(folder, new).await?;
    let failed = |err| protocol::Failure::from(err).message;
    ctx.codespaces.name(folder, new).map_err(failed)?;
    let _ = ctx.out.send(protocol::event(&Event::Codespaces));
    Ok(match renamed.is_empty() {
        true => format!("The work is named {new}."),
        false => format!(
            "The work is named {new}. That is now the branch in {}.",
            all_of(&renamed)
        ),
    })
}

/// `wb add`: asks you on a card in the conversation, then puts the worktrees in.
async fn add(
    ctx: &Ctx,
    asked: &Value,
    hung_up: impl Future<Output = ()>,
) -> Result<String, String> {
    let names = words(&asked["repos"]);
    let conversation = asked["conversation"].as_str().unwrap_or_default();
    let Some(cwd) = ctx.conversations.cwd(conversation) else {
        return Err("wb works in a conversation started in the Workbench.".into());
    };
    let folder = Path::new(&cwd);
    if !repos::takes(folder) {
        return Err(
            "This folder is a git repository, or inside one. A repo is added to a codespace \
             that is not a repo itself: a scratchpad."
                .into(),
        );
    }
    if names.is_empty() {
        return Err(USAGE.into());
    }
    let found = ctx.places.found().await;
    let mut clones = Vec::new();
    for name in &names {
        let clone = repos::pick(&found, name)
            .map_err(|why| format!("{why}. `wb repos` lists the clones there are."))?;
        let own = repos::name_of(Path::new(&clone.path));
        if folder.join(&own).exists() {
            return Err(format!("{own} is already in this codespace, at ./{own}"));
        }
        if !clones.contains(&clone.path) {
            clones.push(clone.path.clone());
        }
    }
    let codespace = ctx.codespaces.at(folder);
    let branch = match codespace.and_then(|codespace| codespace.work) {
        Some(work) => work,
        None => repos::branch_for(folder),
    };

    static ASKED: AtomicU64 = AtomicU64::new(1);
    let listing: Vec<String> = clones
        .iter()
        .map(|clone| repos::name_of(Path::new(clone)))
        .collect();
    let ask = Ask {
        id: format!("wb-{}", ASKED.fetch_add(1, Ordering::Relaxed)),
        title: format!("Add {} to this codespace?", all_of(&listing)),
        message: Some(format!(
            "A worktree of {} in this codespace's folder, on the branch {branch}. {} not changed.",
            match clones.len() {
                1 => "your clone",
                _ => "each of your clones",
            },
            match clones.len() {
                1 => "The clone is",
                _ => "The clones are",
            },
        )),
        options: vec!["Add".into(), "No".into()],
        input: false,
    };
    let id = ask.id.clone();
    let answer = ctx
        .conversations
        .ask(conversation, ask)
        .map_err(|_| "This conversation is not running.".to_string())?;
    let answer = tokio::select! {
        answer = answer => answer.ok().flatten(),
        () = hung_up => {
            ctx.conversations.withdraw(conversation, &id);
            return Err("wb was ended before the user answered.".into());
        }
    };
    if answer.as_deref() != Some("Add") {
        return Err("The user said no. Nothing was added.".into());
    }

    let mut out = String::from("Added to this codespace:\n");
    for clone in &clones {
        let added = repos::add(Path::new(clone), folder, &branch)
            .await
            .map_err(|err| format!("{out}{}", protocol::Failure::from(err).message))?;
        let from = added
            .from
            .map_or_else(String::new, |from| format!(", new from {from}"));
        out.push_str(&format!(
            "  ./{}  a worktree of {clone}, on the branch {}{from}\n",
            added.name, added.branch
        ));
        let _ = ctx.codespaces.took(folder, &added.name);
    }
    let _ = ctx.out.send(protocol::event(&Event::Codespaces));
    out.push_str("Work, commit and push in these folders.");
    Ok(out)
}

/// `a`, `a and b`, `a, b and c`.
fn all_of(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [most @ .., last] => format!("{} and {last}", most.join(", ")),
    }
}

/// `wb`, as the agent runs it: asks the wbd named by `WB_SOCKET` and prints what it says.
pub fn client(args: &[String]) -> ExitCode {
    let asked = match args.first().map(String::as_str) {
        Some("repos") => json!({ "ask": "repos", "words": &args[1..] }),
        Some("add") => json!({
            "ask": "add",
            "repos": &args[1..],
            "conversation": std::env::var("WB_CONVERSATION").unwrap_or_default(),
        }),
        Some("name") => json!({
            "ask": "name",
            "name": args.get(1),
            "conversation": std::env::var("WB_CONVERSATION").unwrap_or_default(),
        }),
        _ => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
    };
    let said = (|| -> Option<Value> {
        let socket = std::env::var_os("WB_SOCKET")?;
        let mut stream = std::os::unix::net::UnixStream::connect(socket).ok()?;
        stream.write_all(format!("{asked}\n").as_bytes()).ok()?;
        let mut line = String::new();
        std::io::BufReader::new(stream).read_line(&mut line).ok()?;
        serde_json::from_str(&line).ok()
    })();
    let Some(said) = said else {
        eprintln!("wb could not reach the Workbench. It works in a conversation started there.");
        return ExitCode::FAILURE;
    };
    let text = said["text"].as_str().unwrap_or_default();
    match said["ok"] == true {
        true => {
            println!("{text}");
            ExitCode::SUCCESS
        }
        false => {
            eprintln!("{text}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_read_as_a_sentence() {
        let names = |all: &[&str]| all.iter().map(|name| name.to_string()).collect::<Vec<_>>();
        assert_eq!(all_of(&names(&["web"])), "web");
        assert_eq!(all_of(&names(&["web", "api"])), "web and api");
        assert_eq!(all_of(&names(&["web", "api", "docs"])), "web, api and docs");
    }
}
