//! Claude Code, through `claude -p` with stream-json in and out: one JSON
//! object per line each way, the interface Anthropic's SDK speaks. Its
//! permission prompts come over the same lines as control requests.

use protocol::Ask;
use serde_json::{Value, json};
use tokio::process::Command;

use super::result_text;
use crate::fold::Update;

/// `WB_CLAUDE` names another binary. A new conversation is given its id,
/// and one Claude Code already has is resumed by it.
pub fn command(id: &str, resume: bool) -> Command {
    let mut command = Command::new(std::env::var("WB_CLAUDE").unwrap_or_else(|_| "claude".into()));
    command.args([
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--permission-prompt-tool",
        "stdio",
    ]);
    command.args([if resume { "--resume" } else { "--session-id" }, id]);
    command
}

pub fn prompt(text: &str) -> String {
    json!({
        "type": "user",
        "message": { "role": "user", "content": [{ "type": "text", "text": text }] },
        "parent_tool_use_id": null,
    })
    .to_string()
}

pub fn stop() -> String {
    json!({
        "type": "control_request",
        "request_id": "wb-stop",
        "request": { "subtype": "interrupt" },
    })
    .to_string()
}

/// Answers a permission prompt. `detail` is the tool's input as it was
/// asked about, which an allow hands back.
pub fn answer(ask: &Ask, answer: Option<&str>, detail: &Value) -> String {
    let response = match answer {
        Some("Allow") => json!({ "behavior": "allow", "updatedInput": detail }),
        _ => json!({ "behavior": "deny", "message": "You said no." }),
    };
    json!({
        "type": "control_response",
        "response": { "subtype": "success", "request_id": ask.id, "response": response },
    })
    .to_string()
}

pub fn decode(value: &Value) -> Vec<Update> {
    match value["type"].as_str().unwrap_or_default() {
        // The reply as it is written, a piece at a time.
        "stream_event" => {
            let delta = &value["event"]["delta"];
            let piece = |key: &str| delta[key].as_str().unwrap_or_default().to_string();
            match delta["type"].as_str() {
                Some("text_delta") => vec![Update::Text(piece("text"))],
                Some("thinking_delta") => vec![Update::Thinking(piece("thinking"))],
                _ => Vec::new(),
            }
        }
        // A finished part of the reply. Its text was streamed already; its tool calls are new.
        "assistant" => blocks(value)
            .filter(|block| block["type"] == "tool_use")
            .map(|block| Update::ToolStart {
                id: text(block, "id"),
                name: text(block, "name"),
                input: block["input"].clone(),
            })
            .collect(),
        // What the tools returned comes back as if you had said it.
        "user" => blocks(value)
            .filter(|block| block["type"] == "tool_result")
            .map(|block| Update::ToolEnd {
                id: text(block, "tool_use_id"),
                failed: block["is_error"] == true,
                output: result_text(&block["content"]),
            })
            .collect(),
        "result" => {
            let failed = value["is_error"] == true || value["subtype"] != "success";
            let reason = value["result"]
                .as_str()
                .filter(|reason| !reason.is_empty())
                .or(value["subtype"].as_str())
                .unwrap_or("Claude Code stopped");
            vec![Update::TurnEnd {
                error: failed.then(|| reason.to_string()),
            }]
        }
        "control_request" if value["request"]["subtype"] == "can_use_tool" => {
            let request = &value["request"];
            let input = &request["input"];
            // What it wants to do, in its own words where it gave them.
            let message = ["command", "file_path", "path", "url", "description"]
                .iter()
                .find_map(|key| input[*key].as_str())
                .map(str::to_string);
            vec![Update::Ask {
                ask: Ask {
                    id: text(value, "request_id"),
                    title: format!("Allow {}?", text(request, "tool_name")),
                    message,
                    options: vec!["Allow".into(), "Deny".into()],
                    input: false,
                },
                detail: input.clone(),
            }]
        }
        "control_cancel_request" => vec![Update::AskGone],
        _ => Vec::new(),
    }
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_string()
}

/// The parts of a message's content.
fn blocks(value: &Value) -> impl Iterator<Item = &Value> {
    value["message"]["content"].as_array().into_iter().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_permission_prompt_is_a_question_and_allow_hands_the_input_back() {
        let line = r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"rm -rf build","description":"Remove the build"}}}"#;
        let updates = decode(&serde_json::from_str(line).unwrap());
        let [Update::Ask { ask, detail }] = updates.as_slice() else {
            panic!("a question");
        };
        assert_eq!(ask.title, "Allow Bash?");
        assert_eq!(ask.message.as_deref(), Some("rm -rf build"));

        let allow: Value = serde_json::from_str(&answer(ask, Some("Allow"), detail)).unwrap();
        assert_eq!(allow["response"]["request_id"], "r1");
        assert_eq!(allow["response"]["response"]["behavior"], "allow");
        assert_eq!(
            allow["response"]["response"]["updatedInput"]["command"],
            "rm -rf build"
        );
        let deny: Value = serde_json::from_str(&answer(ask, None, detail)).unwrap();
        assert_eq!(deny["response"]["response"]["behavior"], "deny");
    }

    #[test]
    fn a_failed_result_says_why() {
        let line =
            r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":""}"#;
        assert_eq!(
            decode(&serde_json::from_str(line).unwrap()),
            [Update::TurnEnd {
                error: Some("error_during_execution".into())
            }]
        );
    }
}
