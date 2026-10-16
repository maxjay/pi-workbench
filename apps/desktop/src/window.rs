//! A window and the folders open in it. Each folder is a workspace: its
//! files, panes, terminals and Pi sessions. The window shows one at a time,
//! the folder of the session in front, so it follows wherever that session
//! is: picked from the sidebar, started in another folder, or moved there
//! by Pi itself (an extension can switch Pi to a session in another folder).
use crate::workspace::{Integrations, WindowLink, Workbench};
use floem::{
    ext_event::create_ext_action,
    prelude::*,
    reactive::{Effect, ReadSignal, Scope},
    window::WindowId,
};
use pi_client::{
    Launch,
    sessions::{self, SessionSummary},
};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    time::SystemTime,
};

pub struct Window {
    pub id: WindowId,
    scope: Scope,
    me: Weak<Window>,
    launch: Launch,
    /// Session keys, unique across the window's folders.
    session_keys: Rc<Cell<u64>>,
    /// Open folders, in the order they were opened.
    workspaces: RwSignal<Vec<Rc<Workbench>>>,
    /// The folder shown.
    active: RwSignal<PathBuf>,
    /// Folders Pi has sessions in, newest first.
    folders: RwSignal<Vec<(PathBuf, SystemTime)>>,
    /// Stored sessions of folders that are not open, once asked for.
    stored: RwSignal<HashMap<PathBuf, Vec<SessionSummary>>>,
    /// Folders open in the sidebar.
    pub expanded: RwSignal<HashSet<PathBuf>>,
    /// Folders that have finished closing, while the window closes.
    closed: RefCell<HashSet<PathBuf>>,
}

impl Window {
    pub fn new(id: WindowId, launch: Launch) -> Rc<Self> {
        Rc::new_cyclic(|me| Self {
            id,
            scope: Scope::current(),
            me: me.clone(),
            launch,
            session_keys: Rc::default(),
            workspaces: RwSignal::new(Vec::new()),
            active: RwSignal::new(PathBuf::new()),
            folders: RwSignal::new(Vec::new()),
            stored: RwSignal::new(HashMap::new()),
            expanded: RwSignal::new(HashSet::new()),
            closed: RefCell::new(HashSet::new()),
        })
    }

    pub fn workspaces(&self) -> ReadSignal<Vec<Rc<Workbench>>> {
        self.workspaces.read_only()
    }

    /// The folder shown.
    pub fn active(&self) -> ReadSignal<PathBuf> {
        self.active.read_only()
    }

    /// An open folder's workspace.
    pub fn workspace(&self, root: &Path) -> Option<Rc<Workbench>> {
        self.workspaces
            .with_untracked(|open| open.iter().find(|w| w.root() == root).cloned())
    }

    /// Tracked: an open folder's workspace.
    pub fn workspace_tracked(&self, root: &Path) -> Option<Rc<Workbench>> {
        self.workspaces
            .with(|open| open.iter().find(|w| w.root() == root).cloned())
    }

    /// The workspace shown.
    pub fn shown(&self) -> Option<Rc<Workbench>> {
        self.workspace(&self.active.get_untracked())
    }

    pub fn folders_untracked(&self) -> Vec<PathBuf> {
        Effect::untrack(|| self.folders())
    }

    /// The folders for the sidebar: the open ones, as opened, then the
    /// others Pi has sessions in, newest first. Tracked.
    pub fn folders(&self) -> Vec<PathBuf> {
        let mut folders: Vec<PathBuf> = self
            .workspaces
            .with(|open| open.iter().map(|w| w.root().to_path_buf()).collect());
        self.folders.with(|known| {
            for (folder, _) in known {
                let folder = folder.canonicalize().unwrap_or_else(|_| folder.clone());
                if !folders.contains(&folder) {
                    folders.push(folder);
                }
            }
        });
        folders
    }

    /// When a session last changed in a folder Pi knows. Tracked.
    pub fn last_used(&self, folder: &Path) -> Option<SystemTime> {
        self.folders.with(|known| {
            known
                .iter()
                .find(|(known, _)| sessions::belongs(known, folder))
                .map(|(_, when)| *when)
        })
    }

    /// Stored sessions of a folder that is not open, once read. Tracked.
    pub fn stored(&self, folder: &Path) -> Option<Vec<SessionSummary>> {
        self.stored.with(|stored| stored.get(folder).cloned())
    }

    /// Reads a folder's stored sessions, off the UI thread.
    pub fn load_stored(&self, folder: PathBuf) {
        let stored = self.stored;
        let finish = create_ext_action(self.scope, move |(folder, list)| {
            stored.update(|stored| {
                stored.insert(folder, list);
            })
        });
        std::thread::spawn(move || {
            let list = sessions::list(&folder);
            finish((folder, list))
        });
    }

    /// Re-reads which folders Pi has sessions in, off the UI thread.
    pub fn refresh_folders(&self) {
        let folders = self.folders;
        let finish = create_ext_action(self.scope, move |list| folders.set(list));
        std::thread::spawn(move || finish(sessions::folders()));
    }

    /// A folder's workspace, opened (and its saved layout restored) if it
    /// was not.
    pub fn open(&self, folder: &Path) -> anyhow::Result<Rc<Workbench>> {
        let (root, layout) = crate::workspace_paths(folder)?;
        if let Some(open) = self.workspace(&root) {
            return Ok(open);
        }
        let scope = self.scope.create_child();
        let me = self.me.clone();
        let done = root.clone();
        let integrations = Integrations {
            pi: self.launch.clone(),
            session_keys: self.session_keys.clone(),
        };
        let state = scope.enter(|| {
            Workbench::new(root.clone(), layout, integrations, move || {
                if let Some(window) = me.upgrade() {
                    window.closed_one(done.clone());
                }
            })
        });
        let (go, known) = (self.me.clone(), self.me.clone());
        state.link_window(WindowLink {
            go: Box::new(move |folder, draft| {
                if let Some(window) = go.upgrade() {
                    window.go(&folder, draft);
                }
            }),
            folders: Box::new(move || {
                known
                    .upgrade()
                    .map(|window| window.folders_untracked())
                    .unwrap_or_default()
            }),
        });
        // Pi moving one of its sessions to another folder takes the window
        // there too.
        let (moved, from, me) = (
            state.sessions.moved(),
            Rc::downgrade(&state),
            self.me.clone(),
        );
        scope.enter(|| {
            Effect::new(move |last: Option<u64>| {
                let Some(moved) = moved.get() else {
                    return last.unwrap_or_default();
                };
                if last != Some(moved.serial)
                    && let (Some(window), Some(from)) = (me.upgrade(), from.upgrade())
                {
                    Effect::untrack(|| window.move_session(&from, moved.session, &moved.to));
                }
                moved.serial
            });
        });
        state.restore();
        self.workspaces.update(|open| open.push(state.clone()));
        Ok(state)
    }

    /// Shows a folder, opening it if need be; `None` (and a message in the
    /// status bar) if it cannot be opened.
    pub fn show(&self, folder: &Path) -> Option<Rc<Workbench>> {
        match self.open(folder) {
            Ok(state) => {
                let root = state.root().to_path_buf();
                if self.active.get_untracked() != root {
                    self.active.set(root.clone());
                }
                self.expanded.update(|open| {
                    open.insert(root);
                });
                Some(state)
            }
            Err(err) => {
                if let Some(shown) = self.shown() {
                    shown.report(format!("{err:#}"));
                }
                None
            }
        }
    }

    /// Brings a folder forward; with `Some(draft)`, in a new session there
    /// with the draft in its composer.
    pub fn go(&self, folder: &Path, draft: Option<String>) {
        let Some(state) = self.show(folder) else {
            return;
        };
        if let Some(draft) = draft {
            state.when_restored(move |state| {
                let session = state.sessions.create();
                state.show_session(session.key);
                if !draft.is_empty() {
                    session.set_draft(&draft);
                }
            });
        }
    }

    /// Brings an open session forward, in its folder.
    pub fn show_session(&self, folder: &Path, key: u64) {
        if let Some(state) = self.show(folder) {
            state.when_restored(move |state| state.show_session(key));
        }
    }

    /// Resumes a stored session, in its folder.
    pub fn resume(&self, folder: &Path, summary: SessionSummary) {
        if let Some(state) = self.show(folder) {
            state.when_restored(move |state| {
                let session = state.sessions.resume(&summary);
                state.show_session(session.key);
            });
        }
    }

    /// Pi moved a session from one folder to another: the session goes to
    /// that folder's workspace, Pi carrying on as it was, and the window
    /// follows it if it was in front.
    fn move_session(&self, from: &Rc<Workbench>, key: u64, to: &Path) {
        let Ok(target) = self.open(to) else {
            from.report(format!(
                "Pi moved to {}, which cannot be opened",
                to.display()
            ));
            return;
        };
        if Rc::ptr_eq(from, &target) {
            return;
        }
        let in_front = self.active.get_untracked() == from.root()
            && from.sessions.active_key().get_untracked() == Some(key);
        let popped = from.pets.out.get_untracked() == Some(key);
        let Some(session) = from.sessions.detach(key) else {
            return;
        };
        from.close_session(key);
        if popped {
            from.pets.pop_in();
        }
        let title = session.title().get_untracked();
        target.sessions.adopt(session);
        from.sessions.refresh_recent();
        target.sessions.refresh_recent();
        let folder = target.root().file_name().map_or_else(
            || target.root().display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        target.report(format!("Pi moved \u{201c}{title}\u{201d} to {folder}"));
        if in_front {
            self.show(target.root());
        }
        target.when_restored(move |state| {
            state.show_session(key);
            if popped {
                state.pets.pop_out(key);
            }
        });
        self.refresh_folders();
    }

    /// Closes the window once every folder has: each saves its layout and
    /// asks about unsaved files, shown while it does.
    pub fn request_close(&self) {
        self.closed.borrow_mut().clear();
        self.close_next();
    }

    fn close_next(&self) {
        let next = self.workspaces.with_untracked(|open| {
            open.iter()
                .find(|w| !self.closed.borrow().contains(w.root()))
                .cloned()
        });
        match next {
            Some(state) => {
                if self.active.get_untracked() != state.root() {
                    self.active.set(state.root().to_path_buf());
                }
                state.request_quit();
            }
            None => floem::close_window(self.id),
        }
    }

    fn closed_one(&self, root: PathBuf) {
        self.closed.borrow_mut().insert(root);
        self.close_next();
    }
}
