//! Owns shared editor buffers, saves, and reloads.
use floem::reactive::{ReadSignal, Scope};
use floem::{
    ext_event::create_ext_action,
    prelude::*,
    views::editor::{
        core::{cursor::CursorAffinity, editor::EditType, selection::Selection},
        text::Document,
        text_document::TextDocument,
    },
};
use std::{
    path::{Path, PathBuf},
    rc::Rc,
};
use workbench_core::files;

pub struct OpenDocument {
    scope: Scope,
    path: PathBuf,
    text: Rc<TextDocument>,
    baseline: RwSignal<String>,
    revision: RwSignal<u64>,
    saved_revision: RwSignal<u64>,
    saving: RwSignal<bool>,
    read_generation: RwSignal<u64>,
}

struct SaveSnapshot {
    text: String,
    baseline: String,
    revision: u64,
}

#[derive(Clone, Copy)]
struct ReloadTicket {
    revision: u64,
    generation: u64,
}

impl Drop for OpenDocument {
    fn drop(&mut self) {
        self.scope.dispose();
    }
}

impl OpenDocument {
    fn new(parent: Scope, path: PathBuf, content: String) -> Rc<Self> {
        // Views can come and go without destroying a buffer or its undo history.
        let scope = parent.create_child();
        scope.enter(|| {
            let revision = RwSignal::new(0);
            let text = Rc::new(TextDocument::new(scope, content.clone()));
            text.add_on_update(move |_| revision.update(|r| *r += 1));
            Rc::new(Self {
                scope,
                path,
                text,
                baseline: RwSignal::new(content),
                revision,
                saved_revision: RwSignal::new(0),
                saving: RwSignal::new(false),
                read_generation: RwSignal::new(0),
            })
        })
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }
    pub fn text(&self) -> Rc<TextDocument> {
        self.text.clone()
    }
    /// Increments on every edit, including reloads.
    pub fn revision(&self) -> ReadSignal<u64> {
        self.revision.read_only()
    }
    pub fn dirty(&self) -> bool {
        self.revision.get() != self.saved_revision.get()
    }
    pub fn saving(&self) -> bool {
        self.saving.get()
    }
    pub fn can_close(&self) -> bool {
        !self.dirty() && !self.saving()
    }

    fn start_save(&self) -> Option<SaveSnapshot> {
        if self.saving.get_untracked() {
            return None;
        }
        let text = self.text.text().to_string();
        let baseline = self.baseline.get_untracked();
        let revision = self.revision.get_untracked();
        if text == baseline {
            self.saved_revision.set(revision);
            return None;
        }
        self.saving.set(true);
        // A reload started before this save must not overwrite its result.
        self.read_generation.update(|generation| *generation += 1);
        Some(SaveSnapshot {
            text,
            baseline,
            revision,
        })
    }

    fn finish_save(
        &self,
        snapshot: SaveSnapshot,
        result: Result<(), String>,
    ) -> Result<(), String> {
        self.saving.set(false);
        result?;
        self.baseline.set(snapshot.text);
        // Edits made after the snapshot remain dirty.
        self.saved_revision.set(snapshot.revision);
        Ok(())
    }

    fn start_reload(&self) -> Result<ReloadTicket, String> {
        if self.saving.get_untracked() {
            return Err("Wait for saving to finish.".into());
        }
        self.read_generation.update(|generation| *generation += 1);
        Ok(ReloadTicket {
            revision: self.revision.get_untracked(),
            generation: self.read_generation.get_untracked(),
        })
    }

    fn finish_reload(&self, ticket: ReloadTicket, text: String) -> Result<(), String> {
        if self.revision.get_untracked() != ticket.revision
            || self.read_generation.get_untracked() != ticket.generation
        {
            return Err(
                "Reload cancelled: the document changed or a newer operation started.".into(),
            );
        }
        self.text.edit_single(
            Selection::region(0, self.text.text().len(), CursorAffinity::Backward),
            &text,
            EditType::Other,
        );
        self.baseline.set(text);
        self.saved_revision.set(self.revision.get_untracked());
        Ok(())
    }
}

pub struct Documents {
    scope: Scope,
    root: PathBuf,
    items: RwSignal<Vec<Rc<OpenDocument>>>,
    status: RwSignal<String>,
}

impl Documents {
    pub fn new(scope: Scope, root: PathBuf, status: RwSignal<String>) -> Rc<Self> {
        Rc::new(Self {
            scope,
            root,
            status,
            items: RwSignal::new(Vec::new()),
        })
    }

    pub fn items(&self) -> ReadSignal<Vec<Rc<OpenDocument>>> {
        self.items.read_only()
    }

    pub fn get(&self, path: &Path) -> Option<Rc<OpenDocument>> {
        self.items
            .with_untracked(|docs| docs.iter().find(|doc| doc.path == path).cloned())
    }

    pub fn all_saved(&self) -> bool {
        self.items
            .with_untracked(|docs| docs.iter().all(|doc| doc.can_close()))
    }

    pub fn any_saving(&self) -> bool {
        self.items
            .with_untracked(|docs| docs.iter().any(|doc| doc.saving()))
    }

    pub fn insert(&self, path: PathBuf, text: String) {
        if self.get(&path).is_none() {
            let doc = OpenDocument::new(self.scope, path, text);
            self.items.update(|docs| docs.push(doc));
        }
    }

    pub fn remove(&self, path: &Path) {
        self.items
            .update(|docs| docs.retain(|doc| doc.path != path));
    }

    pub fn save(&self, doc: Rc<OpenDocument>) {
        let Some(snapshot) = doc.start_save() else {
            return;
        };
        self.status.set("Saving…".into());
        let path = doc.path.clone();
        let root = self.root.clone();
        let status = self.status;
        let finish = create_ext_action(self.scope, move |(snapshot, result)| {
            match doc.finish_save(snapshot, result) {
                Ok(()) => status.set(format!(
                    "Saved {}",
                    doc.path.strip_prefix(&root).unwrap_or(&doc.path).display()
                )),
                Err(err) => status.set(err),
            }
        });
        std::thread::spawn(move || {
            let result = files::save_text(&path, &snapshot.baseline, &snapshot.text)
                .map_err(|e| e.to_string());
            finish((snapshot, result));
        });
    }

    pub fn save_all(&self) {
        for doc in self.items.get_untracked() {
            if doc.dirty() {
                self.save(doc);
            }
        }
    }

    pub fn reload(self: &Rc<Self>, path: PathBuf) {
        let Some(doc) = self.get(&path) else {
            return;
        };
        let ticket = match doc.start_reload() {
            Ok(ticket) => ticket,
            Err(err) => {
                self.status.set(err);
                return;
            }
        };
        let documents = self.clone();
        let finish = create_ext_action(self.scope, move |result: Result<String, String>| {
            // Closing and reopening the same path creates a different buffer.
            if !documents
                .get(&doc.path)
                .is_some_and(|current| Rc::ptr_eq(&current, &doc))
            {
                return;
            }
            match result.and_then(|text| doc.finish_reload(ticket, text)) {
                Ok(()) => documents.status.set("Reloaded from disk".into()),
                Err(err) => documents.status.set(err),
            }
        });
        std::thread::spawn(move || finish(files::read_text(&path).map_err(|e| e.to_string())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn edit(doc: &OpenDocument, text: &str) {
        doc.text.edit_single(
            Selection::region(0, doc.text.text().len(), CursorAffinity::Backward),
            text,
            EditType::Other,
        );
    }

    fn with_document(test: impl FnOnce(Rc<OpenDocument>)) {
        let scope = Scope::new();
        scope.enter(|| {
            test(OpenDocument::new(
                scope,
                "test.rs".into(),
                "original".into(),
            ))
        });
        scope.dispose();
    }

    #[test]
    fn edits_during_save_remain_dirty_and_next_save_uses_new_baseline() {
        with_document(|doc| {
            edit(&doc, "first edit");
            let snapshot = doc.start_save().unwrap();
            assert!(doc.start_save().is_none());
            assert!(!doc.can_close());
            edit(&doc, "second edit");
            doc.finish_save(snapshot, Ok(())).unwrap();
            assert!(doc.dirty());
            let next = doc.start_save().unwrap();
            assert_eq!(next.baseline, "first edit");
            assert_eq!(next.text, "second edit");
            doc.finish_save(next, Ok(())).unwrap();
            assert!(doc.can_close());
        });
    }

    #[test]
    fn failed_save_preserves_edits_and_allows_retry() {
        with_document(|doc| {
            edit(&doc, "unsaved");
            let snapshot = doc.start_save().unwrap();
            assert!(doc.finish_save(snapshot, Err("disk full".into())).is_err());
            assert!(!doc.can_close());
            assert!(!doc.saving());
            let retry = doc.start_save().unwrap();
            assert_eq!(retry.baseline, "original");
            assert_eq!(retry.text, "unsaved");
        });
    }

    #[test]
    fn reload_cannot_overwrite_new_edits_or_a_newer_reload() {
        with_document(|doc| {
            let old = doc.start_reload().unwrap();
            let newest = doc.start_reload().unwrap();
            assert!(doc.finish_reload(old, "stale".into()).is_err());
            doc.finish_reload(newest, "disk".into()).unwrap();
            assert_eq!(doc.text.text().to_string(), "disk");
            assert!(doc.can_close());
            let pending = doc.start_reload().unwrap();
            edit(&doc, "keep my edit");
            assert!(doc.finish_reload(pending, "discarded".into()).is_err());
            assert_eq!(doc.text.text().to_string(), "keep my edit");
            assert!(doc.dirty());
        });
    }

    #[test]
    fn save_invalidates_pending_reload_and_blocks_another_reload() {
        with_document(|doc| {
            edit(&doc, "edited");
            let pending = doc.start_reload().unwrap();
            let snapshot = doc.start_save().unwrap();
            assert!(doc.start_reload().is_err());
            doc.finish_save(snapshot, Ok(())).unwrap();
            assert!(doc.finish_reload(pending, "old disk text".into()).is_err());
            assert_eq!(doc.text.text().to_string(), "edited");
        });
    }
}
