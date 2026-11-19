//! Conversations with an agent: one process each while you are talking,
//! and one file each in wbd's data folder, so a conversation can be opened
//! again and carried on. The file holds what the page shows. The agent
//! keeps its own memory of the conversation, under the same id. So a
//! session the agent made outside the workbench is a conversation too: it
//! is listed, and gets its file when it is first opened. When the agent's
//! own session holds more than that file, the session is what is shown.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use protocol::{Agent, Ask, Conversation, ConversationView, Entry, Event};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::ChildStdin;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::oneshot;

use crate::agent::{self, name};
use crate::error::Error;
use crate::files;
use crate::fold::{Folded, Update};
use crate::{repos, wb};

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
    /// What every agent's environment gets, so it can run `wb`. Empty without it.
    env: Arc<Vec<(String, String)>>,
}

/// A conversation whose agent is running.
struct Live {
    agent: Agent,
    stdin: tokio::sync::Mutex<ChildStdin>,
    folded: Mutex<Folded>,
    /// What each open question's driver needs to send its answer.
    asks: Mutex<HashMap<String, Value>>,
    /// The questions the workbench itself asked here, and where each answer goes.
    own: Mutex<HashMap<String, oneshot::Sender<Option<String>>>>,
    /// What the agent was told before anything you wrote.
    brief: Option<String>,
}

/// A conversation as its file holds it.
#[derive(Serialize, Deserialize)]
struct Saved {
    state: Conversation,
    entries: Vec<Entry>,
    /// What the agent is told each time it starts, before anything you write.
    #[serde(default)]
    brief: Option<String>,
}

impl Conversations {
    pub fn new(data_dir: &Path, out: UnboundedSender<String>, env: Vec<(String, String)>) -> Self {
        Conversations {
            dir: data_dir.join("conversations"),
            live: Arc::default(),
            out,
            env: Arc::new(env),
        }
    }

    /// The folder a running conversation is in.
    pub fn cwd(&self, id: &str) -> Option<String> {
        let live = self.get(id)?;
        let folded = live.folded.lock().expect("not poisoned");
        Some(folded.state.cwd.clone())
    }

    /// Asks you something in a running conversation, from the workbench itself and not from
    /// the agent. It shows where the agent's own questions show. What comes back gives your
    /// answer: nothing when you dismissed it.
    pub fn ask(&self, id: &str, ask: Ask) -> Result<oneshot::Receiver<Option<String>>, Error> {
        let live = self
            .get(id)
            .ok_or_else(|| Error::NotFound(format!("conversation {id} isn't running")))?;
        let (answer, answered) = oneshot::channel();
        live.own
            .lock()
            .expect("not poisoned")
            .insert(ask.id.clone(), answer);
        let detail = Value::Null;
        live.folded
            .lock()
            .expect("not poisoned")
            .apply(Update::Ask { ask, detail }, now());
        self.tell(id, &live);
        Ok(answered)
    }

    /// Takes back a question the workbench asked, when nothing waits for its answer any more.
    pub fn withdraw(&self, id: &str, ask: &str) {
        let Some(live) = self.get(id) else { return };
        if live.own.lock().expect("not poisoned").remove(ask).is_none() {
            return;
        }
        {
            let mut folded = live.folded.lock().expect("not poisoned");
            if folded.state.ask.as_ref().is_some_and(|open| open.id == ask) {
                folded.answered();
            }
        }
        self.tell(id, &live);
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
        // The agents' own sessions here that have no file yet: the ones
        // made outside the workbench. Opening one gives it a file.
        let own = agent::sessions(cwd)
            .into_iter()
            .filter(|state| plain(&state.id) && !self.path(&state.id).exists())
            .map(|state| Conversation {
                title: title(&state.title),
                ..state
            });
        list.extend(own);
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
        model: Option<String>,
        effort: Option<String>,
        brief: Option<String>,
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
            model,
            effort,
        };
        let id = state.id.clone();
        let live = self.run(Folded::new(state, Vec::new()), false, brief)?;
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
                self.run(Folded::new(saved.state, saved.entries), true, saved.brief)?
            }
        };
        self.say(id, &live, text).await
    }

    /// Picks a conversation's model and effort. A running agent changes
    /// now. One that isn't running starts with them the next time.
    pub async fn set(
        &self,
        id: &str,
        model: Option<String>,
        effort: Option<String>,
    ) -> Result<(), Error> {
        let Some(live) = self.get(id) else {
            let mut saved = self.saved(id)?;
            (saved.state.model, saved.state.effort) = (model, effort);
            self.store(&saved);
            return Ok(());
        };
        let lines = agent::set(live.agent, model.as_deref(), effort.as_deref());
        live.folded
            .lock()
            .expect("not poisoned")
            .picked(model, effort);
        self.tell(id, &live);
        self.save(&live);
        for line in lines {
            write(&live, line).await?;
        }
        Ok(())
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
            // One the workbench asked is answered to the workbench, not to the agent.
            if let Some(own) = live.own.lock().expect("not poisoned").remove(ask) {
                folded.answered();
                drop(folded);
                self.tell(id, &live);
                let _ = own.send(answer.map(str::to_string));
                return Ok(());
            }
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

    /// A conversation from its file. One that has no file may be a session
    /// the agent kept by itself: that is taken in.
    fn saved(&self, id: &str) -> Result<Saved, Error> {
        let none = || Error::NotFound(format!("no conversation {id}"));
        if !plain(id) {
            return Err(none());
        }
        match read(&self.path(id)) {
            Some(saved) => Ok(self.caught_up(saved)),
            None => self.take_in(id)?.ok_or_else(none),
        }
    }

    /// Reads an agent's own session into a conversation and saves it, so
    /// it is the workbench's own from then on. The agent knows it by the
    /// same id, so it carries on where it was. Nothing, when no agent keeps
    /// such a session. One that is kept and can't be read is an error.
    fn take_in(&self, id: &str) -> Result<Option<Saved>, Error> {
        let Some(view) = agent::session(id, None).map_err(unread)? else {
            return Ok(None);
        };
        let saved = Saved {
            state: Conversation {
                title: title(&view.state.title),
                ..view.state
            },
            entries: view.entries,
            brief: None,
        };
        self.store(&saved);
        Ok(Some(saved))
    }

    /// A conversation with what the agent's own session holds, when that
    /// is more than its file holds, and saved so. Only Pi's sessions are
    /// read, so only a conversation with Pi can be behind.
    fn caught_up(&self, saved: Saved) -> Saved {
        if saved.state.agent != Agent::Pi {
            return saved;
        }
        // A session that can't be read leaves the file as what is shown.
        let own = agent::session(&saved.state.id, Some(&saved.state.cwd));
        let (saved, changed) = longer(saved, own.ok().flatten());
        if changed {
            self.store(&saved);
        }
        saved
    }

    fn path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// Writes what you said, to the page, to the file and to the agent.
    async fn say(&self, id: &str, live: &Arc<Live>, text: &str) -> Result<(), Error> {
        live.folded.lock().expect("not poisoned").you(text, now());
        self.tell(id, live);
        self.save(live);
        write(live, agent::prompt(live.agent, text)).await
    }

    /// Starts the agent's process for a conversation, and hears it until it ends.
    fn run(&self, folded: Folded, resume: bool, brief: Option<String>) -> Result<Arc<Live>, Error> {
        let state = &folded.state;
        let (id, agent, cwd) = (state.id.clone(), state.agent, Path::new(&state.cwd));
        let (model, effort) = (state.model.as_deref(), state.effort.as_deref());
        // In a workspace that takes repos the agent is told about `wb`, each time it starts.
        let note = (!self.env.is_empty() && repos::takes(cwd)).then_some(wb::NOTE);
        let told = match (brief.as_deref(), note) {
            (Some(brief), Some(note)) => Some(format!("{brief}\n\n{note}")),
            (brief, note) => brief.or(note).map(str::to_string),
        };
        let mut child = agent::command(agent, &id, cwd, resume, model, effort, told.as_deref())
            .envs(self.env.iter().cloned())
            .env("WB_CONVERSATION", &id)
            .spawn()
            .map_err(|err| Error::Agent(format!("couldn't start {}: {err}", name(agent))))?;
        let live = Arc::new(Live {
            agent,
            stdin: tokio::sync::Mutex::new(child.stdin.take().expect("stdin is piped")),
            folded: Mutex::new(folded),
            asks: Mutex::default(),
            own: Mutex::default(),
            brief,
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
        let saved = {
            let folded = live.folded.lock().expect("not poisoned");
            Saved {
                state: folded.state.clone(),
                entries: folded.entries.clone(),
                brief: live.brief.clone(),
            }
        };
        self.store(&saved);
    }

    fn store(&self, saved: &Saved) {
        let text = serde_json::to_string(saved).expect("a conversation serializes");
        let path = self.path(&saved.state.id);
        // A conversation that can't be saved still runs. It just can't be opened again.
        if std::fs::create_dir_all(&self.dir).is_ok() {
            let _ = files::write_whole(&path, &text, None);
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

/// Why an agent's own session wasn't taken in.
fn unread(why: String) -> Error {
    Error::Agent(format!("couldn't read the agent's own session: {why}"))
}

/// A conversation's file, or the agent's own session of it where that
/// holds more, and whether it is the session. It holds more when the
/// session went on outside the workbench, and when the file was made by a
/// workbench that read less of the session. What you picked is kept.
fn longer(saved: Saved, own: Option<ConversationView>) -> (Saved, bool) {
    let Some(own) = own.filter(|own| own.entries.len() > saved.entries.len()) else {
        return (saved, false);
    };
    let updated = own.state.updated.max(saved.state.updated);
    let caught_up = Saved {
        state: Conversation {
            updated,
            ..saved.state
        },
        entries: own.entries,
        brief: saved.brief,
    };
    (caught_up, true)
}

/// An id is a file name here, so it must not be a path. Letters, digits,
/// dashes, dots and lines under are what Pi lets an id of your own hold,
/// with a letter or a digit first and last.
fn plain(id: &str) -> bool {
    let ends = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric());
    ends(id.chars().next())
        && ends(id.chars().last())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// What you first wrote, as a title: its first line, cut short. What a tab put in front of it,
/// to tell the agent what you were on, is not what you wrote and is left out.
fn title(text: &str) -> String {
    let own = match text.starts_with("<about ") {
        true => text.split_once("</about>").map_or(text, |(_, own)| own),
        false => text,
    };
    let line = own
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
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
        assert_eq!(
            title(
                "<about where=\"lines:a.ts:1-2\" under=\"yes\">\na.ts\n</about>\n\nWhy is this here?"
            ),
            "Why is this here?"
        );
        let long = "é".repeat(TITLE + 5);
        let cut = title(&long);
        assert_eq!(cut.chars().count(), TITLE + 1);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn an_id_is_one_pi_takes_and_never_a_path() {
        assert!(plain("01a1f3cd-ba84-7488-a4b8-0ac77e9a050d"));
        assert!(plain("my_own.id-1"));
        assert!(plain("7"));
        for id in [
            "", ".", "..", ".hidden", "a/b", "a\\b", "ends.", "-a", "a b",
        ] {
            assert!(!plain(id), "{id}");
        }
    }

    #[test]
    fn the_agent_s_own_session_is_shown_when_it_holds_more() {
        let state = |updated: u64, model: Option<&str>| Conversation {
            id: "s1".into(),
            agent: Agent::Pi,
            cwd: "/code".into(),
            title: "Fix it".into(),
            updated,
            busy: false,
            ask: None,
            stopped: None,
            model: model.map(str::to_string),
            effort: None,
        };
        let said = |texts: &[&str]| -> Vec<Entry> {
            let each = |text: &&str| Entry::Text {
                text: text.to_string(),
            };
            texts.iter().map(each).collect()
        };
        let file = |texts: &[&str]| Saved {
            state: state(5, Some("anthropic/haiku")),
            entries: said(texts),
            brief: Some("Be short".into()),
        };
        let own = |texts: &[&str]| ConversationView {
            state: state(9, None),
            entries: said(texts),
        };

        // The session went on outside the workbench, or the file was cut short.
        let (saved, changed) = longer(file(&["one"]), Some(own(&["one", "two"])));
        assert!(changed);
        assert_eq!(saved.entries, said(&["one", "two"]));
        assert_eq!(saved.state.updated, 9);
        assert_eq!(saved.state.model.as_deref(), Some("anthropic/haiku"));
        assert_eq!(saved.brief.as_deref(), Some("Be short"));

        // The file holds as much, or the agent keeps no such session.
        let (saved, changed) = longer(file(&["one", "note"]), Some(own(&["one", "two"])));
        assert!(!changed);
        assert_eq!(saved.entries, said(&["one", "note"]));
        assert!(!longer(file(&["one"]), None).1);
    }
}
