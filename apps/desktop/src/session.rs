//! One Pi session: its process, its timeline, and what it is waiting for.
use crate::documents::Documents;
use floem::{
    action::exec_after,
    ext_event::{ExtSendTrigger, register_ext_trigger},
    prelude::*,
    reactive::{Effect, ReadSignal, Scope},
    views::editor::{
        core::{cursor::CursorAffinity, editor::EditType, selection::Selection},
        text::Document,
        text_document::TextDocument,
    },
};
use pi_client::{
    Answer, Ask, Event, Launch, PiProcess, events::result_text, preview::file_preview,
    sessions::SessionSummary,
};
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    path::PathBuf,
    rc::{Rc, Weak},
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    User,
    Assistant,
    Thinking,
    Tool,
    /// An extension's question, answered inline.
    Ask,
    Notice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolState {
    /// The model is still generating the arguments.
    Streaming,
    Running,
    Done,
    Failed,
}

/// One item in the timeline. Fields update in place while streaming.
pub struct Entry {
    pub id: u64,
    pub kind: EntryKind,
    /// Tool name for tool entries; `steer` or `follow-up` for queued prompts.
    pub tool: String,
    /// The message, or a tool's target file or command.
    pub title: RwSignal<String>,
    pub body: RwSignal<String>,
    /// Tool progress; `Failed` also marks error notices.
    pub state: RwSignal<Option<ToolState>>,
    pub ask: Option<Ask>,
    /// The answer given to an `Ask`, once answered.
    pub answered: RwSignal<Option<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Idle,
    Working,
    /// An extension is waiting for an answer.
    NeedsInput,
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Model {
    pub provider: String,
    pub id: String,
    pub name: String,
}

impl Model {
    fn from_json(value: &Value) -> Option<Self> {
        let id = value["id"].as_str()?.to_string();
        Some(Self {
            provider: value["provider"].as_str().unwrap_or_default().into(),
            name: value["name"].as_str().unwrap_or(&id).into(),
            id,
        })
    }
}

/// What a session reports back to the workspace.
pub struct Hooks {
    /// Pi changed files and no tool is running; `true` if files may have
    /// been created or deleted.
    pub files_changed: Box<dyn Fn(bool)>,
    /// A run finished; the session list may have changed.
    pub settled: Box<dyn Fn()>,
}

/// Tools whose effects on disk the workbench cannot predict from their arguments.
fn may_create_files(tool: &str) -> bool {
    matches!(tool, "write" | "bash" | "powershell")
}

/// The file or command a tool call acts on, from final or streaming arguments.
fn tool_target(args: &Value, partial: &str) -> String {
    args["command"]
        .as_str()
        .or(args["path"].as_str())
        .map(String::from)
        .or_else(|| file_preview(partial).path)
        .unwrap_or_default()
}

/// A readable view of an edit or write while its arguments stream in.
fn tool_preview(tool: &str, partial: &str) -> String {
    let preview = file_preview(partial);
    let mark = |prefix: char, text: &str| {
        text.lines()
            .map(|line| format!("{prefix} {line}\n"))
            .collect::<String>()
    };
    match tool {
        "edit" => preview
            .edits
            .iter()
            .map(|(old, new)| format!("{}{}", mark('-', old), mark('+', new)))
            .collect::<Vec<_>>()
            .join("···\n"),
        "write" => mark('+', &preview.content.unwrap_or_default()),
        _ => String::new(),
    }
}

/// The same view for complete arguments, as stored in session history.
fn args_preview(tool: &str, args: &Value) -> String {
    let mark = |prefix: char, text: &str| {
        text.lines()
            .map(|line| {
                format!(
                    "{prefix} {line}
"
                )
            })
            .collect::<String>()
    };
    let text = |value: &Value| value.as_str().unwrap_or_default().to_string();
    match tool {
        "edit" => {
            // Older sessions stored a single oldText/newText pair at the top level.
            let edits = match args["edits"].as_array() {
                Some(edits) => edits.clone(),
                None => vec![args.clone()],
            };
            edits
                .iter()
                .map(|e| {
                    format!(
                        "{}{}",
                        mark('-', &text(&e["oldText"])),
                        mark('+', &text(&e["newText"]))
                    )
                })
                .collect::<Vec<_>>()
                .join(
                    "···
",
                )
        }
        "write" => mark('+', &text(&args["content"])),
        _ => String::new(),
    }
}

fn is_file_tool(tool: &str) -> bool {
    matches!(tool, "edit" | "write")
}

pub fn shorten(text: &str) -> String {
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    match line.char_indices().nth(48) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_string(),
    }
}

fn stats_label(data: &Value) -> String {
    let tokens = data["tokens"]["total"].as_f64().unwrap_or_default();
    let tokens = if tokens >= 1000.0 {
        format!("{:.1}k tokens", tokens / 1000.0)
    } else {
        format!("{tokens} tokens")
    };
    let cost = data["cost"].as_f64().unwrap_or_default();
    match data["contextUsage"]["percent"].as_f64() {
        Some(percent) => format!("{tokens} · ${cost:.2} · {percent:.0}% context"),
        None => format!("{tokens} · ${cost:.2}"),
    }
}

struct Streaming {
    tool: String,
    args: String,
    entry: Rc<Entry>,
}

pub struct Session {
    /// Stable within this window; Pi's own id is only known once started.
    pub key: u64,
    scope: Scope,
    root: PathBuf,
    launch: Launch,
    documents: Rc<Documents>,
    hooks: Rc<Hooks>,
    file: RwSignal<Option<PathBuf>>,
    title: RwSignal<String>,
    working: RwSignal<bool>,
    pending_asks: RwSignal<usize>,
    stopped: RwSignal<bool>,
    model: RwSignal<Option<Model>>,
    models: RwSignal<Vec<Model>>,
    stats: RwSignal<String>,
    /// The composer's text, kept per session.
    pub draft: Rc<TextDocument>,
    /// Pi runs tools in parallel; reading files while one is still writing
    /// could pick up a half-written file, so syncing waits for all of them.
    running_tools: Cell<usize>,
    tree_changed: Cell<bool>,
    /// Whether this session holds the workspace's saves for a run.
    holding: Cell<bool>,
    process: RefCell<Option<PiProcess>>,
    entries: RwSignal<Vec<Rc<Entry>>>,
    next_entry: Cell<u64>,
    /// Tool calls by content index while their arguments stream.
    streaming: RefCell<HashMap<u64, String>>,
    tools: RefCell<HashMap<String, Streaming>>,
    reply: RefCell<Option<Rc<Entry>>>,
    thinking: RefCell<Option<Rc<Entry>>>,
    queue: Arc<Mutex<VecDeque<Event>>>,
    trigger: ExtSendTrigger,
}

impl Session {
    /// `resume` continues a session from Pi's store; otherwise Pi starts a new one.
    pub fn new(
        scope: Scope,
        key: u64,
        root: PathBuf,
        launch: Launch,
        documents: Rc<Documents>,
        hooks: Rc<Hooks>,
        resume: Option<&SessionSummary>,
    ) -> Rc<Self> {
        let scope = scope.create_child();
        let session = scope.enter(|| {
            Rc::new(Self {
                key,
                scope,
                root,
                launch,
                documents,
                hooks,
                file: RwSignal::new(resume.map(|s| s.path.clone())),
                title: RwSignal::new(resume.map_or("New session".into(), |s| s.title.clone())),
                working: RwSignal::new(false),
                pending_asks: RwSignal::new(0),
                stopped: RwSignal::new(false),
                model: RwSignal::new(None),
                models: RwSignal::new(Vec::new()),
                stats: RwSignal::new(String::new()),
                draft: Rc::new(TextDocument::new(scope, "")),
                running_tools: Cell::new(0),
                tree_changed: Cell::new(false),
                holding: Cell::new(false),
                process: RefCell::new(None),
                entries: RwSignal::new(Vec::new()),
                next_entry: Cell::new(0),
                streaming: RefCell::new(HashMap::new()),
                tools: RefCell::new(HashMap::new()),
                reply: RefCell::new(None),
                thinking: RefCell::new(None),
                queue: Arc::new(Mutex::new(VecDeque::new())),
                trigger: ExtSendTrigger::new(),
            })
        });
        // Pi's reader thread queues events; this drains them on the UI thread in order.
        let weak: Weak<Self> = Rc::downgrade(&session);
        let trigger = session.trigger;
        scope.enter(|| {
            Effect::new(move |_| {
                trigger.track();
                if let Some(session) = weak.upgrade() {
                    loop {
                        let next = session.queue.lock().unwrap().pop_front();
                        match next {
                            Some(event) => session.handle(event),
                            None => break,
                        }
                    }
                }
            })
        });
        session
    }

    pub fn file(&self) -> ReadSignal<Option<PathBuf>> {
        self.file.read_only()
    }
    pub fn title(&self) -> ReadSignal<String> {
        self.title.read_only()
    }
    pub fn entries(&self) -> ReadSignal<Vec<Rc<Entry>>> {
        self.entries.read_only()
    }
    pub fn working(&self) -> ReadSignal<bool> {
        self.working.read_only()
    }
    pub fn model(&self) -> ReadSignal<Option<Model>> {
        self.model.read_only()
    }
    pub fn models(&self) -> ReadSignal<Vec<Model>> {
        self.models.read_only()
    }
    pub fn stats(&self) -> ReadSignal<String> {
        self.stats.read_only()
    }

    /// Tracked: what the rail and status bar show.
    pub fn status(&self) -> Status {
        if self.pending_asks.get() > 0 {
            Status::NeedsInput
        } else if self.working.get() {
            Status::Working
        } else if self.stopped.get() {
            Status::Stopped
        } else {
            Status::Idle
        }
    }

    fn push(&self, kind: EntryKind, tool: &str, title: String, body: String) -> Rc<Entry> {
        self.push_entry(kind, tool, title, body, None)
    }

    fn push_entry(
        &self,
        kind: EntryKind,
        tool: &str,
        title: String,
        body: String,
        ask: Option<Ask>,
    ) -> Rc<Entry> {
        let id = self.next_entry.get();
        self.next_entry.set(id + 1);
        let entry = self.scope.enter(|| {
            Rc::new(Entry {
                id,
                kind,
                tool: tool.into(),
                title: RwSignal::new(title),
                body: RwSignal::new(body),
                state: RwSignal::new(None),
                ask,
                answered: RwSignal::new(None),
            })
        });
        self.entries.update(|entries| entries.push(entry.clone()));
        entry
    }

    fn notice(&self, message: impl Into<String>, error: bool) {
        let entry = self.push(EntryKind::Notice, "", message.into(), String::new());
        if error {
            entry.state.set(Some(ToolState::Failed));
        }
    }

    /// Starts Pi if needed and asks for the state the header shows.
    pub fn start(&self) -> anyhow::Result<()> {
        if self.process.borrow().is_some() {
            return Ok(());
        }
        let queue = self.queue.clone();
        let trigger = self.trigger;
        let resume = self.file.get_untracked();
        let mut process =
            PiProcess::spawn(&self.launch, &self.root, resume.as_deref(), move |event| {
                queue.lock().unwrap().push_back(event);
                register_ext_trigger(trigger);
            })
            .map_err(|err| {
                err.context("Is Pi installed? npm install -g @earendil-works/pi-coding-agent")
            })?;
        self.stopped.set(false);
        if resume.is_some() {
            process.request("get_messages")?;
            process.request("get_session_stats")?;
        }
        process.request("get_state")?;
        process.request("get_available_models")?;
        *self.process.borrow_mut() = Some(process);
        Ok(())
    }

    fn with_process(&self, act: impl FnOnce(&mut PiProcess) -> anyhow::Result<()>) {
        let result = match self.process.borrow_mut().as_mut() {
            Some(pi) => act(pi),
            None => Err(anyhow::anyhow!("Pi is not running")),
        };
        if let Err(err) = result {
            self.notice(format!("{err:#}"), true);
        }
    }

    /// Submits the composer. While Pi works, the text steers the run, or
    /// waits for it to finish with `follow_up`. Returns whether it was sent.
    pub fn submit(self: &Rc<Self>, text: String, follow_up: bool) -> bool {
        let text = text.trim().to_string();
        if text.is_empty() {
            return false;
        }
        if self.working.get_untracked() {
            let tool = if follow_up { "follow-up" } else { "steer" };
            self.push(EntryKind::User, tool, text.clone(), String::new());
            self.with_process(|pi| {
                if follow_up {
                    pi.follow_up(&text)
                } else {
                    pi.steer(&text)
                }
            });
            return true;
        }
        if self
            .entries
            .with_untracked(|e| !e.iter().any(|e| e.kind == EntryKind::User))
        {
            self.title.set(shorten(&text));
        }
        self.push(EntryKind::User, "", text.clone(), String::new());
        self.working.set(true);
        self.documents.save_all();
        self.prompt_when_saved(text);
        true
    }

    fn prompt_when_saved(self: &Rc<Self>, text: String) {
        if self.documents.any_saving() {
            let session = self.clone();
            exec_after(Duration::from_millis(30), move |_| {
                session.prompt_when_saved(text)
            });
            return;
        }
        // Saves during the run would race Pi's writes; they wait for it and
        // the edits are merged with Pi's instead.
        self.documents.hold_saves();
        self.holding.set(true);
        let result = self.start().and_then(|()| {
            self.process
                .borrow_mut()
                .as_mut()
                .map_or(Ok(()), |pi| pi.prompt(&text))
        });
        if let Err(err) = result {
            self.finish_run();
            self.notice(format!("{err:#}"), true);
        }
    }

    pub fn stop(&self) {
        if self.working.get_untracked() {
            self.with_process(PiProcess::abort);
        }
    }

    pub fn set_model(&self, model: &Model) {
        self.with_process(|pi| pi.set_model(&model.provider, &model.id));
    }

    pub fn answer(&self, entry: &Entry, answer: Answer, label: String) {
        let Some(ask) = &entry.ask else { return };
        if entry.answered.get_untracked().is_some() {
            return;
        }
        self.with_process(|pi| pi.answer(&ask.id, answer));
        entry.answered.set(Some(label));
        self.pending_asks.update(|n| *n = n.saturating_sub(1));
    }

    /// Ends a run however it stopped: releases held saves and syncs files.
    fn finish_run(&self) {
        self.working.set(false);
        *self.reply.borrow_mut() = None;
        *self.thinking.borrow_mut() = None;
        self.streaming.borrow_mut().clear();
        self.tools.borrow_mut().clear();
        self.running_tools.set(0);
        self.tree_changed.set(false);
        if self.holding.replace(false) {
            self.documents.release_saves();
        }
        (self.hooks.files_changed)(true);
        (self.hooks.settled)();
    }

    fn tool_entry(&self, id: &str, tool: &str) -> Rc<Entry> {
        let mut tools = self.tools.borrow_mut();
        tools
            .entry(id.to_string())
            .or_insert_with(|| Streaming {
                tool: tool.into(),
                args: String::new(),
                entry: self.push(EntryKind::Tool, tool, String::new(), String::new()),
            })
            .entry
            .clone()
    }

    fn end_tool(&self, entry: &Entry, tool: &str, is_error: bool, output: String) {
        entry.state.set(Some(if is_error {
            ToolState::Failed
        } else {
            ToolState::Done
        }));
        // File tools already show their change; others show their output.
        if is_error || !is_file_tool(tool) {
            entry.body.set(output);
        }
    }

    /// Rebuilds the timeline of a resumed session from Pi's messages.
    fn load_history(&self, messages: &[Value]) {
        if self.entries.with_untracked(|e| !e.is_empty()) {
            return;
        }
        for message in messages {
            match message["role"].as_str().unwrap_or_default() {
                "user" => {
                    let text = match &message["content"] {
                        Value::String(text) => text.clone(),
                        content => result_text(&serde_json::json!({ "content": content })),
                    };
                    self.push(EntryKind::User, "", text, String::new());
                }
                "assistant" => {
                    for block in message["content"].as_array().into_iter().flatten() {
                        match block["type"].as_str().unwrap_or_default() {
                            "text" => {
                                let text = block["text"].as_str().unwrap_or_default();
                                self.push(EntryKind::Assistant, "", String::new(), text.into());
                            }
                            "thinking" => {
                                let text = block["thinking"].as_str().unwrap_or_default();
                                if !text.trim().is_empty() {
                                    self.push(EntryKind::Thinking, "", String::new(), text.into());
                                }
                            }
                            "toolCall" => {
                                let tool = block["name"].as_str().unwrap_or_default();
                                let id = block["id"].as_str().unwrap_or_default();
                                let args = &block["arguments"];
                                let entry = self.tool_entry(id, tool);
                                entry.title.set(tool_target(args, ""));
                                entry.body.set(args_preview(tool, args));
                                entry.state.set(Some(ToolState::Running));
                            }
                            _ => {}
                        }
                    }
                    if message["stopReason"] == "error" {
                        let error = message["errorMessage"].as_str().unwrap_or("Model error");
                        self.notice(error, true);
                    }
                }
                "toolResult" => {
                    let id = message["toolCallId"].as_str().unwrap_or_default();
                    let tool = message["toolName"].as_str().unwrap_or_default();
                    let entry = self.tool_entry(id, tool);
                    let is_error = message["isError"].as_bool().unwrap_or_default();
                    self.end_tool(&entry, tool, is_error, result_text(message));
                }
                "bashExecution" => {
                    let command = message["command"].as_str().unwrap_or_default();
                    let output = message["output"].as_str().unwrap_or_default();
                    let entry = self.push(EntryKind::Tool, "bash", command.into(), output.into());
                    entry.state.set(Some(ToolState::Done));
                }
                "compactionSummary" | "branchSummary" => {
                    self.notice("Earlier conversation was summarised", false);
                }
                "custom" if message["display"] == true => {
                    self.notice(result_text(message), false);
                }
                _ => {}
            }
        }
        self.tools.borrow_mut().clear();
    }

    fn handle_response(&self, command: &str, data: Value) {
        match command {
            "get_messages" => {
                let messages = data["messages"].as_array().cloned().unwrap_or_default();
                self.load_history(&messages);
            }
            "get_state" => {
                if let Some(file) = data["sessionFile"].as_str() {
                    self.file.set(Some(file.into()));
                }
                if let Some(name) = data["sessionName"].as_str().filter(|n| !n.is_empty()) {
                    self.title.set(shorten(name));
                }
                self.model.set(Model::from_json(&data["model"]));
            }
            "get_available_models" => {
                let models = data["models"].as_array().into_iter().flatten();
                self.models
                    .set(models.filter_map(Model::from_json).collect());
            }
            "set_model" => self.model.set(Model::from_json(&data)),
            "get_session_stats" => self.stats.set(stats_label(&data)),
            _ => {}
        }
    }

    pub fn handle(&self, event: Event) {
        match event {
            Event::AgentStart => self.working.set(true),
            Event::TextDelta(delta) => {
                *self.thinking.borrow_mut() = None;
                let mut reply = self.reply.borrow_mut();
                let entry = reply.get_or_insert_with(|| {
                    self.push(EntryKind::Assistant, "", String::new(), String::new())
                });
                entry.body.update(|body| body.push_str(&delta));
            }
            Event::ThinkingDelta(delta) => {
                let mut thinking = self.thinking.borrow_mut();
                let entry = thinking.get_or_insert_with(|| {
                    self.push(EntryKind::Thinking, "", String::new(), String::new())
                });
                entry.body.update(|body| body.push_str(&delta));
            }
            Event::ToolCallStart { index, id, tool } => {
                *self.reply.borrow_mut() = None;
                *self.thinking.borrow_mut() = None;
                self.tool_entry(&id, &tool)
                    .state
                    .set(Some(ToolState::Streaming));
                self.streaming.borrow_mut().insert(index, id);
            }
            Event::ToolCallDelta { index, delta } => {
                let Some(id) = self.streaming.borrow().get(&index).cloned() else {
                    return;
                };
                if let Some(call) = self.tools.borrow_mut().get_mut(&id) {
                    call.args.push_str(&delta);
                    call.entry.title.set(tool_target(&Value::Null, &call.args));
                    call.entry.body.set(tool_preview(&call.tool, &call.args));
                }
            }
            Event::ToolStart { id, tool, args } => {
                self.running_tools.set(self.running_tools.get() + 1);
                let entry = self.tool_entry(&id, &tool);
                entry.title.set(tool_target(&args, ""));
                entry.state.set(Some(ToolState::Running));
            }
            Event::ToolEnd {
                id,
                tool,
                is_error,
                output,
            } => {
                let entry = self.tool_entry(&id, &tool);
                self.end_tool(&entry, &tool, is_error, output);
                self.running_tools
                    .set(self.running_tools.get().saturating_sub(1));
                if may_create_files(&tool) {
                    self.tree_changed.set(true);
                }
                if self.running_tools.get() == 0 {
                    (self.hooks.files_changed)(self.tree_changed.replace(false));
                }
            }
            // Also catches anything a tool changed after its end event.
            Event::AgentSettled => {
                self.finish_run();
                self.with_process(|pi| {
                    pi.request("get_session_stats")?;
                    pi.request("get_state")
                });
            }
            Event::Response {
                command,
                error: Some(error),
                ..
            } => {
                if command == "prompt" {
                    self.finish_run();
                }
                self.notice(format!("Pi rejected {command}: {error}"), true);
            }
            // An extension consumed the prompt; no run or settle event follows.
            Event::Response {
                command,
                handled: true,
                ..
            } if command == "prompt" => self.finish_run(),
            Event::Response { command, data, .. } => self.handle_response(&command, data),
            Event::Ask(ask) => {
                let (title, message) = (ask.title.clone(), ask.message.clone());
                self.push_entry(EntryKind::Ask, "", title, message, Some(ask));
                self.pending_asks.update(|n| *n += 1);
            }
            Event::Notice { message, error } => self.notice(message, error),
            Event::SetDraft(text) => {
                let len = self.draft.text().len();
                self.draft.edit_single(
                    Selection::region(0, len, CursorAffinity::Backward),
                    &text,
                    EditType::Other,
                );
            }
            Event::Exited(message) => {
                self.process.borrow_mut().take();
                self.pending_asks.set(0);
                self.finish_run();
                self.stopped.set(true);
                self.notice(format!("Pi stopped: {message}"), true);
            }
            Event::Other => {}
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.holding.replace(false) {
            self.documents.release_saves();
        }
        self.scope.dispose();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_client::AskKind;
    use serde_json::json;

    fn with_session(test: impl FnOnce(Rc<Session>, Rc<RefCell<Vec<bool>>>)) {
        let scope = Scope::new();
        scope.enter(|| {
            let status = RwSignal::new(String::new());
            let documents = Documents::new(scope, "/".into(), status);
            let changed = Rc::new(RefCell::new(Vec::new()));
            let log = changed.clone();
            let hooks = Rc::new(Hooks {
                files_changed: Box::new(move |tree| log.borrow_mut().push(tree)),
                settled: Box::new(|| {}),
            });
            let session = Session::new(
                scope,
                1,
                "/".into(),
                Launch::default(),
                documents,
                hooks,
                None,
            );
            test(session, changed);
        });
        scope.dispose();
    }

    #[test]
    fn streamed_edits_preview_live_and_sync_when_done() {
        with_session(|session, changed| {
            session.handle(Event::AgentStart);
            session.handle(Event::ThinkingDelta("Plan: rename".into()));
            session.handle(Event::ToolCallStart {
                index: 0,
                id: "c1".into(),
                tool: "edit".into(),
            });
            for delta in [
                r#"{"path":"src/a.rs","edits":[{"oldText":"hel"#,
                r#"lo","newText":"hello pi"}]}"#,
            ] {
                session.handle(Event::ToolCallDelta {
                    index: 0,
                    delta: delta.into(),
                });
            }
            let entries = session.entries().get();
            assert_eq!(entries[0].kind, EntryKind::Thinking);
            let tool = entries[1].clone();
            assert_eq!(tool.title.get(), "src/a.rs");
            assert_eq!(tool.body.get(), "- hello\n+ hello pi\n");
            assert_eq!(tool.state.get(), Some(ToolState::Streaming));

            session.handle(Event::ToolStart {
                id: "c1".into(),
                tool: "edit".into(),
                args: json!({"path": "src/a.rs"}),
            });
            session.handle(Event::ToolEnd {
                id: "c1".into(),
                tool: "edit".into(),
                is_error: false,
                output: "ok".into(),
            });
            assert_eq!(tool.state.get(), Some(ToolState::Done));
            assert_eq!(*changed.borrow(), [false]);

            session.handle(Event::TextDelta("Done.".into()));
            session.handle(Event::AgentSettled);
            assert_eq!(session.entries().get()[2].body.get(), "Done.");
            assert_eq!(session.status(), Status::Idle);
            assert_eq!(*changed.borrow(), [false, true]);
        });
    }

    #[test]
    fn files_sync_only_after_parallel_tools_all_finish() {
        with_session(|session, changed| {
            for (id, tool) in [("a", "edit"), ("b", "write")] {
                session.handle(Event::ToolStart {
                    id: id.into(),
                    tool: tool.into(),
                    args: json!({"path": "x"}),
                });
            }
            let end = |id: &str, tool: &str| Event::ToolEnd {
                id: id.into(),
                tool: tool.into(),
                is_error: false,
                output: String::new(),
            };
            session.handle(end("b", "write"));
            assert!(changed.borrow().is_empty());
            session.handle(end("a", "edit"));
            assert_eq!(*changed.borrow(), [true]);
        });
    }

    #[test]
    fn history_rebuilds_the_timeline_with_tool_results() {
        with_session(|session, _| {
            let messages = vec![
                json!({"role": "user", "content": "Rename greet"}),
                json!({"role": "assistant", "content": [
                    {"type": "text", "text": "On it."},
                    {"type": "toolCall", "id": "t1", "name": "edit",
                     "arguments": {"path": "a.rs", "edits": [{"oldText": "greet", "newText": "welcome"}]}}
                ]}),
                json!({"role": "toolResult", "toolCallId": "t1", "toolName": "edit",
                       "content": [{"type": "text", "text": "ok"}], "isError": false}),
                json!({"role": "system", "content": ""}),
            ];
            session.handle(Event::Response {
                command: "get_messages".into(),
                error: None,
                handled: false,
                data: json!({ "messages": messages }),
            });
            let entries = session.entries().get();
            let kinds: Vec<_> = entries.iter().map(|e| e.kind).collect();
            assert_eq!(
                kinds,
                [EntryKind::User, EntryKind::Assistant, EntryKind::Tool]
            );
            assert_eq!(entries[2].title.get(), "a.rs");
            assert_eq!(entries[2].body.get(), "- greet\n+ welcome\n");
            assert_eq!(entries[2].state.get(), Some(ToolState::Done));
        });
    }

    #[test]
    fn questions_wait_for_an_answer() {
        with_session(|session, _| {
            session.handle(Event::Ask(Ask {
                id: "u1".into(),
                title: "Allow rm?".into(),
                message: String::new(),
                kind: AskKind::Confirm,
            }));
            assert_eq!(session.status(), Status::NeedsInput);
            let entry = session.entries().get()[0].clone();
            assert!(entry.ask.is_some());
            // Not running, so the answer cannot reach Pi; it is still recorded.
            session.answer(&entry, Answer::Confirmed(true), "Yes".into());
            assert_eq!(entry.answered.get().as_deref(), Some("Yes"));
            assert_eq!(session.status(), Status::Idle);
        });
    }

    #[test]
    fn state_and_stats_fill_the_header() {
        with_session(|session, _| {
            let respond = |command: &str, data: Value| {
                session.handle(Event::Response {
                    command: command.into(),
                    error: None,
                    handled: false,
                    data,
                })
            };
            respond(
                "get_state",
                json!({"sessionFile": "/s/1.jsonl", "sessionName": "Login fix",
                       "model": {"provider": "anthropic", "id": "m1", "name": "Model One"}}),
            );
            respond(
                "get_session_stats",
                json!({"tokens": {"total": 12345}, "cost": 0.5, "contextUsage": {"percent": 12.0}}),
            );
            assert_eq!(session.title().get(), "Login fix");
            assert_eq!(session.file().get(), Some("/s/1.jsonl".into()));
            assert_eq!(session.model().get().unwrap().name, "Model One");
            assert_eq!(session.stats().get(), "12.3k tokens · $0.50 · 12% context");
        });
    }
}
