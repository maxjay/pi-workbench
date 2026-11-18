//! Every message the page and wbd send each other. JSON-RPC 2.0, one object
//! per line, in both directions.
//!
//! The page asks:
//!   {"jsonrpc":"2.0","id":1,"method":"health"}
//! wbd answers with the same id, and either a result or an error:
//!   {"jsonrpc":"2.0","id":1,"result":{"kind":"health",...}}
//!   {"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"...","data":{"kind":"notFound"}}}
//! and says things on its own as events:
//!   {"jsonrpc":"2.0","method":"event","params":{"event":"ready","version":"2.0.0"}}
//!
//! These Rust types are the source. `cargo test -p protocol` writes the
//! TypeScript ones into app/ui/src/protocol.

mod conversation;
mod pulls;
mod workspace;

use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

pub use conversation::*;
pub use pulls::*;
pub use workspace::*;

/// What the page can ask.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(tag = "method", content = "params")]
#[ts(export)]
pub enum Call {
    /// What wbd is, and what it found beside it.
    #[serde(rename = "health")]
    Health,

    /// The workspaces, the last opened first.
    #[serde(rename = "workspaces.list")]
    WorkspacesList,
    /// Opens a folder as a workspace, or opens it again.
    #[serde(rename = "workspaces.open")]
    WorkspacesOpen { path: String },
    /// Takes a workspace off the list. The folder is not touched.
    #[serde(rename = "workspaces.forget")]
    WorkspacesForget { id: String },
    /// Makes an empty folder for what you asked and opens it as a workspace.
    #[serde(rename = "scratch.new")]
    ScratchNew { text: String },
    /// The folders in `dir`, to pick one to open. `~` is your home folder.
    #[serde(rename = "folders.list")]
    FoldersList { dir: String },

    /// The files under `dir`, relative to it.
    #[serde(rename = "files.list")]
    FilesList { dir: String },
    #[serde(rename = "file.read")]
    FileRead { path: String },
    /// Saves a file. With `expected`, only if the file still holds that text.
    #[serde(rename = "file.write")]
    FileWrite {
        path: String,
        text: String,
        expected: Option<String>,
    },
    /// When each file last changed.
    #[serde(rename = "files.stat")]
    FilesStat { paths: Vec<String> },

    /// Where the repository `dir` is in stands.
    #[serde(rename = "git.status")]
    GitStatus { dir: String },
    /// A file as the last commit has it.
    #[serde(rename = "git.original")]
    GitOriginal { path: String },

    /// The terminals that started in `dir` or under it.
    #[serde(rename = "terminals.list")]
    TerminalsList { dir: String },
    /// Starts your shell in `cwd`.
    #[serde(rename = "terminal.open")]
    TerminalOpen { cwd: String, cols: u16, rows: u16 },
    /// The last of what a terminal printed, to draw it again.
    #[serde(rename = "terminal.read")]
    TerminalRead { id: String },
    /// What you typed. Answered in the order sent.
    #[serde(rename = "terminal.input")]
    TerminalInput { id: String, data: String },
    #[serde(rename = "terminal.resize")]
    TerminalResize { id: String, cols: u16, rows: u16 },
    /// Hangs up on the shell and everything it started.
    #[serde(rename = "terminal.close")]
    TerminalClose { id: String },

    /// The conversations in a folder, the latest first.
    #[serde(rename = "conversations.list")]
    ConversationsList { cwd: String },
    /// Starts a conversation with an agent in a folder, with what you wrote.
    #[serde(rename = "conversation.start")]
    ConversationStart {
        agent: Agent,
        cwd: String,
        text: String,
        model: Option<String>,
        effort: Option<String>,
    },
    /// Picks a conversation's model and effort. None leaves the agent's own.
    #[serde(rename = "conversation.set")]
    ConversationSet {
        id: String,
        model: Option<String>,
        effort: Option<String>,
    },
    /// What an agent offers: its models, their effort levels, whether it is signed in.
    #[serde(rename = "agent.offer")]
    AgentOffer { agent: Agent },
    /// A whole conversation, running or saved.
    #[serde(rename = "conversation.open")]
    ConversationOpen { id: String },
    /// Says something in a conversation. Starts the agent again if it isn't running.
    #[serde(rename = "conversation.send")]
    ConversationSend { id: String, text: String },
    /// Stops what the agent is doing now.
    #[serde(rename = "conversation.stop")]
    ConversationStop { id: String },
    /// Answers what the agent asked: one of its options, or what you wrote.
    #[serde(rename = "conversation.answer")]
    ConversationAnswer {
        id: String,
        ask: String,
        answer: Option<String>,
    },

    /// The pull requests you opened, in every project your workspaces are in.
    #[serde(rename = "pulls.list")]
    PullsList,
    /// A folder's branch and its pull request. With `pr`, that pull request
    /// whatever branch the folder is on.
    #[serde(rename = "pull.status")]
    PullStatus {
        dir: String,
        #[ts(type = "number | null")]
        pr: Option<u64>,
    },
    /// A review thread, with everything said in it.
    #[serde(rename = "pull.thread")]
    PullThread {
        dir: String,
        #[ts(type = "number")]
        pr: u64,
        #[ts(type = "number")]
        thread: u64,
    },
    /// Answers a thread, and resolves it too with `resolve`.
    #[serde(rename = "pull.reply")]
    PullReply {
        dir: String,
        #[ts(type = "number")]
        pr: u64,
        #[ts(type = "number")]
        thread: u64,
        text: String,
        resolve: bool,
    },
    /// Resolves a thread without saying anything.
    #[serde(rename = "pull.resolve")]
    PullResolve {
        dir: String,
        #[ts(type = "number")]
        pr: u64,
        #[ts(type = "number")]
        thread: u64,
    },
    /// What a pull request from a folder's branch would hold.
    #[serde(rename = "pull.before")]
    PullBefore { dir: String },
    /// Opens a pull request from a folder's branch, with the work items linked to it.
    #[serde(rename = "pull.create")]
    PullCreate {
        dir: String,
        title: String,
        body: String,
        draft: bool,
    },
    /// Links work items to a folder's branch, so its pull request carries them.
    #[serde(rename = "pull.link")]
    PullLink {
        dir: String,
        #[ts(type = "Array<number>")]
        ids: Vec<u64>,
    },
    /// Why a run failed.
    #[serde(rename = "pull.log")]
    PullLog {
        dir: String,
        #[ts(type = "number")]
        run: u64,
    },
}

/// A call, with the id its answer will carry.
#[derive(Debug, Deserialize)]
pub struct Request {
    pub id: u64,
    #[serde(flatten)]
    pub call: Call,
}

/// What wbd answers. `kind` says which, so the page can check it got the
/// answer it asked for.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum Answer {
    /// The call did what it was asked and has nothing to say.
    Done,
    Health(Health),
    Workspaces {
        list: Vec<WorkspaceView>,
    },
    Workspace(WorkspaceView),
    Folders {
        /// The folder listed, with `~` resolved.
        dir: String,
        folders: Vec<Folder>,
    },
    Files {
        files: Vec<String>,
        /// True when there were more files than are listed.
        truncated: bool,
    },
    File(FileView),
    Saved {
        /// When the saved file last changed, in milliseconds since 1970.
        #[ts(type = "number")]
        modified: u64,
    },
    Stat {
        /// One per path asked, in order. 0 for a file that is gone.
        #[ts(type = "Array<number>")]
        modified: Vec<u64>,
    },
    Git {
        /// None outside git.
        status: Option<GitStatus>,
    },
    Original {
        /// None when the file isn't in the last commit, or isn't text.
        text: Option<String>,
    },
    Terminals {
        list: Vec<Terminal>,
    },
    Terminal(Terminal),
    /// What a terminal printed lately.
    Printed {
        text: String,
    },
    Conversations {
        list: Vec<Conversation>,
    },
    Conversation(ConversationView),
    Offer(Offer),
    Pulls {
        rows: Vec<PullRow>,
    },
    Pull(PullView),
    Thread(Thread),
    Log(RunLog),
    Before(PullBefore),
    Created(PullRequest),
}

/// wbd, and the programs it works with. A program that isn't found has no version.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Health {
    /// wbd's own version.
    pub version: String,
    /// The WSL distro wbd runs in, if it runs in one.
    pub distro: Option<String>,
    pub pi: Option<String>,
    pub claude: Option<String>,
    pub dova: Option<String>,
}

/// What wbd says without being asked.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(tag = "event", rename_all = "camelCase")]
#[ts(export)]
pub enum Event {
    /// Sent once, first: wbd is up and reading.
    Ready { version: String },
    /// What a terminal just printed.
    Terminal { id: String, data: String },
    /// Every terminal, whenever one opens, closes, or starts or finishes running something.
    Terminals { list: Vec<Terminal> },
    /// What changed in a conversation.
    Conversation { id: String, changes: Vec<Change> },
}

/// Why a call wasn't answered: a JSON-RPC error.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct Failure {
    pub code: i32,
    pub message: String,
    pub data: FailureData,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct FailureData {
    pub kind: FailureKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FailureKind {
    /// The line wasn't a call wbd knows.
    BadRequest,
    /// The thing asked about isn't there.
    NotFound,
    /// Reading or writing failed.
    Io,
    /// A save was refused: the file changed since the editor read it.
    Conflict,
    /// The agent couldn't be started, or refused.
    Agent,
    /// dova has no sign-in to Azure DevOps.
    NotSignedIn,
    /// dova failed some other way.
    Dova,
}

impl Failure {
    pub fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        let code = match kind {
            FailureKind::BadRequest => -32600,
            FailureKind::NotFound
            | FailureKind::Io
            | FailureKind::Conflict
            | FailureKind::Agent
            | FailureKind::NotSignedIn
            | FailureKind::Dova => -32000,
        };
        Self {
            code,
            message: message.into(),
            data: FailureData { kind },
        }
    }
}

/// The line that answers call `id`.
pub fn reply(id: u64, answer: &Answer) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": answer }).to_string()
}

/// The line that refuses call `id`, or a line that had no id wbd could read.
pub fn failed(id: Option<u64>, failure: &Failure) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": failure }).to_string()
}

/// The line that carries an event.
pub fn event(event: &Event) -> String {
    json!({ "jsonrpc": "2.0", "method": "event", "params": event }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_is_read_with_or_without_the_envelope() {
        let full: Request =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":7,"method":"health"}"#).unwrap();
        assert_eq!((full.id, full.call), (7, Call::Health));
        let bare: Request = serde_json::from_str(r#"{"id":8,"method":"health"}"#).unwrap();
        assert_eq!(bare.id, 8);
    }

    #[test]
    fn an_unknown_method_is_not_a_call() {
        assert!(serde_json::from_str::<Request>(r#"{"id":1,"method":"nope"}"#).is_err());
    }

    #[test]
    fn the_lines_wbd_writes() {
        let health = Health {
            version: "2.0.0".into(),
            distro: Some("Ubuntu".into()),
            pi: Some("1.5.2".into()),
            claude: None,
            dova: Some("0.4.2".into()),
        };
        assert_eq!(
            reply(3, &Answer::Health(health)),
            r#"{"id":3,"jsonrpc":"2.0","result":{"claude":null,"distro":"Ubuntu","dova":"0.4.2","kind":"health","pi":"1.5.2","version":"2.0.0"}}"#
        );
        assert_eq!(
            failed(None, &Failure::new(FailureKind::BadRequest, "not JSON")),
            r#"{"error":{"code":-32600,"data":{"kind":"badRequest"},"message":"not JSON"},"id":null,"jsonrpc":"2.0"}"#
        );
        assert_eq!(
            event(&Event::Ready {
                version: "2.0.0".into()
            }),
            r#"{"jsonrpc":"2.0","method":"event","params":{"event":"ready","version":"2.0.0"}}"#
        );
    }
}
