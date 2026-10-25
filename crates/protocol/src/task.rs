//! Tasks: prompts Pi runs on a schedule, each in a folder of its own, and
//! the runs they've had. wb-agent keeps them and runs them; the workbench
//! shows them and changes them.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A task as it's saved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Task {
    pub id: String,
    pub name: String,
    /// What Pi does each run.
    pub prompt: String,
    pub schedule: Schedule,
    /// Every run adds to one conversation; otherwise each run has its own.
    pub continuing: bool,
    pub paused: bool,
    /// The folder Pi works in, which is the task's own.
    pub folder: String,
}

/// When a task runs. Days are 0 for Monday to 6 for Sunday, and times are
/// local, `HH:MM`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum Schedule {
    At {
        days: Vec<u8>,
        time: String,
    },
    Every {
        hours: u32,
    },
    /// Only when you run it.
    Manual,
}

/// A task to make, or, with an id, to change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TaskInput {
    pub id: Option<String>,
    pub name: String,
    pub prompt: String,
    pub schedule: Schedule,
    pub continuing: bool,
}

/// One run of a task.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Run {
    pub id: String,
    /// Milliseconds since the epoch.
    #[ts(type = "number")]
    pub started: u64,
    #[ts(type = "number | null")]
    pub finished: Option<u64>,
    pub trigger: Trigger,
    pub state: RunState,
    /// A line on what it did: the start of Pi's last reply.
    pub summary: String,
    /// Why it failed.
    pub error: Option<String>,
    /// Files Pi wrote or edited in the run, as absolute paths, most recent last.
    pub files: Vec<String>,
    /// The session file the run is in.
    pub session: Option<String>,
    /// The live session running it, while one is.
    pub live: Option<String>,
    /// Where the run starts in its conversation, as an index into its entries.
    pub from: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Trigger {
    Scheduled,
    /// Its time passed while the app wasn't running, or tasks were paused.
    Late,
    /// You ran it.
    Now,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum RunState {
    Running,
    /// Waiting on an answer from you.
    Asking,
    Done,
    Failed,
    /// You stopped it.
    Stopped,
}

/// A task with its runs, newest first, and when it next runs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TaskView {
    pub task: Task,
    pub runs: Vec<Run>,
    /// Milliseconds since the epoch; none when it only runs when you ask.
    #[ts(type = "number | null")]
    pub next: Option<u64>,
}

/// Every task, and whether they're all paused.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TaskList {
    pub tasks: Vec<TaskView>,
    pub paused: bool,
}
