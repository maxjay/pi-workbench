//! Runs the user's own Pi as a background process over its RPC protocol.
//!
//! Nothing is installed into Pi: the workbench starts `pi --mode rpc` in the
//! workspace, so Pi uses its normal settings, credentials, extensions, and
//! sessions. Pi edits files on disk; the workbench learns which files from
//! the event stream.

pub mod events;
pub mod preview;

pub use events::Event;

use anyhow::{Context, Result};
use serde_json::json;
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    time::Duration,
};

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
    /// `PI_WORKBENCH_PI` overrides the program; `PI_WORKBENCH_PI_ARGS` adds
    /// whitespace-separated arguments, such as `--provider x --model y`.
    pub fn from_env() -> Self {
        let mut launch = Self::default();
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

pub struct PiProcess {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
    next_id: u64,
}

impl PiProcess {
    /// `on_event` runs on a background thread, in stream order, and receives
    /// `Event::Exited` last.
    pub fn spawn(
        launch: &Launch,
        cwd: &Path,
        on_event: impl Fn(Event) + Send + 'static,
    ) -> Result<Self> {
        let mut command = Command::new(resolve(&launch.program));
        command
            .args(["--mode", "rpc"])
            .args(&launch.args)
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
        let stdout = child.stdout.take().context("No stdout")?;
        let mut stderr = child.stderr.take().context("No stderr")?;
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
            let mut errors = String::new();
            let _ = stderr.read_to_string(&mut errors);
            let last = errors.lines().rev().find(|l| !l.trim().is_empty());
            on_event(Event::Exited(last.unwrap_or("Pi stopped").trim().into()));
        });
        Ok(Self {
            stdin: child.stdin.take(),
            child: Some(child),
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
}

impl Drop for PiProcess {
    fn drop(&mut self) {
        // Closing stdin asks Pi to exit; kill it if it has not after a grace period.
        self.stdin.take();
        if let Some(mut child) = self.child.take() {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(2));
                if matches!(child.try_wait(), Ok(None)) {
                    let _ = child.kill();
                }
                let _ = child.wait();
            });
        }
    }
}
