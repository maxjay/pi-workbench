//! Syntax highlighting for open files, computed off the UI thread after edits.
use floem::{
    action::exec_after,
    ext_event::create_ext_action,
    prelude::*,
    reactive::{Effect, ReadSignal, Scope},
    views::editor::{text::Document, text_document::TextDocument},
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use workbench_text::highlight::{Highlighter, Span};

/// Short enough that colours follow typing, long enough to skip keystrokes.
const DELAY: Duration = Duration::from_millis(120);

pub type Lines = Arc<Vec<Vec<Span>>>;

struct File {
    lines: RwSignal<Option<Lines>>,
    /// Owns the effect that follows edits, so closing the file frees it.
    scope: Scope,
}

pub struct Syntax {
    scope: Scope,
    highlighter: Arc<Highlighter>,
    files: RefCell<HashMap<PathBuf, File>>,
}

/// One file's highlighting: at most one pass runs, and a newer edit cancels it.
struct Worker {
    path: PathBuf,
    text: std::rc::Weak<TextDocument>,
    lines: RwSignal<Option<Lines>>,
    highlighter: Arc<Highlighter>,
    scope: Scope,
    /// The newest edit; a pass for an older one stops early.
    latest: Arc<AtomicU64>,
    running: Cell<bool>,
}

impl Worker {
    fn start(self: &Rc<Self>) {
        if self.running.get() {
            // The running pass sees it is stale and starts again when done.
            return;
        }
        let Some(text) = self.text.upgrade() else {
            return;
        };
        self.running.set(true);
        let generation = self.latest.load(Ordering::Relaxed);
        let worker = self.clone();
        let finish =
            create_ext_action(self.scope, move |result: Option<Option<Vec<Vec<Span>>>>| {
                worker.running.set(false);
                match result {
                    // Finished for the newest text: show it.
                    Some(lines) if worker.latest.load(Ordering::Relaxed) == generation => {
                        worker.lines.set(lines.map(Arc::new))
                    }
                    // Stale or cancelled: highlight the newest text instead.
                    _ => worker.start(),
                }
            });
        // The rope is shared, so only the thread pays for turning it into a string.
        let rope = text.text();
        let (highlighter, path, latest) = (
            self.highlighter.clone(),
            self.path.clone(),
            self.latest.clone(),
        );
        std::thread::spawn(move || {
            let snapshot = rope.to_string();
            let first = snapshot.lines().next().unwrap_or_default();
            let result = match highlighter.language_for(&path, first) {
                Some(language) => highlighter
                    .highlight_until(language, &snapshot, || {
                        latest.load(Ordering::Relaxed) != generation
                    })
                    .map(Some),
                None => Some(None),
            };
            finish(result)
        });
    }
}

impl Syntax {
    pub fn new(scope: Scope) -> Rc<Self> {
        Rc::new(Self {
            scope,
            highlighter: Arc::new(Highlighter::new()),
            files: RefCell::new(HashMap::new()),
        })
    }

    /// Highlights a file now and after each burst of edits.
    pub fn track(
        &self,
        path: &Path,
        text: Rc<TextDocument>,
        revision: ReadSignal<u64>,
    ) -> ReadSignal<Option<Lines>> {
        if let Some(file) = self.files.borrow().get(path) {
            return file.lines.read_only();
        }
        let lines = self.scope.enter(|| RwSignal::new(None));
        let scope = self.scope.create_child();
        self.files
            .borrow_mut()
            .insert(path.to_path_buf(), File { lines, scope });
        let worker = Rc::new(Worker {
            path: path.to_path_buf(),
            text: Rc::downgrade(&text),
            lines,
            highlighter: self.highlighter.clone(),
            scope,
            latest: Arc::new(AtomicU64::new(0)),
            running: Cell::new(false),
        });
        scope.enter(|| {
            Effect::new(move |_| {
                revision.track();
                let current = worker.latest.fetch_add(1, Ordering::Relaxed) + 1;
                // The first pass runs at once so a file opens already coloured.
                if current == 1 {
                    worker.start();
                } else {
                    let worker = worker.clone();
                    exec_after(DELAY, move |_| {
                        if worker.latest.load(Ordering::Relaxed) == current {
                            worker.start();
                        }
                    });
                }
            })
        });
        lines.read_only()
    }

    pub fn forget(&self, path: &Path) {
        if let Some(file) = self.files.borrow_mut().remove(path) {
            file.scope.dispose();
        }
    }
}
