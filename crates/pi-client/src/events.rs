//! The subset of Pi's RPC event stream the workbench reacts to.
use anyhow::{Context, Result};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    AgentStart,
    /// The run finished, including retries and queued follow-ups.
    AgentSettled,
    TextDelta(String),
    ThinkingDelta(String),
    /// The model began streaming a tool call's arguments.
    ToolCallStart {
        index: u64,
        id: String,
        tool: String,
    },
    ToolCallDelta {
        index: u64,
        delta: String,
    },
    /// A tool is about to run with complete arguments.
    ToolStart {
        id: String,
        tool: String,
        args: Value,
    },
    ToolEnd {
        id: String,
        tool: String,
        is_error: bool,
        output: String,
    },
    /// Reply to a command this client sent.
    Response {
        command: String,
        error: Option<String>,
        /// Pi consumed a prompt itself (an extension command), so no run follows.
        handled: bool,
        data: Value,
    },
    /// An extension needs an answer before the run can continue.
    Ask(Ask),
    /// An extension notification, or a model error ending a response.
    Notice {
        message: String,
        error: bool,
    },
    /// An extension replaced the prompt draft.
    SetDraft(String),
    /// Emitted by the client, not Pi, when the process stops.
    Exited(String),
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AskKind {
    Select(Vec<String>),
    Confirm,
    /// Single-line text; `editor` requests are multi-line.
    Input {
        multiline: bool,
        prefill: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ask {
    pub id: String,
    pub title: String,
    pub message: String,
    pub kind: AskKind,
}

fn string(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_string()
}

/// The text parts of a tool result or message content.
pub fn result_text(result: &Value) -> String {
    result["content"]
        .as_array()
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part["text"].as_str())
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn ui_request(value: &Value) -> Event {
    let ask = |kind| {
        Event::Ask(Ask {
            id: string(value, "id"),
            title: string(value, "title"),
            message: string(value, "message"),
            kind,
        })
    };
    match value["method"].as_str().unwrap_or_default() {
        "select" => ask(AskKind::Select(
            value["options"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|o| o.as_str().map(String::from))
                .collect(),
        )),
        "confirm" => ask(AskKind::Confirm),
        "input" => ask(AskKind::Input {
            multiline: false,
            prefill: String::new(),
        }),
        "editor" => ask(AskKind::Input {
            multiline: true,
            prefill: string(value, "prefill"),
        }),
        "notify" => Event::Notice {
            message: string(value, "message"),
            error: value["notifyType"] == "error",
        },
        "set_editor_text" => Event::SetDraft(string(value, "text")),
        _ => Event::Other,
    }
}

pub fn decode(line: &str) -> Result<Event> {
    let value: Value = serde_json::from_str(line).context("Pi sent invalid JSON")?;
    Ok(match value["type"].as_str().unwrap_or_default() {
        "agent_start" => Event::AgentStart,
        "agent_settled" => Event::AgentSettled,
        "message_update" => {
            let update = &value["assistantMessageEvent"];
            let index = update["contentIndex"].as_u64().unwrap_or_default();
            match update["type"].as_str().unwrap_or_default() {
                "text_delta" => Event::TextDelta(string(update, "delta")),
                "thinking_delta" => Event::ThinkingDelta(string(update, "delta")),
                "toolcall_start" => Event::ToolCallStart {
                    index,
                    id: string(update, "id"),
                    tool: string(update, "toolName"),
                },
                "toolcall_delta" => Event::ToolCallDelta {
                    index,
                    delta: string(update, "delta"),
                },
                _ => Event::Other,
            }
        }
        "tool_execution_start" => Event::ToolStart {
            id: string(&value, "toolCallId"),
            tool: string(&value, "toolName"),
            args: value["args"].clone(),
        },
        "tool_execution_end" => Event::ToolEnd {
            id: string(&value, "toolCallId"),
            tool: string(&value, "toolName"),
            is_error: value["isError"].as_bool().unwrap_or_default(),
            output: result_text(&value["result"]),
        },
        "response" => Event::Response {
            command: string(&value, "command"),
            error: (value["success"] == Value::Bool(false)).then(|| {
                value["error"]
                    .as_str()
                    .unwrap_or("Command failed")
                    .to_string()
            }),
            handled: value["data"]["disposition"] == "handled",
            data: value["data"].clone(),
        },
        "message_end" if value["message"]["stopReason"] == "error" => Event::Notice {
            message: value["message"]["errorMessage"]
                .as_str()
                .unwrap_or("The model returned an error")
                .to_string(),
            error: true,
        },
        "extension_ui_request" => ui_request(&value),
        _ => Event::Other,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_streaming_and_tool_events() {
        let delta = r#"{"type":"message_update","assistantMessageEvent":{"type":"toolcall_delta","contentIndex":1,"delta":"{\"pa"}}"#;
        assert_eq!(
            decode(delta).unwrap(),
            Event::ToolCallDelta {
                index: 1,
                delta: "{\"pa".into()
            }
        );
        let end = r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"edit","result":{"content":[{"type":"text","text":"ok"}]},"isError":false}"#;
        assert_eq!(
            decode(end).unwrap(),
            Event::ToolEnd {
                id: "c1".into(),
                tool: "edit".into(),
                is_error: false,
                output: "ok".into()
            }
        );
        let failed = r#"{"type":"response","command":"prompt","success":false,"error":"busy"}"#;
        assert_eq!(
            decode(failed).unwrap(),
            Event::Response {
                command: "prompt".into(),
                error: Some("busy".into()),
                handled: false,
                data: Value::Null,
            }
        );
        let handled = r#"{"type":"response","command":"prompt","success":true,"data":{"disposition":"handled"}}"#;
        assert!(matches!(
            decode(handled).unwrap(),
            Event::Response { handled: true, .. }
        ));
        let select = r#"{"type":"extension_ui_request","id":"u1","method":"select","title":"Allow?","options":["Allow","Block"]}"#;
        assert_eq!(
            decode(select).unwrap(),
            Event::Ask(Ask {
                id: "u1".into(),
                title: "Allow?".into(),
                message: String::new(),
                kind: AskKind::Select(vec!["Allow".into(), "Block".into()]),
            })
        );
        let failed = r#"{"type":"message_end","message":{"role":"assistant","stopReason":"error","errorMessage":"No API key"}}"#;
        assert_eq!(
            decode(failed).unwrap(),
            Event::Notice {
                message: "No API key".into(),
                error: true
            }
        );
        assert_eq!(decode(r#"{"type":"turn_start"}"#).unwrap(), Event::Other);
        assert!(decode("not json").is_err());
    }
}
