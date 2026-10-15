//! The window's state: panes of files, Pi sessions and terminals, the
//! documents they show, confirmations and the saved layout.
use crate::{
    documents::Documents,
    explorer::Explorer,
    lens::{Comparison, Lens},
    panes::{Direction, Pane, PaneId, Panes, Tab},
    session::Session,
    sessions::Sessions,
    syntax::Syntax,
    terminal::Terminal,
};
use floem::ext_event::create_ext_action;
use floem::prelude::*;
use floem::reactive::{Effect, ReadSignal, Scope};
use floem::views::editor::{
    core::{
        buffer::rope_text::RopeText, cursor::CursorAffinity, editor::EditType, selection::Selection,
    },
    text::Document,
    text_document::TextDocument,
};
use pi_client::{Launch, sessions::SessionSummary};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashSet},
    ops::Range,
    path::{Path, PathBuf},
    rc::Rc,
};
use workbench_core::{
    MAX_OPEN_DOCUMENTS, files,
    git::FileStatus,
    layout::{self, Layout, SavedNode, SavedTab},
};

/// The finder overlay: quick-open by file name, or search file contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Finder {
    Files,
    Search,
    /// The workbench's commands and Pi's slash commands.
    Commands,
    Themes,
}

/// Creates `<repository>-worktrees/pi-<time>` beside the repository on a new
/// branch `pi/pi-<time>`; returns the workspace folder inside it and the branch.
fn add_worktree(root: &Path) -> anyhow::Result<(PathBuf, String)> {
    use anyhow::Context;
    let repo = workbench_core::git::Repo::discover(root)?
        .context("This folder is not in a git repository")?;
    let name = repo
        .root()
        .file_name()
        .context("The repository has no folder name")?
        .to_string_lossy()
        .into_owned();
    let parent = repo
        .root()
        .parent()
        .context("The repository has no parent folder")?;
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let slug = format!("pi-{seconds}");
    let branch = format!("pi/{slug}");
    let tree = parent.join(format!("{name}-worktrees")).join(&slug);
    std::fs::create_dir_all(tree.parent().context("No parent folder")?)?;
    repo.add_worktree(&tree, &branch)?;
    // Open the same folder of the repository the window shows; workspace
    // paths are canonical, so compare with the canonical repository root.
    let repo_root = repo
        .root()
        .canonicalize()
        .unwrap_or_else(|_| repo.root().to_path_buf());
    let inside = root.strip_prefix(&repo_root).unwrap_or(Path::new(""));
    Ok((tree.join(inside), branch))
}

/// Where a pane's cursor should go once its file and diff are ready.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JumpTo {
    Line(usize),
    FirstChange,
    LastChange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Jump {
    pub pane: PaneId,
    pub path: PathBuf,
    pub to: JumpTo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Confirm {
    Quit,
    Close(PathBuf),
    Reload(PathBuf),
}

/// External services the workbench uses.
pub struct Integrations {
    pub pi: Launch,
}

/// One terminal tab's shell, started the first time the tab is shown.
pub struct TerminalSlot {
    pub key: u64,
    /// Shown in its tab: Terminal 1, Terminal 2, …
    pub number: usize,
    pub terminal: RwSignal<Option<Rc<Terminal>>>,
    failed: Cell<bool>,
}

pub struct Workbench {
    pub documents: Rc<Documents>,
    pub explorer: Rc<Explorer>,
    pub lens: Rc<Lens>,
    pub syntax: Rc<Syntax>,
    pub sessions: Rc<Sessions>,
    pub panes: Rc<Panes>,
    scope: Scope,
    pending: RefCell<HashSet<PathBuf>>,
    root: PathBuf,
    layout_path: PathBuf,
    layout: RwSignal<Layout>,
    layout_writable: bool,
    status: RwSignal<String>,
    confirm: RwSignal<Option<Confirm>>,
    finder: RwSignal<Option<Finder>>,
    terminals: RefCell<BTreeMap<u64, Rc<TerminalSlot>>>,
    next_terminal: Cell<u64>,
    terminal_focused: RwSignal<bool>,
    /// For Ctrl+`: the last pane that was not showing a terminal, and the
    /// last terminal shown.
    last_pane: Rc<Cell<Option<PaneId>>>,
    last_terminal: Rc<Cell<Option<u64>>>,
    /// A tab being dragged, and where it would land if dropped now.
    dragging: RwSignal<Option<(PaneId, Tab)>>,
    drop_target: RwSignal<Option<(PaneId, Option<Direction>)>>,
    /// A tab pressed but not yet moved far enough to count as a drag.
    pressed: RefCell<Option<(PaneId, Tab, floem::kurbo::Point)>>,
    /// Where the pointer is while dragging, for the tab that follows it.
    drag_point: RwSignal<floem::kurbo::Point>,
    /// Each pane's body, to find which pane the pointer is over.
    pane_views: RefCell<BTreeMap<PaneId, floem::ViewId>>,
    jump: RwSignal<Option<Jump>>,
    /// Whether restoring opens a session when none was saved; off in tests,
    /// which must not start Pi.
    start_session: Cell<bool>,
    /// Things that must live as long as the window, such as the file watcher.
    kept: RefCell<Vec<Box<dyn std::any::Any>>>,
    close_window: Box<dyn Fn()>,
}

impl Workbench {
    pub fn new(
        root: PathBuf,
        layout_path: PathBuf,
        integrations: Integrations,
        close_window: impl Fn() + 'static,
    ) -> Rc<Self> {
        let (layout, message, layout_writable) = match layout::load(&layout_path) {
            Ok(layout) => (layout, "Ready · Open a file from the explorer".into(), true),
            Err(err) => (
                Layout::default(),
                format!("{err}. Using defaults; the saved layout will be preserved."),
                false,
            ),
        };
        let status = RwSignal::new(message);
        let documents = Documents::new(Scope::current(), root.clone(), status);
        let explorer = Explorer::new(Scope::current(), root.clone(), status);
        let lens = Lens::new(Scope::current(), root.clone(), status);
        let (synced, tree, changed) = (documents.clone(), explorer.clone(), lens.clone());
        let snapshots = lens.clone();
        let sessions = Sessions::new(
            Scope::current(),
            root.clone(),
            integrations.pi,
            documents.clone(),
            move |tree_changed| {
                // Pi wrote to disk; fold its changes into open buffers.
                synced.sync_from_disk("Pi");
                changed.refresh();
                if tree_changed {
                    tree.refresh();
                }
            },
            move |done| snapshots.snapshot(done),
            status,
        );
        // "Since session start" and "since Pi's last turn" follow the front session.
        let (turns_lens, turns_sessions) = (lens.clone(), sessions.clone());
        Effect::new(move |_| {
            let turns = turns_sessions
                .active()
                .map(|session| session.turns().get())
                .unwrap_or_default();
            turns_lens.set_turns(turns);
        });
        let panes = Panes::new(Scope::current());
        let (last_pane, last_terminal) = (Rc::new(Cell::new(None)), Rc::new(Cell::new(None)));
        // The front session is the one in the focused pane, and Ctrl+` keeps
        // track of where it should go back to.
        let (follow_panes, follow_sessions) = (panes.clone(), sessions.clone());
        let (follow_pane, follow_terminal) = (last_pane.clone(), last_terminal.clone());
        Effect::new(move |_| {
            let id = follow_panes.focused().get();
            let Some(pane) = follow_panes.pane(id) else {
                return;
            };
            match pane.active.get() {
                Some(Tab::Session(key)) => {
                    follow_pane.set(Some(id));
                    if follow_sessions.active_key().get_untracked() != Some(key) {
                        follow_sessions.select(key);
                    }
                }
                Some(Tab::Terminal(key)) => follow_terminal.set(Some(key)),
                _ => follow_pane.set(Some(id)),
            }
        });
        Rc::new(Self {
            documents,
            explorer,
            lens,
            syntax: Syntax::new(Scope::current()),
            sessions,
            panes,
            scope: Scope::current(),
            pending: RefCell::new(HashSet::new()),
            root,
            layout_path,
            layout_writable,
            status,
            layout: RwSignal::new(layout),
            confirm: RwSignal::new(None),
            finder: RwSignal::new(None),
            terminals: RefCell::new(BTreeMap::new()),
            next_terminal: Cell::new(0),
            terminal_focused: RwSignal::new(false),
            last_pane,
            last_terminal,
            dragging: RwSignal::new(None),
            drop_target: RwSignal::new(None),
            pressed: RefCell::new(None),
            drag_point: RwSignal::new(floem::kurbo::Point::ZERO),
            pane_views: RefCell::new(BTreeMap::new()),
            jump: RwSignal::new(None),
            start_session: Cell::new(false),
            kept: RefCell::new(Vec::new()),
            close_window: Box::new(close_window),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    /// Makes the window open a session once restored, if it has none.
    pub fn start_with_session(&self) {
        self.start_session.set(true);
    }
    pub fn focused(&self) -> ReadSignal<PaneId> {
        self.panes.focused()
    }
    pub fn focus_pane(&self, id: PaneId) {
        self.panes.focus(id);
    }
    pub fn layout(&self) -> ReadSignal<Layout> {
        self.layout.read_only()
    }
    pub fn status(&self) -> ReadSignal<String> {
        self.status.read_only()
    }
    /// Shows a message in the status bar.
    pub fn report(&self, message: String) {
        self.status.set(message);
    }
    pub fn confirmation(&self) -> ReadSignal<Option<Confirm>> {
        self.confirm.read_only()
    }
    /// Opens the command picker, or closes it if it is open.
    pub fn toggle_palette(&self) {
        if self.finder.get_untracked() == Some(Finder::Commands) {
            self.dismiss_finder();
        } else {
            self.open_finder(Finder::Commands);
        }
    }
    pub fn finder(&self) -> ReadSignal<Option<Finder>> {
        self.finder.read_only()
    }
    pub fn open_finder(&self, mode: Finder) {
        if self.finder.get_untracked() == Some(Finder::Themes) && mode != Finder::Themes {
            crate::ui::color::revert();
        }
        self.finder.set(Some(mode));
    }
    /// Closes the picker; a theme only previewed is put back.
    pub fn dismiss_finder(&self) {
        if self.finder.get_untracked() == Some(Finder::Themes) {
            crate::ui::color::revert();
        }
        self.finder.set(None);
    }

    /// Makes a theme the one in use and records it in the settings file.
    pub fn choose_theme(&self, name: &str) {
        match crate::ui::color::load(name) {
            // Only a theme the settings file records is chosen; otherwise the
            // next read of the file would put the old one back.
            Ok(theme) => match workbench_core::settings::set_string("theme", name) {
                Ok(()) => {
                    crate::ui::color::choose(theme.clone());
                    self.status.set(format!("Theme: {}", theme.name));
                }
                Err(err) => {
                    crate::ui::color::revert();
                    self.status.set(format!("Settings: {err:#}"));
                }
            },
            Err(err) => self.status.set(format!("{err:#}")),
        }
    }

    /// Puts a Pi slash command in the active session's composer, ready for
    /// its arguments.
    pub fn start_pi_command(&self, name: &str) {
        let session = self.show_conversation();
        // In front of anything already typed, which may be its arguments.
        let text = format!("/{name} ");
        session.draft.edit_single(
            Selection::caret(0, CursorAffinity::Forward),
            &text,
            EditType::InsertChars,
        );
        session.focus_composer(text.len());
    }
    pub fn dismiss_confirmation(&self) {
        self.confirm.set(None);
    }

    // ---- Sessions as tabs ----

    /// Shows a session: where it already is, or else in the focused pane.
    pub fn show_session(&self, key: u64) {
        let tab = Tab::Session(key);
        let pane = self
            .panes
            .find(&tab)
            .unwrap_or_else(|| self.panes.focused().get_untracked());
        self.panes.show(pane, tab);
        self.panes.focus(pane);
        self.sessions.select(key);
    }

    /// Shows the front session, starting one if there is none.
    pub fn show_conversation(&self) -> Rc<Session> {
        let session = self
            .sessions
            .active_untracked()
            .unwrap_or_else(|| self.sessions.create());
        self.show_session(session.key);
        session
    }

    pub fn new_session(&self) {
        let session = self.sessions.create();
        let tab = Tab::Session(session.key);
        let pane = self.panes.focused().get_untracked();
        self.panes.show(pane, tab);
    }

    /// Stops a session's Pi and closes its tabs; it stays in Recent.
    pub fn close_session(&self, key: u64) {
        self.sessions.close(key);
        self.panes.remove_everywhere(&Tab::Session(key));
    }

    // ---- Terminals as tabs ----

    pub fn terminal(&self, key: u64) -> Option<Rc<TerminalSlot>> {
        self.terminals.borrow().get(&key).cloned()
    }

    fn add_terminal(&self) -> u64 {
        let key = self.next_terminal.get() + 1;
        self.next_terminal.set(key);
        let number = self
            .terminals
            .borrow()
            .values()
            .map(|slot| slot.number)
            .max()
            .unwrap_or(0)
            + 1;
        let slot = Rc::new(TerminalSlot {
            key,
            number,
            terminal: self.scope.enter(|| RwSignal::new(None)),
            failed: Cell::new(false),
        });
        self.terminals.borrow_mut().insert(key, slot);
        key
    }

    /// Opens a new terminal tab in a pane and focuses it.
    pub fn new_terminal(&self, pane: PaneId) -> u64 {
        let key = self.add_terminal();
        self.panes.show(pane, Tab::Terminal(key));
        self.panes.focus(pane);
        key
    }

    pub fn new_terminal_here(&self) {
        let pane = self.panes.focused().get_untracked();
        self.new_terminal(pane);
    }

    /// Ctrl+`: from a terminal, back to where you were; otherwise to the last
    /// terminal, or a new one in a pane along the bottom.
    pub fn show_terminal(&self) {
        let focused = self.panes.focused_pane();
        if matches!(focused.active.get_untracked(), Some(Tab::Terminal(_))) {
            if let Some(back) = self
                .last_pane
                .get()
                .filter(|id| self.panes.pane(*id).is_some())
            {
                self.panes.focus(back);
            }
            return;
        }
        if let Some(key) = self.last_terminal.get()
            && let Some(pane) = self.panes.find(&Tab::Terminal(key))
        {
            self.panes.show(pane, Tab::Terminal(key));
            self.panes.focus(pane);
            return;
        }
        let pane = self.panes.split_root(Direction::Down, 0.3);
        self.new_terminal(pane.id);
    }

    /// Starts a terminal's shell at its tab's size, or resizes the running one.
    pub fn ensure_terminal(&self, key: u64, columns: usize, lines: usize) {
        let Some(slot) = self.terminal(key) else {
            return;
        };
        if let Some(terminal) = slot.terminal.get_untracked() {
            terminal.resize(columns, lines);
            return;
        }
        if slot.failed.get() {
            return;
        }
        match Terminal::start(self.scope, &self.root, columns, lines) {
            Ok(terminal) => slot.terminal.set(Some(terminal)),
            Err(err) => {
                slot.failed.set(true);
                self.status.set(format!("Terminal: {err:#}"));
            }
        }
    }

    pub fn restart_terminal(&self, key: u64) {
        let Some(slot) = self.terminal(key) else {
            return;
        };
        if let Some(old) = slot.terminal.get_untracked() {
            let (columns, lines) = old.size();
            slot.terminal.set(None);
            slot.failed.set(false);
            self.ensure_terminal(key, columns, lines);
        }
    }

    /// A terminal no tab shows any more ends with its shell.
    fn drop_unused_terminal(&self, key: u64) {
        if self.panes.holding(&Tab::Terminal(key)).is_empty() {
            self.terminals.borrow_mut().remove(&key);
        }
    }

    pub fn terminal_focused(&self) -> ReadSignal<bool> {
        self.terminal_focused.read_only()
    }
    pub fn set_terminal_focused(&self, focused: bool) {
        if self.terminal_focused.get_untracked() != focused {
            self.terminal_focused.set(focused);
        }
    }

    // ---- Opening, restoring and closing ----

    pub fn restore(self: &Rc<Self>) {
        self.explorer.refresh();
        let layout = self.layout.get_untracked();
        let tree = layout.pane_tree();
        let mut files = Vec::new();
        let mut sessions = Vec::new();
        for pane in tree.panes() {
            if let SavedNode::Pane { tabs, .. } = pane {
                for tab in tabs {
                    match tab {
                        SavedTab::File(path) => files.push(self.root.join(path)),
                        SavedTab::Session(path) => sessions.push(path.clone()),
                        SavedTab::Terminal => {}
                    }
                }
            }
        }
        files.sort();
        files.dedup();
        // Startup runs once, before interaction. Reserve the saved files so
        // interactive opens share the same limit and do not read twice.
        self.pending.borrow_mut().extend(files.iter().cloned());
        let state = self.clone();
        let finish = create_ext_action(self.scope, move |(files, summaries)| {
            state.finish_restore(layout, tree, files, summaries);
        });
        let root = self.root.clone();
        std::thread::spawn(move || {
            let files: Vec<(PathBuf, Result<String, String>)> = files
                .into_iter()
                .map(|path| {
                    let result = files::within(&root, &path)
                        .and_then(|p| files::read_text(&p))
                        .map_err(|e| e.to_string());
                    (path, result)
                })
                .collect();
            // Only this workspace's sessions are resumed.
            let known = pi_client::sessions::list(&root);
            let summaries: Vec<SessionSummary> = sessions
                .iter()
                .filter_map(|path| known.iter().find(|s| s.path == *path).cloned())
                .collect();
            finish((files, summaries))
        });
    }

    fn finish_restore(
        &self,
        layout: Layout,
        tree: SavedNode,
        files: Vec<(PathBuf, Result<String, String>)>,
        summaries: Vec<SessionSummary>,
    ) {
        // One update, so the panes are built once.
        Effect::batch(|| self.apply_restore(layout, tree, files, summaries));
    }

    fn apply_restore(
        &self,
        layout: Layout,
        tree: SavedNode,
        files: Vec<(PathBuf, Result<String, String>)>,
        summaries: Vec<SessionSummary>,
    ) {
        for (path, result) in files {
            self.pending.borrow_mut().remove(&path);
            match result {
                Ok(text) => self.documents.insert(path, text),
                Err(err) => self.status.set(err),
            }
        }
        // Tabs opened while the layout was loading are kept.
        let opened: Vec<Tab> = self
            .panes
            .all()
            .iter()
            .flat_map(|pane| pane.tabs.get_untracked())
            .collect();
        let order = self.panes.restore(&tree, |saved| match saved {
            SavedTab::File(path) => {
                let path = self.root.join(path);
                self.documents.get(&path).map(|_| Tab::File(path))
            }
            SavedTab::Session(path) => {
                let summary = summaries.iter().find(|s| s.path == *path)?;
                Some(Tab::Session(self.sessions.resume(summary).key))
            }
            SavedTab::Terminal => Some(Tab::Terminal(self.add_terminal())),
        });
        if let Some(&focused) = order.get(layout.focused) {
            self.panes.focus(focused);
        }
        let focused = self.panes.focused().get_untracked();
        for tab in opened {
            if self.panes.find(&tab).is_none() {
                self.panes.show(focused, tab);
            }
        }
        // A window always starts with a session to talk to.
        let has_session = self.panes.all().iter().any(|pane| {
            pane.tabs
                .with_untracked(|tabs| tabs.iter().any(|t| matches!(t, Tab::Session(_))))
        });
        if !has_session && self.start_session.get() {
            self.show_conversation();
        }
    }

    pub fn open(self: &Rc<Self>, path: PathBuf, pane: PaneId) {
        // Pi and git spell paths differently; one file must have one document.
        let path = path.canonicalize().unwrap_or(path);
        let pane = self
            .panes
            .pane(pane)
            .unwrap_or_else(|| self.panes.focused_pane());
        pane.requested.set(Some(path.clone()));
        if self.documents.get(&path).is_some() {
            self.panes.show(pane.id, Tab::File(path));
            return;
        }
        if self.pending.borrow().contains(&path) {
            return;
        }
        if self.documents.items().with_untracked(Vec::len) + self.pending.borrow().len()
            >= MAX_OPEN_DOCUMENTS
        {
            self.status.set(format!(
                "Close a file before opening more ({MAX_OPEN_DOCUMENTS}-document limit)."
            ));
            self.drop_jump(&path);
            return;
        }
        self.pending.borrow_mut().insert(path.clone());
        self.status.set("Opening…".into());
        let state = self.clone();
        let reading = path.clone();
        let finish = create_ext_action(self.scope, move |result| state.finish_open(path, result));
        let root = self.root.clone();
        // The settings file is the one file outside the workspace that opens.
        let settings = workbench_core::settings::path()
            .ok()
            .map(|p| p.canonicalize().unwrap_or(p));
        std::thread::spawn(move || {
            let allowed = if settings.as_ref() == Some(&reading) {
                Ok(reading.clone())
            } else {
                files::within(&root, &reading)
            };
            finish(
                allowed
                    .and_then(|p| files::read_text(&p))
                    .map_err(|e| e.to_string()),
            );
        });
    }

    /// Checks out the current commit as a new branch in a sibling folder and
    /// opens it in a new window with a fresh session, so Pi works apart from
    /// this checkout.
    pub fn new_worktree_session(&self) {
        self.status.set("Creating a worktree…".into());
        let status = self.status;
        let finish = create_ext_action(
            self.scope,
            move |result: anyhow::Result<(PathBuf, String)>| match result.and_then(
                |(path, branch)| {
                    crate::open_window(&path)?;
                    Ok(branch)
                },
            ) {
                Ok(branch) => status.set(format!("Opened a new window on branch {branch}")),
                Err(err) => status.set(format!("Worktree: {err:#}")),
            },
        );
        let root = self.root.clone();
        std::thread::spawn(move || finish(add_worktree(&root)));
    }

    /// Opens the settings file, creating it with every setting explained.
    pub fn open_settings(self: &Rc<Self>) {
        match crate::settings::file() {
            Ok(path) => {
                self.open(path, self.panes.focused().get_untracked());
                self.status
                    .set("Settings apply as you save; fonts and keys in new windows.".into());
            }
            Err(err) => self.status.set(format!("Settings: {err:#}")),
        }
    }

    /// A jump into a file that never opened must not fire later.
    fn drop_jump(&self, path: &Path) {
        if self
            .jump
            .with_untracked(|j| j.as_ref().is_some_and(|j| j.path == path))
        {
            self.jump.set(None);
        }
    }

    fn finish_open(&self, path: PathBuf, result: Result<String, String>) {
        self.pending.borrow_mut().remove(&path);
        match result {
            Ok(text) => {
                self.documents.insert(path.clone(), text);
                // One read can serve several panes, but must not replace a
                // later selection in any of them.
                for pane in self.panes.all() {
                    if pane.requested.get_untracked().as_ref() == Some(&path) {
                        self.panes.show(pane.id, Tab::File(path.clone()));
                    }
                }
                self.status.set("Ready".into());
            }
            Err(err) => {
                self.drop_jump(&path);
                self.status.set(err)
            }
        }
    }

    /// Shows a tab the user picked, and focuses its pane.
    pub fn select_tab(&self, pane: PaneId, tab: Tab) {
        if self.jump.with_untracked(Option::is_some) {
            self.jump.set(None);
        }
        if let Some(p) = self.panes.pane(pane) {
            p.requested.set(match &tab {
                Tab::File(path) => Some(path.clone()),
                _ => None,
            });
        }
        self.panes.show(pane, tab);
        self.panes.focus(pane);
    }

    /// The file showing in the focused pane.
    pub fn active_path(&self) -> Option<PathBuf> {
        self.panes.focused_pane().active_file()
    }

    pub fn keep(&self, value: impl std::any::Any) {
        self.kept.borrow_mut().push(Box::new(value));
    }

    pub fn jump(&self) -> RwSignal<Option<Jump>> {
        self.jump
    }

    /// The focused pane and the file it shows, if it shows one.
    fn focused_file(&self) -> Option<(Rc<Pane>, PathBuf)> {
        let pane = self.panes.focused_pane();
        let path = pane.active_file()?;
        Some((pane, path))
    }

    /// The lines of an open file's unseen changes, in order.
    pub fn change_lines(&self, path: &Path) -> Vec<usize> {
        let Some(comparison) = self
            .lens
            .file(path)
            .and_then(|file| file.comparison().get_untracked())
        else {
            return Vec::new();
        };
        Effect::untrack(|| {
            comparison
                .hunks
                .iter()
                .enumerate()
                .filter(|(index, _)| !self.lens.hidden(&comparison, *index))
                .map(|(_, (hunk, _))| comparison.line_of(hunk))
                .collect()
        })
    }

    /// The change under the cursor of the focused pane, compared against the
    /// text as it is now. Reports why when there is none.
    fn change_at_cursor(&self, action: &str) -> Option<(PathBuf, Rc<Comparison>, usize)> {
        let found = self.focused_file().and_then(|(pane, path)| {
            let file = self.lens.file(&path)?;
            let line = pane.cursor_line.get_untracked();
            let Some(comparison) = file.current() else {
                return Some(Err(()));
            };
            let index = comparison.at_line(line)?;
            Some(Ok((path, comparison, index)))
        });
        match found {
            Some(Ok(found)) => Some(found),
            Some(Err(())) => {
                self.status
                    .set("Still comparing the latest edit; try again in a moment.".into());
                None
            }
            None => {
                self.status
                    .set(format!("Put the cursor on a change to {action}."));
                None
            }
        }
    }

    /// Moves to the next unseen change, continuing into the next changed file.
    pub fn next_change(self: &Rc<Self>, forward: bool) {
        if let Some((pane, path)) = self.focused_file() {
            let line = pane.cursor_line.get_untracked();
            let lines = self.change_lines(&path);
            let target = if forward {
                lines.iter().find(|l| **l > line)
            } else {
                lines.iter().rev().find(|l| **l < line)
            };
            if let Some(&target) = target {
                self.jump.set(Some(Jump {
                    pane: pane.id,
                    path,
                    to: JumpTo::Line(target),
                }));
                return;
            }
        }
        let files: Vec<PathBuf> = self.lens.changes().with_untracked(|changes| {
            changes
                .iter()
                .filter(|c| c.status != FileStatus::Deleted && !c.binary)
                .map(|c| c.path.clone())
                .collect()
        });
        if files.is_empty() {
            let label = self.lens.label(self.lens.since().get_untracked());
            self.status.set(format!("No changes since {label}"));
            return;
        }
        let current = self.focused_file().map(|(_, path)| path);
        let at = current.and_then(|path| files.iter().position(|f| *f == path));
        let next = match (at, forward) {
            (Some(i), true) => (i + 1) % files.len(),
            (Some(i), false) => (i + files.len() - 1) % files.len(),
            (None, true) => 0,
            (None, false) => files.len() - 1,
        };
        let pane = self.panes.focused().get_untracked();
        let path = files[next].clone();
        self.open(path.clone(), pane);
        self.jump.set(Some(Jump {
            pane,
            path,
            to: if forward {
                JumpTo::FirstChange
            } else {
                JumpTo::LastChange
            },
        }));
    }

    /// Puts the baseline's lines back in place of the change under the cursor.
    pub fn revert_change(&self) {
        let Some((path, comparison, index)) = self.change_at_cursor("revert it") else {
            return;
        };
        let Some(doc) = self.documents.get(&path) else {
            return;
        };
        let hunk = &comparison.hunks[index].0;
        replace_lines(&doc.text(), hunk.current.clone(), &comparison.removed(hunk));
        self.status
            .set("Reverted the change; undo brings it back.".into());
    }

    /// Stages the change under the cursor, as `git add -p` would one hunk:
    /// only the lines it covers go to the index, as the editor has them.
    pub fn stage_change(&self) {
        let Some((path, comparison, index)) = self.change_at_cursor("stage it") else {
            return;
        };
        let Some(doc) = self.documents.get(&path) else {
            return;
        };
        let lines = comparison.hunks[index].0.current.clone();
        let text = doc.text().text().to_string();
        let status = self.status;
        let finish =
            create_ext_action(self.scope, move |result: anyhow::Result<()>| match result {
                Ok(()) => status.set(
                    "Staged the change as the editor shows it; the file keeps your other edits."
                        .into(),
                ),
                Err(err) => status.set(format!("Stage: {err:#}")),
            });
        let root = self.root.clone();
        std::thread::spawn(move || {
            let stage = || -> anyhow::Result<()> {
                use anyhow::Context;
                let repo = workbench_core::git::Repo::discover(&root)?
                    .context("This folder is not in a git repository")?;
                let entry = repo
                    .staged_text(&path)?
                    .context("The file is not tracked yet; add it from the terminal")?;
                let next = workbench_core::diff::LineDiff::apply_lines(&entry.text, &text, lines);
                anyhow::ensure!(next != entry.text, "That change is already staged");
                repo.stage_text(&path, &entry, &next)
            };
            finish(stage())
        });
    }

    /// Puts the change under the cursor into the active session's composer.
    pub fn ask_about_change(&self) {
        let Some((path, comparison, index)) = self.change_at_cursor("ask Pi about it") else {
            return;
        };
        let hunk = comparison.hunks[index].0.clone();
        let Some(doc) = self.documents.get(&path) else {
            return;
        };
        let text = doc.text().text().to_string();
        let lines: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
        let code = lines
            .get(hunk.current.clone())
            .unwrap_or_default()
            .join("\n");
        let name = path.strip_prefix(&self.root).unwrap_or(&path).display();
        let question = format!(
            "About this change in `{name}` (lines {}-{}):\n```\n{code}\n```\n",
            hunk.current.start + 1,
            hunk.current.end.max(hunk.current.start + 1),
        );
        // Beside the code, so the change stays in view.
        let session = self
            .sessions
            .active_untracked()
            .unwrap_or_else(|| self.sessions.create());
        let tab = Tab::Session(session.key);
        if self.panes.find(&tab).is_none() {
            let from = self.panes.focused().get_untracked();
            let to = self
                .panes
                .neighbour(from, Direction::Right)
                .unwrap_or_else(|| self.panes.split(from, Direction::Right).id);
            self.panes.show(to, tab);
        }
        self.show_session(session.key);
        let len = session.draft.text().len();
        session.draft.edit_single(
            Selection::caret(len, CursorAffinity::Forward),
            &question,
            EditType::InsertChars,
        );
    }

    pub fn mark_change_seen(self: &Rc<Self>) {
        let Some((_, comparison, index)) = self.change_at_cursor("mark it seen") else {
            return;
        };
        self.lens.mark_seen(comparison.hunks[index].1);
        self.next_change(true);
    }

    /// Opens a file in the focused pane and puts the cursor on `line`.
    pub fn open_at(self: &Rc<Self>, path: PathBuf, line: usize) {
        let pane = self.panes.focused().get_untracked();
        self.open(path.clone(), pane);
        self.jump.set(Some(Jump {
            pane,
            path,
            to: JumpTo::Line(line),
        }));
    }

    /// Opens a file beside the focused pane, splitting it if there is no pane
    /// to its right, so the conversation stays in view.
    pub fn open_beside(self: &Rc<Self>, path: PathBuf) {
        let from = self.panes.focused().get_untracked();
        let to = self
            .panes
            .neighbour(from, Direction::Right)
            .unwrap_or_else(|| self.panes.split(from, Direction::Right).id);
        self.open(path, to);
        self.panes.focus(to);
    }

    pub fn save_active(&self) {
        if let Some(path) = self.active_path()
            && let Some(doc) = self.documents.get(&path)
        {
            self.documents.save(doc);
        }
    }

    /// Closes a tab. A file closes when no other pane shows it, asking first
    /// if it has unsaved edits; a session keeps running in the rail; a
    /// terminal ends with its last tab.
    pub fn close_tab(&self, pane: PaneId, tab: Tab) {
        match &tab {
            Tab::File(path) if self.panes.holding(&tab).len() <= 1 => {
                self.close_file(path.clone());
            }
            Tab::Terminal(key) => {
                self.panes.remove(pane, &tab);
                self.drop_unused_terminal(*key);
            }
            _ => self.panes.remove(pane, &tab),
        }
    }

    /// Ctrl+W: the focused tab, or the focused pane once it has none.
    pub fn close_active(&self) {
        let pane = self.panes.focused_pane();
        match pane.active.get_untracked() {
            Some(tab) => self.close_tab(pane.id, tab),
            None => {
                self.panes.close(pane.id);
            }
        }
    }

    /// Closes a pane and its tabs, unless a file only it shows has unsaved edits.
    pub fn close_pane(&self, id: PaneId) {
        let Some(pane) = self.panes.pane(id) else {
            return;
        };
        let tabs = pane.tabs.get_untracked();
        let unsaved = tabs.iter().find_map(|tab| match tab {
            Tab::File(path)
                if self.panes.holding(tab).len() == 1
                    && self
                        .documents
                        .get(path)
                        .is_some_and(|d| d.dirty() || d.saving()) =>
            {
                Some(path.clone())
            }
            _ => None,
        });
        if let Some(path) = unsaved {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            self.status
                .set(format!("Save or close {name} before closing its pane."));
            return;
        }
        if self.panes.all().len() <= 1 {
            return;
        }
        for tab in tabs {
            match &tab {
                Tab::File(path) if self.panes.holding(&tab).len() == 1 => self.remove_file(path),
                Tab::Terminal(key) => {
                    self.panes.remove(id, &tab);
                    self.drop_unused_terminal(*key);
                }
                _ => self.panes.remove(id, &tab),
            }
        }
        self.panes.close(id);
    }

    pub fn close_file(&self, path: PathBuf) {
        let Some(doc) = self.documents.get(&path) else {
            self.panes.remove_everywhere(&Tab::File(path));
            return;
        };
        if doc.saving() {
            self.status.set("Wait for saving to finish.".into());
        } else if doc.dirty() {
            self.confirm.set(Some(Confirm::Close(path)));
        } else {
            self.remove_file(&path);
            self.dismiss_confirmation();
        }
    }

    fn remove_file(&self, path: &Path) {
        self.lens.forget(path);
        self.syntax.forget(path);
        self.documents.remove(path);
        self.panes.remove_everywhere(&Tab::File(path.to_path_buf()));
        for pane in self.panes.all() {
            if pane.requested.get_untracked().as_deref() == Some(path) {
                pane.requested.set(None);
            }
        }
    }

    pub fn reload_active(&self) {
        let Some(path) = self.active_path() else {
            return;
        };
        let Some(doc) = self.documents.get(&path) else {
            return;
        };
        if doc.saving() {
            self.status.set("Wait for saving to finish.".into());
        } else if doc.dirty() {
            self.confirm.set(Some(Confirm::Reload(path)));
        } else {
            self.documents.reload(path);
        }
    }

    pub fn close_when_saved(&self) {
        match self.confirm.get_untracked() {
            Some(Confirm::Quit) => self.request_quit(),
            Some(Confirm::Close(path)) => self.close_file(path),
            _ => {}
        }
    }

    pub fn discard_confirmed(&self) {
        if self.documents.any_saving() {
            self.status.set("Wait for saving to finish.".into());
            return;
        }
        match self.confirm.get_untracked() {
            Some(Confirm::Quit) => self.finish_quit(),
            Some(Confirm::Close(path)) => self.remove_file(&path),
            Some(Confirm::Reload(path)) => self.documents.reload(path),
            None => {}
        }
        self.dismiss_confirmation();
    }

    // ---- Arranging panes ----

    /// Splits the focused pane. Its file or session shows on both sides; a
    /// terminal gets a new shell beside it.
    pub fn split(&self, direction: Direction) {
        let from = self.panes.focused_pane();
        let new = self.panes.split(from.id, direction);
        match from.active.get_untracked() {
            Some(Tab::Terminal(_)) => {
                self.new_terminal(new.id);
            }
            Some(tab) => self.panes.show(new.id, tab),
            None => {}
        }
        self.panes.focus(new.id);
    }

    pub fn focus_direction(&self, direction: Direction) {
        let from = self.panes.focused().get_untracked();
        if let Some(to) = self.panes.neighbour(from, direction) {
            self.panes.focus(to);
        }
    }

    /// Moves the focused tab to the pane on one side, making one if needed.
    pub fn move_tab(&self, direction: Direction) {
        let from = self.panes.focused_pane();
        let Some(tab) = from.active.get_untracked() else {
            return;
        };
        let alone = from.tabs.with_untracked(Vec::len) == 1;
        let to = match self.panes.neighbour(from.id, direction) {
            Some(to) => to,
            // A lone tab moving into a new pane would leave this one empty.
            None if alone => return,
            None => self.panes.split(from.id, direction).id,
        };
        self.panes.move_tab(from.id, &tab, to, None);
        self.close_if_empty(from.id);
    }

    fn close_if_empty(&self, id: PaneId) {
        let empty = self
            .panes
            .pane(id)
            .is_some_and(|pane| pane.tabs.with_untracked(Vec::is_empty));
        if empty {
            self.panes.close(id);
        }
    }

    pub fn toggle_maximized(&self) {
        let id = self.panes.focused().get_untracked();
        self.panes.toggle_maximized(id);
    }

    pub fn drop_target(&self) -> ReadSignal<Option<(PaneId, Option<Direction>)>> {
        self.drop_target.read_only()
    }
    pub fn dragging(&self) -> ReadSignal<Option<(PaneId, Tab)>> {
        self.dragging.read_only()
    }
    pub fn drag_point(&self) -> ReadSignal<floem::kurbo::Point> {
        self.drag_point.read_only()
    }
    /// Records a pane's body so drags can find it.
    pub fn register_pane_view(&self, pane: PaneId, view: floem::ViewId) {
        self.pane_views.borrow_mut().insert(pane, view);
    }

    /// A tab was pressed; it becomes a drag once the pointer moves away.
    pub fn press_tab(&self, pane: PaneId, tab: Tab, at: floem::kurbo::Point) {
        *self.pressed.borrow_mut() = Some((pane, tab, at));
    }

    /// The pointer moved anywhere in the window. Returns whether a drag is on.
    pub fn pointer_moved(&self, at: floem::kurbo::Point) -> bool {
        if self.dragging.with_untracked(Option::is_none) {
            let started = self
                .pressed
                .borrow()
                .as_ref()
                .is_some_and(|(_, _, from)| from.distance(at) > 6.0);
            if !started {
                return false;
            }
            let (pane, tab, _) = self.pressed.borrow_mut().take().expect("pressed above");
            self.dragging.set(Some((pane, tab)));
        }
        self.drag_point.set(at);
        let over = self.pane_views.borrow().iter().find_map(|(pane, view)| {
            let rect = view.get_visual_rect();
            rect.contains(at).then(|| {
                let side = side_at(at.x - rect.x0, at.y - rect.y0, rect.width(), rect.height());
                (*pane, side)
            })
        });
        match over {
            Some((pane, side)) => self.drag_over(pane, side),
            None if self.drop_target.with_untracked(Option::is_some) => self.drop_target.set(None),
            None => {}
        }
        true
    }

    /// The pointer was released: drops a dragged tab where it is over.
    pub fn pointer_released(&self) -> bool {
        self.pressed.borrow_mut().take();
        if self.dragging.with_untracked(Option::is_none) {
            return false;
        }
        match self.drop_target.get_untracked() {
            Some((pane, side)) => self.drop_tab(pane, side, None),
            None => self.end_drag(),
        }
        true
    }
    /// Where a dragged tab would land: in the pane, or in a new pane on one
    /// of its sides.
    pub fn drag_over(&self, pane: PaneId, side: Option<Direction>) {
        if self.dragging.with_untracked(Option::is_some)
            && self.drop_target.get_untracked() != Some((pane, side))
        {
            self.drop_target.set(Some((pane, side)));
        }
    }
    pub fn end_drag(&self) {
        if self.dragging.with_untracked(Option::is_some) {
            self.dragging.set(None);
        }
        if self.drop_target.with_untracked(Option::is_some) {
            self.drop_target.set(None);
        }
    }
    /// Drops the dragged tab on a pane, or on its side to split it, or before
    /// another tab when `index` is given.
    pub fn drop_tab(&self, pane: PaneId, side: Option<Direction>, index: Option<usize>) {
        let Some((from, tab)) = self.dragging.get_untracked() else {
            return;
        };
        self.end_drag();
        let target = match side {
            // A pane's only tab dropped on its own side would leave it empty.
            Some(_)
                if from == pane
                    && self
                        .panes
                        .pane(from)
                        .is_some_and(|p| p.tabs.with_untracked(Vec::len) == 1) =>
            {
                return;
            }
            Some(direction) => self.panes.split(pane, direction).id,
            None => pane,
        };
        self.panes.move_tab(from, &tab, target, index);
        if from != target {
            self.close_if_empty(from);
        }
    }

    pub fn toggle_explorer(&self) {
        self.layout
            .update(|layout| layout.explorer_visible = !layout.explorer_visible);
    }
    pub fn move_explorer(&self) {
        self.layout
            .update(|layout| layout.explorer_right = !layout.explorer_right);
    }
    pub fn resize_explorer(&self, width: f64) {
        self.layout
            .update(|layout| layout.explorer_width = width.clamp(160.0, 480.0));
    }

    /// One pane with every tab, and the explorer as it first was.
    pub fn reset_layout(&self) {
        self.layout.set(Layout::default());
        self.panes.collapse();
    }

    fn persist_layout(&self) -> anyhow::Result<()> {
        if !self.layout_writable {
            return Ok(());
        }
        let mut layout = self.layout.get_untracked();
        layout.open_files = self.documents.items().with_untracked(|docs| {
            docs.iter()
                .filter_map(|doc| {
                    doc.path()
                        .strip_prefix(&self.root)
                        .ok()
                        .map(Path::to_path_buf)
                })
                .collect()
        });
        layout.panes = Some(self.panes.save(|tab| {
            match tab {
                Tab::File(path) => path
                    .strip_prefix(&self.root)
                    .ok()
                    .map(|p| SavedTab::File(p.to_path_buf())),
                Tab::Session(key) => self
                    .sessions
                    .find(*key)
                    .and_then(|session| session.file().get_untracked())
                    .map(SavedTab::Session),
                Tab::Terminal(_) => Some(SavedTab::Terminal),
            }
        }));
        let order = self.panes.tree().with_untracked(|tree| tree.leaves());
        let focused = self.panes.focused().get_untracked();
        layout.focused = order.iter().position(|&id| id == focused).unwrap_or(0);
        layout::save(&self.layout_path, &layout)
    }

    pub fn request_quit(&self) {
        if self.documents.all_saved() {
            self.finish_quit();
        } else {
            self.confirm.set(Some(Confirm::Quit));
        }
    }

    fn finish_quit(&self) {
        // A theme only previewed must not stay on in the other windows.
        self.dismiss_finder();
        match self.persist_layout() {
            Ok(()) => (self.close_window)(),
            Err(err) => self.status.set(format!("Could not save layout: {err}")),
        }
        self.dismiss_confirmation();
    }
}

/// Which side of a pane a point is nearest, if it is near one: dropping
/// there splits the pane; in the middle, the tab joins it.
pub fn side_at(x: f64, y: f64, width: f64, height: f64) -> Option<Direction> {
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let (fx, fy) = (x / width, y / height);
    let edges = [
        (fx, Direction::Left),
        (1.0 - fx, Direction::Right),
        (fy, Direction::Up),
        (1.0 - fy, Direction::Down),
    ];
    edges
        .into_iter()
        .filter(|(distance, _)| *distance < 0.25)
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, side)| side)
}

/// Replaces whole lines, keeping the buffer's line ending; one undoable edit.
fn replace_lines(text: &TextDocument, lines: Range<usize>, replacement: &[&str]) {
    let rope = text.rope_text();
    let total = rope.num_lines();
    let start = rope.offset_of_line(lines.start.min(total.saturating_sub(1)));
    let at_end = lines.end >= total;
    let end = if at_end {
        rope.len()
    } else {
        rope.offset_of_line(lines.end)
    };
    let newline = if text.text().to_string().contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut new = replacement.join(newline);
    if !replacement.is_empty() && !at_end {
        new.push_str(newline);
    }
    text.edit_single(
        Selection::region(start, end, CursorAffinity::Backward),
        &new,
        EditType::Other,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use floem::views::editor::{
        core::{cursor::CursorAffinity, editor::EditType, selection::Selection},
        text::Document,
    };
    use std::cell::Cell;

    fn with_workspace(test: impl FnOnce(Rc<Workbench>, Rc<Cell<bool>>)) {
        let scope = Scope::new();
        let dir = tempfile::tempdir().unwrap();
        scope.enter(|| {
            let closed = Rc::new(Cell::new(false));
            let on_close = closed.clone();
            let state = Workbench::new(
                dir.path().to_path_buf(),
                dir.path().join("layout.json"),
                Integrations {
                    pi: Launch::default(),
                },
                move || on_close.set(true),
            );
            test(state, closed);
        });
        scope.dispose();
    }

    fn focused(state: &Workbench) -> Rc<Pane> {
        state.panes.focused_pane()
    }

    fn showing(state: &Workbench, pane: PaneId) -> Option<Tab> {
        state
            .panes
            .pane(pane)
            .and_then(|p| p.active.get_untracked())
    }

    fn file(state: &Workbench, name: &str, text: &str) -> PathBuf {
        let path = state.root.join(name);
        state.documents.insert(path.clone(), text.into());
        path
    }

    fn dirty_document(state: &Workbench) -> PathBuf {
        let path = file(state, "test.rs", "original");
        let doc = state.documents.get(&path).unwrap();
        let text = doc.text();
        text.edit_single(
            Selection::region(0, text.text().len(), CursorAffinity::Backward),
            "unsaved",
            EditType::Other,
        );
        state.select_tab(focused(state).id, Tab::File(path.clone()));
        path
    }

    #[test]
    fn late_open_and_restore_do_not_steal_selection() {
        with_workspace(|state, _| {
            let pane = focused(&state).id;
            let slow = state.root.join("slow.rs");
            let chosen = file(&state, "chosen.rs", "chosen");
            state.pending.borrow_mut().insert(slow.clone());
            state.open(slow.clone(), pane);
            state.select_tab(pane, Tab::File(chosen.clone()));
            state.finish_open(slow.clone(), Ok("slow".into()));
            assert_eq!(showing(&state, pane), Some(Tab::File(chosen.clone())));
            assert!(state.panes.find(&Tab::File(slow)).is_none());
            let saved = SavedNode::Pane {
                tabs: vec![SavedTab::File("previous.rs".into())],
                active: Some(0),
            };
            state.finish_restore(
                Layout::default(),
                saved,
                vec![(state.root.join("previous.rs"), Ok("old".into()))],
                Vec::new(),
            );
            let pane = focused(&state).id;
            assert_eq!(showing(&state, pane), Some(Tab::File(chosen)));
        });
    }

    #[test]
    fn one_document_serves_two_panes_and_closes_with_its_last_tab() {
        with_workspace(|state, _| {
            let left = focused(&state).id;
            let right = state.panes.split(left, Direction::Right).id;
            let shared = state.root.join("shared.rs");
            state.pending.borrow_mut().insert(shared.clone());
            state.open(shared.clone(), left);
            state.open(shared.clone(), right);
            assert_eq!(state.pending.borrow().len(), 1);
            state.finish_open(shared.clone(), Ok("shared".into()));
            let tab = Tab::File(shared.clone());
            assert_eq!(showing(&state, left), Some(tab.clone()));
            assert_eq!(showing(&state, right), Some(tab.clone()));
            state.close_tab(left, tab.clone());
            assert!(
                state.documents.get(&shared).is_some(),
                "still showing on the right"
            );
            state.close_tab(right, tab);
            assert!(state.documents.get(&shared).is_none());
        });
    }

    #[test]
    fn splitting_shows_the_tab_twice_and_moving_closes_an_emptied_pane() {
        with_workspace(|state, _| {
            let first = focused(&state).id;
            let a = file(&state, "a.rs", "a");
            let b = file(&state, "b.rs", "b");
            state.select_tab(first, Tab::File(a.clone()));
            state.select_tab(first, Tab::File(b.clone()));
            state.split(Direction::Right);
            let second = focused(&state).id;
            assert_ne!(first, second);
            assert_eq!(showing(&state, second), Some(Tab::File(b.clone())));
            // Moving the right pane's only tab left empties and closes it.
            state.move_tab(Direction::Left);
            assert!(state.panes.pane(second).is_none());
            assert_eq!(state.panes.focused().get_untracked(), first);
            // A lone tab cannot move to a side with nothing there.
            let tabs = focused(&state).tabs.get_untracked().len();
            state.move_tab(Direction::Down);
            assert_eq!(state.panes.all().len(), 2, "it splits off a pane");
            assert_eq!(
                state.panes.pane(first).unwrap().tabs.get_untracked().len(),
                tabs - 1
            );
        });
    }

    #[test]
    fn closing_a_pane_waits_for_unsaved_files_only_it_shows() {
        with_workspace(|state, _| {
            let first = focused(&state).id;
            let path = dirty_document(&state);
            let second = state.panes.split(first, Direction::Right).id;
            state.panes.focus(first);
            state.close_pane(first);
            assert!(state.panes.pane(first).is_some(), "refused");
            assert!(state.status().get_untracked().contains("test.rs"));
            // Shown on the other side too, it may go.
            state.panes.show(second, Tab::File(path.clone()));
            state.close_pane(first);
            assert!(state.panes.pane(first).is_none());
            assert!(state.documents.get(&path).is_some());
        });
    }

    #[test]
    fn terminal_tabs_end_their_shell_with_their_last_tab() {
        with_workspace(|state, _| {
            let pane = focused(&state).id;
            let first = state.new_terminal(pane);
            let second = state.new_terminal(pane);
            assert_eq!(state.terminal(first).unwrap().number, 1);
            assert_eq!(state.terminal(second).unwrap().number, 2);
            state.close_tab(pane, Tab::Terminal(first));
            assert!(state.terminal(first).is_none());
            assert_eq!(showing(&state, pane), Some(Tab::Terminal(second)));
            // Numbers continue after the highest one open.
            assert_eq!(state.terminal(state.new_terminal(pane)).unwrap().number, 3);
        });
    }

    #[test]
    fn restore_builds_the_saved_panes() {
        with_workspace(|state, _| {
            let saved = SavedNode::Split {
                horizontal: false,
                sizes: vec![0.7, 0.3],
                children: vec![
                    SavedNode::Pane {
                        tabs: vec![
                            SavedTab::File("first.rs".into()),
                            SavedTab::File("second.rs".into()),
                            SavedTab::File("missing.rs".into()),
                        ],
                        active: Some(1),
                    },
                    SavedNode::Pane {
                        tabs: vec![SavedTab::Terminal, SavedTab::Terminal],
                        active: Some(0),
                    },
                ],
            };
            let layout = Layout {
                focused: 1,
                ..Default::default()
            };
            state.finish_restore(
                layout,
                saved,
                vec![
                    (state.root.join("first.rs"), Ok("first".into())),
                    (state.root.join("second.rs"), Ok("second".into())),
                    (state.root.join("missing.rs"), Err("gone".into())),
                ],
                Vec::new(),
            );
            let panes = state.panes.all();
            assert_eq!(panes.len(), 2);
            assert_eq!(
                panes[0].tabs.get_untracked().len(),
                2,
                "the missing file is left out"
            );
            assert_eq!(
                panes[0].active.get_untracked(),
                Some(Tab::File(state.root.join("second.rs")))
            );
            assert_eq!(panes[1].tabs.get_untracked().len(), 2);
            assert_eq!(state.panes.focused().get_untracked(), panes[1].id);
            assert_eq!(state.status().get_untracked(), "gone");
        });
    }

    #[test]
    fn open_limit_counts_pending_reads_and_failed_open_preserves_selection() {
        with_workspace(|state, _| {
            let pane = focused(&state).id;
            let selected = file(&state, "selected.rs", "selected");
            state.select_tab(pane, Tab::File(selected.clone()));
            let pending: Vec<_> = (1..MAX_OPEN_DOCUMENTS)
                .map(|i| state.root.join(format!("pending-{i}.rs")))
                .collect();
            state.pending.borrow_mut().extend(pending.iter().cloned());
            let extra = state.root.join("extra.rs");
            state.open(extra.clone(), pane);
            assert!(!state.pending.borrow().contains(&extra));
            assert!(state.status().get().contains("limit"));
            // Requesting a file that is already loading still works at the limit.
            state.open(pending[0].clone(), pane);
            state.finish_open(pending[0].clone(), Err("read failed".into()));
            assert!(!state.pending.borrow().contains(&pending[0]));
            assert_eq!(state.status().get(), "read failed");
            assert_eq!(showing(&state, pane), Some(Tab::File(selected)));
        });
    }

    #[test]
    fn dirty_close_waits_for_confirmation_and_discard_closes_every_tab() {
        with_workspace(|state, _| {
            let path = dirty_document(&state);
            let other = state.panes.split(focused(&state).id, Direction::Right).id;
            state.panes.show(other, Tab::File(path.clone()));
            state.close_file(path.clone());
            assert_eq!(
                state.confirmation().get(),
                Some(Confirm::Close(path.clone()))
            );
            state.close_when_saved();
            assert!(state.documents.get(&path).is_some());
            state.discard_confirmed();
            assert!(state.documents.get(&path).is_none());
            assert!(state.panes.find(&Tab::File(path)).is_none());
            assert!(state.confirmation().get().is_none());
        });
    }

    #[test]
    fn cancelling_reload_and_quit_keeps_edits_open() {
        with_workspace(|state, closed| {
            let path = dirty_document(&state);
            state.reload_active();
            assert_eq!(
                state.confirmation().get(),
                Some(Confirm::Reload(path.clone()))
            );
            state.dismiss_confirmation();
            state.request_quit();
            assert_eq!(state.confirmation().get(), Some(Confirm::Quit));
            state.close_when_saved();
            assert!(!closed.get());
            state.dismiss_confirmation();
            assert!(state.documents.get(&path).unwrap().dirty());
        });
    }

    #[test]
    fn normal_close_saves_the_panes_before_closing_the_window() {
        with_workspace(|state, closed| {
            let first = focused(&state).id;
            let path = file(&state, "test.rs", "saved");
            state.select_tab(first, Tab::File(path));
            let below = state.panes.split(first, Direction::Down).id;
            state.new_terminal(below);
            state.request_quit();
            assert!(closed.get());
            let restored = layout::load(&state.layout_path).unwrap();
            assert_eq!(restored.open_files, vec![PathBuf::from("test.rs")]);
            assert_eq!(
                restored.panes,
                Some(SavedNode::Split {
                    horizontal: false,
                    sizes: vec![0.5, 0.5],
                    children: vec![
                        SavedNode::Pane {
                            tabs: vec![SavedTab::File("test.rs".into())],
                            active: Some(0),
                        },
                        SavedNode::Pane {
                            tabs: vec![SavedTab::Terminal],
                            active: Some(0),
                        },
                    ],
                })
            );
            assert_eq!(restored.focused, 1);
        });
    }

    #[test]
    fn failed_layout_save_keeps_window_open() {
        with_workspace(|state, closed| {
            std::fs::create_dir(&state.layout_path).unwrap();
            state.request_quit();
            assert!(!closed.get());
            assert!(state.status().get().contains("Could not save layout"));
        });
    }
}
