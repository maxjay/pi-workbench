//! One live session: a `pi --mode rpc` process and the transcript of what
//! it said. A task reads Pi's lines into the transcript and sends the
//! workbench what changed, gathered up so a fast stream of words costs a
//! message every [`FLUSH`], not one per word.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use protocol::{Event, FromAgent, Session, SessionSummary};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::mpsc;

use crate::error::Error;
use crate::pi::{self, PiEvent};
use crate::transcript::Transcript;

/// Where everything wb-agent says goes.
pub type Out = mpsc::UnboundedSender<FromAgent>;

const FLUSH: Duration = Duration::from_millis(40);

pub struct Launch {
    pub pi: PathBuf,
    pub cwd: PathBuf,
    /// A saved session to resume.
    pub file: Option<PathBuf>,
}

pub struct Live {
    pub id: String,
    transcript: Mutex<Transcript>,
    stdin: tokio::sync::Mutex<Option<ChildStdin>>,
    /// Stop Pi once the run settles.
    closing: AtomicBool,
    out: Out,
}

impl Live {
    /// Starts Pi. `on_close` runs when it has stopped after [`Live::close`].
    pub fn spawn(
        id: String,
        launch: Launch,
        out: Out,
        on_close: impl FnOnce() + Send + 'static,
    ) -> Result<Arc<Live>, Error> {
        let mut command = Command::new(&launch.pi);
        command.args(["--mode", "rpc"]);
        if let Some(file) = &launch.file {
            command.arg("--session").arg(file);
        }
        let mut child = command
            .current_dir(&launch.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|err| Error::Pi(format!("couldn't start {}: {err}", launch.pi.display())))?;
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");

        let mut transcript = Transcript::new(&launch.cwd);
        transcript.state.file = launch.file.map(|file| file.display().to_string());
        let live = Arc::new(Live {
            id,
            transcript: Mutex::new(transcript),
            stdin: tokio::sync::Mutex::new(child.stdin.take()),
            closing: AtomicBool::new(false),
            out,
        });

        let last_error = Arc::new(Mutex::new(String::new()));
        tokio::spawn({
            let last_error = last_error.clone();
            let id = live.id.clone();
            async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    eprintln!("pi {id}: {line}");
                    if !line.trim().is_empty() {
                        *last_error.lock().expect("not poisoned") = line;
                    }
                }
            }
        });

        let reader = live.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            let mut tick = tokio::time::interval(FLUSH);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    line = lines.next_line() => match line {
                        Ok(Some(line)) => {
                            if let Some(event) = pi::decode(&line) {
                                reader.on_event(event).await;
                            }
                        }
                        _ => break,
                    },
                    _ = tick.tick() => reader.flush(),
                }
            }
            let status = child.wait().await;
            let last = std::mem::take(&mut *last_error.lock().expect("not poisoned"));
            let reason = match (last.is_empty(), status) {
                (false, _) => format!("Pi stopped: {last}"),
                (true, Ok(status)) if !status.success() => format!("Pi stopped ({status})"),
                _ => "Pi stopped".to_string(),
            };
            reader.edit(|t| t.exited(reason));
            if reader.closing.load(Ordering::SeqCst) {
                on_close();
            }
        });
        Ok(live)
    }

    async fn on_event(&self, event: PiEvent) {
        let settled = event == PiEvent::AgentSettled;
        self.transcript.lock().expect("not poisoned").apply(event);
        if settled {
            if self.closing.load(Ordering::SeqCst) {
                self.stop().await;
            } else {
                // The model, the name or the thinking level may have changed.
                let _ = self.send(pi::command("get_state")).await;
            }
        }
    }

    /// Writes one command to Pi.
    pub async fn send(&self, command: Value) -> Result<(), Error> {
        let mut stdin = self.stdin.lock().await;
        let stdin = stdin
            .as_mut()
            .ok_or_else(|| Error::Pi(self.stopped_reason()))?;
        let mut line = command.to_string();
        line.push('\n');
        let written = stdin.write_all(line.as_bytes()).await;
        match written.and(stdin.flush().await) {
            Ok(()) => Ok(()),
            Err(_) => Err(Error::Pi(self.stopped_reason())),
        }
    }

    fn stopped_reason(&self) -> String {
        self.transcript
            .lock()
            .expect("not poisoned")
            .state
            .stopped
            .clone()
            .unwrap_or_else(|| "Pi has stopped".into())
    }

    /// Changes the transcript and tells the workbench right away.
    pub fn edit(&self, change: impl FnOnce(&mut Transcript)) {
        change(&mut self.transcript.lock().expect("not poisoned"));
        self.flush();
    }

    fn flush(&self) {
        let changes = self.transcript.lock().expect("not poisoned").changes();
        for change in changes {
            let _ = self.out.send(FromAgent::Event(Event::Session {
                id: self.id.clone(),
                change,
            }));
        }
    }

    pub fn snapshot(&self) -> Session {
        self.flush();
        self.transcript
            .lock()
            .expect("not poisoned")
            .session(&self.id)
    }

    pub fn summary(&self, updated: u64) -> SessionSummary {
        let transcript = self.transcript.lock().expect("not poisoned");
        SessionSummary {
            id: Some(self.id.clone()),
            file: transcript.state.file.clone(),
            title: transcript.state.title.clone(),
            cwd: transcript.state.cwd.clone(),
            updated,
            busy: transcript.state.busy,
        }
    }

    pub fn is_busy(&self) -> bool {
        self.transcript.lock().expect("not poisoned").state.busy
    }

    pub fn is_stopped(&self) -> bool {
        self.transcript
            .lock()
            .expect("not poisoned")
            .state
            .stopped
            .is_some()
    }

    pub fn file(&self) -> Option<String> {
        self.transcript
            .lock()
            .expect("not poisoned")
            .state
            .file
            .clone()
    }

    pub fn cwd(&self) -> PathBuf {
        PathBuf::from(&self.transcript.lock().expect("not poisoned").state.cwd)
    }

    /// Stops Pi now if it's idle, or once the run settles.
    pub async fn close(&self) {
        self.closing.store(true, Ordering::SeqCst);
        if !self.is_busy() {
            self.stop().await;
        }
    }

    /// Stops Pi now, even mid-run.
    pub async fn quit(&self) {
        self.closing.store(true, Ordering::SeqCst);
        self.stop().await;
    }

    /// Closing Pi's stdin asks it to finish and exit.
    async fn stop(&self) {
        self.stdin.lock().await.take();
    }
}
