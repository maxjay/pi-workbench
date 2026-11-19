//! The sessions Pi keeps by itself: one JSON Lines file each, in a folder
//! named after the folder Pi worked in. They are read here, so that a
//! session made outside the workbench can be listed, opened and carried
//! on. Nothing is written. Only what the workbench shows is read. Pi's
//! docs/session-format.md says what a file holds.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Split};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use protocol::{Agent, Conversation, ConversationView, Entry, Tone, Tool, ToolStatus};
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

/// One line of a session file. Only what holds the lines together has a
/// set shape. The rest is taken as it comes, so that no shape of a message
/// loses its line.
#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type", default)]
    kind: String,
    id: Option<String>,
    #[serde(rename = "parentId")]
    parent: Option<String>,
    /// Only the first line has it.
    #[serde(default)]
    cwd: Value,
    /// What was said, on a line of the type `message`: its `role`, and its
    /// `content`, which is a text or a list of parts.
    #[serde(default)]
    message: Value,
    /// The line is JSON, and what it says could not be read.
    #[serde(skip)]
    unread: bool,
}

/// What holds a line in its place: all that is read of a line whose
/// message can't be read.
#[derive(Deserialize)]
struct Bare {
    #[serde(rename = "type", default)]
    kind: String,
    id: Option<String>,
    #[serde(rename = "parentId")]
    parent: Option<String>,
}

/// The lines of a session file that are JSON, in order. A line that isn't
/// is passed over, as Pi passes over it.
struct Lines {
    read: Split<BufReader<File>>,
    /// Why reading stopped before the end of the file, when it did.
    failed: Option<std::io::Error>,
}

impl Iterator for Lines {
    type Item = Line;

    fn next(&mut self) -> Option<Line> {
        loop {
            match self.read.next()? {
                Ok(bytes) => {
                    if let Some(line) = parse(&bytes) {
                        return Some(line);
                    }
                }
                Err(err) => {
                    self.failed = Some(err);
                    return None;
                }
            }
        }
    }
}

/// One line. Nothing, for a line that isn't JSON.
fn parse(line: &[u8]) -> Option<Line> {
    // Pi reads bytes that are not text as the mark for an unknown character.
    let text = String::from_utf8_lossy(line);
    if let Ok(line) = serde_json::from_str::<Line>(&text) {
        return Some(line);
    }
    if let Ok(line) = serde_json::from_str::<Line>(&mended(&text)) {
        return Some(line);
    }
    // The line still follows one line and is followed by others.
    let bare = serde_json::from_str::<Bare>(&text).ok()?;
    Some(Line {
        kind: bare.kind,
        id: bare.id,
        parent: bare.parent,
        cwd: Value::Null,
        message: Value::Null,
        unread: true,
    })
}

/// A line with every half of a character that stands without its other
/// half turned into the mark for an unknown character. JavaScript writes
/// such a half as `\ud83d`, so Pi's files can hold one, and it is not text.
fn mended(line: &str) -> String {
    let bytes = line.as_bytes();
    // The number in a `\uXXXX` that starts at `at`.
    let unit = |at: usize| {
        let escape = bytes.get(at) == Some(&b'\\') && bytes.get(at + 1) == Some(&b'u');
        let digits = line.get(at + 2..at + 6).filter(|_| escape)?;
        u16::from_str_radix(digits, 16).ok()
    };
    let mut out = String::with_capacity(line.len());
    let (mut at, mut kept) = (0, 0);
    while at < bytes.len() {
        if bytes[at] != b'\\' {
            at += 1;
            continue;
        }
        match unit(at) {
            // Any other escape: the backslash and the character after it.
            None => at += 2,
            Some(0xD800..=0xDBFF) if matches!(unit(at + 6), Some(0xDC00..=0xDFFF)) => at += 12,
            Some(0xD800..=0xDFFF) => {
                out.push_str(&line[kept..at]);
                out.push_str("\\ufffd");
                at += 6;
                kept = at;
            }
            Some(_) => at += 6,
        }
    }
    out.push_str(&line[kept..]);
    out
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

/// One session, whole, whichever folder it is in. Nothing, when Pi keeps
/// no session with this id. A session that is there and can't be read is
/// an error that says why. `near` is the folder the session worked in,
/// where that is known: its sessions are looked at first.
pub fn open(root: &Path, id: &str, near: Option<&str>) -> Result<Option<ConversationView>, String> {
    let near = near.map(|cwd| root.join(folder(cwd)));
    let all = || {
        let folders = std::fs::read_dir(root).into_iter().flatten().flatten();
        folders.map(|folder| folder.path())
    };
    // Pi ends a session file's name with the session's id.
    let name = format!("_{id}.jsonl");
    let named = |path: &PathBuf| {
        let file = path.file_name().and_then(|file| file.to_str());
        file.is_some_and(|file| file.ends_with(&name))
    };
    let mut why = None;
    let folders = near.iter().cloned().chain(all());
    for path in folders.flat_map(|folder| files(&folder)).filter(named) {
        match whole(&path) {
            Ok(view) if view.state.id == id => return Ok(Some(view)),
            Ok(_) => {}
            Err(reason) => why = why.or(Some(reason)),
        }
    }
    if let Some(why) = why {
        return Err(why);
    }
    // A file that was copied or given another name still starts with its
    // id. A list goes by that, so what is listed opens. Each file is opened
    // to see, so only the folder's own are, where the folder is known.
    let folders: Vec<PathBuf> = near.map_or_else(|| all().collect(), |near| vec![near]);
    let said = |path: &PathBuf| session(path).is_ok_and(|(state, _)| state.id == id);
    let found = folders.iter().flat_map(|folder| files(folder)).find(said);
    found.map(|path| whole(&path)).transpose()
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
    let (mut state, mut lines) = session(path).ok()?;
    state.title = lines.find_map(|line| you(&line))?;
    Some(state)
}

/// A session with everything said in it, or why it can't be read. What
/// could be read is shown, with a note where something is missing.
fn whole(path: &Path) -> Result<ConversationView, String> {
    let (mut state, mut lines) = session(path)?;
    let all: Vec<Line> = lines.by_ref().collect();
    state.title = all
        .iter()
        .find_map(you)
        .ok_or_else(|| format!("{} has nothing you wrote in it", path.display()))?;
    let (on_branch, unbroken) = branch(all);
    let mut shown = Vec::new();
    if !unbroken {
        let text = "What came before this is missing from Pi's session file.";
        shown.push(notice(Tone::Warning, text));
    }
    shown.extend(entries(&on_branch));
    if let Some(err) = lines.failed {
        let text = format!("Pi's session file could not be read to its end: {err}");
        shown.push(notice(Tone::Failed, &text));
    }
    if shown.is_empty() {
        let text = "Nothing was said on the branch of this session that Pi carries on from.";
        shown.push(notice(Tone::Info, text));
    }
    Ok(ConversationView {
        state,
        entries: shown,
    })
}

/// A session file's first line as a conversation with no title yet, and
/// the lines after it. `updated` is when the file last changed.
fn session(path: &Path) -> Result<(Conversation, Lines), String> {
    let failed = |err: std::io::Error| format!("{}: {err}", path.display());
    let file = File::open(path).map_err(failed)?;
    let changed = file
        .metadata()
        .and_then(|data| data.modified())
        .map_err(failed)?;
    let mut lines = Lines {
        read: BufReader::new(file).split(b'\n'),
        failed: None,
    };
    let first = lines.next().filter(|line| line.kind == "session");
    let Some((id, cwd)) = first.and_then(|line| Some((line.id?, line.cwd.as_str()?.to_string())))
    else {
        return Err(match lines.failed.take() {
            Some(err) => failed(err),
            None => format!("{} does not start as a Pi session does", path.display()),
        });
    };
    let state = Conversation {
        id,
        agent: Agent::Pi,
        cwd,
        title: String::new(),
        updated: changed
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_millis() as u64),
        busy: false,
        ask: None,
        stopped: None,
        model: None,
        effort: None,
    };
    Ok((state, lines))
}

/// What you wrote on a line, if it is a message of yours and has any text.
fn you(line: &Line) -> Option<String> {
    if line.kind != "message" || line.message["role"] != "user" {
        return None;
    }
    let text = result_text(&line.message["content"]);
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// The lines that lead to the last one, each through the one it follows.
/// A session is a tree once you go back in it and write something else.
/// Pi carries on from the last line, so its branch is the conversation.
/// False with them when a line on the way is not in the file, so that
/// what came before it is not known. Pi itself then sees no more.
fn branch(lines: Vec<Line>) -> (Vec<Line>, bool) {
    // Lines with no ids are from an old Pi, which had no branches.
    let Some(mut at) = lines.last().and_then(|line| line.id.clone()) else {
        return (lines, true);
    };
    let mut by_id: HashMap<String, Line> = lines
        .into_iter()
        .filter_map(|line| Some((line.id.clone()?, line)))
        .collect();
    let mut path = Vec::new();
    let mut unbroken = true;
    loop {
        // Taking each line out as it is passed ends a loop of parents.
        let Some(line) = by_id.remove(&at) else {
            unbroken = false;
            break;
        };
        let parent = line.parent.clone();
        path.push(line);
        match parent {
            Some(parent) => at = parent,
            None => break,
        }
    }
    path.reverse();
    (path, unbroken)
}

/// What the page shows of a session: what you wrote, and what Pi said,
/// thought and did. What Pi was told about itself is not shown.
fn entries(lines: &[Line]) -> Vec<Entry> {
    // A tool's result is a message of its own. It names the call it answers.
    let results: HashMap<&str, &Value> = lines
        .iter()
        .map(|line| &line.message)
        .filter(|message| message["role"] == "toolResult")
        .filter_map(|message| Some((message["toolCallId"].as_str()?, message)))
        .collect();
    let mut entries = Vec::new();
    for line in lines {
        match line.kind.as_str() {
            "message" if line.unread => {
                entries.push(notice(Tone::Warning, "A message here could not be read."));
            }
            "message" => said(line, &results, &mut entries),
            // The messages before it stay in the file, and are shown.
            "compaction" => entries.push(notice(Tone::Info, "Compacted the conversation")),
            _ => {}
        }
    }
    entries
}

/// What one message shows.
fn said(line: &Line, results: &HashMap<&str, &Value>, entries: &mut Vec<Entry>) {
    let message = &line.message;
    let text = |from: &Value, key: &str| {
        let text = from[key].as_str().filter(|text| !text.is_empty());
        text.map(str::to_string)
    };
    match message["role"].as_str() {
        Some("user") => entries.extend(you(line).map(|text| Entry::You { text })),
        Some("assistant") => {
            entries.extend(text(message, "content").map(|text| Entry::Text { text }));
            for part in message["content"].as_array().into_iter().flatten() {
                let entry = match part["type"].as_str() {
                    Some("text") => text(part, "text").map(|text| Entry::Text { text }),
                    Some("thinking") => text(part, "thinking").map(|text| Entry::Thinking { text }),
                    Some("toolCall") => tool(part, results),
                    _ => None,
                };
                entries.extend(entry);
            }
            // A reply that failed has no words of its own. Its error says why.
            match message["stopReason"].as_str() {
                Some("error") => {
                    let why = text(message, "errorMessage");
                    let why = why.as_deref().unwrap_or("Pi's reply failed.");
                    entries.push(notice(Tone::Failed, why));
                }
                Some("aborted") => entries.push(notice(Tone::Warning, "Stopped")),
                _ => {}
            }
        }
        // A command you ran yourself in Pi, with `!`. No call comes before it.
        Some("bashExecution") => {
            let command = text(message, "command").unwrap_or_default();
            let mark = match message["excludeFromContext"] == true {
                true => "!!",
                false => "!",
            };
            entries.push(Entry::You {
                text: format!("{mark}{command}"),
            });
            entries.push(Entry::Tool(Tool {
                id: line.id.clone().unwrap_or_default(),
                name: "bash".to_string(),
                title: command,
                status: match message["exitCode"] == 0 && message["cancelled"] != true {
                    true => ToolStatus::Done,
                    false => ToolStatus::Failed,
                },
                diff: None,
                output: tail(&text(message, "output").unwrap_or_default(), OUTPUT_LIMIT),
            }));
        }
        _ => {}
    }
}

/// A tool call, with its result where it has one.
fn tool(call: &Value, results: &HashMap<&str, &Value>) -> Option<Entry> {
    let id = call["id"].as_str()?;
    let (title, diff) = describe(&call["arguments"]);
    let result = results.get(id);
    Some(Entry::Tool(Tool {
        id: id.to_string(),
        name: call["name"].as_str().unwrap_or_default().to_string(),
        title,
        // A call with no result was cut off. Nothing runs it now.
        status: match result {
            Some(result) if result["isError"] != true => ToolStatus::Done,
            _ => ToolStatus::Failed,
        },
        diff,
        output: result.map_or_else(String::new, |result| {
            tail(&result_text(&result["content"]), OUTPUT_LIMIT)
        }),
    }))
}

fn notice(tone: Tone, text: &str) -> Entry {
    Entry::Notice {
        tone,
        text: text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use protocol::Diff;
    use serde_json::json;

    use super::*;

    const CWD: &str = "/home/max/code/shop";

    /// Writes a session file where Pi would, and gives its path.
    fn write(root: &Path, cwd: &str, id: &str, lines: &[Value]) -> PathBuf {
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        write_text(root, cwd, id, &text)
    }

    /// The same, from the text of the file: for lines that are not good JSON.
    fn write_text(root: &Path, cwd: &str, id: &str, text: &str) -> PathBuf {
        let folder = root.join(folder(cwd));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join(format!("2026-01-02T03-04-05-678Z_{id}.jsonl"));
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

    fn answer(id: &str, parent: &str, text: &str) -> Value {
        assistant(id, parent, json!([{ "type": "text", "text": text }]))
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

    fn opened(root: &Path, id: &str) -> ConversationView {
        open(root, id, None).unwrap().unwrap()
    }

    /// What was said, one word for each kind of entry and then its text.
    fn told(view: &ConversationView) -> Vec<String> {
        let each = |entry: &Entry| match entry {
            Entry::You { text } => format!("you: {text}"),
            Entry::Text { text } => format!("text: {text}"),
            Entry::Thinking { text } => format!("thinking: {text}"),
            Entry::Tool(tool) => format!("tool: {} {}", tool.name, tool.title),
            Entry::Notice { tone, text } => format!("{tone:?}: {text}"),
        };
        view.entries.iter().map(each).collect()
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
            answer("a8", "a7", "One test fails."),
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
        let view = opened(root.path(), "s1");
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
        assert_eq!(open(root.path(), "s2", None), Ok(None));
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
        let view = opened(root.path(), "s1");
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
        let why = open(root.path(), "s1", None).unwrap_err();
        assert!(why.ends_with("_s1.jsonl has nothing you wrote in it"));
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
        assert_eq!(opened(root.path(), "s1").entries.len(), 7);
        // A file with the id in its name that is no session says so.
        let why = open(root.path(), "s3", None).unwrap_err();
        assert!(why.ends_with("1_s3.jsonl does not start as a Pi session does"));
    }

    #[test]
    fn follows_the_branch_pi_carries_on_from() {
        let root = tempfile::tempdir().unwrap();
        // You went back to the start and wrote something else. Then you put
        // a label on the answer and gave the session a name, as Pi's `/tree`
        // and `/name` do: those lines are the last ones, and say nothing.
        let lines = [
            first("s1", CWD),
            user("a1", None, "Name a fruit"),
            answer("a2", "a1", "Apple."),
            user("a3", Some("a2"), "Another"),
            answer("a4", "a3", "Pear."),
            user("b3", Some("a2"), "A red one"),
            answer("b4", "b3", "Cherry."),
            json!({ "type": "label", "id": "b5", "parentId": "b4", "targetId": "b4", "label": "mark" }),
            json!({ "type": "session_info", "id": "b6", "parentId": "b5", "name": "Fruit" }),
        ];
        write(root.path(), CWD, "s1", &lines);
        assert_eq!(
            told(&opened(root.path(), "s1")),
            [
                "you: Name a fruit",
                "text: Apple.",
                "you: A red one",
                "text: Cherry."
            ]
        );
    }

    #[test]
    fn says_why_a_reply_failed_or_stopped() {
        let root = tempfile::tempdir().unwrap();
        // As Pi wrote them: a reply the provider refused, and one you stopped.
        let refused = json!({
            "role": "assistant", "content": [], "stopReason": "error",
            "errorMessage": "401 {\"type\":\"error\",\"error\":{\"type\":\"authentication_error\"}}",
        });
        let stopped = json!({
            "role": "assistant", "content": [], "stopReason": "aborted",
            "errorMessage": "This operation was aborted",
        });
        let lines = [
            first("s1", CWD),
            user("a1", None, "Say ok"),
            said("a2", Some("a1"), refused),
            user("a3", Some("a2"), "Run sleep 30"),
            assistant(
                "a4",
                "a3",
                json!([call("t1", "bash", json!({ "command": "sleep 30" }))]),
            ),
            result("a5", "a4", "t1", true, "Command aborted"),
            said("a6", Some("a5"), stopped),
        ];
        write(root.path(), CWD, "s1", &lines);
        assert_eq!(
            told(&opened(root.path(), "s1")),
            [
                "you: Say ok",
                "Failed: 401 {\"type\":\"error\",\"error\":{\"type\":\"authentication_error\"}}",
                "you: Run sleep 30",
                "tool: bash sleep 30",
                "Warning: Stopped",
            ]
        );
    }

    #[test]
    fn shows_a_command_you_ran_and_where_pi_compacted() {
        let root = tempfile::tempdir().unwrap();
        // As Pi wrote them after `/compact` and `!echo from-bang`.
        let compaction = json!({
            "type": "compaction", "id": "a3", "parentId": "a2", "summary": "## Goal\n- Say a word.",
            "firstKeptEntryId": "a2", "tokensBefore": 3462, "fromHook": false,
            "systemMessage": { "role": "system", "content": "", "sections": {} },
        });
        let ran = json!({
            "role": "bashExecution", "command": "echo from-bang", "output": "from-bang\n",
            "exitCode": 0, "cancelled": false, "truncated": false,
        });
        let lines = [
            first("s1", CWD),
            user("a1", None, "Say alpha"),
            answer("a2", "a1", "alpha"),
            compaction,
            said("a4", Some("a3"), ran),
        ];
        write(root.path(), CWD, "s1", &lines);
        let view = opened(root.path(), "s1");
        assert_eq!(
            told(&view),
            [
                "you: Say alpha",
                "text: alpha",
                "Info: Compacted the conversation",
                "you: !echo from-bang",
                "tool: bash echo from-bang",
            ]
        );
        let Entry::Tool(tool) = &view.entries[4] else {
            panic!("a tool");
        };
        assert_eq!(
            (tool.status, tool.output.as_str()),
            (ToolStatus::Done, "from-bang\n")
        );
    }

    #[test]
    fn mends_half_a_character() {
        // A whole pair, and an escaped backslash before a `u`, stay.
        let good = r#"{"a":"\ud83d\ude00 \\ud83d \n \u00e9"}"#;
        assert_eq!(mended(good), good);
        assert_eq!(
            mended(r#"{"a":"x\ud83d","b":"\ude00y","c":"\ud83d\ud83d\ude00"}"#),
            r#"{"a":"x\ufffd","b":"\ufffdy","c":"\ufffd\ud83d\ude00"}"#
        );
        // A line that ends inside an escape is left as it is.
        assert_eq!(mended(r#"{"a":"\ud8"#), r#"{"a":"\ud8"#);
    }

    #[test]
    fn reads_a_line_that_holds_half_a_character() {
        let root = tempfile::tempdir().unwrap();
        // The first message is as Pi wrote it when half a character came in
        // what you sent. It is not good JSON text, and everything follows it.
        let odd = r#"{"type":"message","id":"a1","parentId":null,"timestamp":"2026-11-19T12:44:31.192Z","message":{"role":"user","content":[{"type":"text","text":"Say ok. Ignore this odd character: \ud83d"}],"timestamp":1795092271190}}"#;
        let text = format!(
            "{}\n{odd}\n{}\n{}\n{}\n",
            first("s1", CWD),
            answer("a2", "a1", "ok"),
            user("a3", Some("a2"), "Say done."),
            answer("a4", "a3", "done"),
        );
        write_text(root.path(), CWD, "s1", &text);
        let listed = list(root.path(), CWD);
        assert_eq!(
            listed[0].title,
            "Say ok. Ignore this odd character: \u{fffd}"
        );
        assert_eq!(
            told(&opened(root.path(), "s1")),
            [
                "you: Say ok. Ignore this odd character: \u{fffd}",
                "text: ok",
                "you: Say done.",
                "text: done",
            ]
        );
    }

    #[test]
    fn keeps_what_follows_a_message_it_cannot_read() {
        let root = tempfile::tempdir().unwrap();
        // A message nested deeper than is read. Its line still has its place.
        let deep = format!("{}1{}", "[".repeat(300), "]".repeat(300));
        let odd = format!(
            r#"{{"type":"message","id":"a2","parentId":"a1","message":{{"role":"assistant","content":{deep}}}}}"#
        );
        let text = format!(
            "{}\n{}\n{odd}\n{}\n{}\n",
            first("s1", CWD),
            user("a1", None, "Say ok"),
            user("a3", Some("a2"), "Say done."),
            answer("a4", "a3", "done"),
        );
        write_text(root.path(), CWD, "s1", &text);
        assert_eq!(
            told(&opened(root.path(), "s1")),
            [
                "you: Say ok",
                "Warning: A message here could not be read.",
                "you: Say done.",
                "text: done",
            ]
        );
    }

    #[test]
    fn says_when_what_came_before_is_missing() {
        let root = tempfile::tempdir().unwrap();
        // The line `a2` was cut short as it was written, so nothing leads
        // back from `a3`. Pi itself then sees the session from `a3` on.
        let text = format!(
            "{}\n{}\n{}\n{}\n{}\n",
            first("s1", CWD),
            user("a1", None, "Name a fruit"),
            r#"{"type":"message","id":"a2","parentId":"a1","message":{"role":"assi"#,
            user("a3", Some("a2"), "Another"),
            answer("a4", "a3", "Pear."),
        );
        write_text(root.path(), CWD, "s1", &text);
        let view = opened(root.path(), "s1");
        assert_eq!(view.state.title, "Name a fruit");
        assert_eq!(
            told(&view),
            [
                "Warning: What came before this is missing from Pi's session file.",
                "you: Another",
                "text: Pear.",
            ]
        );
    }

    #[test]
    fn says_when_nothing_is_on_the_branch() {
        let root = tempfile::tempdir().unwrap();
        // The last line starts again from before anything was said.
        let lines = [
            first("s1", CWD),
            user("a1", None, "Name a fruit"),
            answer("a2", "a1", "Apple."),
            json!({ "type": "model_change", "id": "b1", "parentId": null, "modelId": "haiku" }),
        ];
        write(root.path(), CWD, "s1", &lines);
        assert_eq!(
            told(&opened(root.path(), "s1")),
            ["Info: Nothing was said on the branch of this session that Pi carries on from."]
        );
    }

    #[test]
    fn reads_a_session_from_an_old_pi_and_a_text_for_content() {
        let root = tempfile::tempdir().unwrap();
        // Version 1: no ids, and what was said is a text, not a list of parts.
        let text = concat!(
            r#"{"type":"session","id":"s1","cwd":"/home/max/code/shop"}"#,
            "\n",
            r#"{"type":"message","message":{"role":"user","content":"Name a fruit"}}"#,
            "\n",
            r#"{"type":"message","message":{"role":"assistant","content":"Apple."}}"#,
            "\n",
        );
        write_text(root.path(), CWD, "s1", text);
        assert_eq!(
            told(&opened(root.path(), "s1")),
            ["you: Name a fruit", "text: Apple."]
        );
    }

    #[test]
    fn opens_what_is_listed_whatever_its_file_is_called() {
        let root = tempfile::tempdir().unwrap();
        let path = write(root.path(), CWD, "s1", &session_lines("s1"));
        // A copy under another name is listed by the id in its first line.
        let copy = path.with_file_name("copy-of-it.jsonl");
        std::fs::rename(&path, &copy).unwrap();
        assert_eq!(list(root.path(), CWD)[0].id, "s1");
        assert_eq!(opened(root.path(), "s1").entries.len(), 7);
        assert_eq!(
            open(root.path(), "s1", Some(CWD))
                .unwrap()
                .unwrap()
                .entries
                .len(),
            7
        );
        // An id that Pi lets you choose, with a dot and a line under it.
        let own = "my_own.id-1";
        write(
            root.path(),
            CWD,
            own,
            &[first(own, CWD), user("a1", None, "Say ok")],
        );
        assert_eq!(told(&opened(root.path(), own)), ["you: Say ok"]);
        // It ends another id, and is not that session.
        assert_eq!(open(root.path(), "own.id-1", Some(CWD)), Ok(None));
    }
}
