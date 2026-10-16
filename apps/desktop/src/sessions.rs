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
    /// Shared by every folder's sessions in a window, so a key names one
    /// session wherever it moves.
    keys: Rc<Cell<u64>>,
    recent: RwSignal<Vec<SessionSummary>>,
    cue: RwSignal<Option<Cue>>,
    renew: RwSignal<Option<Cue>>,
    moved: RwSignal<Option<Moved>>,
}

/// A session Pi moved into another folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Moved {
    pub session: u64,
    pub to: PathBuf,
    pub serial: u64,
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
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        scope: Scope,
        root: PathBuf,
        launch: Launch,
        keys: Rc<Cell<u64>>,
        documents: Rc<Documents>,
        files_changed: impl Fn(bool) + 'static,
        snapshot: impl Fn(Box<dyn FnOnce(Option<String>)>) + 'static,
        status: RwSignal<String>,
    ) -> Rc<Self> {
        let recent = RwSignal::new(Vec::new());
        let cue = RwSignal::new(None::<Cue>);
        let renew = RwSignal::new(None::<Cue>);
        let moved = RwSignal::new(None::<Moved>);
        let serial = Rc::new(Cell::new(0u64));
        let (renew_serial, moved_serial) = (serial.clone(), serial.clone());
        Rc::new_cyclic(|this: &std::rc::Weak<Self>| {
            let this = this.clone();
            Self {
                scope,
                hooks: Rc::new(Hooks {
                    renew: Box::new(move |session, goal| {
                        renew_serial.set(renew_serial.get() + 1);
                        renew.set(Some(Cue {
                            session,
                            action: goal,
                            serial: renew_serial.get(),
                        }));
                    }),
                    cue: Box::new(move |session, action| {
                        serial.set(serial.get() + 1);
                        cue.set(Some(Cue {
                            session,
                            action,
                            serial: serial.get(),
                        }));
                    }),
                    moved: Box::new(move |session, to| {
                        moved_serial.set(moved_serial.get() + 1);
                        moved.set(Some(Moved {
                            session,
                            to,
                            serial: moved_serial.get(),
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
                keys,
                recent,
                cue,
                renew,
                moved,
            }
        })
    }

    /// How Pi is started.
    pub fn launch(&self) -> &Launch {
        &self.launch
    }

    /// The latest `/kill`: the session to replace, with what to do next in
    /// `action`.
    pub fn renew(&self) -> ReadSignal<Option<Cue>> {
        self.renew.read_only()
    }

    /// The latest session Pi moved into another folder.
    pub fn moved(&self) -> ReadSignal<Option<Moved>> {
        self.moved.read_only()
    }

    /// Takes an open session away, Pi still running, for another folder's
    /// sessions to adopt.
    pub fn detach(&self, key: u64) -> Option<Rc<Session>> {
        let session = self.find(key)?;
        self.close(key);
        Some(session)
    }

    /// Takes in a session Pi moved here from another folder, in front.
    pub fn adopt(&self, session: Rc<Session>) {
        session.rebind(
            self.root.clone(),
            self.documents.clone(),
            self.hooks.clone(),
        );
        let key = session.key;
        Effect::batch(|| {
            self.open.update(|open| open.push(session));
            self.active.set(Some(key));
        });
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

    /// Tracked: a session, by key.
    pub fn find_tracked(&self, key: u64) -> Option<Rc<Session>> {
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
        let key = self.keys.get() + 1;
        self.keys.set(key);
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

    fn sessions_in(scope: Scope, root: &str, keys: Rc<Cell<u64>>) -> Rc<Sessions> {
        let status = RwSignal::new(String::new());
        let documents = Documents::new(scope, root.into(), status);
        Sessions::new(
            scope,
            root.into(),
            Launch::default(),
            keys,
            documents,
            |_| {},
            |done| done(None),
            status,
        )
    }

    fn with_sessions(test: impl FnOnce(Rc<Sessions>)) {
        let scope = Scope::new();
        scope.enter(|| test(sessions_in(scope, "/", Rc::default())));
        scope.dispose();
    }

    #[test]
    fn a_session_moves_to_another_folders_sessions_in_front() {
        let scope = Scope::new();
        scope.enter(|| {
            let keys = Rc::new(Cell::new(0));
            let (here, there) = (
                sessions_in(scope, "/here", keys.clone()),
                sessions_in(scope, "/there", keys),
            );
            let stays = here.add(None).key;
            let moving = here.add(None).key;
            let waiting = there.add(None).key;
            assert_ne!(waiting, moving, "keys are unique across folders");
            let session = here.detach(moving).unwrap();
            there.adopt(session);
            assert!(here.find(moving).is_none());
            assert_eq!(here.active_key().get_untracked(), Some(stays));
            let moved = there.find(moving).unwrap();
            assert_eq!(moved.root(), PathBuf::from("/there"));
            assert_eq!(there.active_key().get_untracked(), Some(moving));
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
