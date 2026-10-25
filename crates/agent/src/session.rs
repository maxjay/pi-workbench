//! One live session: a `pi --mode rpc` process and the transcript of what
//! it said. A task reads Pi's lines into the transcript and sends the
//! workbench what changed, gathered up so a fast stream of words costs a
//! message every [`FLUSH`], not one per word. Another listens on the
//! session's socket for the `ask_user` tool's questions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use protocol::{Ask, AskKind, AskReply, Event, FromAgent, Session, SessionSummary};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{mpsc, oneshot};

use crate::error::Error;
use crate::pi::{self, PiEvent};
use crate::question;
use crate::transcript::Transcript;

/// Where everything wb-agent says goes.
pub type Out = mpsc::UnboundedSender<FromAgent>;

const FLUSH: Duration = Duration::from_millis(40);

/// How long to wait for Pi to answer a command wb-agent needs the reply to.
const REPLY: Duration = Duration::from_secs(10);

pub struct Launch {
    pub pi: PathBuf,
    pub cwd: PathBuf,
    /// A saved session to resume.
    pub file: Option<PathBuf>,
    /// The `ask_user` extension, to load into Pi.
    pub extension: Option<PathBuf>,
    pub done: bool,
}

type Waiting<T> = Mutex<HashMap<String, oneshot::Sender<T>>>;

pub struct Live {
    pub id: String,
    transcript: Mutex<Transcript>,
    stdin: tokio::sync::Mutex<Option<ChildStdin>>,
    /// Stop Pi once the run settles.
    closing: AtomicBool,
    out: Out,
    next: AtomicU64,
    /// Commands waiting on Pi's reply, by the id they were sent with.
    replies: Waiting<Result<Value, String>>,
    /// `ask_user` questions waiting on an answer, by their ask's id.
    questions: Waiting<AskReply>,
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
        // Without its socket, the question tool would have no one to ask.
        let socket = question::socket_path(&id);
        let listener = launch
            .extension
            .as_ref()
            .and_then(|extension| match listen(&socket) {
                Ok(listener) => Some((extension, listener)),
                Err(err) => {
                    eprintln!("wb-agent: no question tool for {id}: {err}");
                    None
                }
            });
        if let Some((extension, _)) = &listener {
            command.arg("--extension").arg(extension);
            command.env("WB_ASK", &socket);
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
        transcript.state.done = launch.done;
        let live = Arc::new(Live {
            id,
            transcript: Mutex::new(transcript),
            stdin: tokio::sync::Mutex::new(child.stdin.take()),
            closing: AtomicBool::new(false),
            out,
            next: AtomicU64::new(1),
            replies: Mutex::default(),
            questions: Mutex::default(),
        });
        let asking = listener.map(|(_, listener)| {
            let live = Arc::downgrade(&live);
            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    tokio::spawn(asked(live.clone(), stream));
                }
            })
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
            if let Some(asking) = asking {
                asking.abort();
            }
            let _ = std::fs::remove_file(question::socket_path(&reader.id));
            reader.questions.lock().expect("not poisoned").clear();
            reader.replies.lock().expect("not poisoned").clear();
            reader.edit(|t| t.exited(reason));
            if reader.closing.load(Ordering::SeqCst) {
                on_close();
            }
        });
        Ok(live)
    }

    async fn on_event(&self, event: PiEvent) {
        if let PiEvent::Response {
            id: Some(id),
            error,
            data,
            ..
        } = &event
        {
            let waiting = self.replies.lock().expect("not poisoned").remove(id);
            if let Some(waiting) = waiting {
                let _ = waiting.send(match error {
                    Some(error) => Err(error.clone()),
                    None => Ok(data.clone()),
                });
                return;
            }
        }
        let settled = event == PiEvent::AgentSettled;
        self.transcript.lock().expect("not poisoned").apply(event);
        if settled {
            if self.closing.load(Ordering::SeqCst) {
                self.stop().await;
            } else {
                let _ = self.refresh().await;
            }
        }
    }

    /// Asks Pi again for what can change under a session: the model, the
    /// name, the thinking level and the levels the model takes, and usage.
    pub async fn refresh(&self) -> Result<(), Error> {
        for command in [
            "get_state",
            "get_available_thinking_levels",
            "get_session_stats",
        ] {
            self.send(pi::command(command)).await?;
        }
        Ok(())
    }

    /// A new id for something sent to Pi, unique in this session.
    pub fn next_id(&self) -> String {
        format!("wb{}", self.next.fetch_add(1, Ordering::SeqCst))
    }

    /// Sends a command and waits for Pi's reply to it.
    pub async fn request(&self, command: Value) -> Result<Value, Error> {
        let id = self.next_id();
        let (tx, rx) = oneshot::channel();
        self.replies
            .lock()
            .expect("not poisoned")
            .insert(id.clone(), tx);
        self.send(pi::with_id(command, &id)).await?;
        match tokio::time::timeout(REPLY, rx).await {
            Ok(Ok(Ok(data))) => Ok(data),
            Ok(Ok(Err(error))) => Err(Error::Pi(error)),
            Ok(Err(_)) => Err(Error::Pi(self.stopped_reason())),
            Err(_) => {
                self.replies.lock().expect("not poisoned").remove(&id);
                Err(Error::Pi("Pi didn't answer in time".into()))
            }
        }
    }

    /// Answers what the session is asking: an `ask_user` question, or an
    /// extension's dialog.
    pub async fn answer(&self, ask: &str, reply: AskReply) -> Result<(), Error> {
        let question = self.questions.lock().expect("not poisoned").remove(ask);
        match question {
            Some(question) => {
                let _ = question.send(reply.clone());
            }
            None => self.send(pi::answer(ask, &reply)).await?,
        }
        self.edit(|t| t.answered_with(&reply));
        Ok(())
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
            asking: transcript.state.ask.is_some(),
            stopped: transcript.state.stopped.is_some(),
            done: transcript.state.done,
        }
    }

    pub fn is_done(&self) -> bool {
        self.transcript.lock().expect("not poisoned").state.done
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

/// A socket for the question tool, at `path`, replacing one left behind.
fn listen(path: &Path) -> std::io::Result<UnixListener> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let _ = std::fs::remove_file(path);
    UnixListener::bind(path)
}

/// One `ask_user` call: show its questions, and send back the answers, or
/// give up if the tool stops waiting first.
async fn asked(live: Weak<Live>, stream: UnixStream) {
    let (read, mut write) = stream.into_split();
    let mut lines = BufReader::new(read).lines();
    let Ok(Some(line)) = lines.next_line().await else {
        return;
    };
    let Some(questions) = question::questions(&line) else {
        let _ = write
            .write_all(question::reply(&AskReply::Cancelled).as_bytes())
            .await;
        return;
    };
    let Some(session) = live.upgrade() else {
        return;
    };
    let id = format!("q{}", session.next_id());
    let (tx, rx) = oneshot::channel();
    session
        .questions
        .lock()
        .expect("not poisoned")
        .insert(id.clone(), tx);
    let title = questions[0].question.clone();
    session.edit(|t| {
        t.apply(PiEvent::Ask(Ask {
            id: id.clone(),
            title,
            message: None,
            kind: AskKind::Questions { questions },
        }))
    });
    drop(session);

    tokio::select! {
        reply = rx => {
            if let Ok(reply) = reply {
                let _ = write.write_all(question::reply(&reply).as_bytes()).await;
            }
        }
        // The tool stopped waiting: the run was stopped.
        _ = lines.next_line() => {
            if let Some(session) = live.upgrade() {
                session.questions.lock().expect("not poisoned").remove(&id);
                session.edit(|t| {
                    if t.state.ask.as_ref().is_some_and(|ask| ask.id == id) {
                        t.answered();
                    }
                });
            }
        }
    }
}
