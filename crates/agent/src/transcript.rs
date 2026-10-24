//! A live session as wb-agent keeps it: Pi's events applied to the
//! session's state and entries, remembering what changed since the
//! workbench last heard, so it's sent only that.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::path::Path;

use protocol::{Change, Entry, Queued, Session, SessionState, StatusLine, Tone, Tool, ToolStatus};
use serde_json::Value;

use crate::messages;
use crate::pi::{Block, PiEvent};

pub struct Transcript {
    pub state: SessionState,
    pub entries: Vec<Entry>,
    /// The name Pi keeps for the session, if it has one.
    name: Option<String>,
    /// Prompts sent, oldest first, until Pi says what it did with each:
    /// their text, and where they'd go in the conversation.
    prompts: VecDeque<(String, usize)>,
    /// The content blocks of the assistant message being written, by
    /// their index in it, to the entries showing them.
    blocks: HashMap<u64, usize>,
    changed: BTreeSet<usize>,
    state_changed: bool,
    /// The entries were replaced as a whole.
    reloaded: bool,
}

impl Transcript {
    pub fn new(cwd: &Path) -> Self {
        Transcript {
            state: SessionState {
                title: untitled().into(),
                cwd: cwd.display().to_string(),
                ..SessionState::default()
            },
            entries: Vec::new(),
            name: None,
            prompts: VecDeque::new(),
            blocks: HashMap::new(),
            changed: BTreeSet::new(),
            state_changed: true,
            reloaded: false,
        }
    }

    pub fn session(&self, id: &str) -> Session {
        Session {
            id: id.to_string(),
            state: self.state.clone(),
            entries: self.entries.clone(),
        }
    }

    pub fn apply(&mut self, event: PiEvent) {
        // A question blocks the run, so a run moving on means it was
        // answered, or Pi stopped waiting.
        let moving = matches!(
            event,
            PiEvent::Block { .. }
                | PiEvent::MessageStart(_)
                | PiEvent::ToolStart { .. }
                | PiEvent::ToolEnd { .. }
                | PiEvent::AgentSettled
        );
        if moving && self.state.ask.is_some() {
            self.answered();
        }
        match event {
            PiEvent::AgentStart => {
                self.state.busy = true;
                self.state.stopped = None;
                self.state_changed = true;
            }
            PiEvent::AgentSettled => self.settled(),
            PiEvent::MessageStart(message) => match message["role"].as_str() {
                Some("user") => {
                    self.push(messages::user(&message));
                    self.retitle();
                }
                Some("assistant") => self.blocks.clear(),
                _ => {}
            },
            PiEvent::Block { index, update } => self.block(index, update),
            PiEvent::MessageEnd(message) => match message["role"].as_str() {
                Some("assistant") => self.assistant_end(&message),
                Some("user" | "toolResult") => {}
                _ => {
                    if let Some(entry) = messages::other(&message) {
                        self.push(entry);
                    }
                }
            },
            PiEvent::ToolStart { id, name, args } => {
                let index = self.tool_index(&id).unwrap_or_else(|| {
                    self.push(Entry::Tool(Tool {
                        id: id.clone(),
                        name: name.clone(),
                        args: Value::Null,
                        status: ToolStatus::Running,
                        output: String::new(),
                        images: Vec::new(),
                    }))
                });
                self.edit_tool(index, |tool| {
                    tool.args = args;
                    tool.status = ToolStatus::Running;
                });
            }
            PiEvent::ToolUpdate { id, partial } => {
                if let Some(index) = self.tool_index(&id) {
                    self.edit_tool(index, |tool| {
                        tool.output = messages::text(&partial["content"]);
                        tool.images = messages::images(&partial["content"]);
                    });
                }
            }
            PiEvent::ToolEnd { id, failed, result } => {
                if let Some(index) = self.tool_index(&id) {
                    self.edit_tool(index, |tool| messages::finish(tool, &result, failed));
                    if let Entry::Tool(tool) = &self.entries[index] {
                        let tool = tool.clone();
                        self.touched(&tool);
                    }
                }
            }
            PiEvent::Queue {
                steering,
                follow_up,
            } => {
                let now = steering
                    .into_iter()
                    .map(|text| Queued { text, later: false });
                let later = follow_up
                    .into_iter()
                    .map(|text| Queued { text, later: true });
                self.state.queued = now.chain(later).collect();
                self.state_changed = true;
            }
            PiEvent::Named(name) => {
                self.name = name;
                self.retitle();
            }
            PiEvent::ThinkingLevel(level) => {
                self.state.thinking = Some(level);
                self.state_changed = true;
            }
            PiEvent::Response {
                command,
                error,
                data,
            } => match error {
                Some(error) => {
                    if command == "prompt" {
                        self.prompts.pop_front();
                    }
                    self.notice(error, Tone::Error);
                }
                None => self.response(&command, &data),
            },
            PiEvent::Ask(ask) => {
                self.state.ask = Some(ask);
                self.state_changed = true;
            }
            PiEvent::Status { key, text } => {
                self.state.statuses.retain(|line| line.key != key);
                if let Some(text) = text {
                    self.state.statuses.push(StatusLine { key, text });
                }
                self.state_changed = true;
            }
            PiEvent::Notice { text, tone } => self.notice(text, tone),
            PiEvent::Other => {}
        }
    }

    /// A prompt was sent. Pi shows it as a message once a run takes it up;
    /// one an extension command handles never becomes one, so it's shown
    /// here instead.
    pub fn sent(&mut self, text: &str) {
        self.prompts
            .push_back((text.to_string(), self.entries.len()));
    }

    /// The question was answered, or Pi gave up waiting.
    pub fn answered(&mut self) {
        self.state.ask = None;
        self.state_changed = true;
    }

    pub fn exited(&mut self, reason: String) {
        self.settled();
        self.state.ask = None;
        self.state.stopped = Some(reason);
    }

    /// What changed since this was last asked, oldest first.
    pub fn changes(&mut self) -> Vec<Change> {
        let mut changes = Vec::new();
        if self.reloaded {
            changes.push(Change::Entries {
                entries: self.entries.clone(),
            });
        } else {
            changes.extend(self.changed.iter().map(|&index| Change::Entry {
                index: index as u32,
                entry: Box::new(self.entries[index].clone()),
            }));
        }
        if self.state_changed {
            changes.push(Change::State {
                state: Box::new(self.state.clone()),
            });
        }
        self.changed.clear();
        self.state_changed = false;
        self.reloaded = false;
        changes
    }

    fn settled(&mut self) {
        self.state.busy = false;
        self.state_changed = true;
        // A call still open won't run now: the run was stopped first.
        for index in 0..self.entries.len() {
            if let Entry::Tool(tool) = &mut self.entries[index]
                && matches!(tool.status, ToolStatus::Writing | ToolStatus::Running)
            {
                tool.status = ToolStatus::Failed;
                self.changed.insert(index);
            }
        }
    }

    fn block(&mut self, index: u64, update: Block) {
        match update {
            Block::TextStart => {
                self.start_block(
                    index,
                    Entry::Text {
                        text: String::new(),
                    },
                );
            }
            Block::ThinkingStart => {
                self.start_block(
                    index,
                    Entry::Thinking {
                        text: String::new(),
                    },
                );
            }
            Block::TextDelta(delta) | Block::ThinkingDelta(delta) => {
                let at = match self.blocks.get(&index) {
                    Some(&at) => at,
                    None => self.start_block(
                        index,
                        Entry::Text {
                            text: String::new(),
                        },
                    ),
                };
                if let Entry::Text { text } | Entry::Thinking { text } = &mut self.entries[at] {
                    text.push_str(&delta);
                    self.changed.insert(at);
                }
            }
            Block::TextEnd(content) => self.end_block(index, Entry::Text { text: content }),
            Block::ThinkingEnd(content) => self.end_block(index, Entry::Thinking { text: content }),
            Block::ToolStart { id, name } => {
                self.start_block(
                    index,
                    Entry::Tool(Tool {
                        id,
                        name,
                        args: Value::Null,
                        status: ToolStatus::Writing,
                        output: String::new(),
                        images: Vec::new(),
                    }),
                );
            }
            Block::ToolEnd(call) => {
                if let Some(&at) = self.blocks.get(&index) {
                    self.edit_tool(at, |tool| tool.args = call["arguments"].clone());
                }
            }
            Block::Other => {}
        }
    }

    fn start_block(&mut self, index: u64, entry: Entry) -> usize {
        let at = self.push(entry);
        self.blocks.insert(index, at);
        at
    }

    fn end_block(&mut self, index: u64, entry: Entry) {
        match self.blocks.get(&index) {
            Some(&at) => self.set(at, entry),
            None => {
                self.start_block(index, entry);
            }
        }
    }

    /// The finished message replaces what was streamed of it.
    fn assistant_end(&mut self, message: &Value) {
        let content = message["content"].as_array().cloned().unwrap_or_default();
        for (index, block) in content.iter().enumerate() {
            let Some(entry) = messages::block(block) else {
                continue;
            };
            match (self.blocks.get(&(index as u64)).copied(), entry) {
                // A call keeps what's known of its run.
                (Some(at), Entry::Tool(done)) => self.edit_tool(at, |tool| {
                    tool.name = done.name;
                    tool.args = done.args;
                }),
                (Some(at), entry) => self.set(at, entry),
                (None, Entry::Tool(mut tool)) => {
                    tool.status = ToolStatus::Writing;
                    self.push(Entry::Tool(tool));
                }
                (None, entry) => {
                    self.push(entry);
                }
            }
        }
        if let Some(entry) = messages::ending(message) {
            self.push(entry);
        }
        self.blocks.clear();
    }

    fn response(&mut self, command: &str, data: &Value) {
        match command {
            "prompt" => {
                let Some((text, at)) = self.prompts.pop_front() else {
                    return;
                };
                if data["disposition"] == "handled" {
                    // Before what the command itself said.
                    let at = at.min(self.entries.len());
                    self.entries.insert(
                        at,
                        Entry::User {
                            text,
                            images: Vec::new(),
                        },
                    );
                    self.reloaded = true;
                    self.retitle();
                }
            }
            "get_state" => {
                let text = |key: &str| data[key].as_str().map(str::to_string);
                self.state.file = text("sessionFile");
                self.name = text("sessionName");
                self.state.thinking = text("thinkingLevel");
                self.state.model = match (
                    data["model"]["provider"].as_str(),
                    data["model"]["id"].as_str(),
                ) {
                    (Some(provider), Some(id)) => Some(format!("{provider}/{id}")),
                    _ => None,
                };
                self.state_changed = true;
                self.retitle();
            }
            "get_messages" => {
                let list = data["messages"].as_array().cloned().unwrap_or_default();
                self.entries = messages::entries(&list);
                self.changed.clear();
                self.reloaded = true;
                self.state.files.clear();
                let tools: Vec<Tool> = self
                    .entries
                    .iter()
                    .filter_map(|entry| match entry {
                        Entry::Tool(tool) => Some(tool.clone()),
                        _ => None,
                    })
                    .collect();
                for tool in &tools {
                    self.touched(tool);
                }
                self.retitle();
            }
            _ => {}
        }
    }

    fn notice(&mut self, text: String, tone: Tone) {
        if !text.trim().is_empty() {
            self.push(Entry::Notice { text, tone });
        }
    }

    /// Notes a file Pi wrote or edited.
    fn touched(&mut self, tool: &Tool) {
        if tool.status != ToolStatus::Done || !matches!(tool.name.as_str(), "write" | "edit") {
            return;
        }
        let Some(path) = tool.args["path"].as_str() else {
            return;
        };
        let path = Path::new(&self.state.cwd).join(path).display().to_string();
        self.state.files.retain(|known| *known != path);
        self.state.files.push(path);
        self.state_changed = true;
    }

    fn retitle(&mut self) {
        let first = self.entries.iter().find_map(|entry| match entry {
            Entry::User { text, .. } if !text.trim().is_empty() => Some(text.as_str()),
            _ => None,
        });
        let title = match (&self.name, first) {
            (Some(name), _) if !name.trim().is_empty() => name.trim().to_string(),
            (_, Some(text)) => shorten(text),
            _ => untitled().into(),
        };
        if title != self.state.title {
            self.state.title = title;
            self.state_changed = true;
        }
    }

    fn push(&mut self, entry: Entry) -> usize {
        self.entries.push(entry);
        let index = self.entries.len() - 1;
        self.changed.insert(index);
        index
    }

    fn set(&mut self, index: usize, entry: Entry) {
        self.entries[index] = entry;
        self.changed.insert(index);
    }

    fn tool_index(&self, id: &str) -> Option<usize> {
        self.entries
            .iter()
            .rposition(|entry| matches!(entry, Entry::Tool(tool) if tool.id == id))
    }

    fn edit_tool(&mut self, index: usize, edit: impl FnOnce(&mut Tool)) {
        if let Entry::Tool(tool) = &mut self.entries[index] {
            edit(tool);
            self.changed.insert(index);
        }
    }
}

fn untitled() -> &'static str {
    "New scratchpad"
}

/// The first line of a prompt, cut to a title's length.
pub fn shorten(text: &str) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .trim();
    match line.char_indices().nth(60) {
        Some((cut, _)) => format!("{}…", line[..cut].trim_end()),
        None => line.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pi::decode;

    fn run(transcript: &mut Transcript, lines: &[&str]) {
        for line in lines {
            transcript.apply(decode(line).expect("a JSON line"));
        }
    }

    #[test]
    fn follows_a_run_with_a_tool_call() {
        let mut t = Transcript::new(Path::new("/scratch/1"));
        t.changes();
        run(
            &mut t,
            &[
                r#"{"type":"agent_start"}"#,
                r#"{"type":"message_start","message":{"role":"user","content":"make hello.txt"}}"#,
                r#"{"type":"message_start","message":{"role":"assistant","content":[]}}"#,
                r#"{"type":"message_update","assistantMessageEvent":{"type":"text_start","contentIndex":0}}"#,
                r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"On "}}"#,
                r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"it."}}"#,
                r#"{"type":"message_update","assistantMessageEvent":{"type":"toolcall_start","contentIndex":1,"id":"c1","toolName":"write"}}"#,
            ],
        );
        assert_eq!(t.state.title, "make hello.txt");
        assert!(matches!(&t.entries[1], Entry::Text { text } if text == "On it."));
        assert!(matches!(&t.entries[2], Entry::Tool(tool) if tool.status == ToolStatus::Writing));
        let changes = t.changes();
        assert_eq!(changes.len(), 4, "three entries and the state");

        run(
            &mut t,
            &[
                r#"{"type":"message_end","message":{"role":"assistant","stopReason":"toolUse","content":[{"type":"text","text":"On it."},{"type":"toolCall","id":"c1","name":"write","arguments":{"path":"hello.txt","content":"hi"}}]}}"#,
                r#"{"type":"tool_execution_start","toolCallId":"c1","toolName":"write","args":{"path":"hello.txt","content":"hi"}}"#,
                r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"write","isError":false,"result":{"content":[{"type":"text","text":"Wrote it"}]}}"#,
                r#"{"type":"agent_settled","aborted":false}"#,
            ],
        );
        assert_eq!(
            t.entries.len(),
            3,
            "the finished message replaces what streamed"
        );
        let Entry::Tool(tool) = &t.entries[2] else {
            panic!("expected the tool call")
        };
        assert_eq!(tool.status, ToolStatus::Done);
        assert_eq!(tool.output, "Wrote it");
        assert_eq!(t.state.files, ["/scratch/1/hello.txt"]);
        assert!(!t.state.busy);
    }

    #[test]
    fn a_stopped_run_fails_its_open_calls() {
        let mut t = Transcript::new(Path::new("/s"));
        run(
            &mut t,
            &[
                r#"{"type":"agent_start"}"#,
                r#"{"type":"tool_execution_start","toolCallId":"c1","toolName":"bash","args":{"command":"sleep 9"}}"#,
                r#"{"type":"agent_settled","aborted":true}"#,
            ],
        );
        assert!(matches!(&t.entries[0], Entry::Tool(tool) if tool.status == ToolStatus::Failed));
    }

    #[test]
    fn loads_a_saved_conversation() {
        let mut t = Transcript::new(Path::new("/s"));
        t.changes();
        run(
            &mut t,
            &[
                r#"{"type":"response","command":"get_state","success":true,"data":{"sessionFile":"/x.jsonl","sessionName":"Tidy up","thinkingLevel":"high","model":{"provider":"local","id":"qwen"}}}"#,
                r#"{"type":"response","command":"get_messages","success":true,"data":{"messages":[{"role":"user","content":"hi"},{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"Hello"}]}]}}"#,
            ],
        );
        assert_eq!(t.state.title, "Tidy up");
        assert_eq!(t.state.model.as_deref(), Some("local/qwen"));
        let changes = t.changes();
        assert!(matches!(&changes[0], Change::Entries { entries } if entries.len() == 2));
    }

    #[test]
    fn refusals_and_questions_show() {
        let mut t = Transcript::new(Path::new("/s"));
        run(
            &mut t,
            &[
                r#"{"type":"response","command":"prompt","success":false,"error":"No model selected"}"#,
                r#"{"type":"extension_ui_request","id":"u1","method":"confirm","title":"Delete it?"}"#,
            ],
        );
        assert!(matches!(
            &t.entries[0],
            Entry::Notice {
                tone: Tone::Error,
                ..
            }
        ));
        assert!(t.state.ask.is_some());
        t.answered();
        assert!(t.state.ask.is_none());
    }

    #[test]
    fn a_handled_command_shows_as_asked() {
        let mut t = Transcript::new(Path::new("/s"));
        t.sent("/rpc-input");
        t.sent("hello");
        run(
            &mut t,
            &[
                r#"{"type":"extension_ui_request","id":"u1","method":"notify","message":"You entered: hi"}"#,
                r#"{"type":"response","command":"prompt","success":true,"data":{"disposition":"handled"}}"#,
                r#"{"type":"response","command":"prompt","success":true,"data":{"disposition":"started"}}"#,
            ],
        );
        assert_eq!(
            t.entries.len(),
            2,
            "a started prompt shows when Pi takes it up"
        );
        assert!(matches!(&t.entries[0], Entry::User { text, .. } if text == "/rpc-input"));
        assert_eq!(t.state.title, "/rpc-input");
    }

    #[test]
    fn shortens_titles() {
        assert_eq!(shorten("\n  fix the build  \nthen more"), "fix the build");
        assert_eq!(shorten(&"a".repeat(80)).chars().count(), 61);
    }
}
