//! Folds what an agent says into the entries the page shows, and remembers
//! what changed since the page was last told. Pure: no agent's name appears
//! here, and nothing is read or written.

use std::collections::BTreeSet;

use protocol::{Ask, Change, Conversation, ConversationView, Diff, Entry, Tone, Tool, ToolStatus};
use serde_json::Value;

/// One thing an agent said, in the workbench's own words. Each driver turns
/// its agent's lines into these.
#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    /// More of the reply.
    Text(String),
    Thinking(String),
    ToolStart {
        id: String,
        name: String,
        input: Value,
    },
    ToolEnd {
        id: String,
        failed: bool,
        output: String,
    },
    /// The agent waits for an answer. `detail` is what the driver needs to
    /// send the answer back, and is not shown.
    Ask {
        ask: Ask,
        detail: Value,
    },
    /// The agent stopped waiting for an answer by itself.
    AskGone,
    Notice {
        tone: Tone,
        text: String,
    },
    /// The agent finished, with nothing more coming until you write again.
    TurnEnd {
        error: Option<String>,
    },
}

/// No more of a tool's output than this is kept: the end of it, where a
/// failure usually says why.
pub const OUTPUT_LIMIT: usize = 20_000;

pub struct Folded {
    pub state: Conversation,
    pub entries: Vec<Entry>,
    changed: BTreeSet<usize>,
    state_changed: bool,
}

impl Folded {
    pub fn new(state: Conversation, entries: Vec<Entry>) -> Self {
        Folded {
            state,
            entries,
            changed: BTreeSet::new(),
            state_changed: false,
        }
    }

    pub fn view(&self) -> ConversationView {
        ConversationView {
            state: self.state.clone(),
            entries: self.entries.clone(),
        }
    }

    /// What you wrote. The agent is working from here until its turn ends.
    pub fn you(&mut self, text: &str, now: u64) {
        self.push(Entry::You {
            text: text.to_string(),
        });
        self.state.busy = true;
        self.state.stopped = None;
        self.state.updated = now;
        self.state_changed = true;
    }

    pub fn apply(&mut self, update: Update, now: u64) {
        match update {
            Update::Text(more) => match self.entries.last_mut() {
                Some(Entry::Text { text }) => {
                    text.push_str(&more);
                    self.changed.insert(self.entries.len() - 1);
                }
                _ => self.push(Entry::Text { text: more }),
            },
            Update::Thinking(more) => match self.entries.last_mut() {
                Some(Entry::Thinking { text }) => {
                    text.push_str(&more);
                    self.changed.insert(self.entries.len() - 1);
                }
                _ => self.push(Entry::Thinking { text: more }),
            },
            Update::ToolStart { id, name, input } => {
                let (title, diff) = describe(&input);
                let tool = Tool {
                    id,
                    name,
                    title,
                    status: ToolStatus::Running,
                    diff,
                    output: String::new(),
                };
                // An agent may name a call before it runs it: the same id is the same tool.
                match self.tool(&tool.id) {
                    Some(index) => {
                        self.entries[index] = Entry::Tool(tool);
                        self.changed.insert(index);
                    }
                    None => self.push(Entry::Tool(tool)),
                }
            }
            Update::ToolEnd { id, failed, output } => {
                let Some(index) = self.tool(&id) else { return };
                if let Entry::Tool(tool) = &mut self.entries[index] {
                    tool.status = match failed {
                        true => ToolStatus::Failed,
                        false => ToolStatus::Done,
                    };
                    tool.output = tail(&output, OUTPUT_LIMIT);
                }
                self.changed.insert(index);
            }
            Update::Ask { ask, .. } => {
                self.state.ask = Some(ask);
                self.state_changed = true;
            }
            Update::AskGone => self.answered(),
            Update::Notice { tone, text } => self.push(Entry::Notice { tone, text }),
            Update::TurnEnd { error } => {
                if let Some(text) = error {
                    self.push(Entry::Notice {
                        tone: Tone::Failed,
                        text,
                    });
                }
                self.state.busy = false;
                self.state.ask = None;
                self.state.updated = now;
                self.state_changed = true;
            }
        }
    }

    /// You answered what the agent asked, or it stopped asking.
    pub fn answered(&mut self) {
        if self.state.ask.take().is_some() {
            self.state_changed = true;
        }
    }

    /// You picked its model and effort.
    pub fn picked(&mut self, model: Option<String>, effort: Option<String>) {
        self.state.model = model;
        self.state.effort = effort;
        self.state_changed = true;
    }

    /// The agent's process ended. That is only worth saying if it was working.
    pub fn stopped(&mut self, reason: String) {
        if self.state.busy || self.state.ask.is_some() {
            self.state.stopped = Some(reason);
        }
        self.state.busy = false;
        self.state.ask = None;
        self.state_changed = true;
    }

    /// What changed since this was last asked.
    pub fn take(&mut self) -> Vec<Change> {
        let mut changes: Vec<Change> = std::mem::take(&mut self.changed)
            .into_iter()
            .map(|index| Change::Entry {
                index: index as u32,
                entry: self.entries[index].clone(),
            })
            .collect();
        if std::mem::take(&mut self.state_changed) {
            changes.push(Change::State {
                state: self.state.clone(),
            });
        }
        changes
    }

    fn push(&mut self, entry: Entry) {
        self.entries.push(entry);
        self.changed.insert(self.entries.len() - 1);
    }

    fn tool(&self, id: &str) -> Option<usize> {
        self.entries
            .iter()
            .rposition(|entry| matches!(entry, Entry::Tool(tool) if tool.id == id))
    }
}

/// What a tool call acts on, from its input: a few words for its row, and
/// what it changes if it is an edit. The agents name these fields differently.
pub fn describe(input: &Value) -> (String, Option<Diff>) {
    let text = |keys: &[&str]| {
        keys.iter()
            .find_map(|key| input[*key].as_str())
            .map(str::to_string)
    };
    let path = text(&["path", "file_path"]);
    let title = path
        .clone()
        .or_else(|| text(&["command", "pattern", "url", "query", "description"]))
        .unwrap_or_default();
    let Some(path) = path else {
        return (title, None);
    };
    let edit = |old: &str, new: &str, from: &Value| {
        Some((
            from[old].as_str()?.to_string(),
            from[new].as_str()?.to_string(),
        ))
    };
    // One replacement, several, or a whole new file.
    let replaced = edit("oldText", "newText", input)
        .or_else(|| edit("old_string", "new_string", input))
        .or_else(|| {
            let edits = input["edits"].as_array()?;
            let pairs: Vec<_> = edits
                .iter()
                .filter_map(|each| {
                    edit("oldText", "newText", each)
                        .or_else(|| edit("old_string", "new_string", each))
                })
                .collect();
            let join = |pick: fn(&(String, String)) -> &String| {
                pairs
                    .iter()
                    .map(pick)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            (!pairs.is_empty()).then(|| (join(|pair| &pair.0), join(|pair| &pair.1)))
        });
    let diff = match replaced {
        Some((old, new)) => Some(Diff {
            path,
            old: Some(old),
            new,
        }),
        None => input["content"].as_str().map(|content| Diff {
            path,
            old: None,
            new: content.to_string(),
        }),
    };
    (title, diff)
}

/// The last `limit` bytes of a text, cut at a character.
pub fn tail(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let mut cut = text.len() - limit;
    while !text.is_char_boundary(cut) {
        cut += 1;
    }
    format!("…{}", &text[cut..])
}

#[cfg(test)]
mod tests {
    use protocol::Agent;
    use serde_json::json;

    use super::*;

    fn folded() -> Folded {
        Folded::new(
            Conversation {
                id: "c1".into(),
                agent: Agent::Pi,
                cwd: "/code".into(),
                title: "Fix it".into(),
                updated: 0,
                busy: false,
                ask: None,
                stopped: None,
                model: None,
                effort: None,
            },
            Vec::new(),
        )
    }

    #[test]
    fn text_grows_in_place_and_a_tool_starts_a_new_entry() {
        let mut f = folded();
        f.you("Fix it", 1);
        f.apply(Update::Text("Look".into()), 2);
        f.apply(Update::Text("ing.".into()), 2);
        f.apply(
            Update::ToolStart {
                id: "t1".into(),
                name: "read".into(),
                input: json!({ "path": "src/a.ts" }),
            },
            2,
        );
        f.apply(Update::Text("Done.".into()), 3);
        assert_eq!(f.entries.len(), 4);
        assert_eq!(
            f.entries[1],
            Entry::Text {
                text: "Looking.".into()
            }
        );
        assert!(matches!(&f.entries[2], Entry::Tool(tool) if tool.title == "src/a.ts"));
        assert!(f.state.busy);
    }

    #[test]
    fn a_tool_ends_where_it_started() {
        let mut f = folded();
        let start = |id: &str| Update::ToolStart {
            id: id.into(),
            name: "bash".into(),
            input: json!({ "command": "npm test" }),
        };
        f.apply(start("t1"), 1);
        f.apply(start("t1"), 1);
        f.apply(
            Update::ToolEnd {
                id: "t1".into(),
                failed: true,
                output: "1 failed".into(),
            },
            2,
        );
        assert_eq!(f.entries.len(), 1);
        assert!(matches!(
            &f.entries[0],
            Entry::Tool(tool) if tool.status == ToolStatus::Failed && tool.output == "1 failed"
        ));
    }

    #[test]
    fn says_what_an_edit_changed_whichever_agent_made_it() {
        let pi = describe(&json!({ "path": "a.ts", "oldText": "one", "newText": "two" }));
        let claude =
            describe(&json!({ "file_path": "a.ts", "old_string": "one", "new_string": "two" }));
        assert_eq!(pi, claude);
        assert_eq!(
            pi.1,
            Some(Diff {
                path: "a.ts".into(),
                old: Some("one".into()),
                new: "two".into()
            })
        );
        let written = describe(&json!({ "path": "new.md", "content": "# Hi" }));
        assert_eq!(written.1.unwrap().old, None);
        assert_eq!(describe(&json!({ "command": "ls" })), ("ls".into(), None));
    }

    #[test]
    fn tells_what_changed_once() {
        let mut f = folded();
        f.you("Hi", 5);
        f.apply(Update::Text("Hel".into()), 6);
        let first = f.take();
        assert_eq!(first.len(), 3);
        assert!(matches!(first[2], Change::State { .. }));
        f.apply(Update::Text("lo".into()), 6);
        assert_eq!(
            f.take(),
            [Change::Entry {
                index: 1,
                entry: Entry::Text {
                    text: "Hello".into()
                }
            }]
        );
        assert!(f.take().is_empty());
    }

    #[test]
    fn a_turn_ends_with_its_failure_and_clears_the_question() {
        let mut f = folded();
        f.you("Hi", 1);
        f.apply(
            Update::Ask {
                ask: Ask {
                    id: "a1".into(),
                    title: "Run it?".into(),
                    message: None,
                    options: vec!["Yes".into(), "No".into()],
                    input: false,
                },
                detail: Value::Null,
            },
            2,
        );
        assert!(f.state.ask.is_some());
        f.apply(
            Update::TurnEnd {
                error: Some("out of credit".into()),
            },
            3,
        );
        assert!(!f.state.busy && f.state.ask.is_none());
        assert!(matches!(
            f.entries.last(),
            Some(Entry::Notice {
                tone: Tone::Failed,
                ..
            })
        ));
    }

    #[test]
    fn keeps_the_end_of_long_output() {
        let long = format!("{}end", "x".repeat(OUTPUT_LIMIT));
        let kept = tail(&long, OUTPUT_LIMIT);
        assert!(kept.starts_with('…') && kept.ends_with("end"));
    }
}
