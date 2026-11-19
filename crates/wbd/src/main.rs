//! wbd: the workbench's process inside WSL, next to the code and the agents.
//! It reads the page's calls from stdin, one JSON object per line, and
//! writes answers and events to stdout through one writer.

mod agent;
mod conversations;
mod dova;
mod error;
mod files;
mod fold;
mod git;
mod health;
mod pulls;
mod review;
mod terminals;
mod tickets;
mod workspaces;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use conversations::Conversations;
use error::Error;
use protocol::{Answer, Call, Event, Failure, FailureKind, Request, WorkspaceView};
use terminals::Terminals;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;
use workspaces::Workspaces;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What wbd keeps between calls.
struct Ctx {
    workspaces: Workspaces,
    terminals: Terminals,
    conversations: Conversations,
    dova: dova::Dova,
    pulls: pulls::Pulls,
    reviews: review::Reviews,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("{VERSION}");
        return;
    }
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

    let ctx = Arc::new(Ctx {
        workspaces: Workspaces::new(&data_dir()),
        terminals: Terminals::new(out.clone()),
        conversations: Conversations::new(&data_dir(), out.clone()),
        dova: dova::Dova::default(),
        pulls: pulls::Pulls::default(),
        reviews: review::Reviews::default(),
    });
    ctx.terminals.start();

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
    let _ = stop.send(());
    let _ = writer.await;
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
        Ok(Request { id, call }) => match answer(ctx, call).await {
            Ok(answer) => protocol::reply(id, &answer),
            Err(err) => protocol::failed(Some(id), &err.into()),
        },
        Err(err) => protocol::failed(
            id_of(line),
            &Failure::new(FailureKind::BadRequest, err.to_string()),
        ),
    }
}

async fn answer(ctx: &Ctx, call: Call) -> Result<Answer, Error> {
    Ok(match call {
        Call::Health => Answer::Health(health::check().await),

        Call::WorkspacesList => {
            let mut list = Vec::new();
            for workspace in ctx.workspaces.all() {
                list.push(view(workspace).await);
            }
            Answer::Workspaces { list }
        }
        Call::WorkspacesOpen { path } => Answer::Workspace(view(ctx.workspaces.open(&path)?).await),
        Call::WorkspacesForget { id } => {
            ctx.workspaces.forget(&id)?;
            Answer::Done
        }
        Call::ScratchNew { text } => Answer::Workspace(view(ctx.workspaces.scratch(&text)?).await),
        Call::FoldersList { dir } => {
            let (dir, folders) = workspaces::folders(&dir)?;
            Answer::Folders { dir, folders }
        }

        Call::FilesList { dir } => {
            let (files, truncated) = files::list(Path::new(&dir));
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
            rows: tickets::list(&ctx.dova, ctx.workspaces.all(), &words).await?,
        },
        Call::TicketOpen { dir, id } => {
            Answer::Ticket(tickets::open(&ctx.dova, &ctx.reviews, &dir, id).await?)
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
        Call::TicketCreate { dir, title, kind } => Answer::Filed {
            id: tickets::create(&ctx.dova, &dir, &title, &kind).await?,
        },
        Call::TicketStart { dir, id } => Answer::Started {
            branch: tickets::start(&ctx.dova, &ctx.reviews, &dir, id).await?,
        },
        Call::TicketSession { dir, id } => Answer::Session {
            brief: tickets::brief(&ctx.dova, &ctx.reviews, &dir, id).await?,
            cwd: dir,
        },
        Call::PullSession { dir, pr } => {
            let (cwd, brief) = ctx
                .reviews
                .session(&ctx.dova, &data_dir(), &dir, pr)
                .await?;
            Answer::Session { cwd, brief }
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
        Call::PullsList => Answer::Pulls {
            rows: ctx.pulls.list(&ctx.dova, ctx.workspaces.all()).await?,
        },
        Call::ConversationAnswer { id, ask, answer } => {
            ctx.conversations
                .answer(&id, &ask, answer.as_deref())
                .await?;
            Answer::Done
        }
    })
}

/// A workspace as Home lists it: whether its folder is still there, and where its git stands.
async fn view(workspace: protocol::Workspace) -> WorkspaceView {
    let path = Path::new(&workspace.path);
    let exists = path.is_dir();
    let git = match exists {
        true => git::status(path).await.map(|status| status.summary),
        false => None,
    };
    WorkspaceView {
        workspace,
        exists,
        git,
    }
}

/// The id of a line that couldn't be read as a call, if it has one.
fn id_of(line: &str) -> Option<u64> {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()?
        .get("id")?
        .as_u64()
}
