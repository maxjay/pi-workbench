//! Opens files, selects documents in two panes, and handles confirmations and layout.
use crate::{
    documents::Documents, explorer::Explorer, lens::Lens, sessions::Sessions, syntax::Syntax,
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
use pi_client::Launch;
use std::{
    cell::RefCell,
    collections::HashSet,
    ops::Range,
    path::{Path, PathBuf},
    rc::Rc,
};
use workbench_core::{
    MAX_OPEN_DOCUMENTS,
    diff::Hunk,
    files,
    git::FileStatus,
    layout::{self, Layout},
};

/// The finder overlay: quick-open by file name, or search file contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Finder {
    Files,
    Search,
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
    pub pane: Pane,
    pub path: PathBuf,
    pub to: JumpTo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pane {
    Primary,
    Secondary,
}

impl Pane {
    pub const ALL: [Self; 2] = [Self::Primary, Self::Secondary];

    pub fn index(self) -> usize {
        match self {
            Self::Primary => 0,
            Self::Secondary => 1,
        }
    }
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

pub struct Workbench {
    pub documents: Rc<Documents>,
    pub explorer: Rc<Explorer>,
    pub lens: Rc<Lens>,
    pub syntax: Rc<Syntax>,
    pub sessions: Rc<Sessions>,
    /// The primary pane shows the active session's conversation instead of a file.
    conversation: RwSignal<bool>,
    scope: Scope,
    active: [RwSignal<Option<PathBuf>>; 2],
    requested: [RwSignal<Option<PathBuf>>; 2],
    focused: RwSignal<Pane>,
    pending: RefCell<HashSet<PathBuf>>,
    root: PathBuf,
    layout_path: PathBuf,
    layout: RwSignal<Layout>,
    layout_writable: bool,
    status: RwSignal<String>,
    confirm: RwSignal<Option<Confirm>>,
    palette: RwSignal<bool>,
    finder: RwSignal<Option<Finder>>,
    /// Each pane's cursor line, kept by the pane for change navigation.
    cursor_line: [RwSignal<usize>; 2],
    cursor_col: [RwSignal<usize>; 2],
    jump: RwSignal<Option<Jump>>,
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
        Rc::new(Self {
            documents,
            explorer,
            lens,
            syntax: Syntax::new(Scope::current()),
            sessions,
            conversation: RwSignal::new(true),
            scope: Scope::current(),
            active: [RwSignal::new(None), RwSignal::new(None)],
            requested: [RwSignal::new(None), RwSignal::new(None)],
            focused: RwSignal::new(Pane::Primary),
            pending: RefCell::new(HashSet::new()),
            root,
            layout_path,
            layout_writable,
            status,
            layout: RwSignal::new(layout),
            confirm: RwSignal::new(None),
            palette: RwSignal::new(false),
            finder: RwSignal::new(None),
            cursor_line: [RwSignal::new(0), RwSignal::new(0)],
            cursor_col: [RwSignal::new(0), RwSignal::new(0)],
            jump: RwSignal::new(None),
            kept: RefCell::new(Vec::new()),
            close_window: Box::new(close_window),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn active(&self, pane: Pane) -> ReadSignal<Option<PathBuf>> {
        self.active[pane.index()].read_only()
    }
    pub fn focused(&self) -> ReadSignal<Pane> {
        self.focused.read_only()
    }
    pub fn focus_pane(&self, pane: Pane) {
        self.focused.set(pane);
    }
    pub fn conversation(&self) -> ReadSignal<bool> {
        self.conversation.read_only()
    }
    pub fn show_conversation(&self) {
        self.conversation.set(true);
        self.focused.set(Pane::Primary);
    }
    pub fn layout(&self) -> ReadSignal<Layout> {
        self.layout.read_only()
    }
    pub fn status(&self) -> ReadSignal<String> {
        self.status.read_only()
    }
    pub fn confirmation(&self) -> ReadSignal<Option<Confirm>> {
        self.confirm.read_only()
    }
    pub fn palette(&self) -> ReadSignal<bool> {
        self.palette.read_only()
    }
    pub fn toggle_palette(&self) {
        self.palette.update(|shown| *shown = !*shown);
    }
    pub fn finder(&self) -> ReadSignal<Option<Finder>> {
        self.finder.read_only()
    }
    pub fn open_finder(&self, mode: Finder) {
        self.palette.set(false);
        self.finder.set(Some(mode));
    }
    pub fn dismiss_finder(&self) {
        self.finder.set(None);
    }
    pub fn dismiss_palette(&self) {
        self.palette.set(false);
    }
    pub fn dismiss_confirmation(&self) {
        self.confirm.set(None);
    }

    pub fn restore(self: &Rc<Self>) {
        self.explorer.refresh();
        let layout = self.layout.get_untracked();
        let paths: Vec<_> = layout
            .open_files
            .iter()
            .map(|p| self.root.join(p))
            .collect();
        // Startup runs once, before interaction. Reserve the normalized layout's
        // files so interactive opens share the same limit and do not read twice.
        self.pending.borrow_mut().extend(paths.iter().cloned());
        let state = self.clone();
        let finish = create_ext_action(self.scope, move |results| {
            state.finish_restore(layout, results);
        });
        let root = self.root.clone();
        std::thread::spawn(move || {
            finish(
                paths
                    .into_iter()
                    .map(|path| {
                        let result = files::within(&root, &path)
                            .and_then(|p| files::read_text(&p))
                            .map_err(|e| e.to_string());
                        (path, result)
                    })
                    .collect(),
            );
        });
    }

    fn finish_restore(&self, layout: Layout, results: Vec<(PathBuf, Result<String, String>)>) {
        for (path, result) in results {
            self.pending.borrow_mut().remove(&path);
            match result {
                Ok(text) => self.documents.insert(path, text),
                Err(err) => self.status.set(err),
            }
        }
        for pane in Pane::ALL {
            let index = pane.index();
            let requested = self.requested[index].get_untracked();
            if let Some(path) = requested
                .as_ref()
                .filter(|p| self.documents.get(p).is_some())
            {
                self.active[index].set(Some(path.clone()));
            } else if requested.is_none() && self.active[index].get_untracked().is_none() {
                let saved = layout.active[index].as_ref().map(|p| self.root.join(p));
                let fallback = self
                    .documents
                    .items()
                    .with_untracked(|docs| docs.first().map(|doc| doc.path().clone()));
                self.active[index].set(
                    saved
                        .filter(|p| self.documents.get(p).is_some())
                        .or(fallback),
                );
            }
        }
    }

    pub fn open(self: &Rc<Self>, path: PathBuf, pane: Pane) {
        if pane == Pane::Primary {
            self.conversation.set(false);
        }
        self.requested[pane.index()].set(Some(path.clone()));
        if self.documents.get(&path).is_some() {
            self.active[pane.index()].set(Some(path));
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
            return;
        }
        self.pending.borrow_mut().insert(path.clone());
        self.status.set("Opening…".into());
        let state = self.clone();
        let reading = path.clone();
        let finish = create_ext_action(self.scope, move |result| state.finish_open(path, result));
        let root = self.root.clone();
        std::thread::spawn(move || {
            finish(
                files::within(&root, &reading)
                    .and_then(|p| files::read_text(&p))
                    .map_err(|e| e.to_string()),
            );
        });
    }

    fn finish_open(&self, path: PathBuf, result: Result<String, String>) {
        self.pending.borrow_mut().remove(&path);
        match result {
            Ok(text) => {
                self.documents.insert(path.clone(), text);
                // One read can serve both panes, but must not replace a later selection.
                for pane in Pane::ALL {
                    if self.requested[pane.index()].get_untracked().as_ref() == Some(&path) {
                        self.active[pane.index()].set(Some(path.clone()));
                    }
                }
                if self.active[1].get_untracked().is_none()
                    && self.requested[1].get_untracked().is_none()
                {
                    self.active[1].set(Some(path));
                }
                self.status.set("Ready".into());
            }
            Err(err) => self.status.set(err),
        }
    }

    pub fn select_document(&self, pane: Pane, path: PathBuf) {
        if self.documents.get(&path).is_some() {
            if pane == Pane::Primary {
                self.conversation.set(false);
            }
            self.requested[pane.index()].set(Some(path.clone()));
            self.active[pane.index()].set(Some(path));
            self.focused.set(pane);
        }
    }

    pub fn active_path(&self) -> Option<PathBuf> {
        let pane = self.focused.get_untracked();
        if pane == Pane::Primary && self.conversation.get_untracked() {
            return None;
        }
        self.active(pane).get_untracked()
    }

    pub fn keep(&self, value: impl std::any::Any) {
        self.kept.borrow_mut().push(Box::new(value));
    }

    pub fn cursor_line(&self, pane: Pane) -> RwSignal<usize> {
        self.cursor_line[pane.index()]
    }
    pub fn cursor_col(&self, pane: Pane) -> RwSignal<usize> {
        self.cursor_col[pane.index()]
    }
    pub fn jump(&self) -> RwSignal<Option<Jump>> {
        self.jump
    }

    /// The file in the focused pane, unless the conversation covers it.
    fn focused_file(&self) -> Option<(Pane, PathBuf)> {
        let pane = self.focused.get_untracked();
        if pane == Pane::Primary && self.conversation.get_untracked() {
            return None;
        }
        self.active(pane).get_untracked().map(|path| (pane, path))
    }

    /// The hunks of an open file under the lens; seen ones only if asked.
    pub fn hunks(&self, path: &Path, include_seen: bool) -> Vec<Hunk> {
        let (Some(file), Some(doc)) = (self.lens.file(path), self.documents.get(path)) else {
            return Vec::new();
        };
        let Some(diff) = file.diff().get_untracked() else {
            return Vec::new();
        };
        let text = doc.text().text().to_string();
        let seen = self.lens.seen().get_untracked();
        diff.hunks_iter()
            .filter(|hunk| include_seen || !seen.contains(&self.lens.hunk_key(&file, hunk, &text)))
            .collect()
    }

    /// The hunk under the cursor of the focused pane.
    fn hunk_at_cursor(&self) -> Option<(PathBuf, Hunk)> {
        let (pane, path) = self.focused_file()?;
        let line = self.cursor_line[pane.index()].get_untracked();
        let hunk = self.hunks(&path, true).into_iter().find(|hunk| {
            hunk.current.contains(&line) || (hunk.current.is_empty() && hunk.current.start == line)
        })?;
        Some((path, hunk))
    }

    /// Moves to the next unseen change, continuing into the next changed file.
    pub fn next_change(self: &Rc<Self>, forward: bool) {
        if let Some((pane, path)) = self.focused_file() {
            let line = self.cursor_line[pane.index()].get_untracked();
            let hunks = self.hunks(&path, false);
            let target = if forward {
                hunks.iter().find(|h| h.current.start > line)
            } else {
                hunks.iter().rev().find(|h| h.current.start < line)
            };
            if let Some(hunk) = target {
                self.jump.set(Some(Jump {
                    pane,
                    path,
                    to: JumpTo::Line(hunk.current.start),
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
        let pane = self.focused.get_untracked();
        let path = files[next].clone();
        self.open(path.clone(), pane);
        self.focused.set(pane);
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
        let Some((path, hunk)) = self.hunk_at_cursor() else {
            self.status
                .set("Put the cursor on a change to revert it.".into());
            return;
        };
        let (Some(file), Some(doc)) = (self.lens.file(&path), self.documents.get(&path)) else {
            return;
        };
        let base = file
            .base_text()
            .and_then(|b| (*b).clone())
            .unwrap_or_default();
        let base_lines: Vec<&str> = base.split('\n').map(|l| l.trim_end_matches('\r')).collect();
        let restored = base_lines.get(hunk.base.clone()).unwrap_or_default();
        replace_lines(&doc.text(), hunk.current.clone(), restored);
        self.status
            .set("Reverted the change; undo brings it back.".into());
    }

    /// Puts the change under the cursor into the active session's composer.
    pub fn ask_about_change(&self) {
        let Some((path, hunk)) = self.hunk_at_cursor() else {
            self.status
                .set("Put the cursor on a change to ask Pi about it.".into());
            return;
        };
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
        let session = self
            .sessions
            .active_untracked()
            .unwrap_or_else(|| self.sessions.create());
        let len = session.draft.text().len();
        session.draft.edit_single(
            Selection::caret(len, CursorAffinity::Forward),
            &question,
            EditType::InsertChars,
        );
        self.show_conversation();
    }

    pub fn mark_change_seen(self: &Rc<Self>) {
        let Some((path, hunk)) = self.hunk_at_cursor() else {
            self.status
                .set("Put the cursor on a change to mark it seen.".into());
            return;
        };
        let (Some(file), Some(doc)) = (self.lens.file(&path), self.documents.get(&path)) else {
            return;
        };
        let text = doc.text().text().to_string();
        self.lens.mark_seen(self.lens.hunk_key(&file, &hunk, &text));
        self.next_change(true);
    }

    /// Opens a file in the focused pane and puts the cursor on `line`.
    pub fn open_at(self: &Rc<Self>, path: PathBuf, line: usize) {
        let pane = self.focused.get_untracked();
        self.open(path.clone(), pane);
        self.focused.set(pane);
        self.jump.set(Some(Jump {
            pane,
            path,
            to: JumpTo::Line(line),
        }));
    }

    /// Opens a file next to the conversation, turning on the second pane.
    pub fn open_beside(self: &Rc<Self>, path: PathBuf) {
        if !self.layout.get_untracked().split {
            self.layout.update(|layout| layout.split = true);
        }
        self.open(path, Pane::Secondary);
        self.focused.set(Pane::Secondary);
    }

    pub fn save_active(&self) {
        if let Some(path) = self.active_path()
            && let Some(doc) = self.documents.get(&path)
        {
            self.documents.save(doc);
        }
    }

    pub fn close_active(&self) {
        if let Some(path) = self.active_path() {
            self.close_file(path);
        }
    }

    pub fn close_file(&self, path: PathBuf) {
        let Some(doc) = self.documents.get(&path) else {
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
        let fallback = self
            .documents
            .items()
            .with_untracked(|docs| docs.last().map(|doc| doc.path().clone()));
        for pane in Pane::ALL {
            let index = pane.index();
            if self.active[index].get_untracked().as_deref() == Some(path) {
                self.active[index].set(fallback.clone());
            }
            if self.requested[index].get_untracked().as_deref() == Some(path) {
                self.requested[index].set(None);
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

    pub fn toggle_split(&self) {
        self.layout.update(|layout| layout.split = !layout.split);
        if !self.layout.get_untracked().split {
            self.focused.set(Pane::Primary);
        } else if self.active[1].get_untracked().is_none() {
            self.active[1].set(self.active[0].get_untracked());
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
    pub fn resize_split(&self, fraction: f64) {
        self.layout
            .update(|layout| layout.split_fraction = fraction.clamp(0.2, 0.8));
    }

    pub fn reset_layout(&self) {
        self.layout.set(Layout::default());
        self.focused.set(Pane::Primary);
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
        for pane in Pane::ALL {
            layout.active[pane.index()] = self
                .active(pane)
                .get_untracked()
                .and_then(|p| p.strip_prefix(&self.root).ok().map(Path::to_path_buf));
        }
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
        match self.persist_layout() {
            Ok(()) => (self.close_window)(),
            Err(err) => self.status.set(format!("Could not save layout: {err}")),
        }
        self.dismiss_confirmation();
    }
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

    fn dirty_document(state: &Workbench) -> PathBuf {
        let path = state.root.join("test.rs");
        state.documents.insert(path.clone(), "original".into());
        let doc = state.documents.get(&path).unwrap();
        let text = doc.text();
        text.edit_single(
            Selection::region(0, text.text().len(), CursorAffinity::Backward),
            "unsaved",
            EditType::Other,
        );
        state.select_document(Pane::Primary, path.clone());
        state.select_document(Pane::Secondary, path.clone());
        path
    }

    #[test]
    fn late_open_and_restore_do_not_steal_selection() {
        with_workspace(|state, _| {
            let slow = state.root.join("slow.rs");
            let chosen = state.root.join("chosen.rs");
            state.pending.borrow_mut().insert(slow.clone());
            state.open(slow.clone(), Pane::Primary);
            state.documents.insert(chosen.clone(), "chosen".into());
            state.select_document(Pane::Primary, chosen.clone());
            state.finish_open(slow, Ok("slow".into()));
            let layout = Layout {
                active: [Some("previous.rs".into()), None],
                ..Default::default()
            };
            state.finish_restore(
                layout,
                vec![(state.root.join("previous.rs"), Ok("old".into()))],
            );
            assert_eq!(state.active(Pane::Primary).get(), Some(chosen));
        });
    }

    #[test]
    fn opening_shared_document_satisfies_both_panes_and_close_selects_fallback() {
        with_workspace(|state, _| {
            let shared = state.root.join("shared.rs");
            let remaining = state.root.join("remaining.rs");
            state
                .documents
                .insert(remaining.clone(), "remaining".into());
            state.pending.borrow_mut().insert(shared.clone());
            for pane in Pane::ALL {
                state.open(shared.clone(), pane);
            }
            assert_eq!(state.pending.borrow().len(), 1);
            state.finish_open(shared.clone(), Ok("shared".into()));
            for pane in Pane::ALL {
                assert_eq!(state.active(pane).get(), Some(shared.clone()));
            }
            assert!(state.pending.borrow().is_empty());
            state.close_file(shared.clone());
            assert!(state.documents.get(&shared).is_none());
            for pane in Pane::ALL {
                assert_eq!(state.active(pane).get(), Some(remaining.clone()));
            }
        });
    }

    #[test]
    fn hiding_second_pane_moves_command_target_to_primary() {
        with_workspace(|state, _| {
            let first = state.root.join("first.rs");
            let second = state.root.join("second.rs");
            state.documents.insert(first.clone(), "first".into());
            state.documents.insert(second.clone(), "second".into());
            // The second pane starts hidden; show it, then hide it again.
            state.toggle_split();
            state.select_document(Pane::Primary, first);
            state.select_document(Pane::Secondary, second.clone());
            state.toggle_split();
            assert_eq!(state.focused().get(), Pane::Primary);
            assert_eq!(state.active(Pane::Secondary).get(), Some(second));
        });
    }

    #[test]
    fn restore_honours_saved_selections_and_requests_made_during_loading() {
        with_workspace(|state, _| {
            let first = state.root.join("first.rs");
            let second = state.root.join("second.rs");
            let layout = Layout {
                active: [Some("second.rs".into()), Some("first.rs".into())],
                ..Default::default()
            };
            state.finish_restore(
                layout.clone(),
                vec![
                    (first.clone(), Ok("first".into())),
                    (second.clone(), Ok("second".into())),
                ],
            );
            assert_eq!(state.active(Pane::Primary).get(), Some(second));
            assert_eq!(state.active(Pane::Secondary).get(), Some(first));

            let requested = state.root.join("requested.rs");
            state.pending.borrow_mut().insert(requested.clone());
            state.open(requested.clone(), Pane::Secondary);
            state.finish_restore(layout, vec![(requested.clone(), Ok("requested".into()))]);
            assert_eq!(state.active(Pane::Secondary).get(), Some(requested));
            assert!(state.pending.borrow().is_empty());
        });
    }

    #[test]
    fn open_limit_counts_pending_reads_and_failed_open_preserves_selection() {
        with_workspace(|state, _| {
            let selected = state.root.join("selected.rs");
            state.documents.insert(selected.clone(), "selected".into());
            state.select_document(Pane::Primary, selected.clone());
            let pending: Vec<_> = (1..MAX_OPEN_DOCUMENTS)
                .map(|i| state.root.join(format!("pending-{i}.rs")))
                .collect();
            state.pending.borrow_mut().extend(pending.iter().cloned());
            let extra = state.root.join("extra.rs");
            state.open(extra.clone(), Pane::Primary);
            assert!(!state.pending.borrow().contains(&extra));
            assert!(state.status().get().contains("limit"));

            // Requesting a file that is already loading still works at the limit.
            state.open(pending[0].clone(), Pane::Primary);
            state.finish_open(pending[0].clone(), Err("read failed".into()));
            assert!(!state.pending.borrow().contains(&pending[0]));
            assert_eq!(state.status().get(), "read failed");
            assert_eq!(state.active(Pane::Primary).get(), Some(selected));
        });
    }

    #[test]
    fn dirty_close_waits_for_confirmation_and_discard_closes_both_panes() {
        with_workspace(|state, _| {
            let path = dirty_document(&state);
            state.close_file(path.clone());
            assert_eq!(
                state.confirmation().get(),
                Some(Confirm::Close(path.clone()))
            );
            state.close_when_saved();
            assert!(state.documents.get(&path).is_some());
            state.discard_confirmed();
            assert!(state.documents.get(&path).is_none());
            for pane in Pane::ALL {
                assert!(state.active(pane).get().is_none());
            }
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
    fn normal_close_persists_selected_files_before_closing_window() {
        with_workspace(|state, closed| {
            let path = state.root.join("test.rs");
            state.documents.insert(path.clone(), "saved".into());
            state.select_document(Pane::Primary, path.clone());
            state.request_quit();
            assert!(closed.get());
            let restored = layout::load(&state.layout_path).unwrap();
            assert_eq!(restored.open_files, vec![PathBuf::from("test.rs")]);
            assert_eq!(restored.active[0], Some("test.rs".into()));
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
