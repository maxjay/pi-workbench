//! Runs the user's own Pi as a background process over its RPC protocol.
//!
//! Nothing is installed into Pi: the workbench starts `pi --mode rpc` in the
//! workspace, so Pi uses its normal settings, credentials, extensions, and
//! sessions. Pi edits files on disk; the workbench learns which files from
//! the event stream.

pub mod events;
pub mod preview;
pub mod sessions;
mod tree;

pub use events::{Ask, AskKind, Event};

use anyhow::{Context, Result};
use serde_json::json;
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};
use tree::Tree;

/// How to start Pi. Extra arguments select a provider, model, or session mode.
#[derive(Clone, Debug)]
pub struct Launch {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl Default for Launch {
    fn default() -> Self {
        Self {
            program: "pi".into(),
            args: Vec::new(),
            env: Vec::new(),
        }
    }
}

impl Launch {
    /// `PI_WORKBENCH_PI` overrides the program; `PI_WORKBENCH_PI_ARGS` sets the
    /// whitespace-separated arguments, such as `--provider x --model y`.
    pub fn from_env() -> Self {
        Self::from_settings(None, &[])
    }

    /// Like [`Self::from_env`], starting from a configured program and
    /// arguments; the environment variables still win.
    pub fn from_settings(program: Option<&str>, args: &[String]) -> Self {
        let mut launch = Self::default();
        if let Some(program) = program.filter(|p| !p.trim().is_empty()) {
            launch.program = program.to_owned();
        }
        launch.args = args.to_vec();
        if let Ok(program) = std::env::var("PI_WORKBENCH_PI") {
            launch.program = program;
        }
        if let Ok(args) = std::env::var("PI_WORKBENCH_PI_ARGS") {
            launch.args = args.split_whitespace().map(String::from).collect();
        }
        launch
    }
}

/// Windows needs the shim's extension, which `Command` does not search for.
fn resolve(program: &str) -> PathBuf {
    let path = Path::new(program);
    if !cfg!(windows) || path.extension().is_some() || path.components().count() > 1 {
        return path.to_path_buf();
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .flat_map(|dir| ["exe", "cmd", "bat"].map(|ext| dir.join(program).with_extension(ext)))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| path.to_path_buf())
}

/// Asks Pi something on the side ("by the way"), with a session's history
/// for context but leaving it untouched: the session is forked into a
/// folder of its own that is thrown away after, and Pi answers once, with
/// no tools, and exits. `session` is the session's file, if it has one.
/// Blocks until Pi answers or `timeout` passes.
pub fn ask_aside(
    launch: &Launch,
    cwd: &Path,
    session: Option<&Path>,
    question: &str,
    timeout: Duration,
) -> Result<String> {
    let aside = std::env::temp_dir().join(format!(
        "pi-workbench-btw-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |t| t.as_nanos())
    ));
    std::fs::create_dir_all(&aside).context("Could not make a folder for the side question")?;
    let mut command = Command::new(resolve(&launch.program));
    command
        .args(&launch.args)
        .args(["-p", "--no-tools", "--session-dir"])
        .arg(&aside);
    match session {
        Some(session) => command.arg("--fork").arg(session),
        None => command.arg("--no-session"),
    };
    // The question goes in on stdin, which Pi reads as the prompt: on
    // Windows `pi` is a .cmd shim, which cannot be given an argument with a
    // line break in it, and its command line is short besides.
    command
        .envs(launch.env.iter().map(|(k, v)| (k, v)))
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let result = (|| {
        let mut child = command
            .spawn()
            .with_context(|| format!("Could not start `{}`", launch.program))?;
        // The whole tree: on Windows the child is the `.cmd` shim, with Pi
        // under it, and both must end on a timeout.
        let tree = Tree::adopt(&child);
        if let Some(mut stdin) = child.stdin.take() {
            let question = question.to_string();
            // Written from a thread, so a Pi slow to read cannot stall this
            // one; dropped after, which closes it.
            std::thread::spawn(move || {
                let _ = stdin.write_all(question.as_bytes());
            });
        }
        // Both pipes are drained at once, so neither fills and stalls Pi.
        let drain = |pipe: Option<Box<dyn std::io::Read + Send>>| {
            let (sent, got) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let mut text = String::new();
                if let Some(mut pipe) = pipe {
                    let _ = pipe.read_to_string(&mut text);
                }
                let _ = sent.send(text);
            });
            got
        };
        let answer = drain(child.stdout.take().map(|p| Box::new(p) as _));
        let errors = drain(child.stderr.take().map(|p| Box::new(p) as _));
        let started = std::time::Instant::now();
        let mut timed_out = false;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break Some(status);
            }
            if started.elapsed() >= timeout {
                timed_out = true;
                if let Some(tree) = tree {
                    tree.kill();
                }
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        if timed_out {
            anyhow::bail!("Pi took too long to answer");
        }
        // Anything still holding the pipes open is not waited on for long.
        let collect = |got: std::sync::mpsc::Receiver<String>| {
            got.recv_timeout(Duration::from_secs(2)).unwrap_or_default()
        };
        let (answer, errors) = (collect(answer), collect(errors));
        let answer = answer.trim().to_string();
        if status.is_some_and(|s| s.success()) && !answer.is_empty() {
            Ok(answer)
        } else {
            anyhow::bail!(
                "{}",
                errors
                    .lines()
                    .rev()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("Pi could not answer")
                    .trim()
            )
        }
    })();
    let _ = std::fs::remove_dir_all(&aside);
    result
}

/// A reply to an extension's question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Value(String),
    Confirmed(bool),
    Cancelled,
}

pub struct PiProcess {
    child: Option<Child>,
    tree: Option<Tree>,
    stdin: Option<ChildStdin>,
    next_id: u64,
}

impl PiProcess {
    /// `on_event` runs on a background thread, in stream order, and receives
    /// `Event::Exited` last.
    /// `session` resumes a session file from Pi's store; `None` starts a new one.
    pub fn spawn(
        launch: &Launch,
        cwd: &Path,
        session: Option<&Path>,
        on_event: impl Fn(Event) + Send + 'static,
    ) -> Result<Self> {
        let mut command = Command::new(resolve(&launch.program));
        command.args(["--mode", "rpc"]).args(&launch.args);
        if let Some(session) = session {
            command.arg("--session").arg(session);
        }
        command
            .envs(launch.env.iter().map(|(k, v)| (k, v)))
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = command
            .spawn()
            .with_context(|| format!("Could not start `{}`", launch.program))?;
        let tree = Tree::adopt(&child);
        let stdout = child.stdout.take().context("No stdout")?;
        let stderr = child.stderr.take().context("No stderr")?;
        // Read stderr as it arrives: a full pipe would block Pi mid-run.
        let last_error = Arc::new(Mutex::new(String::new()));
        let errors = {
            let last_error = last_error.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    if !line.trim().is_empty() {
                        *last_error.lock().unwrap() = line.trim().to_string();
                    }
                }
            })
        };
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if line.trim().is_empty() {
                    continue;
                }
                // Unknown or malformed lines are skipped rather than ending the session.
                if let Ok(event) = events::decode(&line) {
                    on_event(event);
                }
            }
            let _ = errors.join();
            let last = std::mem::take(&mut *last_error.lock().unwrap());
            on_event(Event::Exited(if last.is_empty() {
                "Pi stopped".into()
            } else {
                last
            }));
        });
        Ok(Self {
            stdin: child.stdin.take(),
            child: Some(child),
            tree,
            next_id: 0,
        })
    }

    fn send(&mut self, mut command: serde_json::Value) -> Result<()> {
        self.next_id += 1;
        command["id"] = json!(self.next_id.to_string());
        let stdin = self.stdin.as_mut().context("Pi has stopped")?;
        // RPC is strict JSONL: one object per line, LF-terminated.
        writeln!(stdin, "{command}")?;
        stdin.flush().context("Pi has stopped")
    }

    pub fn prompt(&mut self, message: &str) -> Result<()> {
        self.send(json!({ "type": "prompt", "message": message }))
    }

    pub fn abort(&mut self) -> Result<()> {
        self.send(json!({ "type": "abort" }))
    }

    /// Delivered after the current tool calls, before the next model call.
    pub fn steer(&mut self, message: &str) -> Result<()> {
        self.send(json!({ "type": "steer", "message": message }))
    }

    /// Delivered once the run would otherwise finish.
    pub fn follow_up(&mut self, message: &str) -> Result<()> {
        self.send(json!({ "type": "follow_up", "message": message }))
    }

    /// Commands answered by a `response` event carrying the same name.
    pub fn request(&mut self, command: &str) -> Result<()> {
        self.send(json!({ "type": command }))
    }

    pub fn set_model(&mut self, provider: &str, id: &str) -> Result<()> {
        self.send(json!({ "type": "set_model", "provider": provider, "modelId": id }))
    }

    pub fn answer(&mut self, id: &str, answer: Answer) -> Result<()> {
        let mut response = json!({ "type": "extension_ui_response", "id": id });
        match answer {
            Answer::Value(value) => response["value"] = json!(value),
            Answer::Confirmed(confirmed) => response["confirmed"] = json!(confirmed),
            Answer::Cancelled => response["cancelled"] = json!(true),
        }
        let stdin = self.stdin.as_mut().context("Pi has stopped")?;
        writeln!(stdin, "{response}")?;
        stdin.flush().context("Pi has stopped")
    }
}

impl Drop for PiProcess {
    fn drop(&mut self) {
        // Closing stdin asks Pi to exit; kill it if it has not after a grace period.
        self.stdin.take();
        let tree = self.tree.take();
        if let Some(mut child) = self.child.take() {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(2));
                if matches!(child.try_wait(), Ok(None)) {
                    // Closing the job ends the whole tree; kill() only the direct child.
                    if let Some(tree) = tree {
                        tree.kill();
                    }
                    let _ = child.kill();
                }
                let _ = child.wait();
            });
        }
    }
}
