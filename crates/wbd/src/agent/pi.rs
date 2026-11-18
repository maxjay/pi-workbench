//! Pi, through `pi --mode rpc`: one JSON object per line each way.
//! See Pi's docs/rpc.md. Only what the workbench uses is read.

use protocol::{Ask, Tone};
use serde_json::{Value, json};
use tokio::process::Command;

use super::result_text;
use crate::fold::Update;

/// `WB_PI` names another binary. The session id makes Pi open this
/// conversation's own session, or start it.
pub fn command(id: &str) -> Command {
    let mut command = Command::new(std::env::var("WB_PI").unwrap_or_else(|_| "pi".into()));
    command.args(["--mode", "rpc", "--session-id", id]);
    command
}

pub fn prompt(text: &str) -> String {
    json!({ "type": "prompt", "message": text }).to_string()
}

pub fn stop() -> String {
    json!({ "type": "abort" }).to_string()
}

/// Pi's questions come from extensions' dialogs. `detail` holds which kind
/// it was, as each is answered in its own way.
pub fn answer(ask: &Ask, answer: Option<&str>, detail: &Value) -> String {
    let mut reply = json!({ "type": "extension_ui_response", "id": ask.id });
    match (detail["method"].as_str(), answer) {
        (_, None) => reply["cancelled"] = json!(true),
        (Some("confirm"), Some(answer)) => reply["confirmed"] = json!(answer == "Yes"),
        (_, Some(answer)) => reply["value"] = json!(answer),
    }
    reply.to_string()
}

pub fn decode(value: &Value) -> Vec<Update> {
    let text = |key: &str| value[key].as_str().unwrap_or_default().to_string();
    let notice = |tone, text: String| vec![Update::Notice { tone, text }];
    match value["type"].as_str().unwrap_or_default() {
        "message_update" => {
            let event = &value["assistantMessageEvent"];
            let delta = event["delta"].as_str().unwrap_or_default().to_string();
            match event["type"].as_str() {
                Some("text_delta") => vec![Update::Text(delta)],
                Some("thinking_delta") => vec![Update::Thinking(delta)],
                _ => Vec::new(),
            }
        }
        "tool_execution_start" => vec![Update::ToolStart {
            id: text("toolCallId"),
            name: text("toolName"),
            input: value["args"].clone(),
        }],
        "tool_execution_end" => vec![Update::ToolEnd {
            id: text("toolCallId"),
            failed: value["isError"] == true,
            output: result_text(&value["result"]["content"]),
        }],
        // Pi won't continue on its own: no retries or queued messages left.
        "agent_settled" => vec![Update::TurnEnd { error: None }],
        // A command Pi refused. A prompt it refused is a turn that never started.
        "response" if value["success"] == false => {
            let error = value["error"].as_str().unwrap_or("Pi refused the command");
            match value["command"].as_str() {
                Some("prompt") => vec![Update::TurnEnd {
                    error: Some(error.to_string()),
                }],
                _ => notice(Tone::Failed, error.to_string()),
            }
        }
        "extension_ui_request" => dialog(value),
        "compaction_start" => notice(Tone::Info, "Compacting the conversation".into()),
        "compaction_end" => match value["errorMessage"].as_str() {
            Some(error) => notice(Tone::Failed, format!("Compacting failed: {error}")),
            None => notice(Tone::Info, "Compacted the conversation".into()),
        },
        "auto_retry_start" => notice(
            Tone::Warning,
            format!(
                "Retrying ({} of {}): {}",
                value["attempt"],
                value["maxAttempts"],
                text("errorMessage")
            ),
        ),
        "auto_retry_end" if value["success"] == false => notice(
            Tone::Failed,
            format!("Gave up retrying: {}", text("finalError")),
        ),
        "extension_error" => notice(
            Tone::Failed,
            format!("An extension failed: {}", text("error")),
        ),
        _ => Vec::new(),
    }
}

/// An extension's dialog, as a question. `notify` is not a question: it is said.
fn dialog(value: &Value) -> Vec<Update> {
    let method = value["method"].as_str().unwrap_or_default();
    let text = |key: &str| value[key].as_str().map(str::to_string);
    let (options, input) = match method {
        "confirm" => (vec!["Yes".to_string(), "No".to_string()], false),
        "select" => (
            value["options"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|option| option.as_str().map(str::to_string))
                .collect(),
            false,
        ),
        "input" | "editor" => (Vec::new(), true),
        "notify" => {
            return vec![Update::Notice {
                tone: match value["notifyType"].as_str() {
                    Some("error") => Tone::Failed,
                    Some("warning") => Tone::Warning,
                    _ => Tone::Info,
                },
                text: text("message").unwrap_or_default(),
            }];
        }
        _ => return Vec::new(),
    };
    vec![Update::Ask {
        ask: Ask {
            id: text("id").unwrap_or_default(),
            title: text("title").unwrap_or_default(),
            message: text("message"),
            options,
            input,
        },
        detail: json!({ "method": method }),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers_each_kind_of_dialog_its_own_way() {
        let line =
            r#"{"type":"extension_ui_request","id":"u1","method":"confirm","title":"Run rm?"}"#;
        let updates = decode(&serde_json::from_str(line).unwrap());
        let [Update::Ask { ask, detail }] = updates.as_slice() else {
            panic!("a question");
        };
        assert_eq!(ask.options, ["Yes", "No"]);
        let reply = |picked| serde_json::from_str::<Value>(&answer(ask, picked, detail)).unwrap();
        assert_eq!(reply(Some("Yes"))["confirmed"], true);
        assert_eq!(reply(Some("No"))["confirmed"], false);
        assert_eq!(reply(None)["cancelled"], true);
        let picked = answer(ask, Some("staging"), &json!({ "method": "select" }));
        assert!(picked.contains(r#""value":"staging""#));
    }

    #[test]
    fn a_refused_prompt_ends_the_turn_with_the_reason() {
        let line = r#"{"type":"response","command":"prompt","success":false,"error":"No model"}"#;
        assert_eq!(
            decode(&serde_json::from_str(line).unwrap()),
            [Update::TurnEnd {
                error: Some("No model".into())
            }]
        );
    }
}
