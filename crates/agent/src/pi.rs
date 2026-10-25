//! Pi's RPC protocol (`pi --mode rpc`): the lines Pi writes, read into
//! [`PiEvent`]s, and the commands wb-agent writes back. Only what the
//! workbench uses is read; everything else is [`PiEvent::Other`].
//! See Pi's docs/rpc.md and docs/json.md.

use protocol::{Ask, AskKind, AskReply, PiCommand, Tone};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq)]
pub enum PiEvent {
    AgentStart,
    /// Pi won't continue on its own: no retries or queued messages left.
    AgentSettled,
    MessageStart(Value),
    /// A content block of the assistant message being written.
    Block {
        index: u64,
        update: Block,
    },
    /// The finished message, which replaces what was streamed.
    MessageEnd(Value),
    ToolStart {
        id: String,
        name: String,
        args: Value,
    },
    ToolUpdate {
        id: String,
        partial: Value,
    },
    ToolEnd {
        id: String,
        failed: bool,
        result: Value,
    },
    Queue {
        steering: Vec<String>,
        follow_up: Vec<String>,
    },
    Named(Option<String>),
    ThinkingLevel(String),
    /// The reply to a command wb-agent sent, with the command's id if it had one.
    Response {
        id: Option<String>,
        command: String,
        error: Option<String>,
        data: Value,
    },
    /// Output from a shell command run with [`bash`].
    BashOutput {
        id: String,
        delta: String,
    },
    Ask(Ask),
    Status {
        key: String,
        text: Option<String>,
    },
    Notice {
        text: String,
        tone: Tone,
    },
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    TextStart,
    TextDelta(String),
    TextEnd(String),
    ThinkingStart,
    ThinkingDelta(String),
    ThinkingEnd(String),
    ToolStart {
        id: String,
        name: String,
    },
    /// The complete call, with its arguments.
    ToolEnd(Value),
    Other,
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_string()
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item.as_str().map(str::to_string))
        .collect()
}

/// One line from Pi. None for a line that isn't JSON.
pub fn decode(line: &str) -> Option<PiEvent> {
    let value: Value = serde_json::from_str(line).ok()?;
    Some(match value["type"].as_str().unwrap_or_default() {
        "agent_start" => PiEvent::AgentStart,
        "agent_settled" => PiEvent::AgentSettled,
        "message_start" => PiEvent::MessageStart(value["message"].clone()),
        "message_end" => PiEvent::MessageEnd(value["message"].clone()),
        "message_update" => {
            let update = &value["assistantMessageEvent"];
            PiEvent::Block {
                index: update["contentIndex"].as_u64().unwrap_or_default(),
                update: block(update),
            }
        }
        "tool_execution_start" => PiEvent::ToolStart {
            id: text(&value, "toolCallId"),
            name: text(&value, "toolName"),
            args: value["args"].clone(),
        },
        "tool_execution_update" => PiEvent::ToolUpdate {
            id: text(&value, "toolCallId"),
            partial: value["partialResult"].clone(),
        },
        "tool_execution_end" => PiEvent::ToolEnd {
            id: text(&value, "toolCallId"),
            failed: value["isError"].as_bool().unwrap_or_default(),
            result: value["result"].clone(),
        },
        "queue_update" => PiEvent::Queue {
            steering: strings(&value["steering"]),
            follow_up: strings(&value["followUp"]),
        },
        "session_info_changed" => PiEvent::Named(value["name"].as_str().map(str::to_string)),
        "thinking_level_changed" => PiEvent::ThinkingLevel(text(&value, "level")),
        "response" => PiEvent::Response {
            id: value["id"].as_str().map(str::to_string),
            command: text(&value, "command"),
            error: (value["success"] == false).then(|| {
                value["error"]
                    .as_str()
                    .unwrap_or("Pi refused the command")
                    .to_string()
            }),
            data: value["data"].clone(),
        },
        "bash_execution_update" => PiEvent::BashOutput {
            id: text(&value, "id"),
            delta: text(&value, "delta"),
        },
        "extension_ui_request" => ui_request(&value),
        "compaction_start" => notice("Compacting the conversation…", Tone::Info),
        "compaction_end" => match value["errorMessage"].as_str() {
            Some(error) => notice(&format!("Compacting failed: {error}"), Tone::Error),
            None if value["aborted"] == true => notice("Compacting stopped", Tone::Warning),
            None => notice("Compacted the conversation", Tone::Info),
        },
        "auto_retry_start" => notice(
            &format!(
                "Retrying ({} of {}): {}",
                value["attempt"],
                value["maxAttempts"],
                text(&value, "errorMessage")
            ),
            Tone::Warning,
        ),
        "auto_retry_end" if value["success"] == false => notice(
            &format!("Gave up retrying: {}", text(&value, "finalError")),
            Tone::Error,
        ),
        "extension_error" => notice(
            &format!(
                "An extension failed ({}): {}",
                text(&value, "extensionPath"),
                text(&value, "error")
            ),
            Tone::Error,
        ),
        _ => PiEvent::Other,
    })
}

fn notice(text: &str, tone: Tone) -> PiEvent {
    PiEvent::Notice {
        text: text.to_string(),
        tone,
    }
}

fn block(update: &Value) -> Block {
    match update["type"].as_str().unwrap_or_default() {
        "text_start" => Block::TextStart,
        "text_delta" => Block::TextDelta(text(update, "delta")),
        "text_end" => Block::TextEnd(text(update, "content")),
        "thinking_start" => Block::ThinkingStart,
        "thinking_delta" => Block::ThinkingDelta(text(update, "delta")),
        "thinking_end" => Block::ThinkingEnd(text(update, "content")),
        "toolcall_start" => Block::ToolStart {
            id: text(update, "id"),
            name: text(update, "toolName"),
        },
        "toolcall_end" => Block::ToolEnd(update["toolCall"].clone()),
        _ => Block::Other,
    }
}

fn ui_request(value: &Value) -> PiEvent {
    let optional = |key: &str| value[key].as_str().map(str::to_string);
    let ask = |kind| {
        PiEvent::Ask(Ask {
            id: text(value, "id"),
            title: text(value, "title"),
            message: optional("message"),
            kind,
        })
    };
    match value["method"].as_str().unwrap_or_default() {
        "select" => ask(AskKind::Select {
            options: strings(&value["options"]),
        }),
        "confirm" => ask(AskKind::Confirm),
        "input" => ask(AskKind::Input {
            placeholder: optional("placeholder"),
        }),
        "editor" => ask(AskKind::Editor {
            prefill: text(value, "prefill"),
        }),
        "notify" => notice(
            &text(value, "message"),
            match value["notifyType"].as_str() {
                Some("error") => Tone::Error,
                Some("warning") => Tone::Warning,
                _ => Tone::Info,
            },
        ),
        "setStatus" => PiEvent::Status {
            key: text(value, "statusKey"),
            text: optional("statusText"),
        },
        _ => PiEvent::Other,
    }
}

/// The commands in `get_commands`'s reply.
pub fn commands(data: &Value) -> Vec<PiCommand> {
    data["commands"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|command| {
            Some(PiCommand {
                name: command["name"].as_str()?.to_string(),
                description: command["description"].as_str().map(str::to_string),
                source: text(command, "source"),
            })
        })
        .collect()
}

// Commands. Each is one line; replies come back as `response` events.

pub fn command(kind: &str) -> Value {
    json!({ "type": kind })
}

pub fn prompt(text: &str) -> Value {
    json!({ "type": "prompt", "message": text })
}

pub fn steer(text: &str) -> Value {
    json!({ "type": "steer", "message": text })
}

pub fn follow_up(text: &str) -> Value {
    json!({ "type": "follow_up", "message": text })
}

/// A shell command, as `!` runs it in Pi's terminal. Its output goes into
/// the context unless `excluded`.
pub fn bash(id: &str, command: &str, excluded: bool) -> Value {
    json!({ "type": "bash", "id": id, "command": command, "excludeFromContext": excluded })
}

/// A command whose reply wb-agent waits for, matched by `id`.
pub fn with_id(mut command: Value, id: &str) -> Value {
    command["id"] = json!(id);
    command
}

pub fn set_model(provider: &str, model: &str) -> Value {
    json!({ "type": "set_model", "provider": provider, "modelId": model })
}

pub fn set_thinking(level: &str) -> Value {
    json!({ "type": "set_thinking_level", "level": level })
}

pub fn answer(ask: &str, reply: &AskReply) -> Value {
    let mut answer = json!({ "type": "extension_ui_response", "id": ask });
    match reply {
        AskReply::Value { value } => answer["value"] = json!(value),
        AskReply::Confirmed { confirmed } => answer["confirmed"] = json!(confirmed),
        // Only wb-agent's own question tool asks several; an extension's
        // dialog takes one value.
        AskReply::Answers { answers } => answer["value"] = json!(answers.join("\n")),
        AskReply::Cancelled => answer["cancelled"] = json!(true),
    }
    answer
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_streaming_and_tool_events() {
        let delta = r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":1,"delta":"Hel"}}"#;
        assert_eq!(
            decode(delta),
            Some(PiEvent::Block {
                index: 1,
                update: Block::TextDelta("Hel".into())
            })
        );
        let end = r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"edit","result":{"content":[]},"isError":true}"#;
        assert!(matches!(
            decode(end),
            Some(PiEvent::ToolEnd { failed: true, .. })
        ));
        let refused = r#"{"type":"response","command":"prompt","success":false,"error":"busy"}"#;
        assert_eq!(
            decode(refused),
            Some(PiEvent::Response {
                id: None,
                command: "prompt".into(),
                error: Some("busy".into()),
                data: Value::Null
            })
        );
        assert_eq!(decode(r#"{"type":"turn_start"}"#), Some(PiEvent::Other));
        assert_eq!(decode("not json"), None);
    }

    #[test]
    fn reads_extension_requests() {
        let select = r#"{"type":"extension_ui_request","id":"u1","method":"select","title":"Allow?","options":["Allow","Block"]}"#;
        assert_eq!(
            decode(select),
            Some(PiEvent::Ask(Ask {
                id: "u1".into(),
                title: "Allow?".into(),
                message: None,
                kind: AskKind::Select {
                    options: vec!["Allow".into(), "Block".into()]
                }
            }))
        );
        let cleared =
            r#"{"type":"extension_ui_request","id":"u2","method":"setStatus","statusKey":"x"}"#;
        assert_eq!(
            decode(cleared),
            Some(PiEvent::Status {
                key: "x".into(),
                text: None
            })
        );
        let warned = r#"{"type":"extension_ui_request","id":"u3","method":"notify","message":"Careful","notifyType":"warning"}"#;
        assert_eq!(
            decode(warned),
            Some(PiEvent::Notice {
                text: "Careful".into(),
                tone: Tone::Warning
            })
        );
    }

    #[test]
    fn reads_commands() {
        let data = serde_json::json!({ "commands": [
            { "name": "skill:grafana", "description": "Dashboards", "source": "skill" },
            { "description": "no name" },
        ]});
        let found = commands(&data);
        assert_eq!(found.len(), 1);
        assert_eq!(
            (found[0].name.as_str(), found[0].source.as_str()),
            ("skill:grafana", "skill")
        );
    }

    #[test]
    fn writes_answers() {
        let reply = AskReply::Confirmed { confirmed: true };
        assert_eq!(
            answer("u1", &reply).to_string(),
            r#"{"type":"extension_ui_response","id":"u1","confirmed":true}"#
        );
        assert_eq!(
            answer("u2", &AskReply::Cancelled).to_string(),
            r#"{"type":"extension_ui_response","id":"u2","cancelled":true}"#
        );
    }
}
