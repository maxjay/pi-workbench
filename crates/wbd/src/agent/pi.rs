//! Pi, through `pi --mode rpc`: one JSON object per line each way.
//! See Pi's docs/rpc.md. Only what the workbench uses is read.

use protocol::{Ask, Model, Offer, Tone};
use serde_json::{Value, json};
use tokio::process::Command;

use super::result_text;
use crate::fold::Update;

/// `WB_PI` names another binary. The session id makes Pi open this
/// conversation's own session, or start it.
pub fn command(id: &str, model: Option<&str>, effort: Option<&str>) -> Command {
    let mut command = program();
    command.args(["--session-id", id]);
    if let Some(model) = model {
        command.args(["--model", model]);
    }
    if let Some(effort) = effort {
        command.args(["--thinking", effort]);
    }
    command
}

fn program() -> Command {
    let mut command = Command::new(std::env::var("WB_PI").unwrap_or_else(|_| "pi".into()));
    command.args(["--mode", "rpc"]);
    command
}

/// The lines that change a running Pi's model and thinking level. A model
/// is known here as `provider/id`.
pub fn set(model: Option<&str>, effort: Option<&str>) -> Vec<String> {
    let model = model
        .and_then(|model| model.split_once('/'))
        .map(|(provider, id)| {
            json!({ "type": "set_model", "provider": provider, "modelId": id }).to_string()
        });
    let effort =
        effort.map(|level| json!({ "type": "set_thinking_level", "level": level }).to_string());
    model.into_iter().chain(effort).collect()
}

/// A Pi that keeps no session, and what to ask it to learn what it offers.
pub fn offer_asks() -> (Command, Vec<String>) {
    let mut command = program();
    command.arg("--no-session");
    let asks = ["get_state", "get_available_models"];
    (
        command,
        asks.map(|ask| json!({ "type": ask }).to_string()).into(),
    )
}

pub fn is_offer_answer(value: &Value) -> bool {
    value["type"] == "response"
        && matches!(
            value["command"].as_str(),
            Some("get_state" | "get_available_models")
        )
}

/// Pi lists the models it has a sign-in for, so none means not signed in.
pub fn offer(answers: &[Value]) -> Offer {
    let data = |command: &str| {
        answers
            .iter()
            .find(|answer| answer["command"] == command)
            .map_or(&Value::Null, |answer| &answer["data"])
    };
    let id = |model: &Value| {
        Some(format!(
            "{}/{}",
            model["provider"].as_str()?,
            model["id"].as_str()?
        ))
    };
    let models: Vec<Model> = data("get_available_models")["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|model| {
            Some(Model {
                id: id(model)?,
                name: model["name"].as_str().or(model["id"].as_str())?.to_string(),
                efforts: efforts(model),
            })
        })
        .collect();
    let state = data("get_state");
    Offer {
        signed_in: !models.is_empty(),
        model: id(&state["model"]),
        effort: state["thinkingLevel"].as_str().map(str::to_string),
        models,
    }
}

/// The thinking levels a model takes. Its `thinkingLevelMap` says which: a
/// level set to null is one it doesn't have, and the two highest count
/// only where they are named.
fn efforts(model: &Value) -> Vec<String> {
    if model["reasoning"] != true {
        return Vec::new();
    }
    let level = |name: &str| model["thinkingLevelMap"].get(name);
    let usual = ["minimal", "low", "medium", "high"]
        .into_iter()
        .filter(|name| !level(name).is_some_and(Value::is_null));
    let highest = ["xhigh", "max"]
        .into_iter()
        .filter(|name| level(name).is_some_and(|to| !to.is_null()));
    ["off"]
        .into_iter()
        .chain(usual)
        .chain(highest)
        .map(str::to_string)
        .collect()
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
    fn says_what_it_offers_from_its_state_and_its_models() {
        let answers = [
            json!({ "type": "response", "command": "get_state", "data": {
                "model": { "provider": "anthropic", "id": "claude-haiku-5-5" },
                "thinkingLevel": "medium",
            }}),
            json!({ "type": "response", "command": "get_available_models", "data": { "models": [
                { "provider": "anthropic", "id": "claude-haiku-5-5", "name": "Claude Haiku 5.5",
                  "reasoning": true, "thinkingLevelMap": { "minimal": null, "xhigh": "xhigh" } },
                { "provider": "local", "id": "plain", "name": "Plain", "reasoning": false },
            ]}}),
        ];
        assert!(answers.iter().all(is_offer_answer));
        let offer = offer(&answers);
        assert!(offer.signed_in);
        assert_eq!(offer.model.as_deref(), Some("anthropic/claude-haiku-5-5"));
        assert_eq!(offer.effort.as_deref(), Some("medium"));
        assert_eq!(
            offer.models[0].efforts,
            ["off", "low", "medium", "high", "xhigh"]
        );
        assert!(offer.models[1].efforts.is_empty());
        assert!(!super::offer(&[]).signed_in);
    }

    #[test]
    fn changes_the_model_and_the_level_with_a_line_each() {
        let lines = set(Some("anthropic/claude-sonnet-5-5"), Some("high"));
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains(r#""modelId":"claude-sonnet-5-5""#));
        assert!(lines[0].contains(r#""provider":"anthropic""#));
        assert!(lines[1].contains(r#""level":"high""#));
        assert!(set(None, None).is_empty());
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
