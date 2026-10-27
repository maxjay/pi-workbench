//! Pi's messages (docs/message-types.md) as conversation entries: the
//! user's, the assistant's content blocks, and tool results.

use protocol::{Entry, Image, Tone, Tool, ToolStatus};
use serde_json::Value;

/// The text of a message's or result's content: a string, or the text
/// blocks of a list.
pub fn text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| block["type"] == "text")
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

pub fn images(content: &Value) -> Vec<Image> {
    content
        .as_array()
        .into_iter()
        .flatten()
        .filter(|block| block["type"] == "image")
        .filter_map(|block| {
            Some(Image {
                mime_type: block["mimeType"].as_str()?.to_string(),
                data: block["data"].as_str()?.to_string(),
            })
        })
        .collect()
}

pub fn user(message: &Value) -> Entry {
    Entry::User {
        text: text(&message["content"]),
        images: images(&message["content"]),
    }
}

/// One content block of an assistant message. None for one with nothing
/// to show, such as redacted thinking.
pub fn block(block: &Value) -> Option<Entry> {
    match block["type"].as_str()? {
        "text" => Some(Entry::Text {
            text: block["text"].as_str()?.to_string(),
        }),
        "thinking" => {
            let text = block["thinking"].as_str().unwrap_or_default();
            (!text.trim().is_empty()).then(|| Entry::Thinking {
                text: text.to_string(),
            })
        }
        "toolCall" => Some(Entry::Tool(Tool {
            id: block["id"].as_str()?.to_string(),
            name: block["name"].as_str().unwrap_or_default().to_string(),
            args: block["arguments"].clone(),
            status: ToolStatus::Running,
            output: String::new(),
            images: Vec::new(),
            details: Value::Null,
            browser: None,
        })),
        _ => None,
    }
}

/// What to say when an assistant message ended badly.
pub fn ending(message: &Value) -> Option<Entry> {
    match message["stopReason"].as_str()? {
        "error" => Some(Entry::Notice {
            text: message["errorMessage"]
                .as_str()
                .unwrap_or("The model returned an error")
                .to_string(),
            tone: Tone::Error,
        }),
        "aborted" => Some(Entry::Notice {
            text: "Stopped".into(),
            tone: Tone::Info,
        }),
        "length" => Some(Entry::Notice {
            text: "The reply hit the model's length limit".into(),
            tone: Tone::Warning,
        }),
        _ => None,
    }
}

/// Fills in a tool call from its result.
pub fn finish(tool: &mut Tool, result: &Value, failed: bool) {
    tool.output = text(&result["content"]);
    tool.images = images(&result["content"]);
    tool.details = result["details"].clone();
    tool.status = if failed {
        ToolStatus::Failed
    } else {
        ToolStatus::Done
    };
}

/// The other messages Pi keeps in a conversation, when there's something
/// to show for them.
pub fn other(message: &Value) -> Option<Entry> {
    match message["role"].as_str()? {
        "bashExecution" => Some(Entry::Tool(Tool {
            id: String::new(),
            name: "bash".into(),
            args: serde_json::json!({ "command": message["command"] }),
            status: if message["exitCode"].as_i64().unwrap_or(0) == 0 {
                ToolStatus::Done
            } else {
                ToolStatus::Failed
            },
            output: message["output"].as_str().unwrap_or_default().to_string(),
            images: Vec::new(),
            details: Value::Null,
            browser: None,
        })),
        "custom" if message["display"] == true => Some(Entry::Notice {
            text: text(&message["content"]),
            tone: Tone::Info,
        }),
        "compactionSummary" => Some(Entry::Notice {
            text: "Compacted the conversation".into(),
            tone: Tone::Info,
        }),
        _ => None,
    }
}

/// A whole conversation, from `get_messages`.
pub fn entries(messages: &[Value]) -> Vec<Entry> {
    let mut entries = Vec::new();
    for message in messages {
        match message["role"].as_str() {
            Some("user") => entries.push(user(message)),
            Some("assistant") => {
                let content = message["content"].as_array().into_iter().flatten();
                entries.extend(content.filter_map(block));
                entries.extend(ending(message));
            }
            Some("toolResult") => {
                let id = message["toolCallId"].as_str().unwrap_or_default();
                let call = entries.iter_mut().rev().find_map(|entry| match entry {
                    Entry::Tool(tool) if tool.id == id => Some(tool),
                    _ => None,
                });
                if let Some(tool) = call {
                    let failed = message["isError"].as_bool().unwrap_or_default();
                    finish(tool, message, failed);
                }
            }
            _ => entries.extend(other(message)),
        }
    }
    // A call with no result never ran: the run was stopped first.
    for entry in &mut entries {
        if let Entry::Tool(tool) = entry
            && tool.status == ToolStatus::Running
        {
            tool.status = ToolStatus::Failed;
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rebuilds_a_conversation() {
        let messages = [
            json!({ "role": "user", "content": "make hello.txt" }),
            json!({ "role": "assistant", "stopReason": "toolUse", "content": [
                { "type": "thinking", "thinking": "Easy." },
                { "type": "text", "text": "Writing it." },
                { "type": "toolCall", "id": "c1", "name": "write", "arguments": { "path": "hello.txt", "content": "hi" } },
            ]}),
            json!({ "role": "toolResult", "toolCallId": "c1", "toolName": "write", "isError": false,
                    "content": [{ "type": "text", "text": "Wrote 2 bytes" }] }),
            json!({ "role": "assistant", "stopReason": "error", "errorMessage": "503", "content": [] }),
        ];
        let entries = entries(&messages);
        assert_eq!(entries.len(), 5);
        assert!(matches!(&entries[0], Entry::User { text, .. } if text == "make hello.txt"));
        assert!(matches!(&entries[1], Entry::Thinking { .. }));
        let Entry::Tool(tool) = &entries[3] else {
            panic!("expected the tool call")
        };
        assert_eq!(tool.status, ToolStatus::Done);
        assert_eq!(tool.output, "Wrote 2 bytes");
        assert!(matches!(&entries[4], Entry::Notice { tone: Tone::Error, text } if text == "503"));
    }

    #[test]
    fn a_call_without_a_result_failed() {
        let messages = [
            json!({ "role": "assistant", "stopReason": "aborted", "content": [
                { "type": "toolCall", "id": "c1", "name": "bash", "arguments": { "command": "sleep 99" } },
            ]}),
        ];
        let entries = entries(&messages);
        assert!(matches!(&entries[0], Entry::Tool(tool) if tool.status == ToolStatus::Failed));
        assert!(matches!(&entries[1], Entry::Notice { text, .. } if text == "Stopped"));
    }

    #[test]
    fn keeps_images() {
        let user = user(&json!({ "role": "user", "content": [
            { "type": "text", "text": "what's this?" },
            { "type": "image", "mimeType": "image/png", "data": "AA==" },
        ]}));
        assert!(matches!(user, Entry::User { images, .. } if images.len() == 1));
    }
}
