//! The messages between the workbench and wb-agent, one JSON object per
//! line. The workbench sends [`Request`]s; wb-agent answers each with a
//! [`Reply`] and raises [`Event`]s of its own, both as [`FromAgent`].
//!
//! This crate is the only contract between the two. The workbench's
//! TypeScript types are generated from it (`cargo test -p protocol`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A call from the workbench, with the id its reply will carry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Request {
    #[ts(type = "number")]
    pub id: u64,
    #[serde(flatten)]
    pub call: Call,
}

/// What the workbench can ask for. Methods are named `area.action`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "method", content = "params")]
#[ts(export)]
pub enum Call {
    /// Who wb-agent is and where it looks.
    #[serde(rename = "health")]
    Health,
    /// One of Pi's own files, as text.
    #[serde(rename = "config.read")]
    ConfigRead { file: ConfigFile },
    /// Replace one of Pi's own files.
    #[serde(rename = "config.write")]
    ConfigWrite { file: ConfigFile, text: String },
    /// The MCP servers in `mcp.json`.
    #[serde(rename = "mcp.list")]
    McpList,
    /// Turn a server on or off, keeping its entry.
    #[serde(rename = "mcp.setEnabled")]
    McpSetEnabled { name: String, enabled: bool },
    /// The models in `models.json`, and the defaults in `settings.json`.
    #[serde(rename = "models.list")]
    ModelsList,
    /// The model new sessions start with.
    #[serde(rename = "models.setDefault")]
    ModelsSetDefault { provider: String, model: String },
    /// The thinking level new sessions start with.
    #[serde(rename = "models.setThinking")]
    ModelsSetThinking { level: String },
}

/// Everything wb-agent writes: replies to requests, and events.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum FromAgent {
    Reply(Reply),
    Event(Event),
}

/// The answer to the request with the same id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Reply {
    #[ts(type = "number")]
    pub id: u64,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Outcome {
    Ok(Answer),
    Error(Failure),
}

/// A successful reply, by kind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum Answer {
    Health(Health),
    Config(ConfigText),
    Mcp { servers: Vec<McpServer> },
    Models(Models),
    Done,
}

/// Why a request failed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Failure {
    pub code: FailureCode,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FailureCode {
    /// The line wasn't a request wb-agent understands.
    BadRequest,
    /// A file couldn't be read or written.
    Io,
    /// A file isn't valid JSON, or not the shape Pi expects.
    Invalid,
    /// The thing asked about doesn't exist.
    NotFound,
}

/// What wb-agent says without being asked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "event", rename_all = "camelCase")]
#[ts(export)]
pub enum Event {
    /// wb-agent has started and reads requests.
    Ready { version: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Health {
    pub version: String,
    /// Pi's agent directory: `PI_CODING_AGENT_DIR`, or `~/.pi/agent`.
    pub agent_dir: String,
    /// The WSL distro wb-agent runs in, when it runs in one.
    pub distro: Option<String>,
    /// `pi --version`, when Pi is installed.
    pub pi: Option<String>,
}

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

#[cfg(test)]
mod tests {
    use super::*;

    // The wire format, pinned: a change here changes what the other side reads.

    #[test]
    fn requests() {
        let cases = [
            (r#"{"id":1,"method":"health"}"#, Call::Health),
            (
                r#"{"id":1,"method":"config.read","params":{"file":"agents"}}"#,
                Call::ConfigRead {
                    file: ConfigFile::Agents,
                },
            ),
            (
                r#"{"id":1,"method":"mcp.setEnabled","params":{"name":"grafana","enabled":false}}"#,
                Call::McpSetEnabled {
                    name: "grafana".into(),
                    enabled: false,
                },
            ),
        ];
        for (line, call) in cases {
            let request: Request = serde_json::from_str(line).unwrap();
            assert_eq!(request, Request { id: 1, call });
        }
    }

    #[test]
    fn replies() {
        let ok = FromAgent::Reply(Reply {
            id: 7,
            outcome: Outcome::Ok(Answer::Done),
        });
        assert_eq!(
            serde_json::to_string(&ok).unwrap(),
            r#"{"type":"reply","id":7,"ok":{"kind":"done"}}"#
        );
        let failed = FromAgent::Reply(Reply {
            id: 7,
            outcome: Outcome::Error(Failure {
                code: FailureCode::NotFound,
                message: "no server grafana".into(),
            }),
        });
        assert_eq!(
            serde_json::to_string(&failed).unwrap(),
            r#"{"type":"reply","id":7,"error":{"code":"notFound","message":"no server grafana"}}"#
        );
    }

    #[test]
    fn events() {
        let ready = FromAgent::Event(Event::Ready {
            version: "1.0.0".into(),
        });
        let line = serde_json::to_string(&ready).unwrap();
        assert_eq!(
            line,
            r#"{"type":"event","event":"ready","version":"1.0.0"}"#
        );
        assert_eq!(serde_json::from_str::<FromAgent>(&line).unwrap(), ready);
    }
}
