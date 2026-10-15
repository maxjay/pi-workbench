//! Compares an open document with a baseline source and keeps the diff current while editing.
use crate::documents::OpenDocument;
use floem::{
    action::exec_after,
    ext_event::create_ext_action,
    prelude::*,
    reactive::{Effect, ReadSignal, Scope},
    views::editor::{text::Document, text_document::TextDocument},
};
use std::{
    cell::Cell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::Duration,
};
use workbench_core::{baseline::BaselineSource, diff::LineDiff};

/// Coalesces bursts of typing into one background diff.
const REFRESH_DELAY: Duration = Duration::from_millis(150);

pub struct Comparison {
    scope: Scope,
    pub document: Rc<OpenDocument>,
    pub label: String,
    /// Read-only buffer holding the baseline text.
    pub base: Rc<TextDocument>,
    diff: RwSignal<Rc<LineDiff>>,
    /// Newest refresh requested and newest applied; results older than the
    /// applied one are dropped, newer ones always land even while typing.
    requested: Rc<Cell<u64>>,
    applied: Rc<Cell<u64>>,
}

impl Drop for Comparison {
    fn drop(&mut self) {
        // Pending refreshes check these before touching disposed signals.
        self.requested.set(u64::MAX);
        self.applied.set(u64::MAX);
        self.scope.dispose();
    }
}

impl Comparison {
    fn new(
        parent: Scope,
        document: Rc<OpenDocument>,
        label: String,
        base_text: String,
        loaded: Loaded,
    ) -> Rc<Self> {
        let scope = parent.create_child();
        scope.enter(|| {
            let base = Rc::new(TextDocument::new(scope, base_text.clone()));
            let diff = RwSignal::new(Rc::new(loaded.diff));
            let (requested, applied) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
            let revision = document.revision();
            let text = document.text();
            let (pending, landed) = (requested.clone(), applied.clone());
            let is_new = loaded.base.is_none();
            Effect::new(move |_| {
                if revision.get() == loaded.revision {
                    return;
                }
                let current = pending.get() + 1;
                pending.set(current);
                let (pending, landed) = (pending.clone(), landed.clone());
                let (text, base_text) = (text.clone(), base_text.clone());
                exec_after(REFRESH_DELAY, move |_| {
                    if pending.get() != current {
                        return;
                    }
                    let snapshot = text.text().to_string();
                    let finish = create_ext_action(scope, move |result| {
                        if current > landed.get() {
                            landed.set(current);
                            diff.set(Rc::new(result));
                        }
                    });
                    std::thread::spawn(move || {
                        finish(if is_new {
                            LineDiff::all_added(&snapshot)
                        } else {
                            LineDiff::compute(&base_text, &snapshot)
                        })
                    });
                });
            });
            Rc::new(Self {
                scope,
                document,
                label,
                base,
                diff,
                requested,
                applied,
            })
        })
    }

    pub fn path(&self) -> &PathBuf {
        self.document.path()
    }
    pub fn diff(&self) -> ReadSignal<Rc<LineDiff>> {
        self.diff.read_only()
    }
}

/// A baseline and its first diff, computed off the UI thread.
struct Loaded {
    base: Option<String>,
    diff: LineDiff,
    /// Document revision the diff was computed against.
    revision: u64,
}

fn load(
    source: &dyn BaselineSource,
    path: &Path,
    text: &str,
    revision: u64,
) -> Result<Loaded, String> {
    let base = source.load(path).map_err(|err| format!("{err:#}"))?;
    Ok(Loaded {
        diff: match &base {
            Some(base) => LineDiff::compute(base, text),
            None => LineDiff::all_added(text),
        },
        base,
        revision,
    })
}

pub fn summary(diff: &LineDiff, label: &str) -> String {
    match diff.hunks() {
        0 => format!("No changes from {label}"),
        hunks => format!(
            "{hunks} change{} from {label} · +{} −{}",
            if hunks == 1 { "" } else { "s" },
            diff.added(),
            diff.removed()
        ),
    }
}

pub struct Changes {
    scope: Scope,
    source: Arc<dyn BaselineSource>,
    status: RwSignal<String>,
    current: RwSignal<Option<Rc<Comparison>>>,
    /// The request allowed to open: only the newest, and closing cancels it.
    loading: RwSignal<Option<(u64, PathBuf)>>,
    next_request: Cell<u64>,
}

impl Changes {
    pub fn new(
        scope: Scope,
        source: Arc<dyn BaselineSource>,
        status: RwSignal<String>,
    ) -> Rc<Self> {
        Rc::new(Self {
            scope,
            source,
            status,
            current: RwSignal::new(None),
            loading: RwSignal::new(None),
            next_request: Cell::new(0),
        })
    }

    pub fn current(&self) -> ReadSignal<Option<Rc<Comparison>>> {
        self.current.read_only()
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.current
            .with_untracked(|current| current.as_ref().map(|c| c.path().clone()))
    }

    pub fn active(&self) -> bool {
        self.current.with_untracked(Option::is_some) || self.loading.with_untracked(Option::is_some)
    }

    fn start(&self, path: PathBuf) -> u64 {
        let request = self.next_request.get() + 1;
        self.next_request.set(request);
        self.loading.set(Some((request, path)));
        request
    }

    pub fn show(self: &Rc<Self>, document: Rc<OpenDocument>) {
        let path = document.path().clone();
        let label = self.source.label();
        let request = self.start(path.clone());
        self.status.set(format!("Comparing with {label}…"));
        let text = document.text().text().to_string();
        let revision = document.revision().get_untracked();
        let source = self.source.clone();
        let changes = self.clone();
        let finish = create_ext_action(self.scope, move |result| {
            changes.finish_show(request, document, label, result)
        });
        std::thread::spawn(move || finish(load(&*source, &path, &text, revision)));
    }

    fn finish_show(
        &self,
        request: u64,
        document: Rc<OpenDocument>,
        label: String,
        result: Result<Loaded, String>,
    ) {
        if self
            .loading
            .with_untracked(|l| l.as_ref().map(|(id, _)| *id))
            != Some(request)
        {
            return;
        }
        self.loading.set(None);
        match result {
            Ok(mut loaded) => {
                self.status.set(summary(&loaded.diff, &label));
                let (label, base) = match loaded.base.take() {
                    Some(base) => (label, base),
                    None => (format!("{label} · new file"), String::new()),
                };
                self.current.set(Some(Comparison::new(
                    self.scope, document, label, base, loaded,
                )));
            }
            Err(err) => self.status.set(format!("Cannot compare: {err}")),
        }
    }

    pub fn close(&self) {
        self.loading.set(None);
        self.current.set(None);
    }

    /// Drops a comparison of, or a pending request for, a closed document.
    pub fn forget(&self, path: &Path) {
        let loading = self
            .loading
            .with_untracked(|l| l.as_ref().is_some_and(|(_, p)| p == path));
        if loading || self.path().as_deref() == Some(path) {
            self.close();
        }
    }
}

/// Test-only: runs the background half of `show` inline, as there is no event loop.
#[cfg(test)]
impl Changes {
    pub fn show_now(&self, document: Rc<OpenDocument>) {
        let request = self.start(document.path().clone());
        self.finish_now(request, document);
    }

    fn finish_now(&self, request: u64, document: Rc<OpenDocument>) {
        let text = document.text().text().to_string();
        let loaded = load(
            &*self.source,
            document.path(),
            &text,
            document.revision().get_untracked(),
        );
        self.finish_show(request, document, self.source.label(), loaded);
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::documents::Documents;

    pub struct Fixed(pub Option<&'static str>);

    impl BaselineSource for Fixed {
        fn label(&self) -> String {
            "fixed".into()
        }
        fn load(&self, _: &Path) -> anyhow::Result<Option<String>> {
            Ok(self.0.map(String::from))
        }
    }

    fn with_changes(source: Fixed, test: impl FnOnce(Rc<Changes>, Rc<OpenDocument>)) {
        let scope = Scope::new();
        scope.enter(|| {
            let status = RwSignal::new(String::new());
            let documents = Documents::new(scope, "/".into(), status);
            documents.insert("/a.rs".into(), "one\ntwo\n".into());
            let document = documents.get(Path::new("/a.rs")).unwrap();
            test(Changes::new(scope, Arc::new(source), status), document);
        });
        scope.dispose();
    }

    #[test]
    fn shows_the_diff_and_close_cancels_pending_loads() {
        with_changes(Fixed(Some("one\n")), |changes, document| {
            changes.show_now(document.clone());
            let comparison = changes.current().get().unwrap();
            assert_eq!(comparison.base.text().to_string(), "one\n");
            assert_eq!(comparison.diff().get().added(), 1);
            assert!(changes.status.get().contains("1 change from fixed"));

            let pending = changes.start(document.path().clone());
            changes.close();
            changes.finish_now(pending, document.clone());
            assert!(changes.current().get().is_none());
            assert!(!changes.active());
        });
    }

    #[test]
    fn only_the_newest_request_opens_and_closing_the_file_cancels_it() {
        with_changes(Fixed(Some("one\n")), |changes, document| {
            let stale = changes.start(document.path().clone());
            let newest = changes.start(document.path().clone());
            changes.finish_now(stale, document.clone());
            assert!(changes.current().get().is_none());

            changes.forget(document.path());
            changes.finish_now(newest, document.clone());
            assert!(changes.current().get().is_none());

            changes.show_now(document.clone());
            changes.forget(document.path());
            assert!(!changes.active());
        });
    }

    #[test]
    fn missing_baseline_is_a_new_file_with_every_line_added() {
        with_changes(Fixed(None), |changes, document| {
            changes.show_now(document);
            let comparison = changes.current().get().unwrap();
            assert_eq!(comparison.label, "fixed · new file");
            assert_eq!(comparison.base.text().to_string(), "");
            let diff = comparison.diff().get();
            assert_eq!((diff.added(), diff.removed()), (2, 0));
        });
    }
}
