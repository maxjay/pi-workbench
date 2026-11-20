//! wbd: the workbench's process inside WSL, next to the code and the agents.
//! It reads the page's calls from stdin, one JSON object per line, and
//! writes answers and events to stdout through one writer.

mod agent;
mod codespaces;
mod conversations;
mod dova;
mod error;
mod files;
mod fold;
mod git;
mod health;
mod log;
mod places;
mod pulls;
mod repos;
mod review;
mod terminals;
mod tickets;
mod wb;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use codespaces::Codespaces;
use conversations::Conversations;
use error::Error;
use protocol::{Answer, Call, CodespaceView, Event, Failure, FailureKind, LogKind, Request};
use terminals::Terminals;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What wbd keeps between calls.
struct Ctx {
    /// wbd's one writer: lines for the page.
    out: mpsc::UnboundedSender<String>,
    codespaces: Arc<Codespaces>,
    places: places::Places,
    terminals: Terminals,
    conversations: Conversations,
    dova: dova::Dova,
    pulls: pulls::Pulls,
    reviews: review::Reviews,
    forms: tickets::Forms,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    // Under the name `wb` it is the command an agent runs, not the workbench's own process.
    let args: Vec<String> = std::env::args().collect();
    let named = args.first().map(|name| repos::name_of(Path::new(name)));
    if named.as_deref() == Some("wb") {
        return wb::client(&args[1..]);
    }
    if args.get(1).map(String::as_str) == Some("--version") {
        println!("{VERSION}");
        return ExitCode::SUCCESS;
    }
    // A bug that stops one call is written where the Log page shows it.
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::note(LogKind::Wbd, &format!("wbd failed: {info}"));
        before(info);
    }));
    let started = format!(
        "wbd {VERSION} started. Its own files are in {}",
        data_dir().display()
    );
    log::note(LogKind::Wbd, &started);

    // Everything wbd says goes through this one writer, a line at a time.
    // It stops when told to, after writing what was already said: the
    // terminals keep a sender for as long as wbd runs, so the channel
    // never closes by itself.
    let (out, mut lines) = mpsc::unbounded_channel::<String>();
    let (stop, mut stopped) = oneshot::channel::<()>();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        loop {
            let line = tokio::select! {
                line = lines.recv() => line,
                _ = &mut stopped => None,
            };
            let Some(line) = line else { break };
            if write(&mut stdout, line).await.is_err() {
                return;
            }
        }
        while let Ok(line) = lines.try_recv() {
            if write(&mut stdout, line).await.is_err() {
                return;
            }
        }
    });

    // Where `wb` reaches this wbd. Without it the agents run as before.
    let door = wb::open(&data_dir());
    let (env, socket) = match &door {
        Some(door) => (door.env.clone(), Some(door.socket.clone())),
        None => (Vec::new(), None),
    };
    let codespaces = Arc::new(Codespaces::new(&data_dir()));
    let ctx = Arc::new(Ctx {
        out: out.clone(),
        codespaces: codespaces.clone(),
        places: places::Places::new(&data_dir()),
        terminals: Terminals::new(out.clone()),
        conversations: Conversations::new(&data_dir(), out.clone(), env, codespaces),
        dova: dova::Dova::default(),
        pulls: pulls::Pulls::default(),
        reviews: review::Reviews::default(),
        forms: tickets::Forms::default(),
    });
    ctx.terminals.start();
    if let Some(door) = door {
        tokio::spawn(wb::serve(door.listener, ctx.clone()));
    }

    // The checkouts of pull requests that ended are removed: a while after starting, then
    // each hour.
    let sweeping = ctx.clone();
    tokio::spawn(async move {
        let mut wait = std::time::Duration::from_secs(30);
        loop {
            tokio::time::sleep(wait).await;
            let own = sweeping.places.own();
            sweeping
                .reviews
                .sweep(&sweeping.dova, &own, &data_dir())
                .await;
            wait = std::time::Duration::from_secs(3600);
        }
    });

    let _ = out.send(protocol::event(&Event::Ready {
        version: VERSION.into(),
    }));

    // Each call is answered on its own, so a slow one doesn't hold up the rest.
    let mut answering = JoinSet::new();
    let mut calls = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = calls.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        // What you type in a terminal is handled here, in the order it came.
        if line.contains(r#""method":"terminal.input""#)
            || line.contains(r#""method":"terminal.resize""#)
        {
            let _ = out.send(respond(&ctx, &line).await);
            continue;
        }
        let (out, ctx) = (out.clone(), ctx.clone());
        answering.spawn(async move {
            let _ = out.send(respond(&ctx, &line).await);
        });
    }

    // The page hung up. Finish what was asked, end the agents and the shells, then stop.
    while answering.join_next().await.is_some() {}
    ctx.conversations.shutdown();
    ctx.terminals.shutdown();
    if let Some(socket) = socket {
        let _ = std::fs::remove_file(socket);
    }
    let _ = stop.send(());
    let _ = writer.await;
    ExitCode::SUCCESS
}

async fn write(stdout: &mut tokio::io::Stdout, mut line: String) -> std::io::Result<()> {
    line.push('\n');
    stdout.write_all(line.as_bytes()).await?;
    stdout.flush().await
}

/// Where wbd keeps its own files: `WB_DATA_DIR`, or `workbench` in your data folder.
fn data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("WB_DATA_DIR") {
        return dir.into();
    }
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    data.unwrap_or_else(|| ".".into()).join("workbench")
}

/// The line that answers one line from the page.
async fn respond(ctx: &Ctx, line: &str) -> String {
    match serde_json::from_str::<Request>(line) {
        Ok(Request { id, call }) => {
            let running = written(&call);
            match answer(ctx, call).await {
                Ok(answer) => {
                    if let Some(running) = running {
                        running.done();
                    }
                    protocol::reply(id, &answer)
                }
                Err(err) => {
                    let failure: Failure = err.into();
                    if let Some(running) = running {
                        running.failed(&failure.message);
                    }
                    protocol::failed(Some(id), &failure)
                }
            }
        }
        Err(err) => {
            let said = format!("the page sent a line that is not a call: {err}");
            log::note(LogKind::Wbd, &said);
            protocol::failed(
                id_of(line),
                &Failure::new(FailureKind::BadRequest, err.to_string()),
            )
        }
    }
}

/// Writes a call in the log, with what it was given. What you type in a terminal and the
/// reading of the log itself are left out. What the page asks every few seconds is quiet.
fn written(call: &Call) -> Option<log::Running> {
    let quiet = match call {
        Call::LogRead { .. } | Call::TerminalInput { .. } | Call::TerminalResize { .. } => {
            return None;
        }
        Call::GitStatus { .. } | Call::FilesStat { .. } => true,
        _ => false,
    };
    let said = serde_json::to_value(call).ok()?;
    let params = &said["params"];
    let place = ["dir", "cwd", "path"]
        .into_iter()
        .find(|name| params[name].is_string());
    let given = log::given(params, place.unwrap_or_default());
    let what = format!("{} {given}", said["method"].as_str()?);
    let place = place.and_then(|name| params[name].as_str());
    Some(match quiet {
        true => log::begin_quiet(LogKind::Call, what.trim_end(), place),
        false => log::begin(LogKind::Call, what.trim_end(), place),
    })
}

async fn answer(ctx: &Ctx, call: Call) -> Result<Answer, Error> {
    Ok(match call {
        Call::Health => Answer::Health(health::check().await),

        Call::CodespacesList => {
            let mut list = Vec::new();
            for codespace in ctx.codespaces.all() {
                list.push(view(codespace));
            }
            Answer::Codespaces { list }
        }
        Call::CodespacesOpen { path } => Answer::Codespace(view(ctx.codespaces.open(&path)?)),
        Call::CodespacesForget { id } => {
            ctx.codespaces.forget(&id)?;
            Answer::Done
        }
        Call::ScratchNew { text, repo } => {
            let under = ctx.places.own().join("codespaces");
            let main = repo
                .as_deref()
                .map(|clone| repos::name_of(Path::new(clone)));
            let codespace = ctx.codespaces.scratch(&under, &text, main.as_deref())?;
            if let Some(clone) = repo {
                let folder = Path::new(&codespace.path);
                let work = codespace.work.clone().unwrap_or_default();
                // A codespace whose repo could not be put in is not left behind.
                if let Err(err) = repos::add(Path::new(&clone), folder, &work).await {
                    let _ = std::fs::remove_dir(folder);
                    ctx.codespaces.forget(&codespace.id)?;
                    return Err(err);
                }
            }
            Answer::Codespace(view(codespace))
        }
        Call::ReposList => {
            let found = ctx.places.found().await;
            let folder = |one: &places::Found| {
                let path = Path::new(&one.path);
                protocol::Folder {
                    name: repos::name_of(path),
                    path: one.path.clone(),
                    branch: codespaces::branch(path),
                }
            };
            Answer::Folders {
                dir: ctx.places.repos().display().to_string(),
                folders: found.iter().map(folder).collect(),
            }
        }
        Call::CodespaceRepos { dir } => Answer::Folders {
            folders: repos::inside(Path::new(&dir)),
            dir,
        },
        Call::CodespaceAdd { dir, clone, branch } => {
            let folder = Path::new(&dir);
            if !repos::takes(folder) {
                return Err(Error::Conflict(
                    "this codespace is a repo itself. A repo is added to one that is not".into(),
                ));
            }
            let work = ctx
                .codespaces
                .at(folder)
                .and_then(|codespace| codespace.work);
            let branch = branch.or(work).unwrap_or_else(|| repos::branch_for(folder));
            let added = repos::add(Path::new(&clone), folder, &branch).await?;
            ctx.codespaces.took(folder, &added.name)?;
            let _ = ctx.out.send(protocol::event(&Event::Codespaces));
            Answer::Done
        }
        Call::FoldersList { dir } => {
            let (dir, folders) = codespaces::folders(&dir)?;
            Answer::Folders { dir, folders }
        }

        Call::FolderRead { dir } => {
            // A big folder on a slow drive takes a while, so it is read off this thread.
            let reading = tokio::task::spawn_blocking(move || {
                let dir = Path::new(&dir);
                files::entries(dir).map_err(|err| Error::io(dir, err))
            });
            let (folders, files) = reading.await.map_err(|err| Error::Io(err.to_string()))??;
            Answer::Entries { folders, files }
        }
        Call::FilesList { dir } => {
            let (files, truncated) = files::list(Path::new(&dir)).await;
            Answer::Files { files, truncated }
        }
        Call::FileRead { path } => {
            let path = Path::new(&path);
            Answer::File(files::read(path).map_err(|err| Error::io(path, err))?)
        }
        Call::FileWrite {
            path,
            text,
            expected,
        } => {
            let path = Path::new(&path);
            match files::write(path, &text, expected.as_deref()) {
                Ok(modified) => Answer::Saved { modified },
                Err(files::Unsaved::Changed) => {
                    return Err(Error::Conflict(format!(
                        "{} changed since it was opened",
                        path.display()
                    )));
                }
                Err(files::Unsaved::Io(err)) => return Err(Error::io(path, err)),
            }
        }
        Call::FilesStat { paths } => Answer::Stat {
            modified: files::stat(&paths),
        },

        Call::GitStatus { dir } => Answer::Git {
            status: git::status(Path::new(&dir)).await,
        },
        Call::GitOriginal { path } => Answer::Original {
            text: git::original(Path::new(&path)).await,
        },

        Call::TerminalsList { dir } => Answer::Terminals {
            list: ctx.terminals.list(Path::new(&dir)),
        },
        Call::TerminalOpen { cwd, cols, rows } => {
            Answer::Terminal(ctx.terminals.open(Path::new(&cwd), cols, rows)?)
        }
        Call::TerminalRead { id } => Answer::Printed {
            text: ctx.terminals.read(&id)?,
        },
        Call::TerminalInput { id, data } => {
            ctx.terminals.input(&id, &data)?;
            Answer::Done
        }
        Call::TerminalResize { id, cols, rows } => {
            ctx.terminals.resize(&id, cols, rows)?;
            Answer::Done
        }
        Call::TerminalClose { id } => {
            ctx.terminals.close(&id)?;
            Answer::Done
        }

        Call::ConversationsList { cwd } => Answer::Conversations {
            list: ctx.conversations.list(&cwd),
        },
        Call::ConversationStart {
            agent,
            cwd,
            text,
            model,
            effort,
            brief,
        } => {
            let started = ctx
                .conversations
                .start(agent, &cwd, &text, model, effort, brief);
            Answer::Conversation(started.await?)
        }
        Call::ConversationSet { id, model, effort } => {
            ctx.conversations.set(&id, model, effort).await?;
            Answer::Done
        }
        Call::AgentOffer { agent } => Answer::Offer(agent::offer(agent).await?),
        Call::ConversationOpen { id } => Answer::Conversation(ctx.conversations.open(&id)?),
        Call::ConversationSend { id, text } => {
            ctx.conversations.send(&id, &text).await?;
            Answer::Done
        }
        Call::ConversationStop { id } => {
            ctx.conversations.stop(&id).await?;
            Answer::Done
        }
        Call::PullStatus { dir, pr } => Answer::Pull(pulls::status(&ctx.dova, &dir, pr).await?),
        Call::PullOpen { dir, pr } => Answer::Review(ctx.reviews.open(&ctx.dova, &dir, pr).await?),
        Call::PullFiles { dir, pr, paths } => Answer::Changed {
            files: ctx.reviews.files(&ctx.dova, &dir, pr, &paths).await?,
        },
        Call::PullDiff { dir, pr, path } => Answer::FileDiff {
            lines: review::diff(&ctx.dova, &dir, pr, &path).await?,
        },
        Call::PullComment {
            dir,
            pr,
            text,
            place,
        } => {
            review::comment(&ctx.dova, &dir, pr, &text, place).await?;
            Answer::Done
        }
        Call::PullVote { dir, pr, vote } => {
            ctx.reviews.vote(&ctx.dova, &dir, pr, vote).await?;
            Answer::Done
        }
        Call::PullEdit {
            dir,
            pr,
            title,
            description,
        } => {
            review::edit(&ctx.dova, &dir, pr, title, description).await?;
            Answer::Done
        }
        Call::PullDescription { dir, pr } => Answer::Printed {
            text: ctx.reviews.description(&ctx.dova, &dir, pr).await?,
        },
        Call::PullPicture { name, data } => Answer::Printed {
            text: review::keep_picture(&data_dir(), &name, &data)?,
        },
        Call::PullSet { dir, pr, to } => {
            ctx.reviews.set(&ctx.dova, &dir, pr, to).await?;
            Answer::Done
        }
        Call::PullChecks { dir, pr } => Answer::Checks {
            runs: ctx.reviews.checks(&ctx.dova, &dir, pr).await?,
        },
        Call::PullRun { dir, run } => {
            let (steps, errors) = review::steps(&ctx.dova, &dir, run).await?;
            Answer::Run { steps, errors }
        }
        Call::PullStepLog { dir, run, log } => Answer::Printed {
            text: review::step_log(&ctx.dova, &dir, run, log).await?,
        },
        Call::PullRerun { dir, run } => {
            review::rerun(&ctx.dova, &dir, run).await?;
            Answer::Done
        }
        Call::TicketsList { words } => Answer::Tickets {
            rows: {
                let found = ctx.places.found().await;
                let (clones, _) = pulls::clones(ctx.codespaces.all(), &found).await;
                tickets::list(&ctx.dova, &clones, &words).await?
            },
        },
        Call::TicketOpen { dir, id } => {
            Answer::Ticket(tickets::open(&ctx.dova, &ctx.reviews, &ctx.forms, &dir, id).await?)
        }
        Call::TicketEdit {
            dir,
            id,
            title,
            parts,
        } => {
            tickets::edit(&ctx.dova, &dir, id, title, parts).await?;
            Answer::Done
        }
        Call::TicketField {
            dir,
            id,
            field,
            value,
        } => {
            tickets::set_field(&ctx.dova, &dir, id, &field, value).await?;
            Answer::Done
        }
        Call::TicketChild { dir, id, title } => Answer::Filed {
            id: tickets::child(&ctx.dova, &ctx.reviews, &ctx.forms, &dir, id, &title).await?,
        },
        Call::TicketUnlink { dir, id, child } => {
            tickets::unlink(&ctx.dova, &dir, id, child).await?;
            Answer::Done
        }
        Call::TicketDone { dir, id, done } => {
            tickets::done(&ctx.dova, &ctx.forms, &dir, id, done).await?;
            Answer::Done
        }
        Call::TicketSet {
            dir,
            id,
            state,
            mine,
        } => {
            tickets::set(&ctx.dova, &dir, id, state.as_deref(), mine).await?;
            Answer::Done
        }
        Call::TicketComment { dir, id, text } => {
            tickets::comment(&ctx.dova, &dir, id, &text).await?;
            Answer::Done
        }
        Call::TicketCreate {
            dir,
            title,
            kind,
            team,
        } => Answer::Filed {
            id: tickets::create(&ctx.dova, &dir, &title, &kind, team.as_deref()).await?,
        },
        Call::TicketStart { dir, id } => Answer::Started {
            branch: tickets::start(&ctx.dova, &ctx.reviews, &ctx.forms, &dir, id).await?,
        },
        Call::TicketSession { dir, id } => Answer::Session {
            brief: tickets::brief(&ctx.dova, &ctx.reviews, &ctx.forms, &dir, id).await?,
            cwd: dir,
        },
        Call::PullSession { dir, pr } => {
            let (own, data, found) = (ctx.places.own(), data_dir(), ctx.places.found().await);
            let at = review::Folders {
                own: &own,
                data: &data,
                found: &found,
            };
            let (cwd, brief) = ctx.reviews.session(&ctx.dova, &at, &dir, pr).await?;
            Answer::Session { cwd, brief }
        }
        Call::PullPlace { dir, pr } => {
            let (own, data, found) = (ctx.places.own(), data_dir(), ctx.places.found().await);
            let at = review::Folders {
                own: &own,
                data: &data,
                found: &found,
            };
            let cwd = ctx.reviews.place(&ctx.dova, &at, &dir, pr).await?;
            Answer::Place {
                cwd: cwd.display().to_string(),
            }
        }
        Call::PullsSweep => {
            let own = ctx.places.own();
            ctx.reviews.sweep(&ctx.dova, &own, &data_dir()).await;
            Answer::Done
        }
        Call::SettingsGet => Answer::Settings(settings(ctx).await),
        Call::SettingsSet { repos, own } => {
            ctx.places.set(&repos, &own)?;
            Answer::Settings(settings(ctx).await)
        }
        Call::PullReply {
            dir,
            pr,
            thread,
            text,
            resolve,
        } => {
            pulls::reply(&ctx.dova, &dir, pr, thread, &text, resolve).await?;
            Answer::Done
        }
        Call::PullResolve {
            dir,
            pr,
            thread,
            status,
        } => {
            pulls::resolve(&ctx.dova, &dir, pr, thread, &status).await?;
            Answer::Done
        }
        Call::PullBefore { dir } => Answer::Before(pulls::before(&ctx.dova, &dir).await?),
        Call::PullCreate {
            dir,
            title,
            body,
            draft,
        } => Answer::Created(pulls::create(&ctx.dova, &dir, &title, &body, draft).await?),
        Call::PullLink { dir, ids } => {
            pulls::link(&ctx.dova, &dir, &ids).await?;
            Answer::Done
        }
        Call::PullsTeams => Answer::Teams {
            teams: ctx
                .pulls
                .teams_with_members(
                    &ctx.dova,
                    &ctx.reviews,
                    ctx.codespaces.all(),
                    &ctx.places.found().await,
                )
                .await?,
        },
        Call::PullsFind { text } => {
            let (people, repos, projects) = ctx
                .pulls
                .find(
                    &ctx.dova,
                    ctx.codespaces.all(),
                    &ctx.places.found().await,
                    &text,
                )
                .await;
            Answer::Found {
                people,
                repos,
                projects,
            }
        }
        Call::PullsChosen {
            people,
            repos,
            both,
        } => Answer::Pulls {
            rows: ctx
                .pulls
                .chosen(
                    &ctx.dova,
                    &ctx.reviews,
                    ctx.codespaces.all(),
                    &ctx.places.found().await,
                    pulls::Chosen {
                        people: &people,
                        repos: &repos,
                        both,
                    },
                )
                .await?,
            asked: Vec::new(),
            searched: Vec::new(),
        },
        Call::PullsList { quick: true } => Answer::Pulls {
            rows: ctx
                .pulls
                .mine(
                    &ctx.dova,
                    &ctx.reviews,
                    ctx.codespaces.all(),
                    &ctx.places.found().await,
                )
                .await?,
            asked: Vec::new(),
            searched: Vec::new(),
        },
        Call::PullsList { quick: false } => {
            let (rows, asked, searched) = ctx
                .pulls
                .list(
                    &ctx.dova,
                    &ctx.reviews,
                    ctx.codespaces.all(),
                    &ctx.places.found().await,
                )
                .await?;
            Answer::Pulls {
                rows,
                asked,
                searched,
            }
        }
        Call::ConversationAnswer { id, ask, answer } => {
            ctx.conversations
                .answer(&id, &ask, answer.as_deref())
                .await?;
            Answer::Done
        }
        Call::LogRead { after } => {
            let (entries, rev) = log::read(after);
            Answer::Log {
                entries,
                rev,
                now: log::now(),
            }
        }
    })
}

/// A codespace as Home lists it: whether its folder is still there, and the branch it is on.
/// git is not run for it. In a large repo `git status` takes seconds, on a Windows drive far
/// longer, and nothing that lists or opens a codespace may wait for that.
fn view(codespace: protocol::Codespace) -> CodespaceView {
    let path = Path::new(&codespace.path);
    let exists = path.is_dir();
    let branch = exists.then(|| codespaces::branch(path)).flatten();
    // A folder that is a repo is not looked into for more.
    let inside = match exists && branch.is_none() {
        true => repos::inside(path),
        false => Vec::new(),
    };
    CodespaceView {
        branch,
        repos: inside.into_iter().map(|repo| repo.name).collect(),
        codespace,
        exists,
    }
}

/// The two folders as you wrote them, and what is in them now.
async fn settings(ctx: &Ctx) -> protocol::Settings {
    let (repos, own) = ctx.places.written();
    let (checkouts, kept) = ctx.reviews.kept(&ctx.places.own());
    protocol::Settings {
        repos,
        own,
        clones: ctx.places.found().await.len() as u32,
        checkouts,
        kept,
    }
}

/// The id of a line that couldn't be read as a call, if it has one.
fn id_of(line: &str) -> Option<u64> {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()?
        .get("id")?
        .as_u64()
}
