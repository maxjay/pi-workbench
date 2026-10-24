//! Pi's own files, as Settings shows them: AGENTS.md, the MCP servers in
//! mcp.json, and the models and defaults in models.json and settings.json.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Pi's own files in its agent directory, shared with `pi` in a terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ConfigFile {
    Agents,
    Mcp,
    Models,
    Settings,
}

impl ConfigFile {
    pub fn file_name(self) -> &'static str {
        match self {
            ConfigFile::Agents => "AGENTS.md",
            ConfigFile::Mcp => "mcp.json",
            ConfigFile::Models => "models.json",
            ConfigFile::Settings => "settings.json",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ConfigText {
    pub file: ConfigFile,
    pub path: String,
    /// False for a file that isn't there yet; `text` is then empty.
    pub exists: bool,
    pub text: String,
}

/// An entry in `mcp.json`'s `mcpServers`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct McpServer {
    pub name: String,
    pub kind: McpKind,
    /// The command and its arguments, or the URL.
    pub runs: String,
    pub enabled: bool,
    pub description: Option<String>,
    /// A local server's `env`, as written: `NAME=value`.
    pub env: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum McpKind {
    /// A command Pi starts and talks to over stdio.
    Local,
    /// A URL, over streamable HTTP.
    Remote,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Models {
    pub providers: Vec<Provider>,
    pub defaults: Defaults,
    /// `settings.json`'s `enabledModels`: the patterns Pi cycles through.
    pub enabled: Vec<String>,
}

/// A provider in `models.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Provider {
    pub id: String,
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub models: Vec<Model>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Model {
    pub id: String,
    pub name: Option<String>,
    pub reasoning: bool,
    pub context_window: Option<u32>,
}

/// What new sessions start with, from `settings.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Defaults {
    pub provider: Option<String>,
    pub model: Option<String>,
    pub thinking: Option<String>,
}
