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
    borrow::Cow,
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
};
use workbench_core::{diff::LineDiff, files, merge::merge};

pub struct OpenDocument {
    scope: Scope,
    path: PathBuf,
    text: Rc<TextDocument>,
    baseline: RwSignal<String>,
    revision: RwSignal<u64>,
    saved_revision: RwSignal<u64>,
    saving: RwSignal<bool>,
    read_generation: RwSignal<u64>,
    /// Lines the last outside change touched, highlighted until the next edit.
    incoming: RwSignal<Option<Rc<LineDiff>>>,
    /// A disk change arrived mid-save; merge it once the save settles.
    resync: Cell<Option<&'static str>>,
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
            let incoming = RwSignal::new(None);
            let text = Rc::new(TextDocument::new(scope, content.clone()));
            text.add_on_update(move |_| {
                revision.update(|r| *r += 1);
                // Setting notifies even when unchanged, which would restyle every keystroke.
                if incoming.with_untracked(Option::is_some) {
                    incoming.set(None);
                }
            });
            Rc::new(Self {
                scope,
                path,
                text,
                baseline: RwSignal::new(content),
                revision,
                saved_revision: RwSignal::new(0),
                saving: RwSignal::new(false),
                read_generation: RwSignal::new(0),
                incoming,
                resync: Cell::new(None),
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
    pub fn incoming(&self) -> ReadSignal<Option<Rc<LineDiff>>> {
        self.incoming.read_only()
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

    /// Brings in a change someone else made on disk, keeping unsaved edits.
    fn sync_disk(&self, disk: String, source: &'static str) -> Synced {
        let baseline = self.baseline.get_untracked();
        if disk == baseline {
            return Synced::Unchanged;
        }
        if self.saving.get_untracked() {
            // The save result decides the baseline; merge once it settles.
            self.resync.set(Some(source));
            return Synced::Deferred;
        }
        // The buffer keeps the line ending it opened with, and Pi may write
        // either, so merge without line endings and convert back.
        let ours = self.text.text().to_string();
        let (base_lf, ours_lf, disk_lf) = (lf(&baseline), lf(&ours), lf(&disk));
        let clean = ours_lf == base_lf;
        if disk_lf == base_lf {
            self.baseline.set(disk);
            return Synced::Unchanged;
        }
        let merged = merge(&base_lf, &ours_lf, &disk_lf, source);
        let merged_text = if ours.contains("\r\n") {
            merged.text.replace('\n', "\r\n")
        } else {
            merged.text
        };
        // A reload started earlier must not overwrite the merged text.
        self.read_generation.update(|generation| *generation += 1);
        replace_changed_region(&self.text, &ours, &merged_text);
        let now = self.text.text().to_string();
        if now != ours {
            self.incoming
                .set(Some(Rc::new(LineDiff::compute(&ours, &now))));
        }
        // Clean only if the buffer now says what the disk says.
        if lf(&now) == disk_lf {
            self.saved_revision.set(self.revision.get_untracked());
        }
        self.baseline.set(disk);
        if clean {
            Synced::Reloaded
        } else {
            Synced::Merged {
                conflicts: merged.conflicts,
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Synced {
    Unchanged,
    Deferred,
    Reloaded,
    Merged { conflicts: usize },
}

fn lf(text: &str) -> Cow<'_, str> {
    if text.contains("\r\n") {
        Cow::Owned(text.replace("\r\n", "\n"))
    } else {
        Cow::Borrowed(text)
    }
}

/// Edits only the span that differs, so cursors and undo history elsewhere survive.
fn replace_changed_region(text: &TextDocument, old: &str, new: &str) {
    if old == new {
        return;
    }
    let prefix = old
        .char_indices()
        .zip(new.chars())
        .find(|((_, a), b)| a != b)
        .map_or(old.len().min(new.len()), |((i, _), _)| i);
    let suffix: usize = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    text.edit_single(
        Selection::region(prefix, old.len() - suffix, CursorAffinity::Backward),
        &new[prefix..new.len() - suffix],
        EditType::Other,
    );
}

pub struct Documents {
    scope: Scope,
    root: PathBuf,
    items: RwSignal<Vec<Rc<OpenDocument>>>,
    status: RwSignal<String>,
    /// While Pi works, saves wait so they cannot race its writes; the edits
    /// stay in the buffer and are merged with Pi's.
    /// Sessions currently running; saves wait until none are.
    held: Cell<usize>,
    deferred: RefCell<Vec<PathBuf>>,
    /// Only the newest sync applies; an older read may predate a later write.
    sync_generation: Cell<u64>,
}

impl Documents {
    pub fn new(scope: Scope, root: PathBuf, status: RwSignal<String>) -> Rc<Self> {
        Rc::new(Self {
            scope,
            root,
            status,
            items: RwSignal::new(Vec::new()),
            held: Cell::new(0),
            deferred: RefCell::new(Vec::new()),
            sync_generation: Cell::new(0),
        })
    }

    pub fn hold_saves(&self) {
        self.held.set(self.held.get() + 1);
    }

    /// When the last run ends, merges what Pi wrote and then runs held saves.
    /// This also covers a session closed mid-run, which never settles.
    pub fn release_saves(self: &Rc<Self>) {
        self.held.set(self.held.get().saturating_sub(1));
        if self.held.get() == 0 {
            self.sync_from_disk("Pi");
        }
    }

    fn flush_deferred(self: &Rc<Self>) {
        let paths = std::mem::take(&mut *self.deferred.borrow_mut());
        for path in paths {
            if let Some(doc) = self.get(&path)
                && doc.dirty()
            {
                self.save(doc);
            }
        }
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

    pub fn save(self: &Rc<Self>, doc: Rc<OpenDocument>) {
        if self.held.get() > 0 {
            let mut deferred = self.deferred.borrow_mut();
            if !deferred.contains(&doc.path) {
                deferred.push(doc.path.clone());
            }
            self.status.set("Will save when Pi finishes".into());
            return;
        }
        let Some(snapshot) = doc.start_save() else {
            return;
        };
        self.status.set("Saving…".into());
        let path = doc.path.clone();
        let root = self.root.clone();
        let status = self.status;
        let documents = self.clone();
        let finish = create_ext_action(self.scope, move |(snapshot, result)| {
            match doc.finish_save(snapshot, result) {
                Ok(()) if crate::settings::is_file(&doc.path) => {
                    let problem = crate::settings::current()
                        .1
                        .or_else(crate::ui::color::follow_settings);
                    status.set(match problem {
                        Some(err) => err,
                        None => "Saved settings; fonts and shortcuts apply to new windows".into(),
                    })
                }
                Ok(()) => status.set(format!(
                    "Saved {}",
                    doc.path.strip_prefix(&root).unwrap_or(&doc.path).display()
                )),
                Err(err) => status.set(err),
            }
            if let Some(source) = doc.resync.take() {
                documents.sync_from_disk(source);
            }
        });
        std::thread::spawn(move || {
            let result = files::save_text(&path, &snapshot.baseline, &snapshot.text)
                .map_err(|e| e.to_string());
            finish((snapshot, result));
        });
    }

    pub fn save_all(self: &Rc<Self>) {
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

    /// Re-reads open files and merges changes made outside the editor, so a
    /// file is never both edited here and stale on disk. `source` names who
    /// made the changes, for the status line and conflict markers.
    pub fn sync_from_disk(self: &Rc<Self>, source: &'static str) {
        let generation = self.sync_generation.get() + 1;
        self.sync_generation.set(generation);
        let docs = self.items.get_untracked();
        if docs.is_empty() {
            self.flush_deferred();
            return;
        }
        let paths: Vec<_> = docs.iter().map(|doc| doc.path.clone()).collect();
        let documents = self.clone();
        let finish = create_ext_action(self.scope, move |texts| {
            documents.finish_sync(generation, docs, texts, source)
        });
        std::thread::spawn(move || {
            finish(
                paths
                    .iter()
                    .map(|path| files::read_text(path).ok())
                    .collect(),
            )
        });
    }

    fn finish_sync(
        self: &Rc<Self>,
        generation: u64,
        docs: Vec<Rc<OpenDocument>>,
        texts: Vec<Option<String>>,
        source: &'static str,
    ) {
        if generation != self.sync_generation.get() {
            return;
        }
        let mut updated = Vec::new();
        let mut conflicts = 0;
        for (doc, disk) in docs.into_iter().zip(texts) {
            // Skip buffers closed or reopened while reading, and unreadable or deleted files.
            let current = self.get(&doc.path).is_some_and(|c| Rc::ptr_eq(&c, &doc));
            let Some(disk) = disk.filter(|_| current) else {
                continue;
            };
            let outcome = doc.sync_disk(disk, source);
            if let Synced::Merged { conflicts: n } = outcome {
                conflicts += n;
            }
            if matches!(outcome, Synced::Reloaded | Synced::Merged { .. }) {
                let name = doc.path.strip_prefix(&self.root).unwrap_or(&doc.path);
                updated.push(name.display().to_string());
            }
        }
        if !updated.is_empty() {
            let marked = match conflicts {
                0 => String::new(),
                n => format!(" · {n} conflict(s) marked in the editor"),
            };
            self.status
                .set(format!("{source} updated {}{marked}", updated.join(", ")));
        }
        if self.held.get() == 0 {
            self.flush_deferred();
        }
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

    fn with_text(text: &str, test: impl FnOnce(Rc<OpenDocument>)) {
        let scope = Scope::new();
        scope.enter(|| test(OpenDocument::new(scope, "test.rs".into(), text.into())));
        scope.dispose();
    }

    fn with_document(test: impl FnOnce(Rc<OpenDocument>)) {
        with_text("original", test);
    }

    #[test]
    fn disk_changes_reload_clean_buffers_without_asking() {
        with_document(|doc| {
            assert_eq!(doc.sync_disk("original".into(), "pi"), Synced::Unchanged);
            assert_eq!(doc.sync_disk("from pi".into(), "pi"), Synced::Reloaded);
            assert_eq!(doc.text.text().to_string(), "from pi");
            assert!(doc.can_close());
        });
    }

    #[test]
    fn disk_changes_merge_into_unsaved_edits_and_saving_still_works() {
        with_text("a\nb\nc\n", |doc| {
            edit(&doc, "A\nb\nc\n");
            assert_eq!(
                doc.sync_disk("a\nb\nC\n".into(), "pi"),
                Synced::Merged { conflicts: 0 }
            );
            assert_eq!(doc.text.text().to_string(), "A\nb\nC\n");
            assert!(doc.dirty());
            // The next save compares against what is now on disk.
            assert_eq!(doc.start_save().unwrap().baseline, "a\nb\nC\n");
            // Changes arriving mid-save wait for the save to settle.
            assert_eq!(doc.sync_disk("z\n".into(), "pi"), Synced::Deferred);
        });
    }

    #[test]
    fn outside_changes_stay_highlighted_until_the_next_edit() {
        with_text("a\nb\n", |doc| {
            doc.sync_disk("a\nB\nc\n".into(), "pi");
            let incoming = doc.incoming().get().unwrap();
            assert_eq!(incoming.added(), 2);
            assert_eq!(
                incoming.current[0],
                workbench_core::diff::LineChange::Unchanged
            );
            edit(&doc, "typed");
            assert!(doc.incoming().get().is_none());
        });
    }

    #[test]
    fn crlf_buffers_take_lf_writes_without_false_conflicts() {
        with_text("a\r\nb\r\n", |doc| {
            assert_eq!(doc.sync_disk("a\nB\n".into(), "pi"), Synced::Reloaded);
            assert!(doc.can_close());
            // A second edit must not see every line as changed by the editor.
            assert_eq!(doc.sync_disk("A\nB\n".into(), "pi"), Synced::Reloaded);
            assert_eq!(doc.text.text().to_string(), "A\r\nB\r\n");
            assert!(doc.can_close());
        });
    }

    #[test]
    fn saves_wait_while_held_and_stale_syncs_are_dropped() {
        let scope = Scope::new();
        scope.enter(|| {
            let docs = Documents::new(scope, "/".into(), RwSignal::new(String::new()));
            docs.insert("/a.rs".into(), "one\n".into());
            let doc = docs.get(Path::new("/a.rs")).unwrap();
            edit(&doc, "mine\n");
            docs.hold_saves();
            docs.save(doc.clone());
            assert!(!doc.saving());
            assert_eq!(*docs.deferred.borrow(), [PathBuf::from("/a.rs")]);
            // An older read must not apply once a newer sync has started.
            docs.sync_generation.set(2);
            docs.finish_sync(1, vec![doc.clone()], vec![Some("stale\n".into())], "pi");
            assert_eq!(doc.text.text().to_string(), "mine\n");
        });
        scope.dispose();
    }

    #[test]
    fn overlapping_disk_changes_are_marked_not_prompted() {
        with_text("a\nb\n", |doc| {
            edit(&doc, "mine\nb\n");
            assert_eq!(
                doc.sync_disk("theirs\nb\n".into(), "pi"),
                Synced::Merged { conflicts: 1 }
            );
            let text = doc.text.text().to_string();
            assert!(text.contains("<<<<<<< editor") && text.contains(">>>>>>> pi"));
            assert!(text.ends_with("b\n"));
        });
    }

    #[test]
    fn only_the_changed_span_is_replaced() {
        with_text("keep λ old tail", |doc| {
            let before = doc.text.text().to_string();
            replace_changed_region(&doc.text, &before, "keep λ new tail");
            assert_eq!(doc.text.text().to_string(), "keep λ new tail");
            replace_changed_region(&doc.text, "keep λ new tail", "keep λ new tail!");
            assert_eq!(doc.text.text().to_string(), "keep λ new tail!");
        });
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
