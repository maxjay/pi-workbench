//! Pi sessions: the conversations wb-agent runs, as the workbench shows
//! them. wb-agent keeps each one as a [`SessionState`] and a list of
//! [`Entry`]s, and sends a [`Change`] whenever one of them changes.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A live session: where it stands, and everything said so far.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Session {
    pub id: String,
    pub state: SessionState,
    pub entries: Vec<Entry>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionState {
    /// The name Pi keeps for it, or the first thing asked.
    pub title: String,
    pub cwd: String,
    /// Pi's session file, once Pi has said where it is.
    pub file: Option<String>,
    /// A run has started and not settled yet.
    pub busy: bool,
    /// Messages waiting for the run to reach them.
    pub queued: Vec<Queued>,
    /// A question an extension is waiting on.
    pub ask: Option<Ask>,
    /// Status lines set by extensions.
    pub statuses: Vec<StatusLine>,
    /// `provider/model`.
    pub model: Option<String>,
    pub thinking: Option<String>,
    /// Files Pi wrote or edited, as absolute paths, most recent last.
    pub files: Vec<String>,
    /// Why Pi stopped, once it has.
    pub stopped: Option<String>,
    /// Marked done: it lists under Done until you write in it again.
    pub done: bool,
    /// Tokens and cost so far, and how full the context is.
    pub usage: Option<Usage>,
    /// The thinking levels the model takes; just `off` for one that doesn't think.
    pub levels: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Usage {
    #[ts(type = "number")]
    pub input: u64,
    #[ts(type = "number")]
    pub output: u64,
    /// Dollars.
    pub cost: f64,
    /// Tokens in the context now, when Pi knows.
    #[ts(type = "number | null")]
    pub context: Option<u64>,
    #[ts(type = "number | null")]
    pub window: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Queued {
    pub text: String,
    /// Waits for the run to finish, rather than steering it.
    pub later: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct StatusLine {
    pub key: String,
    pub text: String,
}

/// An extension asking the user something; the run waits for the answer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Ask {
    pub id: String,
    pub title: String,
    pub message: Option<String>,
    pub kind: AskKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum AskKind {
    Select {
        options: Vec<String>,
    },
    Confirm,
    Input {
        placeholder: Option<String>,
    },
    Editor {
        prefill: String,
    },
    /// Pi's `ask_user` tool: one question or several, each answered by
    /// picking a choice or writing one.
    Questions {
        questions: Vec<Question>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Question {
    /// A word or two for its tab.
    pub label: String,
    pub question: String,
    pub choices: Vec<Choice>,
    /// You can write an answer of your own.
    pub other: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Choice {
    pub label: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum AskReply {
    Value {
        value: String,
    },
    Confirmed {
        confirmed: bool,
    },
    /// One for each question, in order.
    Answers {
        answers: Vec<String>,
    },
    Cancelled,
}

/// One thing in the conversation. A reply from Pi is a run of text,
/// thinking and tool entries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum Entry {
    User { text: String, images: Vec<Image> },
    Text { text: String },
    Thinking { text: String },
    Tool(Tool),
    Notice { text: String, tone: Tone },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Tool {
    pub id: String,
    pub name: String,
    /// The arguments, once the model has finished writing them; null before.
    #[ts(type = "unknown")]
    pub args: serde_json::Value,
    pub status: ToolStatus,
    /// The result's text: partial while it runs.
    pub output: String,
    pub images: Vec<Image>,
    /// What the tool kept for itself, such as a todo tool's list.
    #[ts(type = "unknown")]
    pub details: serde_json::Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ToolStatus {
    /// The model is still writing the call.
    Writing,
    Running,
    Done,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Image {
    pub mime_type: String,
    /// Base64.
    pub data: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Tone {
    Info,
    Warning,
    Error,
}

/// What changed in a live session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "change", rename_all = "camelCase")]
#[ts(export)]
pub enum Change {
    /// The entry at `index` is new, or replaces the one there.
    Entry {
        index: u32,
        entry: Box<Entry>,
    },
    State {
        state: Box<SessionState>,
    },
    /// The whole conversation, after Pi loaded it from its file.
    Entries {
        entries: Vec<Entry>,
    },
    /// wb-agent no longer runs it. It can be opened again from its file.
    Closed,
}

/// A session in a list: live, or kept in Pi's store.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionSummary {
    /// Set while wb-agent runs it.
    pub id: Option<String>,
    pub file: Option<String>,
    pub title: String,
    pub cwd: String,
    /// Milliseconds since the epoch.
    #[ts(type = "number")]
    pub updated: u64,
    pub busy: bool,
    /// Waiting on an answer.
    pub asking: bool,
    /// Pi stopped under it.
    pub stopped: bool,
    pub done: bool,
}

/// A model a live session can switch to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Available {
    pub provider: String,
    pub id: String,
    pub name: String,
    pub reasoning: bool,
    #[ts(type = "number | null")]
    pub context_window: Option<u64>,
}

/// A command `/` runs in a session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PiCommand {
    /// Without the `/`, e.g. `skill:grafana`.
    pub name: String,
    pub description: Option<String>,
    /// `extension`, `prompt` or `skill`.
    pub source: String,
}

/// A file opened for viewing.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileView {
    /// Absolute.
    pub path: String,
    pub content: FileContent,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum FileContent {
    Text {
        text: String,
        /// Only the start of a large file is sent.
        truncated: bool,
    },
    #[serde(rename_all = "camelCase")]
    Image {
        mime_type: String,
        data: String,
    },
    /// Neither text nor an image.
    Binary {
        #[ts(type = "number")]
        size: u64,
    },
    Missing,
}
