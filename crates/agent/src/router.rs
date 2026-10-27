//! Each call to the handler that answers it. Handlers read and write
//! files and drive sessions; what Pi's files and lines mean is worked out
//! in `mcp`, `models`, `pi` and `transcript`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use protocol::{
    Answer, Call, Codespace, CodespaceView, ConfigFile, Outcome, SessionSummary, Trigger,
};

use crate::browser::Web;
use crate::codespaces::Codespaces;
use crate::error::Error;
use crate::session::Out;
use crate::sessions::Sessions;
use crate::tasks::Tasks;
use crate::terminals::Terminals;
use crate::{config, files, git, health, mcp, models, pi};

/// What every handler needs: where Pi keeps its files, the sessions, the
/// tasks, the codespaces and their terminals.
pub struct Ctx {
    pub agent_dir: PathBuf,
    pub data_dir: PathBuf,
    pub sessions: Sessions,
    pub tasks: Tasks,
    pub spaces: Arc<Codespaces>,
    pub terminals: Terminals,
    pub web: Arc<Web>,
}

impl Ctx {
    pub fn from_env(out: Out, web: Arc<Web>) -> Self {
        let agent_dir = config::agent_dir();
        let data_dir = config::data_dir();
        let spaces = Arc::new(Codespaces::new(&data_dir));
        let terminals = Terminals::new(out.clone(), spaces.clone());
        // Without its socket or its file, Pi just goes without the tool.
        let tool = Terminals::install(&data_dir.join("extensions"))
            .and_then(|extension| Ok((extension, terminals.serve()?)))
            .map_err(|err| eprintln!("wb-agent: no terminal tool: {err}"))
            .ok();
        let sessions = Sessions::new(
            out.clone(),
            agent_dir.clone(),
            config::scratch_dir(),
            data_dir.clone(),
            spaces.clone(),
            tool,
            web.clone(),
        );
        Ctx {
            tasks: Tasks::new(&data_dir, sessions.clone(), out),
            sessions,
            agent_dir,
            data_dir,
            spaces,
            terminals,
            web,
        }
    }

    /// Each codespace with its git, what runs in it, and its sessions. Pi's
    /// store is read once for them all, and git asked for each at once.
    async fn views(&self, spaces: Vec<Codespace>) -> Vec<CodespaceView> {
        let sessions = self.sessions.under(Path::new("/"));
        let asks: Vec<_> = spaces
            .iter()
            .map(|space| {
                let path = PathBuf::from(&space.path);
                tokio::spawn(async move { git::status(&path).await })
            })
            .collect();
        let mut views = Vec::new();
        for (space, git) in spaces.into_iter().zip(asks) {
            let path = PathBuf::from(&space.path);
            let mine: Vec<SessionSummary> = sessions
                .iter()
                .filter(|s| Path::new(&s.cwd).starts_with(&path))
                .cloned()
                .collect();
            views.push(CodespaceView {
                git: git.await.ok().flatten().map(|status| status.summary),
                exists: path.is_dir(),
                running: self
                    .terminals
                    .list(&path)
                    .into_iter()
                    .filter(|t| t.running)
                    .map(|t| t.title)
                    .collect(),
                sessions: mine.len() as u32,
                need_you: mine.iter().filter(|s| s.asking).count() as u32,
                latest: mine.into_iter().take(3).collect(),
                space,
            });
        }
        views
    }
}

pub async fn handle(ctx: &Ctx, call: Call) -> Outcome {
    match answer(ctx, call).await {
        Ok(answer) => Outcome::Ok(answer),
        Err(err) => Outcome::Error(err.into()),
    }
}

async fn answer(ctx: &Ctx, call: Call) -> Result<Answer, Error> {
    let dir = &ctx.agent_dir;
    let web = &ctx.web;
    Ok(match call {
        Call::BrowserGet => Answer::Browser(web.view()),
        Call::BrowserOpen { url, session } => Answer::WebTab(web.open(&url, session).await?),
        Call::BrowserShow { tab } => {
            web.show(&tab)?;
            Answer::Done
        }
        Call::BrowserNavigate { tab, url } => {
            web.navigate(&tab, &url).await?;
            Answer::Done
        }
        Call::BrowserGo { tab, by } => {
            web.go(&tab, by).await?;
            Answer::Done
        }
        Call::BrowserReload { tab } => {
            web.reload(&tab).await?;
            Answer::Done
        }
        Call::BrowserClose { tab } => {
            web.close(&tab).await?;
            Answer::Done
        }
        Call::BrowserSetSession { tab, session } => {
            web.set_session(&tab, session)?;
            Answer::Done
        }
        Call::BrowserHistory { query } => Answer::Visits {
            list: web.history(&query),
        },
        Call::BrowserShot { tab } => Answer::Shot(web.shot(&tab).await?),
        Call::BrowserAddServer => {
            web.add_server()?;
            Answer::Browser(web.view())
        }
        Call::Health => Answer::Health(health::check(dir).await),
        Call::ConfigRead { file } => Answer::Config(config::read(dir, file)?),
        Call::ConfigWrite { file, text } => {
            config::write(dir, file, &text)?;
            Answer::Done
        }
        Call::McpList => {
            let text = config::read(dir, ConfigFile::Mcp)?.text;
            Answer::Mcp {
                servers: mcp::servers(&text)?,
            }
        }
        Call::McpSetEnabled { name, enabled } => {
            let text = config::read(dir, ConfigFile::Mcp)?.text;
            config::write(
                dir,
                ConfigFile::Mcp,
                &mcp::set_enabled(&text, &name, enabled)?,
            )?;
            Answer::Done
        }
        Call::ModelsList => {
            let models = config::read(dir, ConfigFile::Models)?.text;
            let settings = config::read(dir, ConfigFile::Settings)?.text;
            Answer::Models(models::list(&models, &settings)?)
        }
        Call::ModelsSetDefault { provider, model } => {
            let settings = config::read(dir, ConfigFile::Settings)?.text;
            config::write(
                dir,
                ConfigFile::Settings,
                &models::set_default(&settings, &provider, &model)?,
            )?;
            Answer::Done
        }
        Call::ModelsSetThinking { level } => {
            let settings = config::read(dir, ConfigFile::Settings)?.text;
            config::write(
                dir,
                ConfigFile::Settings,
                &models::set_thinking(&settings, &level)?,
            )?;
            Answer::Done
        }
        Call::SessionsList => Answer::Sessions {
            sessions: ctx.sessions.list(),
        },
        Call::SessionStart { cwd, text } => {
            Answer::Session(Box::new(ctx.sessions.start(cwd, text).await?))
        }
        Call::SessionsSetDone { file, done } => {
            ctx.sessions.set_done(&file, done).await?;
            Answer::Done
        }
        Call::SessionOpen { file } => Answer::Session(Box::new(ctx.sessions.open(&file).await?)),
        Call::SessionRead { file } => Answer::Session(Box::new(ctx.sessions.read(&file)?)),
        Call::SessionGet { id } => Answer::Session(Box::new(ctx.sessions.get(&id)?.snapshot())),
        Call::SessionSend { id, text, later } => {
            let live = ctx.sessions.get(&id)?;
            ctx.sessions.wake(&live).await?;
            // As in Pi's terminal: `!` runs a shell command, `!!` keeps its
            // output out of what the model sees.
            if let Some(command) = text.strip_prefix('!') {
                let (excluded, command) = match command.strip_prefix('!') {
                    Some(command) => (true, command.trim()),
                    None => (false, command.trim()),
                };
                if !command.is_empty() {
                    let shell = live.next_id();
                    live.edit(|t| t.shell(&shell, command));
                    live.send(pi::bash(&shell, command, excluded)).await?;
                    return Ok(Answer::Done);
                }
            }
            let command = match (live.is_busy(), later) {
                (false, _) => {
                    live.edit(|t| t.sent(&text));
                    pi::prompt(&text)
                }
                (true, false) => pi::steer(&text),
                (true, true) => pi::follow_up(&text),
            };
            live.send(command).await?;
            Answer::Done
        }
        Call::SessionUnqueue { id, index } => {
            let live = ctx.sessions.get(&id)?;
            // Pi takes back the whole queue; the rest goes back in, in order.
            let taken = live.request(pi::command("clear_queue")).await?;
            let mut rest = queued(&taken);
            if (index as usize) < rest.len() {
                rest.remove(index as usize);
            }
            for (text, later) in rest {
                let command = if later {
                    pi::follow_up(&text)
                } else {
                    pi::steer(&text)
                };
                live.send(command).await?;
            }
            Answer::Done
        }
        Call::SessionModels { id } => {
            let data = ctx
                .sessions
                .get(&id)?
                .request(pi::command("get_available_models"))
                .await?;
            Answer::Available {
                models: models::available(&data),
            }
        }
        Call::SessionAbort { id } => {
            let live = ctx.sessions.get(&id)?;
            // As Pi's terminal does: take back what's queued first, or the
            // stopped run would carry on with it. Stopping comes first, though.
            let taken = live
                .request(pi::command("clear_queue"))
                .await
                .unwrap_or_default();
            live.send(pi::command("abort")).await?;
            Answer::Unsent {
                texts: queued(&taken).into_iter().map(|(text, _)| text).collect(),
            }
        }
        Call::SessionAnswer { id, ask, reply } => {
            ctx.sessions.get(&id)?.answer(&ask, reply).await?;
            Answer::Done
        }
        Call::SessionSetModel {
            id,
            provider,
            model,
        } => {
            let live = ctx.sessions.get(&id)?;
            live.request(pi::set_model(&provider, &model)).await?;
            live.refresh().await?;
            Answer::Done
        }
        Call::SessionSetThinking { id, level } => {
            let live = ctx.sessions.get(&id)?;
            live.request(pi::set_thinking(&level)).await?;
            live.refresh().await?;
            Answer::Done
        }
        Call::SessionClose { id } => {
            ctx.sessions.close(&id).await?;
            Answer::Done
        }
        Call::TasksList => Answer::Tasks {
            list: ctx.tasks.list(),
        },
        Call::TasksSave { task } => Answer::Tasks {
            list: ctx.tasks.save(task)?,
        },
        Call::TasksSetPaused { id, paused } => Answer::Tasks {
            list: ctx.tasks.set_paused(&id, paused)?,
        },
        Call::TasksPauseAll { paused } => Answer::Tasks {
            list: ctx.tasks.pause_all(paused).await,
        },
        Call::TasksRun { id } => Answer::Tasks {
            list: ctx.tasks.run(&id, Trigger::Now).await?,
        },
        Call::TasksStop { id } => Answer::Tasks {
            list: ctx.tasks.stop(&id).await?,
        },
        Call::TasksScheduleFrom { words } => Answer::Schedule {
            schedule: ctx
                .tasks
                .schedule_from(&words, &ctx.data_dir.join("pi"))
                .await?,
        },
        Call::FilesList { dir } => {
            let (paths, truncated) = files::list(&PathBuf::from(dir));
            Answer::Paths { paths, truncated }
        }
        Call::SessionCommands { id } => {
            let data = ctx
                .sessions
                .get(&id)?
                .request(pi::command("get_commands"))
                .await?;
            Answer::Commands {
                commands: pi::commands(&data),
            }
        }
        Call::FileRead { session, path } => {
            let path = match PathBuf::from(&path) {
                path if path.is_absolute() => path,
                path => ctx.sessions.get(&session)?.cwd().join(path),
            };
            Answer::File(files::read(&path).map_err(|err| Error::io(&path, err))?)
        }
        Call::FileWrite {
            path,
            text,
            expected,
        } => {
            let at = PathBuf::from(&path);
            let modified =
                files::write(&at, &text, expected.as_deref()).map_err(|err| match err {
                    files::Unsaved::Changed => {
                        Error::Conflict(format!("{path} changed since it was opened"))
                    }
                    files::Unsaved::Io(err) => Error::io(&at, err),
                })?;
            Answer::Written { modified }
        }
        Call::FilesStat { paths } => Answer::Stats {
            modified: files::stat(&paths),
        },
        Call::CodespacesList => Answer::Codespaces {
            list: ctx.views(ctx.spaces.all()).await,
        },
        Call::CodespacesOpen { path } => {
            let space = ctx.spaces.open(&path)?;
            let view = ctx.views(vec![space]).await.remove(0);
            Answer::Codespace(Box::new(view))
        }
        Call::CodespacesSetPinned { id, pinned } => {
            ctx.spaces.set_pinned(&id, pinned)?;
            Answer::Codespaces {
                list: ctx.views(ctx.spaces.all()).await,
            }
        }
        Call::CodespacesForget { id } => {
            ctx.spaces.forget(&id)?;
            Answer::Codespaces {
                list: ctx.views(ctx.spaces.all()).await,
            }
        }
        Call::FoldersList { dir } => {
            let (dir, folders) = ctx.spaces.folders(&dir)?;
            Answer::Folders {
                dir: dir.display().to_string(),
                folders,
            }
        }
        Call::SessionsIn { dir } => Answer::Sessions {
            sessions: ctx.sessions.under(Path::new(&dir)),
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
        Call::TerminalRead { id } => Answer::Output {
            data: ctx.terminals.read(&id)?,
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
    })
}

/// What `clear_queue` took back: each message, and whether it was waiting
/// for the run to finish rather than steering it, in the order they'd go.
fn queued(taken: &serde_json::Value) -> Vec<(String, bool)> {
    let texts = |key: &str| -> Vec<String> {
        taken[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|text| text.as_str().map(str::to_string))
            .collect()
    };
    texts("steering")
        .into_iter()
        .map(|text| (text, false))
        .chain(texts("followUp").into_iter().map(|text| (text, true)))
        .collect()
}
