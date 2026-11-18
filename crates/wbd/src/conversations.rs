//! Conversations with an agent: one process each while you are talking,
//! and one file each in wbd's data folder, so a conversation can be opened
//! again and carried on. The file holds what the page shows. The agent
//! keeps its own memory of the conversation, under the same id.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use protocol::{Agent, Conversation, ConversationView, Entry, Event};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;
use tokio::sync::mpsc::UnboundedSender;

use crate::agent;
use crate::error::Error;
use crate::files;
use crate::fold::{Folded, Update};

/// How often what an agent says goes to the page. A reply streaming word
/// by word is then a couple of dozen events a second, not hundreds.
const FLUSH: Duration = Duration::from_millis(40);
/// A conversation's title is no longer than this.
const TITLE: usize = 80;

#[derive(Clone)]
pub struct Conversations {
    dir: PathBuf,
    live: Arc<Mutex<HashMap<String, Arc<Live>>>>,
    /// wbd's one writer: lines for the page.
    out: UnboundedSender<String>,
}

/// A conversation whose agent is running.
struct Live {
    agent: Agent,
    stdin: tokio::sync::Mutex<ChildStdin>,
    folded: Mutex<Folded>,
    /// What each open question's driver needs to send its answer.
    asks: Mutex<HashMap<String, Value>>,
}

/// A conversation as its file holds it.
#[derive(Serialize, Deserialize)]
struct Saved {
    state: Conversation,
    entries: Vec<Entry>,
}

impl Conversations {
    pub fn new(data_dir: &Path, out: UnboundedSender<String>) -> Self {
        Conversations {
            dir: data_dir.join("conversations"),
            live: Arc::default(),
            out,
        }
    }

    /// The conversations in a folder, the latest first.
    pub fn list(&self, cwd: &str) -> Vec<Conversation> {
        let mut list: Vec<Conversation> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| read(&entry.path()))
            .map(|saved| saved.state)
            .filter(|state| state.cwd == cwd)
            .collect();
        // One that is running is further on than its file.
        let live = self.live.lock().expect("not poisoned");
        for state in &mut list {
            if let Some(live) = live.get(&state.id) {
                *state = live.folded.lock().expect("not poisoned").state.clone();
            }
        }
        list.sort_by_key(|state| std::cmp::Reverse(state.updated));
        list
    }

    /// Starts a conversation in a folder with what you wrote.
    pub async fn start(
        &self,
        agent: Agent,
        cwd: &str,
        text: &str,
    ) -> Result<ConversationView, Error> {
        if !Path::new(cwd).is_dir() {
            return Err(Error::NotFound(format!("no folder at {cwd}")));
        }
        let state = Conversation {
            id: new_id()?,
            agent,
            cwd: cwd.to_string(),
            title: title(text),
            updated: now(),
            busy: false,
            ask: None,
            stopped: None,
        };
        let id = state.id.clone();
        let live = self.run(Folded::new(state, Vec::new()), false)?;
        self.say(&id, &live, text).await?;
        Ok(live.folded.lock().expect("not poisoned").view())
    }

    /// A whole conversation, running or saved.
    pub fn open(&self, id: &str) -> Result<ConversationView, Error> {
        if let Some(live) = self.get(id) {
            return Ok(live.folded.lock().expect("not poisoned").view());
        }
        let saved = self.saved(id)?;
        Ok(ConversationView {
            state: saved.state,
            entries: saved.entries,
        })
    }

    /// Says something in a conversation, starting its agent again if it isn't running.
    pub async fn send(&self, id: &str, text: &str) -> Result<(), Error> {
        let live = match self.get(id) {
            Some(live) => live,
            None => {
                let saved = self.saved(id)?;
                self.run(Folded::new(saved.state, saved.entries), true)?
            }
        };
        self.say(id, &live, text).await
    }

    /// Stops what the agent is doing now.
    pub async fn stop(&self, id: &str) -> Result<(), Error> {
        let live = self
            .get(id)
            .ok_or_else(|| Error::NotFound(format!("conversation {id} isn't running")))?;
        write(&live, agent::stop(live.agent)).await
    }

    /// Answers what the agent asked. `answer` is nothing when you dismissed it.
    pub async fn answer(&self, id: &str, ask: &str, answer: Option<&str>) -> Result<(), Error> {
        let live = self
            .get(id)
            .ok_or_else(|| Error::NotFound(format!("conversation {id} isn't running")))?;
        let line = {
            let mut folded = live.folded.lock().expect("not poisoned");
            let asked = folded.state.ask.clone().filter(|asked| asked.id == ask);
            let Some(asked) = asked else {
                return Err(Error::NotFound("that question is no longer open".into()));
            };
            let detail = live
                .asks
                .lock()
                .expect("not poisoned")
                .remove(ask)
                .unwrap_or_default();
            folded.answered();
            agent::answer(live.agent, &asked, answer, &detail)
        };
        self.tell(id, &live);
        write(&live, line).await
    }

    fn get(&self, id: &str) -> Option<Arc<Live>> {
        self.live.lock().expect("not poisoned").get(id).cloned()
    }

    fn saved(&self, id: &str) -> Result<Saved, Error> {
        // An id is a file name here, so it must not be a path.
        let plain = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        plain
            .then(|| read(&self.dir.join(format!("{id}.json"))))
            .flatten()
            .ok_or_else(|| Error::NotFound(format!("no conversation {id}")))
    }

    /// Writes what you said, to the page, to the file and to the agent.
    async fn say(&self, id: &str, live: &Arc<Live>, text: &str) -> Result<(), Error> {
        live.folded.lock().expect("not poisoned").you(text, now());
        self.tell(id, live);
        self.save(live);
        write(live, agent::prompt(live.agent, text)).await
    }

    /// Starts the agent's process for a conversation, and hears it until it ends.
    fn run(&self, folded: Folded, resume: bool) -> Result<Arc<Live>, Error> {
        let (id, agent, cwd) = (
            folded.state.id.clone(),
            folded.state.agent,
            folded.state.cwd.clone(),
        );
        let mut child = agent::command(agent, &id, Path::new(&cwd), resume)
            .spawn()
            .map_err(|err| Error::Agent(format!("couldn't start {}: {err}", name(agent))))?;
        let live = Arc::new(Live {
            agent,
            stdin: tokio::sync::Mutex::new(child.stdin.take().expect("stdin is piped")),
            folded: Mutex::new(folded),
            asks: Mutex::default(),
        });
        self.live
            .lock()
            .expect("not poisoned")
            .insert(id.clone(), live.clone());

        // The last thing it says on stderr is why it stopped, if it stops.
        let last_error = Arc::new(Mutex::new(String::new()));
        let stderr = child.stderr.take().expect("stderr is piped");
        tokio::spawn({
            let last_error = last_error.clone();
            async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if !line.trim().is_empty() {
                        *last_error.lock().expect("not poisoned") = line;
                    }
                }
            }
        });

        let stdout = child.stdout.take().expect("stdout is piped");
        tokio::spawn({
            let (all, live) = (self.clone(), live.clone());
            async move {
                let mut lines = BufReader::new(stdout).lines();
                let mut flush = tokio::time::interval(FLUSH);
                loop {
                    tokio::select! {
                        line = lines.next_line() => {
                            let Ok(Some(line)) = line else { break };
                            if all.hear(&live, &line) {
                                all.tell(&id, &live);
                                all.save(&live);
                            }
                        }
                        _ = flush.tick() => all.tell(&id, &live),
                    }
                }
                let _ = child.wait().await;
                let reason = match last_error.lock().expect("not poisoned").as_str() {
                    "" => format!("{} stopped", name(agent)),
                    last => format!("{} stopped: {last}", name(agent)),
                };
                live.folded.lock().expect("not poisoned").stopped(reason);
                all.tell(&id, &live);
                all.save(&live);
                all.live.lock().expect("not poisoned").remove(&id);
            }
        });
        Ok(live)
    }

    /// Takes in one line from the agent. True when it is worth telling the
    /// page and saving now: the turn ended, or the agent asks something.
    fn hear(&self, live: &Live, line: &str) -> bool {
        let mut now_matters = false;
        let mut folded = live.folded.lock().expect("not poisoned");
        for update in agent::decode(live.agent, line) {
            if let Update::Ask { ask, detail } = &update {
                live.asks
                    .lock()
                    .expect("not poisoned")
                    .insert(ask.id.clone(), detail.clone());
            }
            now_matters |= matches!(update, Update::Ask { .. } | Update::TurnEnd { .. });
            folded.apply(update, now());
        }
        now_matters
    }

    /// Tells the page what changed, if anything did.
    fn tell(&self, id: &str, live: &Live) {
        let changes = live.folded.lock().expect("not poisoned").take();
        if !changes.is_empty() {
            let _ = self.out.send(protocol::event(&Event::Conversation {
                id: id.to_string(),
                changes,
            }));
        }
    }

    fn save(&self, live: &Live) {
        let (id, text) = {
            let folded = live.folded.lock().expect("not poisoned");
            let saved = Saved {
                state: folded.state.clone(),
                entries: folded.entries.clone(),
            };
            (
                folded.state.id.clone(),
                serde_json::to_string(&saved).expect("a conversation serializes"),
            )
        };
        // A conversation that can't be saved still runs. It just can't be opened again.
        if std::fs::create_dir_all(&self.dir).is_ok() {
            let _ = files::write_whole(&self.dir.join(format!("{id}.json")), &text, None);
        }
    }

    /// Ends every agent's process.
    pub fn shutdown(&self) {
        self.live.lock().expect("not poisoned").clear();
    }
}

async fn write(live: &Live, mut line: String) -> Result<(), Error> {
    line.push('\n');
    let mut stdin = live.stdin.lock().await;
    let sent = async {
        stdin.write_all(line.as_bytes()).await?;
        stdin.flush().await
    };
    sent.await
        .map_err(|err| Error::Agent(format!("couldn't reach {}: {err}", name(live.agent))))
}

fn read(path: &Path) -> Option<Saved> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn name(agent: Agent) -> &'static str {
    match agent {
        Agent::Pi => "Pi",
        Agent::Claude => "Claude Code",
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// What you first wrote, as a title: its first line, cut short.
fn title(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    match line.char_indices().nth(TITLE) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_string(),
    }
}

/// A new id, as both agents want one: a UUID, from the kernel.
fn new_id() -> Result<String, Error> {
    let path = Path::new("/proc/sys/kernel/random/uuid");
    std::fs::read_to_string(path)
        .map(|id| id.trim().to_string())
        .map_err(|err| Error::io(path, err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_title_is_the_first_line_cut_short() {
        assert_eq!(title("Fix the login\nand more"), "Fix the login");
        let long = "é".repeat(TITLE + 5);
        let cut = title(&long);
        assert_eq!(cut.chars().count(), TITLE + 1);
        assert!(cut.ends_with('…'));
    }
}
