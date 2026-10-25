//! The messages between the workbench and wb-agent, one JSON object per
//! line. The workbench sends [`Request`]s; wb-agent answers each with a
//! [`Reply`] and raises [`Event`]s of its own, both as [`FromAgent`].
//!
//! This crate is the only contract between the two. The workbench's
//! TypeScript types are generated from it (`cargo test -p protocol`).

use serde::{Deserialize, Serialize};
use ts_rs::TS;

mod config;
mod session;

pub use config::*;
pub use session::*;

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
    /// Scratchpads: the live ones, and the ones in Pi's store.
    #[serde(rename = "sessions.list")]
    SessionsList,
    /// Start Pi in `cwd`, or in a new scratchpad folder, and send `text`.
    #[serde(rename = "session.start")]
    SessionStart {
        cwd: Option<String>,
        text: Option<String>,
    },
    /// Mark a saved session done, or not. Marking a live one done stops Pi.
    #[serde(rename = "sessions.setDone")]
    SessionsSetDone { file: String, done: bool },
    /// Resume a session from Pi's store, or find it if it's live.
    #[serde(rename = "session.open")]
    SessionOpen { file: String },
    /// A saved session, read from its file without starting Pi. Its id is empty.
    #[serde(rename = "session.read")]
    SessionRead { file: String },
    /// A live session as it stands.
    #[serde(rename = "session.get")]
    SessionGet { id: String },
    /// Send a message. While Pi works it steers the run, or with `later`
    /// waits for the run to finish. `!command` runs it in the shell, as in
    /// Pi's terminal; `!!command` keeps its output out of the context.
    #[serde(rename = "session.send")]
    SessionSend {
        id: String,
        text: String,
        later: bool,
    },
    /// Take back a message still waiting to be sent: the one at `index` in
    /// the state's `queued`.
    #[serde(rename = "session.unqueue")]
    SessionUnqueue { id: String, index: u32 },
    /// Stop the run. Messages still waiting to be sent come back, unsent.
    #[serde(rename = "session.abort")]
    SessionAbort { id: String },
    /// Answer an extension's question.
    #[serde(rename = "session.answer")]
    SessionAnswer {
        id: String,
        ask: String,
        reply: AskReply,
    },
    /// The models this session can switch to.
    #[serde(rename = "session.models")]
    SessionModels { id: String },
    #[serde(rename = "session.setModel")]
    SessionSetModel {
        id: String,
        provider: String,
        model: String,
    },
    #[serde(rename = "session.setThinking")]
    SessionSetThinking { id: String, level: String },
    /// Stop Pi for this session once it's idle. Its file stays.
    #[serde(rename = "session.close")]
    SessionClose { id: String },
    /// A file, for viewing. A relative path is taken from the live session's
    /// folder; an absolute one needs no session.
    #[serde(rename = "file.read")]
    FileRead { session: String, path: String },
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
    Mcp {
        servers: Vec<McpServer>,
    },
    Models(Models),
    Sessions {
        sessions: Vec<SessionSummary>,
    },
    Session(Box<Session>),
    Available {
        models: Vec<Available>,
    },
    /// Messages taken back before Pi saw them, oldest first.
    Unsent {
        texts: Vec<String>,
    },
    File(FileView),
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
    /// Pi couldn't be started, or has stopped.
    Pi,
}

/// What wb-agent says without being asked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "event", rename_all = "camelCase")]
#[ts(export)]
pub enum Event {
    /// wb-agent has started and reads requests.
    Ready { version: String },
    /// Something changed in a live session.
    Session { id: String, change: Change },
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

    #[test]
    fn session_calls() {
        let line =
            r#"{"id":2,"method":"session.send","params":{"id":"s1","text":"go on","later":true}}"#;
        assert_eq!(
            serde_json::from_str::<Request>(line).unwrap().call,
            Call::SessionSend {
                id: "s1".into(),
                text: "go on".into(),
                later: true
            }
        );
        let line = r#"{"id":3,"method":"session.answer","params":{"id":"s1","ask":"u1","reply":{"type":"value","value":"Allow"}}}"#;
        assert_eq!(
            serde_json::from_str::<Request>(line).unwrap().call,
            Call::SessionAnswer {
                id: "s1".into(),
                ask: "u1".into(),
                reply: AskReply::Value {
                    value: "Allow".into()
                }
            }
        );
        let answers = AskReply::Answers {
            answers: vec!["A week".into()],
        };
        assert_eq!(
            serde_json::to_string(&answers).unwrap(),
            r#"{"type":"answers","answers":["A week"]}"#
        );
        let line =
            r#"{"id":4,"method":"sessions.setDone","params":{"file":"/s.jsonl","done":true}}"#;
        assert_eq!(
            serde_json::from_str::<Request>(line).unwrap().call,
            Call::SessionsSetDone {
                file: "/s.jsonl".into(),
                done: true
            }
        );
    }

    #[test]
    fn questions() {
        let kind = AskKind::Questions {
            questions: vec![Question {
                label: "Downtime".into(),
                question: "How much downtime?".into(),
                choices: vec![Choice {
                    label: "None".into(),
                    description: Some("Dual-write".into()),
                }],
                other: true,
            }],
        };
        assert_eq!(
            serde_json::to_string(&kind).unwrap(),
            r#"{"type":"questions","questions":[{"label":"Downtime","question":"How much downtime?","choices":[{"label":"None","description":"Dual-write"}],"other":true}]}"#
        );
    }

    #[test]
    fn session_changes() {
        let change = FromAgent::Event(Event::Session {
            id: "s1".into(),
            change: Change::Entry {
                index: 3,
                entry: Box::new(Entry::Tool(Tool {
                    id: "c1".into(),
                    name: "bash".into(),
                    args: serde_json::json!({ "command": "ls" }),
                    status: ToolStatus::Running,
                    output: String::new(),
                    images: Vec::new(),
                    details: serde_json::Value::Null,
                })),
            },
        });
        assert_eq!(
            serde_json::to_string(&change).unwrap(),
            r#"{"type":"event","event":"session","id":"s1","change":{"change":"entry","index":3,"entry":{"kind":"tool","id":"c1","name":"bash","args":{"command":"ls"},"status":"running","output":"","images":[],"details":null}}}"#
        );
        let image = FileContent::Image {
            mime_type: "image/png".into(),
            data: "AA==".into(),
        };
        assert_eq!(
            serde_json::to_string(&image).unwrap(),
            r#"{"type":"image","mimeType":"image/png","data":"AA=="}"#
        );
    }
}
