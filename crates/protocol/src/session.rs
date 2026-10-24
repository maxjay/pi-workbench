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
    Select { options: Vec<String> },
    Confirm,
    Input { placeholder: Option<String> },
    Editor { prefill: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum AskReply {
    Value { value: String },
    Confirmed { confirmed: bool },
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
