//! A window and the folders open in it. Each folder is a workspace: its
//! files, panes, terminals and Pi sessions. The window shows one at a time,
//! the folder of the session in front, so it follows wherever that session
//! is: picked from the sidebar, started in another folder, or moved there
//! by Pi itself (an extension can switch Pi to a session in another folder).
//!
//! Only what was opened is listed: the folders opened in the window and the
//! sessions opened in each, which come back as they were left.
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
};
use workbench_core::layout;

pub struct Window {
    pub id: WindowId,
    scope: Scope,
    me: Weak<Window>,
    launch: Launch,
    /// Session keys, unique across the window's folders.
    session_keys: Rc<Cell<u64>>,
    /// Folders whose workspaces are loaded, in the order they were loaded.
    workspaces: RwSignal<Vec<Rc<Workbench>>>,
    /// Each loaded folder's scope, disposed when the folder is closed.
    scopes: RefCell<HashMap<PathBuf, Scope>>,
    /// The folder shown.
    active: RwSignal<PathBuf>,
    /// The folders opened in the window, in the rail's order, loaded or
    /// not: a folder loads when it is first shown.
    opened: RwSignal<Vec<PathBuf>>,
    /// The sessions open in folders not loaded yet, as they were left.
    stored: RwSignal<HashMap<PathBuf, Vec<SessionSummary>>>,
    /// Folders folded away in the sidebar.
    pub collapsed: RwSignal<HashSet<PathBuf>>,
    /// Folders that have finished closing, while the window closes.
    closed: RefCell<HashSet<PathBuf>>,
    /// The folder shown when the window began closing: it shows each folder
    /// in turn as it closes, which is not what to come back to.
    closing_from: Rc<RefCell<Option<PathBuf>>>,
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
            scopes: RefCell::new(HashMap::new()),
            active: RwSignal::new(PathBuf::new()),
            opened: RwSignal::new(Vec::new()),
            stored: RwSignal::new(HashMap::new()),
            collapsed: RwSignal::new(HashSet::new()),
            closed: RefCell::new(HashSet::new()),
            closing_from: Rc::default(),
        })
    }

    /// Opens what the window had open last time, showing `folder` if given,
    /// else the folder shown last, else `fallback`. What is open is saved as
    /// it changes.
    pub fn start(&self, folder: Option<PathBuf>, fallback: PathBuf) -> Option<Rc<Workbench>> {
        let saved = layout::window_state_path()
            .map(|path| layout::load_window(&path))
            .unwrap_or_default();
        let opened: Vec<PathBuf> = saved
            .folders
            .iter()
            .filter_map(|folder| folder.canonicalize().ok())
            .filter(|folder| folder.is_dir())
            .fold(Vec::new(), |mut opened, folder| {
                if !opened.contains(&folder) {
                    opened.push(folder);
                }
                opened
            });
        self.opened.set(opened.clone());
        let first = folder
            .or_else(|| saved.active.filter(|active| active.is_dir()))
            .unwrap_or(fallback);
        let shown = self.show(&first);
        for folder in opened {
            if self.workspace(&folder).is_none() {
                self.load_stored(folder);
            }
        }
        let (opened, active, closing_from) = (self.opened, self.active, self.closing_from.clone());
        Effect::new(move |_| {
            let (folders, shown) = (opened.get(), active.get());
            let shown = closing_from.borrow().clone().unwrap_or(shown);
            let state = layout::WindowState {
                folders,
                active: Some(shown).filter(|a| !a.as_os_str().is_empty()),
            };
            if let Ok(path) = layout::window_state_path() {
                let _ = layout::save_window(&path, &state);
            }
        });
        shown
    }

    pub fn workspaces(&self) -> ReadSignal<Vec<Rc<Workbench>>> {
        self.workspaces.read_only()
    }

    /// The folder shown.
    pub fn active(&self) -> ReadSignal<PathBuf> {
        self.active.read_only()
    }

    /// A loaded folder's workspace.
    pub fn workspace(&self, root: &Path) -> Option<Rc<Workbench>> {
        self.workspaces
            .with_untracked(|open| open.iter().find(|w| w.root() == root).cloned())
    }

    /// Tracked: a loaded folder's workspace.
    pub fn workspace_tracked(&self, root: &Path) -> Option<Rc<Workbench>> {
        self.workspaces
            .with(|open| open.iter().find(|w| w.root() == root).cloned())
    }

    /// The workspace shown.
    pub fn shown(&self) -> Option<Rc<Workbench>> {
        self.workspace(&self.active.get_untracked())
    }

    pub fn folders_untracked(&self) -> Vec<PathBuf> {
        self.opened.get_untracked()
    }

    /// The folders opened in the window, in the rail's order. Tracked.
    pub fn folders(&self) -> Vec<PathBuf> {
        self.opened.get()
    }

    /// The sessions open in a folder not loaded yet. Tracked.
    pub fn stored(&self, folder: &Path) -> Vec<SessionSummary> {
        self.stored
            .with(|stored| stored.get(folder).cloned().unwrap_or_default())
    }

    /// Reads which sessions a folder not loaded yet had open, off the UI
    /// thread: its saved layout says, and Pi's store has their titles.
    fn load_stored(&self, folder: PathBuf) {
        let stored = self.stored;
        let finish = create_ext_action(self.scope, move |(folder, list)| {
            stored.update(|stored| {
                stored.insert(folder, list);
            })
        });
        std::thread::spawn(move || {
            let list: Vec<SessionSummary> = layout::state_path(&folder)
                .and_then(|path| layout::load(&path))
                .map(|saved| saved.sessions)
                .unwrap_or_default()
                .iter()
                .filter_map(|path| sessions::summarize(path))
                .collect();
            finish((folder, list))
        });
    }

    /// A folder's workspace, loaded (and its saved layout restored) if it
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
        let (go, known, close) = (self.me.clone(), self.me.clone(), self.me.clone());
        state.link_window(WindowLink {
            go: Box::new(move |folder, draft| {
                go.upgrade().is_some_and(|window| window.go(&folder, draft))
            }),
            close: Box::new(move || {
                if let Some(window) = close.upgrade() {
                    window.request_close();
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
        self.scopes.borrow_mut().insert(root.clone(), scope);
        self.stored.update(|stored| {
            stored.remove(&root);
        });
        self.opened.update(|opened| {
            if !opened.contains(&root) {
                opened.push(root);
            }
        });
        self.workspaces.update(|open| open.push(state.clone()));
        Ok(state)
    }

    /// Shows a folder, loading it if need be; `None` (and a message in the
    /// status bar) if it cannot be opened.
    pub fn show(&self, folder: &Path) -> Option<Rc<Workbench>> {
        match self.open(folder) {
            Ok(state) => {
                self.closing_from.borrow_mut().take();
                let root = state.root().to_path_buf();
                if self.active.get_untracked() != root {
                    self.active.set(root.clone());
                }
                self.mark_shown();
                self.collapsed.update(|folded| {
                    folded.remove(&root);
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
    /// with the draft in its composer. Whether it could be opened.
    pub fn go(&self, folder: &Path, draft: Option<String>) -> bool {
        let Some(state) = self.show(folder) else {
            return false;
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
        true
    }

    /// Brings an open session forward, in its folder.
    pub fn show_session(&self, folder: &Path, key: u64) {
        if let Some(state) = self.show(folder) {
            state.when_restored(move |state| state.show_session(key));
        }
    }

    /// Opens a session from Pi's store, in its folder.
    pub fn resume(&self, folder: &Path, summary: SessionSummary) {
        if let Some(state) = self.show(folder) {
            state.when_restored(move |state| {
                let session = state.sessions.resume(&summary);
                state.show_session(session.key);
            });
        }
    }

    /// Takes a session off a folder not loaded yet; it stays in Pi's store.
    pub fn forget(&self, folder: &Path, session: &Path) {
        self.stored.update(|stored| {
            if let Some(list) = stored.get_mut(folder) {
                list.retain(|s| s.path != session);
            }
        });
        if let Ok(path) = layout::state_path(folder)
            && let Ok(mut saved) = layout::load(&path)
        {
            saved.sessions.retain(|s| s != session);
            let _ = layout::save(&path, &saved);
        }
    }

    /// Closes a folder in the window: its layout saved and its sessions
    /// stopped, all of them still in Pi's store. Another folder is shown if
    /// it was; the last folder stays.
    pub fn close_folder(&self, folder: &Path) {
        let others: Vec<PathBuf> = self
            .opened
            .get_untracked()
            .into_iter()
            .filter(|other| other != folder)
            .collect();
        let report = |message: String| {
            if let Some(shown) = self.shown() {
                shown.report(message);
            }
        };
        if let Some(state) = self.workspace(folder)
            && let Err(err) = state.can_release()
        {
            report(err);
            return;
        }
        if self.active.get_untracked() == folder {
            let Some(next) = others.first() else {
                report("The window's only folder stays open".into());
                return;
            };
            if self.show(next).is_none() {
                return;
            }
        }
        if let Some(state) = self.workspace(folder) {
            if let Err(err) = state.release() {
                report(err);
                return;
            }
            self.workspaces
                .update(|open| open.retain(|w| !Rc::ptr_eq(w, &state)));
            if let Some(scope) = self.scopes.borrow_mut().remove(folder) {
                scope.dispose();
            }
        }
        self.stored.update(|stored| {
            stored.remove(folder);
        });
        self.collapsed.update(|folded| {
            folded.remove(folder);
        });
        self.opened.set(others);
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
    }

    /// Tells each folder whether it is the one shown.
    fn mark_shown(&self) {
        let shown = self.active.get_untracked();
        for state in self.workspaces.get_untracked() {
            state.set_shown(state.root() == shown);
        }
    }

    /// Closes the window once every folder has: each saves its layout and
    /// asks about unsaved files, shown while it does.
    pub fn request_close(&self) {
        self.closing_from
            .borrow_mut()
            .get_or_insert_with(|| self.active.get_untracked());
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
                    self.mark_shown();
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
