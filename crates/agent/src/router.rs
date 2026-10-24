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
            sessions: Sessions::new(out, agent_dir.clone(), config::scratch_dir()),
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
        Call::SessionOpen { file } => Answer::Session(Box::new(ctx.sessions.open(&file).await?)),
        Call::SessionGet { id } => Answer::Session(Box::new(ctx.sessions.get(&id)?.snapshot())),
        Call::SessionSend { id, text, later } => {
            let live = ctx.sessions.get(&id)?;
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
        Call::SessionAbort { id } => {
            ctx.sessions.get(&id)?.send(pi::command("abort")).await?;
            Answer::Done
        }
        Call::SessionAnswer { id, ask, reply } => {
            let live = ctx.sessions.get(&id)?;
            live.send(pi::answer(&ask, &reply)).await?;
            live.edit(|t| t.answered());
            Answer::Done
        }
        Call::SessionSetModel {
            id,
            provider,
            model,
        } => {
            let live = ctx.sessions.get(&id)?;
            live.send(pi::set_model(&provider, &model)).await?;
            live.send(pi::command("get_state")).await?;
            Answer::Done
        }
        Call::SessionSetThinking { id, level } => {
            let live = ctx.sessions.get(&id)?;
            live.send(pi::set_thinking(&level)).await?;
            live.send(pi::command("get_state")).await?;
            Answer::Done
        }
        Call::SessionClose { id } => {
            ctx.sessions.close(&id).await?;
            Answer::Done
        }
        Call::FileRead { session, path } => {
            let path = ctx.sessions.get(&session)?.cwd().join(path);
            Answer::File(files::read(&path).map_err(|err| Error::io(&path, err))?)
        }
    })
}
