//! Each call to the handler that answers it. Handlers read and write
//! files and drive sessions; what Pi's files and lines mean is worked out
//! in `mcp`, `models`, `pi` and `transcript`.

use std::path::PathBuf;

use protocol::{Answer, Call, ConfigFile, Outcome};

use crate::error::Error;
use crate::session::Out;
use crate::sessions::Sessions;
use crate::{config, files, health, mcp, models, pi};

/// What every handler needs: where Pi keeps its files, and the sessions.
pub struct Ctx {
    pub agent_dir: PathBuf,
    pub sessions: Sessions,
}

impl Ctx {
    pub fn from_env(out: Out) -> Self {
        let agent_dir = config::agent_dir();
        Ctx {
            sessions: Sessions::new(
                out,
                agent_dir.clone(),
                config::scratch_dir(),
                config::data_dir(),
            ),
            agent_dir,
        }
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
    Ok(match call {
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
        Call::FileRead { session, path } => {
            let path = match PathBuf::from(&path) {
                path if path.is_absolute() => path,
                path => ctx.sessions.get(&session)?.cwd().join(path),
            };
            Answer::File(files::read(&path).map_err(|err| Error::io(&path, err))?)
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
