//! The two agents, each spoken to in its own rpc: one process per
//! conversation, lines in and lines out. This is the only place that
//! knows one agent from the other. Everything a driver hears becomes
//! [`Update`]s, the same for both.

mod claude;
mod pi;
mod pi_sessions;

use std::path::Path;
use std::process::Stdio;

use std::time::Duration;

use protocol::{Agent, Ask, Conversation, ConversationView, Offer};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::error::Error;
use crate::fold::Update;

/// How long an agent gets to say what it offers.
const OFFER_PATIENCE: Duration = Duration::from_secs(30);

pub fn name(agent: Agent) -> &'static str {
    match agent {
        Agent::Pi => "Pi",
        Agent::Claude => "Claude Code",
    }
}

/// The process for a conversation. `resume` when the agent has it already.
/// `model` and `effort` are what you picked for it, where you picked.
/// `brief` is told to the agent before anything you write.
pub fn command(
    agent: Agent,
    id: &str,
    cwd: &Path,
    resume: bool,
    model: Option<&str>,
    effort: Option<&str>,
    brief: Option<&str>,
) -> Command {
    let mut command = match agent {
        Agent::Pi => pi::command(id, model, effort),
        Agent::Claude => claude::command(id, resume, model, effort),
    };
    // Both agents take it the same way: added to what they are told about themselves.
    if let Some(brief) = brief {
        command.args(["--append-system-prompt", brief]);
    }
    command
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

/// The sessions the agents keep by themselves for a folder, as
/// conversations. Those made outside the workbench are among them. Each
/// `title` is the whole of what you first wrote. Only Pi's are read.
pub fn sessions(cwd: &str) -> Vec<Conversation> {
    pi_sessions::root().map_or_else(Vec::new, |root| pi_sessions::list(&root, cwd))
}

/// One of those sessions, whole, or nothing when no agent keeps a session
/// with this id. Its id is what the agent knows it by, so starting the
/// agent with it carries the session on. `near` is the folder it worked
/// in, where that is known. A session that is there and can't be read is
/// an error that says why.
pub fn session(id: &str, near: Option<&str>) -> Result<Option<ConversationView>, String> {
    match pi_sessions::root() {
        Some(root) => pi_sessions::open(&root, id, near),
        None => Ok(None),
    }
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

/// The lines that change a running agent's model and effort.
pub fn set(agent: Agent, model: Option<&str>, effort: Option<&str>) -> Vec<String> {
    match agent {
        Agent::Pi => pi::set(model, effort),
        Agent::Claude => claude::set(model, effort),
    }
}

/// What an agent offers: its models, their effort levels, and whether it
/// is signed in. The agent is started to be asked, and ended.
pub async fn offer(agent: Agent) -> Result<Offer, Error> {
    let failed = |why: String| Error::Agent(format!("couldn't ask {}: {why}", name(agent)));
    let (mut command, asks) = match agent {
        Agent::Pi => pi::offer_asks(),
        Agent::Claude => claude::offer_asks(),
    };
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| failed(err.to_string()))?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    for ask in &asks {
        stdin
            .write_all(format!("{ask}\n").as_bytes())
            .await
            .map_err(|err| failed(err.to_string()))?;
    }
    let mut lines = BufReader::new(child.stdout.take().expect("stdout is piped")).lines();
    let mut answers = Vec::new();
    let hear = async {
        while answers.len() < asks.len() {
            let Ok(Some(line)) = lines.next_line().await else {
                break;
            };
            let answer = serde_json::from_str::<Value>(&line)
                .ok()
                .filter(|value| match agent {
                    Agent::Pi => pi::is_offer_answer(value),
                    Agent::Claude => claude::is_offer_answer(value),
                });
            answers.extend(answer);
        }
    };
    let in_time = tokio::time::timeout(OFFER_PATIENCE, hear).await.is_ok();
    if !in_time || answers.len() < asks.len() {
        return Err(failed("it didn't answer".into()));
    }
    Ok(match agent {
        Agent::Pi => pi::offer(&answers),
        Agent::Claude => claude::offer(&answers),
    })
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
