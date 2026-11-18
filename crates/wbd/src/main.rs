//! wbd: the workbench's process inside WSL, next to the code and the agents.
//! It reads the page's calls from stdin, one JSON object per line, and
//! writes answers and events to stdout through one writer.

mod error;
mod files;
mod git;
mod health;
mod workspaces;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use error::Error;
use protocol::{Answer, Call, Event, Failure, FailureKind, Request, WorkspaceView};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use workspaces::Workspaces;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// What wbd keeps between calls.
struct Ctx {
    workspaces: Workspaces,
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("{VERSION}");
        return;
    }
    let ctx = Arc::new(Ctx {
        workspaces: Workspaces::new(&data_dir()),
    });

    // Everything wbd says goes through this one writer, a line at a time.
    let (out, mut lines) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(mut line) = lines.recv().await {
            line.push('\n');
            if stdout.write_all(line.as_bytes()).await.is_err() || stdout.flush().await.is_err() {
                break;
            }
        }
    });

    let _ = out.send(protocol::event(&Event::Ready {
        version: VERSION.into(),
    }));

    // Each call is answered on its own, so a slow one doesn't hold up the rest.
    let mut calls = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = calls.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let (out, ctx) = (out.clone(), ctx.clone());
        tokio::spawn(async move {
            let _ = out.send(respond(&ctx, &line).await);
        });
    }

    // The page hung up. Finish what was asked, then stop.
    drop(out);
    let _ = writer.await;
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
