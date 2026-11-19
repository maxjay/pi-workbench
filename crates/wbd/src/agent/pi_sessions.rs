//! The sessions Pi keeps by itself: one JSON Lines file each, in a folder
//! named after the folder Pi worked in. They are read here, so that a
//! session made outside the workbench can be listed, opened and carried
//! on. Nothing is written. Only what the workbench shows is read.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use protocol::{Agent, Conversation, ConversationView, Entry, Tool, ToolStatus};
use serde::Deserialize;
use serde_json::Value;

use super::result_text;
use crate::fold::{OUTPUT_LIMIT, describe, tail};

/// Where Pi keeps its sessions: under `~/.pi/agent`, or under the folder
/// `PI_CODING_AGENT_DIR` names, which Pi reads too.
pub fn root() -> Option<PathBuf> {
    let agent = match std::env::var_os("PI_CODING_AGENT_DIR") {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(std::env::var_os("HOME")?).join(".pi/agent"),
    };
    Some(agent.join("sessions"))
}

/// One line of a session file.
#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: String,
    id: Option<String>,
    #[serde(rename = "parentId")]
    parent: Option<String>,
    /// Only the first line has it.
    cwd: Option<String>,
    message: Option<Message>,
}

/// What was said, on a line of the type `message`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Message {
    #[serde(default)]
    role: String,
    /// A text, or a list of parts: text, thinking, tool calls, images.
    #[serde(default)]
    content: Value,
    /// On a tool's result: the call it answers.
    #[serde(default)]
    tool_call_id: Value,
    #[serde(default)]
    is_error: Value,
}

/// Pi's sessions in a folder. `title` is the whole of what you first wrote.
/// A file that can't be read or isn't a session is left out, and so is a
/// session in which you wrote nothing.
pub fn list(root: &Path, cwd: &str) -> Vec<Conversation> {
    files(&root.join(folder(cwd)))
        .filter_map(|path| head(&path))
        // Two folders can share a name here, as a dash in a path reads like a slash.
        .filter(|state| state.cwd == cwd)
        .collect()
}

/// One session, whole, whichever folder it was in. Its file's name ends
/// with its id.
pub fn open(root: &Path, id: &str) -> Option<ConversationView> {
    let name = format!("_{id}.jsonl");
    let named = |path: &PathBuf| {
        let file = path.file_name().and_then(|file| file.to_str());
        file.is_some_and(|file| file.ends_with(&name))
    };
    std::fs::read_dir(root)
        .ok()?
        .flatten()
        .flat_map(|folder| files(&folder.path()))
        .filter(named)
        .find_map(|path| whole(&path).filter(|view| view.state.id == id))
}

/// The name of the folder that holds a folder's sessions. Pi drops the
/// first slash, turns every other slash, backslash and colon into a dash,
/// and puts two dashes at each end. A dot or a dash in the path stays.
fn folder(cwd: &str) -> String {
    let inside = cwd.strip_prefix(['/', '\\']).unwrap_or(cwd);
    format!("--{}--", inside.replace(['/', '\\', ':'], "-"))
}

/// The session files in one folder.
fn files(folder: &Path) -> impl Iterator<Item = PathBuf> + use<> {
    std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
}

/// A session as a list shows it. Only the start of the file is read: up
/// to the first thing you wrote.
fn head(path: &Path) -> Option<Conversation> {
    let (mut state, mut lines) = session(path)?;
    state.title = lines.find_map(|line| you(line.message()?))?;
    Some(state)
}

/// A session with everything said in it.
fn whole(path: &Path) -> Option<ConversationView> {
    let (mut state, lines) = session(path)?;
    let lines: Vec<Line> = lines.collect();
    state.title = lines.iter().find_map(|line| you(line.message()?))?;
    Some(ConversationView {
        state,
        entries: entries(&branch(lines)),
    })
}

/// A session file's first line as a conversation with no title yet, and
/// the lines after it. A line that isn't JSON, or isn't a shape known
/// here, is passed over. `updated` is when the file last changed.
fn session(path: &Path) -> Option<(Conversation, impl Iterator<Item = Line> + use<>)> {
    let file = File::open(path).ok()?;
    let changed = file.metadata().ok()?.modified().ok()?;
    let mut lines = BufReader::new(file)
        .split(b'\n')
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_slice::<Line>(&line).ok());
    let first = lines.next().filter(|line| line.kind == "session")?;
    let state = Conversation {
        id: first.id?,
        agent: Agent::Pi,
        cwd: first.cwd?,
        title: String::new(),
        updated: changed.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64,
        busy: false,
        ask: None,
        stopped: None,
        model: None,
        effort: None,
    };
    Some((state, lines))
}

impl Line {
    fn message(&self) -> Option<&Message> {
        self.message.as_ref().filter(|_| self.kind == "message")
    }
}

/// What you wrote in a message, if it is yours and has any text.
fn you(message: &Message) -> Option<String> {
    let text = match message.role.as_str() {
        "user" => result_text(&message.content),
        _ => return None,
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// The lines that lead to the last one, each through the one it follows.
/// A session is a tree once you go back in it and write something else.
/// Pi carries on from the last line, so its branch is the conversation.
fn branch(lines: Vec<Line>) -> Vec<Line> {
    // Lines with no ids are from an old Pi, which had no branches.
    let Some(mut at) = lines.last().and_then(|line| line.id.clone()) else {
        return lines;
    };
    let mut by_id: HashMap<String, Line> = lines
        .into_iter()
        .filter_map(|line| Some((line.id.clone()?, line)))
        .collect();
    let mut path = Vec::new();
    // Taking each line out as it is passed ends a loop of parents.
    while let Some(line) = by_id.remove(&at) {
        let parent = line.parent.clone();
        path.push(line);
        match parent {
            Some(parent) => at = parent,
            None => break,
        }
    }
    path.reverse();
    path
}

/// What the page shows of a session: what you wrote, and what Pi said,
/// thought and did. What Pi was told about itself is not shown.
fn entries(lines: &[Line]) -> Vec<Entry> {
    let messages = || lines.iter().filter_map(Line::message);
    // A tool's result is a message of its own. It names the call it answers.
    let results: HashMap<&str, &Message> = messages()
        .filter(|message| message.role == "toolResult")
        .filter_map(|message| Some((message.tool_call_id.as_str()?, message)))
        .collect();
    let mut entries = Vec::new();
    for message in messages() {
        if let Some(text) = you(message) {
            entries.push(Entry::You { text });
        }
        if message.role != "assistant" {
            continue;
        }
        for part in message.content.as_array().into_iter().flatten() {
            let text = |key: &str| part[key].as_str().filter(|text| !text.is_empty());
            let entry = match part["type"].as_str() {
                Some("text") => text("text").map(|text| Entry::Text { text: text.into() }),
                Some("thinking") => {
                    text("thinking").map(|text| Entry::Thinking { text: text.into() })
                }
                Some("toolCall") => tool(part, &results),
                _ => None,
            };
            entries.extend(entry);
        }
    }
    entries
}

/// A tool call, with its result where it has one.
fn tool(call: &Value, results: &HashMap<&str, &Message>) -> Option<Entry> {
    let id = call["id"].as_str()?;
    let (title, diff) = describe(&call["arguments"]);
    let result = results.get(id);
    Some(Entry::Tool(Tool {
        id: id.to_string(),
        name: call["name"].as_str().unwrap_or_default().to_string(),
        title,
        // A call with no result was cut off. Nothing runs it now.
        status: match result {
            Some(result) if result.is_error != true => ToolStatus::Done,
            _ => ToolStatus::Failed,
        },
        diff,
        output: result.map_or_else(String::new, |result| {
            tail(&result_text(&result.content), OUTPUT_LIMIT)
        }),
    }))
}

#[cfg(test)]
mod tests {
    use protocol::Diff;
    use serde_json::json;

    use super::*;

    const CWD: &str = "/home/max/code/shop";

    /// Writes a session file where Pi would, and gives its path.
    fn write(root: &Path, cwd: &str, id: &str, lines: &[Value]) -> PathBuf {
        let folder = root.join(folder(cwd));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join(format!("2026-01-02T03-04-05-678Z_{id}.jsonl"));
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        std::fs::write(&path, text).unwrap();
        path
    }

    fn first(id: &str, cwd: &str) -> Value {
        json!({ "type": "session", "version": 3, "id": id, "cwd": cwd })
    }

    /// A line of the type `message`, after the line `parent`.
    fn said(id: &str, parent: Option<&str>, message: Value) -> Value {
        json!({ "type": "message", "id": id, "parentId": parent, "message": message })
    }

    fn user(id: &str, parent: Option<&str>, text: &str) -> Value {
        let content = json!([{ "type": "text", "text": text }]);
        said(id, parent, json!({ "role": "user", "content": content }))
    }

    fn assistant(id: &str, parent: &str, parts: Value) -> Value {
        let message = json!({ "role": "assistant", "content": parts });
        said(id, Some(parent), message)
    }

    fn call(id: &str, name: &str, arguments: Value) -> Value {
        json!({ "type": "toolCall", "id": id, "name": name, "arguments": arguments })
    }

    fn result(id: &str, parent: &str, call: &str, failed: bool, text: &str) -> Value {
        let content = json!([{ "type": "text", "text": text }]);
        let message = json!({
            "role": "toolResult", "toolCallId": call, "isError": failed, "content": content,
        });
        said(id, Some(parent), message)
    }

    /// A session as Pi writes one: its model, what it was told, what you
    /// wrote, a tool that ran, one that failed, one cut off, and the answer.
    fn session_lines(id: &str) -> Vec<Value> {
        let told = json!({
            "role": "system", "content": "", "sections": { "preamble": "You write code." },
        });
        let edit = json!({ "path": "src/auth.ts", "oldText": "/login", "newText": "/sign-in" });
        vec![
            first(id, CWD),
            json!({ "type": "model_change", "id": "a1", "parentId": null, "modelId": "haiku" }),
            json!({ "type": "thinking_level_change", "id": "a2", "parentId": "a1" }),
            said("a3", Some("a2"), told),
            user("a4", Some("a3"), "Fix the login\nand run the tests"),
            assistant(
                "a5",
                "a4",
                json!([
                    { "type": "thinking", "thinking": "The path is wrong.", "thinkingSignature": "abc" },
                    { "type": "text", "text": "I will change it." },
                    call("t1", "edit", edit),
                    call("t2", "bash", json!({ "command": "npm test", "timeout": null })),
                    call("t3", "bash", json!({ "command": "sleep 100" })),
                ]),
            ),
            result("a6", "a5", "t1", false, "Edited src/auth.ts"),
            result("a7", "a6", "t2", true, "1 failed"),
            assistant(
                "a8",
                "a7",
                json!([{ "type": "text", "text": "One test fails." }]),
            ),
        ]
    }

    #[test]
    fn names_a_folder_of_sessions_as_pi_does() {
        assert_eq!(
            folder("/home/max/dova-real-test/storefront"),
            "--home-max-dova-real-test-storefront--"
        );
        // A dot and a dash stay, so two paths can give one name.
        assert_eq!(
            folder("/home/max/.local/share/a.b/c-d"),
            "--home-max-.local-share-a.b-c-d--"
        );
        assert_eq!(folder("/"), "----");
        assert_eq!(folder("C:\\code\\shop"), "--C--code-shop--");
    }

    #[test]
    fn lists_a_folder_s_sessions_with_what_you_first_wrote() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), CWD, "s1", &session_lines("s1"));
        // Its folder has the same name, and it is another folder.
        let other = "/home/max/code-shop";
        write(
            root.path(),
            other,
            "s2",
            &[first("s2", other), user("b1", None, "Hi")],
        );
        let listed = list(root.path(), CWD);
        assert_eq!(listed.len(), 1);
        let state = &listed[0];
        assert_eq!((state.id.as_str(), state.cwd.as_str()), ("s1", CWD));
        assert_eq!(state.agent, Agent::Pi);
        assert_eq!(state.title, "Fix the login\nand run the tests");
        assert!(state.updated > 0 && !state.busy);
        assert!(list(root.path(), "/home/max/elsewhere").is_empty());
    }

    #[test]
    fn turns_a_session_into_what_the_page_shows() {
        let root = tempfile::tempdir().unwrap();
        write(root.path(), CWD, "s1", &session_lines("s1"));
        let view = open(root.path(), "s1").unwrap();
        assert_eq!(view.state.cwd, CWD);
        let tool = |index: usize| match &view.entries[index] {
            Entry::Tool(tool) => tool,
            other => panic!("a tool, not {other:?}"),
        };
        assert_eq!(view.entries.len(), 7);
        assert_eq!(
            view.entries[0],
            Entry::You {
                text: "Fix the login\nand run the tests".into()
            }
        );
        assert_eq!(
            view.entries[1],
            Entry::Thinking {
                text: "The path is wrong.".into()
            }
        );
        assert_eq!(
            view.entries[2],
            Entry::Text {
                text: "I will change it.".into()
            }
        );
        assert_eq!(
            (tool(3).name.as_str(), tool(3).title.as_str()),
            ("edit", "src/auth.ts")
        );
        assert_eq!(tool(3).status, ToolStatus::Done);
        assert_eq!(tool(3).output, "Edited src/auth.ts");
        assert_eq!(
            tool(3).diff,
            Some(Diff {
                path: "src/auth.ts".into(),
                old: Some("/login".into()),
                new: "/sign-in".into()
            })
        );
        assert_eq!(tool(4).title, "npm test");
        assert_eq!(tool(4).status, ToolStatus::Failed);
        assert_eq!(tool(4).output, "1 failed");
        // The last call has no result.
        assert_eq!(tool(5).status, ToolStatus::Failed);
        assert_eq!(tool(5).output, "");
        assert_eq!(
            view.entries[6],
            Entry::Text {
                text: "One test fails.".into()
            }
        );
        assert!(open(root.path(), "s2").is_none());
    }

    #[test]
    fn keeps_the_end_of_a_tool_s_long_output() {
        let root = tempfile::tempdir().unwrap();
        let long = format!("{}end", "x".repeat(OUTPUT_LIMIT));
        let lines = [
            first("s1", CWD),
            user("a1", None, "Run it"),
            assistant(
                "a2",
                "a1",
                json!([call("t1", "bash", json!({ "command": "yes" }))]),
            ),
            result("a3", "a2", "t1", false, &long),
        ];
        write(root.path(), CWD, "s1", &lines);
        let view = open(root.path(), "s1").unwrap();
        let Entry::Tool(tool) = &view.entries[1] else {
            panic!("a tool");
        };
        assert!(tool.output.starts_with('…') && tool.output.ends_with("end"));
        assert_eq!(tool.output.len(), OUTPUT_LIMIT + '…'.len_utf8());
    }

    #[test]
    fn leaves_out_a_session_in_which_you_wrote_nothing() {
        let root = tempfile::tempdir().unwrap();
        let lines = session_lines("s1");
        write(root.path(), CWD, "s1", &lines[..4]);
        // One whose only message from you is a picture.
        let content = json!([{ "type": "image", "data": "aGk=", "mimeType": "image/png" }]);
        let picture = json!({ "role": "user", "content": content });
        write(
            root.path(),
            CWD,
            "s2",
            &[first("s2", CWD), said("b1", None, picture)],
        );
        assert!(list(root.path(), CWD).is_empty());
        assert!(open(root.path(), "s1").is_none());
    }

    #[test]
    fn passes_over_a_broken_line_and_a_file_that_is_no_session() {
        let root = tempfile::tempdir().unwrap();
        let path = write(root.path(), CWD, "s1", &[]);
        let lines = session_lines("s1");
        let mut text = String::new();
        for (index, line) in lines.iter().enumerate() {
            text.push_str(&format!("{line}\n"));
            if index == 3 {
                // Cut short, not text, and a message of no known shape.
                text.push_str("{\"type\":\"message\",\"id\":\"x1\",\"mess\n");
                text.push_str("not json\n\n");
                text.push_str("{\"type\":\"message\",\"id\":\"x2\",\"message\":[1,2]}\n");
            }
        }
        std::fs::write(&path, text).unwrap();
        // Files that are not sessions, beside it.
        let folder = path.parent().unwrap();
        std::fs::write(folder.join("1_s3.jsonl"), "{\"type\":\"message\"}\n").unwrap();
        std::fs::write(folder.join("2_s4.jsonl"), [0xff, 0xfe, b'\n']).unwrap();
        std::fs::write(folder.join("3_s5.jsonl"), "").unwrap();
        std::fs::write(folder.join("notes.txt"), "Hi").unwrap();

        let listed = list(root.path(), CWD);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "s1");
        assert_eq!(open(root.path(), "s1").unwrap().entries.len(), 7);
        assert!(open(root.path(), "s3").is_none());
    }

    #[test]
    fn follows_the_branch_pi_carries_on_from() {
        let root = tempfile::tempdir().unwrap();
        let answer = |id: &str, parent: &str, text: &str| {
            assistant(id, parent, json!([{ "type": "text", "text": text }]))
        };
        // You went back to the start and wrote something else.
        let lines = [
            first("s1", CWD),
            user("a1", None, "Name a fruit"),
            answer("a2", "a1", "Apple."),
            user("a3", Some("a2"), "Another"),
            answer("a4", "a3", "Pear."),
            user("b3", Some("a2"), "A red one"),
            answer("b4", "b3", "Cherry."),
        ];
        write(root.path(), CWD, "s1", &lines);
        let said: Vec<String> = open(root.path(), "s1")
            .unwrap()
            .entries
            .into_iter()
            .map(|entry| match entry {
                Entry::You { text } | Entry::Text { text } => text,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(said, ["Name a fruit", "Apple.", "A red one", "Cherry."]);
    }
}
