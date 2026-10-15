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
    sync::Arc,
    time::Duration,
};
use workbench_text::highlight::{Highlighter, Span};

/// Short enough that colours follow typing, long enough to skip keystrokes.
const DELAY: Duration = Duration::from_millis(120);

pub type Lines = Arc<Vec<Vec<Span>>>;

pub struct Syntax {
    scope: Scope,
    highlighter: Arc<Highlighter>,
    files: RefCell<HashMap<PathBuf, RwSignal<Option<Lines>>>>,
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
        if let Some(lines) = self.files.borrow().get(path) {
            return lines.read_only();
        }
        let lines = self.scope.enter(|| RwSignal::new(None));
        self.files.borrow_mut().insert(path.to_path_buf(), lines);
        let (scope, highlighter, path) = (self.scope, self.highlighter.clone(), path.to_path_buf());
        let pending = Rc::new(Cell::new(0u64));
        self.scope.enter(|| {
            Effect::new(move |_| {
                revision.track();
                let current = pending.get() + 1;
                pending.set(current);
                let (check, text, highlighter, path) = (
                    pending.clone(),
                    text.clone(),
                    highlighter.clone(),
                    path.clone(),
                );
                let highlight = move || {
                    let snapshot = text.text().to_string();
                    let finish = create_ext_action(scope, move |result: Option<Vec<Vec<Span>>>| {
                        if check.get() == current {
                            lines.set(result.map(Arc::new));
                        }
                    });
                    std::thread::spawn(move || {
                        let first = snapshot.lines().next().unwrap_or_default();
                        finish(
                            highlighter
                                .language_for(&path, first)
                                .map(|language| highlighter.highlight(language, &snapshot)),
                        )
                    });
                };
                // The first pass runs at once so a file opens already coloured.
                if current == 1 {
                    highlight();
                } else {
                    let pending = pending.clone();
                    exec_after(DELAY, move |_| {
                        if pending.get() == current {
                            highlight();
                        }
                    });
                }
            })
        });
        lines.read_only()
    }

    pub fn forget(&self, path: &Path) {
        self.files.borrow_mut().remove(path);
    }
}
