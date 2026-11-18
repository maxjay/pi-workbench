//! A conversation with an agent, as the page shows it. The same shapes
//! whichever agent it is with.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Which agent a conversation is with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Agent {
    Pi,
    Claude,
}

/// A conversation: where it is, and where it stands.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Conversation {
    pub id: String,
    pub agent: Agent,
    /// The folder the agent works in.
    pub cwd: String,
    /// What you first wrote, shortened.
    pub title: String,
    /// When something was last said, in milliseconds since 1970.
    #[ts(type = "number")]
    pub updated: u64,
    /// The agent is working.
    pub busy: bool,
    /// What the agent is waiting for you to answer.
    pub ask: Option<Ask>,
    /// Why the agent's process ended, when it ended while working.
    pub stopped: Option<String>,
    /// The model you picked for it. None while it uses the agent's own.
    pub model: Option<String>,
    /// The effort you picked for it. None while it uses the agent's own.
    pub effort: Option<String>,
}

/// What an agent offers, as the agent itself says.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Offer {
    pub signed_in: bool,
    pub models: Vec<Model>,
    /// The model it uses when you pick none.
    pub model: Option<String>,
    /// The effort it uses when you pick none.
    pub effort: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Model {
    /// What the agent knows it by.
    pub id: String,
    pub name: String,
    /// The effort levels it takes, the lowest first. Empty when it has none.
    pub efforts: Vec<String>,
}

/// A question from the agent: a permission, a choice, or something to write.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Ask {
    pub id: String,
    pub title: String,
    pub message: Option<String>,
    /// The answers to pick from. Empty when the answer is written.
    pub options: Vec<String>,
    /// True when you can write your own answer.
    pub input: bool,
}

/// One thing said or done in a conversation.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum Entry {
    You { text: String },
    Text { text: String },
    Thinking { text: String },
    Tool(Tool),
    Notice { tone: Tone, text: String },
}

/// Something the agent did: read a file, ran a command, edited a file.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Tool {
    pub id: String,
    /// The tool's own name, such as `read` or `Bash`.
    pub name: String,
    /// What it acted on: a path, a command, a pattern.
    pub title: String,
    pub status: ToolStatus,
    /// What an edit changed.
    pub diff: Option<Diff>,
    /// What it printed or returned, shortened when long.
    pub output: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ToolStatus {
    Running,
    Done,
    Failed,
}

/// The text an edit replaced, and what it put there. `old` is None for a new file.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Diff {
    pub path: String,
    pub old: Option<String>,
    pub new: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Tone {
    Info,
    Warning,
    Failed,
}

/// A whole conversation: for opening one.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ConversationView {
    pub state: Conversation,
    pub entries: Vec<Entry>,
}

/// What changed in a conversation since the last event about it.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum Change {
    /// An entry is new, or replaces the one at its index.
    Entry {
        index: u32,
        entry: Entry,
    },
    State {
        state: Conversation,
    },
}
