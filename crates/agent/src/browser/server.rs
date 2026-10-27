//! Pi's `browser` server in `mcp.json`: Playwright's MCP server, using the
//! workbench's browser through wb-agent's gate. The workbench adds it once;
//! taken out, it stays out until it's added back from Settings.

use std::path::Path;

use protocol::ConfigFile;
use serde_json::json;

use crate::error::Error;
use crate::{config, mcp};

pub const NAME: &str = "browser";

/// Marks that the server was added once, so taking it out sticks.
const ADDED: &str = "browser-server-added";

/// `WB_SESSION` says which session's Pi is asking; a Pi wb-agent didn't
/// start, in a terminal, has none. `WB_BROWSER_PORT` is set when wb-agent
/// couldn't listen where it usually does.
const COMMAND: &str = r#"exec npx -y @playwright/mcp@0.0.83 --cdp-endpoint "http://127.0.0.1:${WB_BROWSER_PORT:-9339}/s/${WB_SESSION:-terminal}""#;

/// Adds the server the first time wb-agent runs with a browser.
pub fn ensure(agent_dir: &Path, data_dir: &Path) -> Result<(), Error> {
    let mark = data_dir.join(ADDED);
    if mark.exists() {
        return Ok(());
    }
    add(agent_dir)?;
    config::write_file(&mark, "")
}

pub fn add(agent_dir: &Path) -> Result<(), Error> {
    let text = config::read(agent_dir, ConfigFile::Mcp)?.text;
    if mcp::servers(&text)?
        .iter()
        .any(|server| server.name == NAME)
    {
        return Ok(());
    }
    let entry = json!({
        "command": "sh",
        "args": ["-c", COMMAND],
        "description": "Your web tabs in Pi Workbench",
    });
    config::write(agent_dir, ConfigFile::Mcp, &mcp::add(&text, NAME, entry)?)
}

pub fn present(agent_dir: &Path) -> bool {
    config::read(agent_dir, ConfigFile::Mcp)
        .ok()
        .and_then(|config| mcp::servers(&config.text).ok())
        .is_some_and(|servers| servers.iter().any(|s| s.name == NAME))
}
