//! Runs the user's Pi in the background, shows its activity, and reports file changes.
use crate::documents::Documents;
use floem::{
    action::exec_after,
    ext_event::{ExtSendTrigger, register_ext_trigger},
    prelude::*,
    reactive::{Effect, ReadSignal, Scope},
};
use pi_client::{Event, Launch, PiProcess, preview::file_preview};
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
    Tool,
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

/// One line of the activity log. Fields update in place while streaming.
pub struct Entry {
    pub id: u64,
    pub kind: EntryKind,
    /// Tool name for tool entries, otherwise empty.
    pub tool: String,
    /// The message, or a tool's target file or command.
    pub title: RwSignal<String>,
    pub body: RwSignal<String>,
    pub state: RwSignal<Option<ToolState>>,
}

/// Tools whose effects on disk the workbench cannot predict from their arguments.
fn may_create_files(tool: &str) -> bool {
    matches!(tool, "write" | "bash" | "powershell")
}

/// The file or command a tool call acts on, from final or streaming arguments.
fn tool_target(args: &serde_json::Value, partial: &str) -> String {
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

struct Streaming {
    tool: String,
    args: String,
    entry: Rc<Entry>,
}

pub struct Agent {
    scope: Scope,
    root: PathBuf,
    launch: Launch,
    documents: Rc<Documents>,
    /// Called after each tool finishes, with the tool's name.
    on_tool_end: Box<dyn Fn(&str)>,
    process: RefCell<Option<PiProcess>>,
    entries: RwSignal<Vec<Rc<Entry>>>,
    next_entry: Cell<u64>,
    working: RwSignal<bool>,
    visible: RwSignal<bool>,
    /// Tool calls by content index while their arguments stream.
    streaming: RefCell<HashMap<u64, String>>,
    tools: RefCell<HashMap<String, Streaming>>,
    reply: RefCell<Option<Rc<Entry>>>,
    queue: Arc<Mutex<VecDeque<Event>>>,
    trigger: ExtSendTrigger,
}

impl Agent {
    pub fn new(
        scope: Scope,
        root: PathBuf,
        launch: Launch,
        documents: Rc<Documents>,
        on_tool_end: impl Fn(&str) + 'static,
    ) -> Rc<Self> {
        let agent = Rc::new(Self {
            scope,
            root,
            launch,
            documents,
            on_tool_end: Box::new(on_tool_end),
            process: RefCell::new(None),
            entries: RwSignal::new(Vec::new()),
            next_entry: Cell::new(0),
            working: RwSignal::new(false),
            visible: RwSignal::new(false),
            streaming: RefCell::new(HashMap::new()),
            tools: RefCell::new(HashMap::new()),
            reply: RefCell::new(None),
            queue: Arc::new(Mutex::new(VecDeque::new())),
            trigger: ExtSendTrigger::new(),
        });
        // Pi's reader thread queues events; this drains them on the UI thread in order.
        let weak: Weak<Self> = Rc::downgrade(&agent);
        let trigger = agent.trigger;
        Effect::new(move |_| {
            trigger.track();
            if let Some(agent) = weak.upgrade() {
                loop {
                    let next = agent.queue.lock().unwrap().pop_front();
                    match next {
                        Some(event) => agent.handle(event),
                        None => break,
                    }
                }
            }
        });
        agent
    }

    pub fn entries(&self) -> ReadSignal<Vec<Rc<Entry>>> {
        self.entries.read_only()
    }
    pub fn working(&self) -> ReadSignal<bool> {
        self.working.read_only()
    }
    pub fn visible(&self) -> ReadSignal<bool> {
        self.visible.read_only()
    }
    pub fn toggle(&self) {
        self.visible.update(|visible| *visible = !*visible);
    }

    fn push(&self, kind: EntryKind, tool: &str, title: String, body: String) -> Rc<Entry> {
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
            })
        });
        self.entries.update(|entries| entries.push(entry.clone()));
        entry
    }

    fn notice(&self, message: impl Into<String>) {
        self.push(EntryKind::Notice, "", message.into(), String::new());
    }

    /// Saves open files first so Pi reads what the user sees.
    pub fn send(self: &Rc<Self>, text: String) {
        let text = text.trim().to_string();
        if text.is_empty() || self.working.get_untracked() {
            return;
        }
        self.push(EntryKind::User, "", text.clone(), String::new());
        self.working.set(true);
        self.documents.save_all();
        self.prompt_when_saved(text);
    }

    fn prompt_when_saved(self: &Rc<Self>, text: String) {
        if self.documents.any_saving() {
            let agent = self.clone();
            exec_after(Duration::from_millis(30), move |_| {
                agent.prompt_when_saved(text)
            });
            return;
        }
        let result = self.start().and_then(|()| {
            self.process
                .borrow_mut()
                .as_mut()
                .map_or(Ok(()), |pi| pi.prompt(&text))
        });
        if let Err(err) = result {
            self.working.set(false);
            self.notice(format!("{err:#}"));
        }
    }

    fn start(&self) -> anyhow::Result<()> {
        if self.process.borrow().is_some() {
            return Ok(());
        }
        let queue = self.queue.clone();
        let trigger = self.trigger;
        let process = PiProcess::spawn(&self.launch, &self.root, move |event| {
            queue.lock().unwrap().push_back(event);
            register_ext_trigger(trigger);
        })
        .map_err(|err| {
            err.context("Is Pi installed? npm install -g @earendil-works/pi-coding-agent")
        })?;
        *self.process.borrow_mut() = Some(process);
        Ok(())
    }

    pub fn stop(&self) {
        if let Some(pi) = self.process.borrow_mut().as_mut()
            && let Err(err) = pi.abort()
        {
            self.notice(format!("{err:#}"));
        }
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

    pub fn handle(&self, event: Event) {
        match event {
            Event::AgentStart => self.working.set(true),
            Event::TextDelta(delta) => {
                let mut reply = self.reply.borrow_mut();
                let entry = reply.get_or_insert_with(|| {
                    self.push(EntryKind::Assistant, "", String::new(), String::new())
                });
                entry.body.update(|body| body.push_str(&delta));
            }
            Event::ToolCallStart { index, id, tool } => {
                *self.reply.borrow_mut() = None;
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
                    call.entry
                        .title
                        .set(tool_target(&serde_json::Value::Null, &call.args));
                    call.entry.body.set(tool_preview(&call.tool, &call.args));
                }
            }
            Event::ToolStart { id, tool, args } => {
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
                entry.state.set(Some(if is_error {
                    ToolState::Failed
                } else {
                    ToolState::Done
                }));
                // File tools already show their change; others show their output.
                if is_error || !matches!(tool.as_str(), "edit" | "write") {
                    entry.body.set(output);
                }
                (self.on_tool_end)(&tool);
            }
            Event::AgentSettled => {
                self.working.set(false);
                *self.reply.borrow_mut() = None;
                self.streaming.borrow_mut().clear();
                self.tools.borrow_mut().clear();
                // Catch anything a tool changed after its own end event was handled.
                (self.on_tool_end)("");
            }
            Event::Response {
                command,
                error: Some(error),
            } => {
                self.working.set(false);
                self.notice(format!("Pi rejected {command}: {error}"));
            }
            Event::Exited(message) => {
                self.process.borrow_mut().take();
                self.working.set(false);
                self.notice(format!("Pi stopped: {message}"));
            }
            Event::Response { .. } | Event::Other => {}
        }
    }
}

/// Whether Pi may have created or deleted files, so the explorer should refresh.
pub fn tool_changes_tree(tool: &str) -> bool {
    tool.is_empty() || may_create_files(tool)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn with_agent(test: impl FnOnce(Rc<Agent>, Rc<RefCell<Vec<String>>>)) {
        let scope = Scope::new();
        scope.enter(|| {
            let status = RwSignal::new(String::new());
            let documents = Documents::new(scope, "/".into(), status);
            let ended = Rc::new(RefCell::new(Vec::new()));
            let log = ended.clone();
            let agent = Agent::new(
                scope,
                "/".into(),
                Launch::default(),
                documents,
                move |tool| log.borrow_mut().push(tool.to_string()),
            );
            test(agent, ended);
        });
        scope.dispose();
    }

    #[test]
    fn streamed_edits_preview_live_and_report_when_done() {
        with_agent(|agent, ended| {
            agent.handle(Event::AgentStart);
            agent.handle(Event::ToolCallStart {
                index: 0,
                id: "c1".into(),
                tool: "edit".into(),
            });
            for delta in [
                r#"{"path":"src/a.rs","edits":[{"oldText":"hel"#,
                r#"lo","newText":"hello pi"}]}"#,
            ] {
                agent.handle(Event::ToolCallDelta {
                    index: 0,
                    delta: delta.into(),
                });
            }
            let entry = agent.entries().get()[0].clone();
            assert_eq!(entry.title.get(), "src/a.rs");
            assert_eq!(entry.body.get(), "- hello\n+ hello pi\n");
            assert_eq!(entry.state.get(), Some(ToolState::Streaming));

            agent.handle(Event::ToolStart {
                id: "c1".into(),
                tool: "edit".into(),
                args: json!({"path": "src/a.rs"}),
            });
            assert_eq!(entry.state.get(), Some(ToolState::Running));
            agent.handle(Event::ToolEnd {
                id: "c1".into(),
                tool: "edit".into(),
                is_error: false,
                output: "Successfully replaced".into(),
            });
            assert_eq!(entry.state.get(), Some(ToolState::Done));
            assert_eq!(entry.body.get(), "- hello\n+ hello pi\n");
            assert_eq!(*ended.borrow(), ["edit"]);

            agent.handle(Event::TextDelta("Edited ".into()));
            agent.handle(Event::TextDelta("it.".into()));
            agent.handle(Event::AgentSettled);
            let entries = agent.entries().get();
            assert_eq!(entries.len(), 2);
            assert_eq!(entries[1].body.get(), "Edited it.");
            assert!(!agent.working().get());
            assert_eq!(*ended.borrow(), ["edit", ""]);
        });
    }

    #[test]
    fn unstreamed_tools_still_get_an_entry_and_failures_show_output() {
        with_agent(|agent, _| {
            agent.handle(Event::ToolStart {
                id: "c2".into(),
                tool: "bash".into(),
                args: json!({"command": "cargo fmt"}),
            });
            agent.handle(Event::ToolEnd {
                id: "c2".into(),
                tool: "bash".into(),
                is_error: true,
                output: "not found".into(),
            });
            let entry = agent.entries().get()[0].clone();
            assert_eq!(entry.title.get(), "cargo fmt");
            assert_eq!(entry.tool, "bash");
            assert_eq!(entry.body.get(), "not found");
            assert_eq!(entry.state.get(), Some(ToolState::Failed));
        });
    }

    #[test]
    fn process_exit_is_reported_and_unblocks_sending() {
        with_agent(|agent, _| {
            agent.working.set(true);
            agent.handle(Event::Exited("No API key".into()));
            assert!(!agent.working().get());
            assert!(agent.entries().get()[0].title.get().contains("No API key"));
        });
    }
}
