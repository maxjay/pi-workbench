//! The two agents, each spoken to in its own rpc: one process per
//! conversation, lines in and lines out. This is the only place that
//! knows one agent from the other. Everything a driver hears becomes
//! [`Update`]s, the same for both.

mod claude;
mod pi;

use std::path::Path;
use std::process::Stdio;

use protocol::{Agent, Ask};
use serde_json::Value;
use tokio::process::Command;

use crate::fold::Update;

/// The process for a conversation. `resume` when the agent has it already.
pub fn command(agent: Agent, id: &str, cwd: &Path, resume: bool) -> Command {
    let mut command = match agent {
        Agent::Pi => pi::command(id),
        Agent::Claude => claude::command(id, resume),
    };
    command
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

/// What one line from the agent means. Nothing, for a line the workbench doesn't use.
pub fn decode(agent: Agent, line: &str) -> Vec<Update> {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    match agent {
        Agent::Pi => pi::decode(&value),
        Agent::Claude => claude::decode(&value),
    }
}

/// The line that says what you wrote.
pub fn prompt(agent: Agent, text: &str) -> String {
    match agent {
        Agent::Pi => pi::prompt(text),
        Agent::Claude => claude::prompt(text),
    }
}

/// The line that stops what the agent is doing.
pub fn stop(agent: Agent) -> String {
    match agent {
        Agent::Pi => pi::stop(),
        Agent::Claude => claude::stop(),
    }
}

/// The line that answers a question. `answer` is the option picked or the
/// text written, or nothing when you dismissed it. `detail` is what the
/// driver kept when the question came.
pub fn answer(agent: Agent, ask: &Ask, answer: Option<&str>, detail: &Value) -> String {
    match agent {
        Agent::Pi => pi::answer(ask, answer, detail),
        Agent::Claude => claude::answer(ask, answer, detail),
    }
}

/// The text in a tool's result, however the agent wraps it: a string, or
/// a list of parts of which the text ones count.
fn result_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}
