use floem::{
    ext_event::create_ext_action,
    prelude::*,
    reactive::Scope,
    views::editor::{text::Document, text_document::TextDocument},
    window::WindowId,
};
use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
    rc::Rc,
};
use workbench_core::{
    files::{self, Entry},
    layout::{self, Layout},
};

pub struct OpenDocument {
    scope: Scope,
    pub path: PathBuf,
    pub text: Rc<TextDocument>,
    pub baseline: RwSignal<String>,
    pub revision: RwSignal<u64>,
    pub saved_revision: RwSignal<u64>,
    pub saving: RwSignal<bool>,
}

impl Drop for OpenDocument {
    fn drop(&mut self) {
        self.scope.dispose();
    }
}

impl OpenDocument {
    pub fn dirty(&self) -> bool {
        self.revision.get() != self.saved_revision.get()
    }
}

#[derive(Clone)]
pub struct TreeRow {
    pub entry: Entry,
    pub depth: usize,
}

#[derive(Clone)]
pub enum Confirm {
    Quit,
    Close(PathBuf),
    Reload(PathBuf),
}

pub struct Workbench {
    pub scope: Scope,
    pub window: WindowId,
    pub root: PathBuf,
    pub layout_path: PathBuf,
    pub layout: RwSignal<Layout>,
    pub docs: RwSignal<Vec<Rc<OpenDocument>>>,
    pub active: [RwSignal<Option<PathBuf>>; 2],
    pub focused: RwSignal<usize>,
    pub requested: [RwSignal<Option<PathBuf>>; 2],
    pub pending: RwSignal<HashSet<PathBuf>>,
    pub directories: RwSignal<BTreeMap<PathBuf, Vec<Entry>>>,
    pub expanded: RwSignal<HashSet<PathBuf>>,
    pub status: RwSignal<String>,
    pub confirm: RwSignal<Option<Confirm>>,
    pub palette: RwSignal<bool>,
    pub palette_query: RwSignal<String>,
    pub window_width: RwSignal<f64>,
    pub layout_writable: bool,
}

impl Workbench {
    pub fn new(window: WindowId, root: PathBuf, layout_path: PathBuf) -> Rc<Self> {
        let (layout, status, layout_writable) = match layout::load(&layout_path) {
            Ok(layout) => (layout, "Ready · Open a file from the explorer".into(), true),
            Err(err) => (
                Layout::default(),
                format!("{err}. Using defaults; the saved layout will be preserved."),
                false,
            ),
        };
        Rc::new(Self {
            scope: Scope::current(),
            window,
            root,
            layout_path,
            layout: RwSignal::new(layout),
            docs: RwSignal::new(Vec::new()),
            active: [RwSignal::new(None), RwSignal::new(None)],
            requested: [RwSignal::new(None), RwSignal::new(None)],
            focused: RwSignal::new(0),
            pending: RwSignal::new(HashSet::new()),
            directories: RwSignal::new(BTreeMap::new()),
            expanded: RwSignal::new(HashSet::new()),
            status: RwSignal::new(status),
            confirm: RwSignal::new(None),
            palette: RwSignal::new(false),
            palette_query: RwSignal::new(String::new()),
            window_width: RwSignal::new(1280.0),
            layout_writable,
        })
    }

    pub fn document(&self, path: &PathBuf) -> Option<Rc<OpenDocument>> {
        self.docs
            .with_untracked(|docs| docs.iter().find(|doc| doc.path == *path).cloned())
    }

    fn insert(&self, path: PathBuf, content: String) {
        if self.document(&path).is_some() {
            return;
        }
        // Documents belong to the workbench scope, never a temporary tab or async callback.
        let document_scope = self.scope.create_child();
        let document = document_scope.enter(|| {
            let revision = RwSignal::new(0);
            let text = Rc::new(TextDocument::new(document_scope, content.clone()));
            text.add_on_update(move |_| revision.update(|r| *r += 1));
            Rc::new(OpenDocument {
                scope: document_scope,
                path,
                text,
                baseline: RwSignal::new(content),
                revision,
                saved_revision: RwSignal::new(0),
                saving: RwSignal::new(false),
            })
        });
        self.docs.update(|docs| docs.push(document));
    }

    pub fn restore(self: &Rc<Self>) {
        self.read_directory(self.root.clone());
        let layout = self.layout.get_untracked();
        let root = self.root.clone();
        let state = self.clone();
        let finish = create_ext_action(
            self.scope,
            move |results: Vec<(PathBuf, Result<String, String>)>| {
                for (path, result) in results {
                    match result {
                        Ok(text) => state.insert(path, text),
                        Err(err) => state.status.set(format!("Could not restore file: {err}")),
                    }
                }
                for pane in 0..2 {
                    if state.active[pane].get_untracked().is_none()
                        && state.requested[pane].get_untracked().is_none()
                    {
                        let wanted = layout.active[pane].as_ref().map(|p| state.root.join(p));
                        let fallback = state
                            .docs
                            .with_untracked(|docs| docs.first().map(|d| d.path.clone()));
                        state.active[pane]
                            .set(wanted.filter(|p| state.document(p).is_some()).or(fallback));
                    }
                }
            },
        );
        let paths = self.layout.get_untracked().open_files;
        std::thread::spawn(move || {
            let results = paths
                .into_iter()
                .map(|relative| {
                    let path = root.join(relative);
                    let content = files::within(&root, &path)
                        .and_then(|p| files::read_text(&p))
                        .map_err(|e| e.to_string());
                    (path, content)
                })
                .collect();
            finish(results);
        });
    }

    pub fn open(self: &Rc<Self>, path: PathBuf, pane: usize) {
        self.requested[pane].set(Some(path.clone()));
        if self.document(&path).is_some() {
            self.active[pane].set(Some(path));
            return;
        }
        if self.pending.with_untracked(|p| p.contains(&path)) {
            return;
        }
        if self.docs.with_untracked(Vec::len) + self.pending.with_untracked(HashSet::len) >= 64 {
            self.status
                .set("Close a file before opening more (64-document limit).".into());
            return;
        }
        self.pending.update(|pending| {
            pending.insert(path.clone());
        });
        self.status.set(format!("Opening {}", path.display()));
        let root = self.root.clone();
        let state = self.clone();
        let result_path = path.clone();
        let finish = create_ext_action(self.scope, move |result: Result<String, String>| {
            state.pending.update(|pending| {
                pending.remove(&result_path);
            });
            match result {
                Ok(text) => {
                    state.insert(result_path.clone(), text);
                    for pane in 0..2 {
                        if state.requested[pane].get_untracked().as_ref() == Some(&result_path) {
                            state.active[pane].set(Some(result_path.clone()));
                        }
                    }
                    if state.active[1].get_untracked().is_none() {
                        state.active[1].set(Some(result_path));
                    }
                    state.status.set("Ready".into());
                }
                Err(err) => state.status.set(err),
            }
        });
        std::thread::spawn(move || {
            finish(
                files::within(&root, &path)
                    .and_then(|p| files::read_text(&p))
                    .map_err(|e| e.to_string()),
            )
        });
    }

    pub fn save(self: &Rc<Self>, doc: Rc<OpenDocument>) {
        if doc.saving.get_untracked() {
            return;
        }
        let text = doc.text.text().to_string();
        let baseline = doc.baseline.get_untracked();
        let revision = doc.revision.get_untracked();
        if text == baseline {
            doc.saved_revision.set(revision);
            return;
        }
        doc.saving.set(true);
        self.status.set("Saving…".into());
        let path = doc.path.clone();
        let state = self.clone();
        let saved_text = text.clone();
        let finish = create_ext_action(self.scope, move |result: Result<(), String>| {
            doc.saving.set(false);
            match result {
                Ok(()) => {
                    doc.baseline.set(saved_text);
                    // An edit made while saving must remain dirty.
                    doc.saved_revision.set(revision);
                    state.status.set(format!(
                        "Saved {}",
                        doc.path
                            .strip_prefix(&state.root)
                            .unwrap_or(&doc.path)
                            .display()
                    ));
                }
                Err(err) => state.status.set(err),
            }
        });
        std::thread::spawn(move || {
            finish(files::save_text(&path, &baseline, &text).map_err(|e| e.to_string()))
        });
    }

    pub fn save_active(self: &Rc<Self>) {
        if let Some(path) = self.active[self.focused.get_untracked()].get_untracked()
            && let Some(doc) = self.document(&path)
        {
            self.save(doc);
        }
    }

    pub fn save_all(self: &Rc<Self>) {
        for doc in self.docs.get_untracked() {
            if doc.dirty() {
                self.save(doc);
            }
        }
    }

    pub fn close_file(self: &Rc<Self>, path: PathBuf) {
        if self
            .document(&path)
            .is_some_and(|doc| doc.saving.get_untracked())
        {
            self.status
                .set("Wait for this file to finish saving.".into());
            return;
        }
        if self.document(&path).is_some_and(|doc| doc.dirty()) {
            self.confirm.set(Some(Confirm::Close(path)));
            return;
        }
        self.remove_file(&path);
    }

    pub fn remove_file(&self, path: &PathBuf) {
        self.docs
            .update(|docs| docs.retain(|doc| doc.path != *path));
        let fallback = self
            .docs
            .with_untracked(|docs| docs.last().map(|doc| doc.path.clone()));
        for pane in 0..2 {
            if self.active[pane].get_untracked().as_ref() == Some(path) {
                self.active[pane].set(fallback.clone());
            }
            if self.requested[pane].get_untracked().as_ref() == Some(path) {
                self.requested[pane].set(None);
            }
        }
    }

    pub fn reload(self: &Rc<Self>, path: PathBuf) {
        let Some(doc) = self.document(&path) else {
            return;
        };
        if doc.saving.get_untracked() {
            self.status.set("Wait for saving to finish.".into());
            return;
        }
        let revision = doc.revision.get_untracked();
        let state = self.clone();
        let finish = create_ext_action(self.scope, move |result: Result<String, String>| {
            if doc.revision.get_untracked() != revision {
                state
                    .status
                    .set("Reload cancelled: the document was edited while reading.".into());
                return;
            }
            if state.document(&doc.path).is_none() {
                return;
            }
            match result {
                Ok(text) => {
                    use floem::views::editor::core::{
                        cursor::CursorAffinity, editor::EditType, selection::Selection,
                    };
                    doc.text.edit_single(
                        Selection::region(0, doc.text.text().len(), CursorAffinity::Backward),
                        &text,
                        EditType::Other,
                    );
                    doc.baseline.set(text);
                    doc.saved_revision.set(doc.revision.get_untracked());
                    state.status.set("Reloaded from disk".into());
                }
                Err(err) => state.status.set(err),
            }
        });
        std::thread::spawn(move || finish(files::read_text(&path).map_err(|e| e.to_string())));
    }

    pub fn read_directory(self: &Rc<Self>, path: PathBuf) {
        let root = self.root.clone();
        let state = self.clone();
        let result_path = path.clone();
        let finish =
            create_ext_action(
                self.scope,
                move |result: Result<Vec<Entry>, String>| match result {
                    Ok(entries) => state.directories.update(|cache| {
                        cache.insert(result_path, entries);
                    }),
                    Err(err) => state.status.set(err),
                },
            );
        std::thread::spawn(move || {
            finish(files::list_directory(&root, &path).map_err(|e| e.to_string()))
        });
    }

    pub fn toggle_directory(self: &Rc<Self>, path: PathBuf) {
        let opening = !self.expanded.with_untracked(|e| e.contains(&path));
        self.expanded.update(|expanded| {
            if opening {
                expanded.insert(path.clone());
            } else {
                expanded.remove(&path);
            }
        });
        if opening {
            self.read_directory(path);
        }
    }

    pub fn rows(&self) -> Vec<TreeRow> {
        let cache = self.directories.get();
        let expanded = self.expanded.get();
        fn visit(
            path: &PathBuf,
            depth: usize,
            cache: &BTreeMap<PathBuf, Vec<Entry>>,
            expanded: &HashSet<PathBuf>,
            rows: &mut Vec<TreeRow>,
        ) {
            if let Some(entries) = cache.get(path) {
                for entry in entries {
                    rows.push(TreeRow {
                        entry: entry.clone(),
                        depth,
                    });
                    if entry.directory && expanded.contains(&entry.path) {
                        visit(&entry.path, depth + 1, cache, expanded, rows);
                    }
                }
            }
        }
        let mut rows = Vec::new();
        visit(&self.root, 0, &cache, &expanded, &mut rows);
        rows
    }

    pub fn persist_layout(&self) -> anyhow::Result<()> {
        if !self.layout_writable {
            return Ok(());
        }
        let mut layout = self.layout.get_untracked();
        layout.open_files = self.docs.with_untracked(|docs| {
            docs.iter()
                .filter_map(|d| {
                    d.path
                        .strip_prefix(&self.root)
                        .ok()
                        .map(|p| p.to_path_buf())
                })
                .collect()
        });
        for pane in 0..2 {
            layout.active[pane] = self.active[pane]
                .get_untracked()
                .and_then(|p| p.strip_prefix(&self.root).ok().map(|p| p.to_path_buf()));
        }
        layout::save(&self.layout_path, &layout)
    }

    pub fn request_quit(&self) {
        if self.docs.with_untracked(|docs| {
            docs.iter()
                .any(|doc| doc.dirty() || doc.saving.get_untracked())
        }) {
            self.confirm.set(Some(Confirm::Quit));
        } else {
            self.finish_quit();
        }
    }

    pub fn finish_quit(&self) {
        match self.persist_layout() {
            Ok(()) => floem::close_window(self.window),
            Err(err) => self.status.set(format!("Could not save layout: {err}")),
        }
    }
}
