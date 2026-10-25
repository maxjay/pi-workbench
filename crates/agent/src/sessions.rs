//! The live sessions, by id, and starting, resuming and listing them. New
//! scratchpads each get a folder of their own under the scratchpad folder.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use protocol::{Change, Event, FromAgent, Session, SessionSummary};
use serde_json::json;

use crate::done::Done;
use crate::error::Error;
use crate::pi::{self, PiEvent};
use crate::session::{Launch, Live, Out};
use crate::transcript::Transcript;
use crate::{question, store};

#[derive(Clone)]
pub struct Sessions {
    live: Arc<Mutex<HashMap<String, Arc<Live>>>>,
    next: Arc<AtomicU64>,
    out: Out,
    /// The program to run: `WB_PI`, or `pi` on PATH.
    pi: PathBuf,
    agent_dir: PathBuf,
    scratch_dir: PathBuf,
    done: Arc<Done>,
    /// The `ask_user` extension, if it could be written out.
    extension: Option<PathBuf>,
}

impl Sessions {
    pub fn new(out: Out, agent_dir: PathBuf, scratch_dir: PathBuf, data_dir: PathBuf) -> Self {
        let extension = match question::install(&data_dir.join("extensions")) {
            Ok(path) => Some(path),
            Err(err) => {
                eprintln!("wb-agent: no question tool: {err}");
                None
            }
        };
        Sessions {
            live: Arc::default(),
            next: Arc::new(AtomicU64::new(1)),
            out,
            pi: std::env::var_os("WB_PI").map_or_else(|| "pi".into(), PathBuf::from),
            agent_dir,
            scratch_dir,
            done: Arc::new(Done::new(data_dir.join("done.json"))),
            extension,
        }
    }

    /// The program each session runs: `WB_PI`, or `pi`.
    pub fn pi(&self) -> &Path {
        &self.pi
    }

    pub fn get(&self, id: &str) -> Result<Arc<Live>, Error> {
        self.live
            .lock()
            .expect("not poisoned")
            .get(id)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("no live session {id}")))
    }

    /// Starts Pi in `cwd`, or in a new scratchpad folder, and sends `text`.
    pub async fn start(&self, cwd: Option<String>, text: Option<String>) -> Result<Session, Error> {
        let cwd = match cwd {
            Some(cwd) => PathBuf::from(cwd),
            None => new_folder(&self.scratch_dir)?,
        };
        let id = format!("s{}", self.next.fetch_add(1, Ordering::SeqCst));
        let live = self.spawn(id, cwd, None)?;
        live.refresh().await?;
        if let Some(text) = text.filter(|text| !text.trim().is_empty()) {
            live.edit(|t| t.sent(&text));
            live.send(pi::prompt(&text)).await?;
        }
        Ok(live.snapshot())
    }

    /// Resumes a saved session, or finds it if it's already live.
    pub async fn open(&self, file: &str) -> Result<Session, Error> {
        let id = match self.find(file) {
            Some(live) if !live.is_stopped() => return Ok(live.snapshot()),
            // Pi stopped under it: start it again, as the same session.
            Some(live) => live.id.clone(),
            None => format!("s{}", self.next.fetch_add(1, Ordering::SeqCst)),
        };
        let cwd = store::header_cwd(Path::new(file))
            .ok_or_else(|| Error::NotFound(format!("{file} isn't a Pi session")))?;
        let live = self.spawn(id, cwd, Some(PathBuf::from(file)))?;
        live.refresh().await?;
        live.send(pi::command("get_messages")).await?;
        Ok(live.snapshot())
    }

    /// A saved session, from its file, as it would open: for looking at
    /// without starting Pi.
    pub fn read(&self, file: &str) -> Result<Session, Error> {
        if let Some(live) = self.find(file) {
            return Ok(live.snapshot());
        }
        let saved = store::read(Path::new(file))
            .ok_or_else(|| Error::NotFound(format!("{file} isn't a Pi session")))?;
        let mut t = Transcript::new(&saved.cwd);
        let respond = |command: &str, data| PiEvent::Response {
            id: None,
            command: command.into(),
            error: None,
            data,
        };
        let (provider, model) = match saved.model.as_deref().and_then(|m| m.split_once('/')) {
            Some((provider, model)) => (Some(provider), Some(model)),
            None => (None, None),
        };
        t.apply(respond(
            "get_state",
            json!({
                "sessionFile": file,
                "sessionName": saved.name,
                "thinkingLevel": saved.thinking,
                "model": { "provider": provider, "id": model },
            }),
        ));
        t.apply(respond(
            "get_messages",
            json!({ "messages": saved.messages }),
        ));
        t.state.usage = Some(saved.usage);
        t.state.done = self.done.contains(file);
        Ok(t.session(""))
    }

    /// Marks a session done, or not. A live one marked done stops: there's
    /// nothing more for Pi to do in it until you write there again.
    pub async fn set_done(&self, file: &str, done: bool) -> Result<(), Error> {
        self.done.set(file, done)?;
        if let Some(live) = self.find(file) {
            live.edit(|t| {
                t.state.done = done;
                t.touch();
            });
            if done {
                self.close(&live.id).await?;
            }
        }
        Ok(())
    }

    /// Writing in a session marked done brings it back.
    pub async fn wake(&self, live: &Live) -> Result<(), Error> {
        match live.file() {
            Some(file) if live.is_done() => self.set_done(&file, false).await,
            _ => Ok(()),
        }
    }

    fn find(&self, file: &str) -> Option<Arc<Live>> {
        self.live
            .lock()
            .expect("not poisoned")
            .values()
            .find(|live| live.file().as_deref() == Some(file))
            .cloned()
    }

    fn spawn(&self, id: String, cwd: PathBuf, file: Option<PathBuf>) -> Result<Arc<Live>, Error> {
        let done = file
            .as_ref()
            .is_some_and(|file| self.done.contains(&file.display().to_string()));
        let launch = Launch {
            pi: self.pi.clone(),
            cwd,
            file,
            extension: self.extension.clone(),
            done,
        };
        let on_close = {
            let (live, out, id) = (self.live.clone(), self.out.clone(), id.clone());
            move || {
                live.lock().expect("not poisoned").remove(&id);
                let _ = out.send(FromAgent::Event(Event::Session {
                    id,
                    change: Change::Closed,
                }));
            }
        };
        let live = Live::spawn(id.clone(), launch, self.out.clone(), on_close)?;
        self.live
            .lock()
            .expect("not poisoned")
            .insert(id, live.clone());
        Ok(live)
    }

    /// Stops Pi for a session once it's idle; its file stays.
    pub async fn close(&self, id: &str) -> Result<(), Error> {
        let live = self.get(id)?;
        if live.is_stopped() {
            // Already gone: nothing will report it closing.
            self.live.lock().expect("not poisoned").remove(id);
            let _ = self.out.send(FromAgent::Event(Event::Session {
                id: id.to_string(),
                change: Change::Closed,
            }));
            return Ok(());
        }
        live.close().await;
        Ok(())
    }

    /// Stops Pi in sessions that have been idle for `after`, checking now
    /// and then. Nothing is lost: the session stays in Pi's file, and
    /// writing in it starts Pi again.
    pub fn stop_idle(&self, after: Duration) {
        let sessions = self.clone();
        let every = (after / 2).clamp(Duration::from_millis(200), Duration::from_secs(60));
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(every);
            loop {
                tick.tick().await;
                let idle: Vec<Arc<Live>> = sessions
                    .live
                    .lock()
                    .expect("not poisoned")
                    .values()
                    .filter(|live| live.idle_for().is_some_and(|idle| idle >= after))
                    .cloned()
                    .collect();
                for live in idle {
                    live.close().await;
                }
            }
        });
    }

    /// Asks every Pi to exit, and waits a little for them to.
    pub async fn shutdown(&self) {
        let all: Vec<Arc<Live>> = self
            .live
            .lock()
            .expect("not poisoned")
            .drain()
            .map(|(_, live)| live)
            .collect();
        for live in &all {
            live.quit().await;
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while all.iter().any(|live| !live.is_stopped()) && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    /// Scratchpads: the live ones first, then those in Pi's store.
    pub fn list(&self) -> Vec<SessionSummary> {
        let now = millis(SystemTime::now());
        let live: Vec<SessionSummary> = self
            .live
            .lock()
            .expect("not poisoned")
            .values()
            .filter(|live| live.cwd().starts_with(&self.scratch_dir))
            .map(|live| live.summary(now))
            .collect();
        let done = self.done.all();
        let saved = store::under(&self.agent_dir, &self.scratch_dir)
            .into_iter()
            .filter(|saved| !live.iter().any(|live| live.file == saved.file))
            .map(|saved| SessionSummary {
                done: saved.file.as_ref().is_some_and(|file| done.contains(file)),
                ..saved
            });
        live.iter().cloned().chain(saved).collect()
    }
}

pub fn millis(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// A new folder under `root`, named for when it was made.
fn new_folder(root: &Path) -> Result<PathBuf, Error> {
    let stamp = stamp(SystemTime::now());
    let mut folder = root.join(&stamp);
    let mut n = 2;
    while folder.exists() {
        folder = root.join(format!("{stamp}-{n}"));
        n += 1;
    }
    std::fs::create_dir_all(&folder).map_err(|err| Error::io(&folder, err))?;
    Ok(folder)
}

/// `2026-10-24-224107`, in UTC.
fn stamp(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Days to a civil date: Howard Hinnant's algorithm.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}-{:02}{:02}{:02}",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn stamps_folders_with_the_date() {
        let time = UNIX_EPOCH + Duration::from_secs(1_792_881_667);
        assert_eq!(stamp(time), "2026-10-24-224107");
        assert_eq!(stamp(UNIX_EPOCH), "1970-01-01-000000");
    }

    #[test]
    fn new_folders_never_clash() {
        let root = tempfile::tempdir().unwrap();
        let first = new_folder(root.path()).unwrap();
        let second = new_folder(root.path()).unwrap();
        assert_ne!(first, second);
        assert!(second.is_dir());
    }
}
