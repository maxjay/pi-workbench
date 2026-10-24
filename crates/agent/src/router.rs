//! Each call to the handler that answers it. Handlers read and write
//! files; what the files mean is worked out in `mcp` and `models`.

use std::path::PathBuf;

use protocol::{Answer, Call, ConfigFile, Outcome};

use crate::error::Error;
use crate::{config, health, mcp, models};

/// What every handler needs: where Pi keeps its files.
pub struct Ctx {
    pub agent_dir: PathBuf,
}

impl Ctx {
    pub fn from_env() -> Self {
        Ctx {
            agent_dir: config::agent_dir(),
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
    })
}
