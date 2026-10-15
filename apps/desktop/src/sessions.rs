//! The open Pi sessions, which one is in front, and the recent ones Pi has stored.
use crate::{
    documents::Documents,
    session::{Hooks, Session},
};
use floem::{
    ext_event::create_ext_action,
    prelude::*,
    reactive::{Effect, ReadSignal, Scope},
};
use pi_client::{
    Launch,
    sessions::{self, SessionSummary},
};
use std::{cell::Cell, path::PathBuf, rc::Rc};

pub struct Sessions {
    scope: Scope,
    root: PathBuf,
    launch: Launch,
    documents: Rc<Documents>,
    hooks: Rc<Hooks>,
    status: RwSignal<String>,
    open: RwSignal<Vec<Rc<Session>>>,
    active: RwSignal<Option<u64>>,
    next_key: Cell<u64>,
    recent: RwSignal<Vec<SessionSummary>>,
    cue: RwSignal<Option<Cue>>,
}

/// Something a session asked the pet to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cue {
    pub session: u64,
    pub action: String,
    /// Counts up, so the same cue twice is two cues.
    pub serial: u64,
}

impl Sessions {
    pub fn new(
        scope: Scope,
        root: PathBuf,
        launch: Launch,
        documents: Rc<Documents>,
        files_changed: impl Fn(bool) + 'static,
        snapshot: impl Fn(Box<dyn FnOnce(Option<String>)>) + 'static,
        status: RwSignal<String>,
    ) -> Rc<Self> {
        let recent = RwSignal::new(Vec::new());
        let cue = RwSignal::new(None::<Cue>);
        let serial = Cell::new(0u64);
        Rc::new_cyclic(|this: &std::rc::Weak<Self>| {
            let this = this.clone();
            Self {
                scope,
                hooks: Rc::new(Hooks {
                    cue: Box::new(move |session, action| {
                        serial.set(serial.get() + 1);
                        cue.set(Some(Cue {
                            session,
                            action,
                            serial: serial.get(),
                        }));
                    }),
                    files_changed: Box::new(files_changed),
                    snapshot: Box::new(snapshot),
                    settled: Box::new(move || {
                        if let Some(sessions) = this.upgrade() {
                            sessions.refresh_recent();
                        }
                    }),
                }),
                root,
                launch,
                documents,
                status,
                open: RwSignal::new(Vec::new()),
                active: RwSignal::new(None),
                next_key: Cell::new(0),
                recent,
                cue,
            }
        })
    }

    /// The latest thing a session asked the pet to do.
    pub fn cue(&self) -> ReadSignal<Option<Cue>> {
        self.cue.read_only()
    }
    pub fn open(&self) -> ReadSignal<Vec<Rc<Session>>> {
        self.open.read_only()
    }

    /// Stored sessions that are not open, newest first.
    pub fn recent(&self) -> Vec<SessionSummary> {
        let open: Vec<_> = self
            .open
            .with(|open| open.iter().filter_map(|s| s.file().get()).collect());
        self.recent.with(|recent| {
            recent
                .iter()
                .filter(|s| !open.contains(&s.path))
                .cloned()
                .collect()
        })
    }

    pub fn active_key(&self) -> ReadSignal<Option<u64>> {
        self.active.read_only()
    }

    /// Tracked.
    pub fn active(&self) -> Option<Rc<Session>> {
        let key = self.active.get()?;
        self.open
            .with(|open| open.iter().find(|s| s.key == key).cloned())
    }

    pub fn find(&self, key: u64) -> Option<Rc<Session>> {
        self.open
            .with_untracked(|open| open.iter().find(|s| s.key == key).cloned())
    }

    pub fn active_untracked(&self) -> Option<Rc<Session>> {
        let key = self.active.get_untracked()?;
        self.open
            .with_untracked(|open| open.iter().find(|s| s.key == key).cloned())
    }

    fn add(&self, resume: Option<&SessionSummary>) -> Rc<Session> {
        let key = self.next_key.get() + 1;
        self.next_key.set(key);
        let session = Session::new(
            self.scope,
            key,
            self.root.clone(),
            self.launch.clone(),
            self.documents.clone(),
            self.hooks.clone(),
            resume,
        );
        // One update, so views showing the active session rebuild once.
        Effect::batch(|| {
            self.open.update(|open| open.push(session.clone()));
            self.active.set(Some(key));
        });
        session
    }

    fn start(&self, session: &Session) {
        if let Err(err) = session.start() {
            self.status.set(format!("{err:#}"));
        }
    }

    pub fn create(&self) -> Rc<Session> {
        let session = self.add(None);
        self.start(&session);
        session
    }

    /// Brings a stored session to the front, starting Pi on it if needed.
    pub fn resume(&self, summary: &SessionSummary) -> Rc<Session> {
        let open = self.open.with_untracked(|open| {
            open.iter()
                .find(|s| s.file().get_untracked().as_ref() == Some(&summary.path))
                .cloned()
        });
        match open {
            Some(session) => {
                self.active.set(Some(session.key));
                session
            }
            None => {
                let session = self.add(Some(summary));
                self.start(&session);
                session
            }
        }
    }

    pub fn select(&self, key: u64) {
        self.active.set(Some(key));
    }

    /// Selects the `index`th open session, as Ctrl+1..9 do.
    pub fn select_index(&self, index: usize) {
        if let Some(key) = self
            .open
            .with_untracked(|open| open.get(index).map(|s| s.key))
        {
            self.active.set(Some(key));
        }
    }

    pub fn cycle(&self, forward: bool) {
        let keys: Vec<u64> = self
            .open
            .with_untracked(|open| open.iter().map(|s| s.key).collect());
        let Some(current) = self.active.get_untracked() else {
            return;
        };
        let Some(at) = keys.iter().position(|&k| k == current) else {
            return;
        };
        let next = if forward {
            (at + 1) % keys.len()
        } else {
            (at + keys.len() - 1) % keys.len()
        };
        self.active.set(Some(keys[next]));
    }

    /// Stops Pi for the session; its history stays in Pi's store.
    pub fn close(&self, key: u64) {
        let index = self
            .open
            .with_untracked(|open| open.iter().position(|s| s.key == key));
        let Some(index) = index else { return };
        Effect::batch(|| {
            self.open.update(|open| {
                open.remove(index);
            });
            if self.active.get_untracked() == Some(key) {
                let next = self.open.with_untracked(|open| {
                    open.get(index.min(open.len().saturating_sub(1)))
                        .map(|s| s.key)
                });
                self.active.set(next);
            }
        });
    }

    /// Re-reads Pi's session store off the UI thread.
    pub fn refresh_recent(&self) {
        let recent = self.recent;
        let root = self.root.clone();
        let finish = create_ext_action(self.scope, move |list| recent.set(list));
        std::thread::spawn(move || finish(sessions::list(&root)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_sessions(test: impl FnOnce(Rc<Sessions>)) {
        let scope = Scope::new();
        scope.enter(|| {
            let status = RwSignal::new(String::new());
            let documents = Documents::new(scope, "/".into(), status);
            let sessions = Sessions::new(
                scope,
                "/".into(),
                Launch::default(),
                documents,
                |_| {},
                |done| done(None),
                status,
            );
            test(sessions);
        });
        scope.dispose();
    }

    #[test]
    fn cycling_and_closing_keep_a_session_in_front() {
        with_sessions(|sessions| {
            let keys: Vec<_> = (0..3).map(|_| sessions.add(None).key).collect();
            assert_eq!(sessions.active_key().get(), Some(keys[2]));
            sessions.cycle(true);
            assert_eq!(sessions.active_key().get(), Some(keys[0]));
            sessions.cycle(false);
            assert_eq!(sessions.active_key().get(), Some(keys[2]));
            sessions.select_index(1);
            assert_eq!(sessions.active_key().get(), Some(keys[1]));
            sessions.close(keys[1]);
            assert_eq!(sessions.active_key().get(), Some(keys[2]));
            sessions.close(keys[2]);
            sessions.close(keys[0]);
            assert_eq!(sessions.active_key().get(), None);
        });
    }
}
