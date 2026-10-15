//! The changes lens: what the workspace is compared against, which files
//! differ, and each open file's diff. Every view reads from here.
use crate::{
    origins::{self, Authorship, Context, Origin, PiLines, Records},
    session::Turn,
};
use floem::{
    action::exec_after,
    ext_event::create_ext_action,
    prelude::*,
    reactive::{Effect, ReadSignal, Scope},
    views::editor::{text::Document, text_document::TextDocument},
};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use workbench_core::{
    diff::{Hunk, LineDiff},
    git::{Base, FileChange, Repo, Rev},
    structure::{self, Changed},
};

/// Coalesces bursts of typing into one background diff.
const REFRESH_DELAY: Duration = Duration::from_millis(150);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Since {
    /// Where this branch left develop (or main, or master).
    Branch,
    /// When the active Pi session sent its first prompt.
    SessionStart,
    /// When the active Pi session sent its latest prompt.
    LastTurn,
    /// The last commit.
    Head,
}

impl Since {
    pub const ALL: [Since; 4] = [
        Since::Branch,
        Since::SessionStart,
        Since::LastTurn,
        Since::Head,
    ];
}

/// A diff together with the baseline and the text revision it came from, so
/// actions on a hunk never mix one version's line numbers with another's text.
pub struct Comparison {
    pub diff: LineDiff,
    base: Arc<Option<String>>,
    revision: u64,
    /// Every hunk with the key that marks it seen.
    pub hunks: Vec<(Hunk, u64)>,
    /// Per hunk: whether it only reformats.
    pub formatting: Vec<bool>,
    /// Per line: who added it, for added lines.
    pub origins: Vec<Option<Origin>>,
}

impl Comparison {
    fn new(
        path: &Path,
        base: Arc<Option<String>>,
        text: &str,
        revision: u64,
        context: &Context,
        structural: impl FnOnce() -> Option<Changed>,
    ) -> Self {
        let diff = match base.as_ref() {
            Some(base) => LineDiff::compute(base, text),
            None => LineDiff::all_added(text),
        };
        let base_lines = lines(base.as_deref().unwrap_or_default());
        let current_lines = lines(text);
        // Identical hunks in one file are told apart by how many came before.
        let mut repeats: HashMap<u64, u64> = HashMap::new();
        let hunks = diff
            .hunks_iter()
            .map(|hunk| {
                let removed = base_lines.get(hunk.base.clone()).unwrap_or_default();
                let added = current_lines.get(hunk.current.clone()).unwrap_or_default();
                let mut hasher = DefaultHasher::new();
                path.hash(&mut hasher);
                removed.hash(&mut hasher);
                added.hash(&mut hasher);
                let content = hasher.finish();
                let count = repeats.entry(content).or_default();
                *count += 1;
                let mut hasher = DefaultHasher::new();
                (content, *count).hash(&mut hasher);
                (hunk, hasher.finish())
            })
            .collect::<Vec<(Hunk, u64)>>();
        let plain: Vec<Hunk> = hunks.iter().map(|(hunk, _)| hunk.clone()).collect();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let formatting = origins::formatting(
            &name,
            base.as_deref(),
            &plain,
            &base_lines,
            &current_lines,
            structural,
        );
        let origins = origins::origins(&diff, &plain, &current_lines, text, context);
        Self {
            diff,
            base,
            revision,
            hunks,
            formatting,
            origins,
        }
    }

    pub fn base_text(&self) -> &str {
        self.base.as_deref().unwrap_or_default()
    }

    /// The baseline lines a hunk replaced, without line endings.
    pub fn removed(&self, hunk: &Hunk) -> Vec<&str> {
        lines(self.base_text())
            .get(hunk.base.clone())
            .unwrap_or_default()
            .to_vec()
    }

    /// The editor line a hunk sits on; a removal after the last line sits on it.
    pub fn line_of(&self, hunk: &Hunk) -> usize {
        hunk.current
            .start
            .min(self.diff.current.len().saturating_sub(1))
    }

    /// The hunk covering `line`, by index.
    pub fn at_line(&self, line: usize) -> Option<usize> {
        self.hunks.iter().position(|(hunk, _)| {
            hunk.current.contains(&line) || (hunk.current.is_empty() && self.line_of(hunk) == line)
        })
    }
}

fn lines(text: &str) -> Vec<&str> {
    text.split('\n').map(|l| l.trim_end_matches('\r')).collect()
}

/// A difftastic result and a hash of the texts it compared.
type Structural = (u64, Option<Changed>);
/// Blame by HEAD and file.
type AuthorshipCache = Mutex<HashMap<(String, PathBuf), Option<Arc<Authorship>>>>;
/// Each finished turn's files, by its before and after snapshots.
type TurnCache = Mutex<HashMap<(String, String), Vec<FileChange>>>;

/// One open file under the lens.
pub struct FileLens {
    pub path: PathBuf,
    text: Rc<TextDocument>,
    revision: ReadSignal<u64>,
    /// Baseline text; `None` inside means the file is new there.
    base: RefCell<Option<Arc<Option<String>>>>,
    /// Who wrote the committed lines, when someone else did.
    authorship: RefCell<Option<Arc<Authorship>>>,
    comparison: RwSignal<Option<Rc<Comparison>>>,
    /// Bumped per diff; shared so a background pass can tell it is stale.
    generation: Arc<AtomicU64>,
    /// The last difftastic result, by the texts it compared; also serialises runs.
    structural: Arc<Mutex<Option<Structural>>>,
    /// Owns the effect that follows edits, so closing the file frees it.
    scope: Scope,
}

impl FileLens {
    /// Tracked: the latest comparison, possibly a few edits old.
    pub fn comparison(&self) -> ReadSignal<Option<Rc<Comparison>>> {
        self.comparison.read_only()
    }

    /// The comparison, only if it matches the text as it is now.
    pub fn current(&self) -> Option<Rc<Comparison>> {
        self.comparison
            .get_untracked()
            .filter(|c| c.revision == self.revision.get_untracked())
    }
}

/// The files one Pi turn changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnFiles {
    pub number: usize,
    pub prompt: String,
    pub files: Vec<FileChange>,
}

/// One open file's baseline and authorship, as read in the background.
type Loaded = (anyhow::Result<Option<String>>, Option<Arc<Authorship>>);

/// Blame for a file at HEAD, reused until HEAD moves.
fn authorship(
    cache: &AuthorshipCache,
    repo: &Repo,
    head: Option<&str>,
    relative: &Path,
) -> Option<Arc<Authorship>> {
    let key = (head?.to_owned(), relative.to_path_buf());
    if let Some(found) = cache.lock().ok()?.get(&key) {
        return found.clone();
    }
    let me = repo.user_email().ok().flatten();
    let found = Authorship::read(repo, relative, me.as_deref());
    cache.lock().ok()?.insert(key, found.clone());
    found
}

/// What one background refresh found.
struct Refreshed {
    head: Option<String>,
    branch_base: Option<Base>,
    changes: Option<anyhow::Result<Vec<FileChange>>>,
    bases: Vec<(PathBuf, Loaded)>,
    turn_files: Vec<TurnFiles>,
    pi: PiLines,
}

pub struct Lens {
    scope: Scope,
    root: PathBuf,
    status: RwSignal<String>,
    repo: RefCell<Option<Repo>>,
    /// The repository root in the same `canonicalize` form as workspace paths.
    repo_root: RefCell<Option<PathBuf>>,
    since: RwSignal<Since>,
    branch_base: RwSignal<Option<Base>>,
    head: RwSignal<Option<String>>,
    /// The active session's turns, oldest first.
    turns: RwSignal<Vec<Turn>>,
    turn_files: RwSignal<Vec<TurnFiles>>,
    /// What Pi's sessions wrote, by file.
    pi: RwSignal<PiLines>,
    records: Arc<Records>,
    /// Blame by HEAD and file, and each finished turn's files: neither
    /// changes until HEAD or the snapshots do.
    authorship: Arc<AuthorshipCache>,
    turn_cache: Arc<TurnCache>,
    show_formatting: RwSignal<bool>,
    changes: RwSignal<Vec<FileChange>>,
    files: RefCell<HashMap<PathBuf, Rc<FileLens>>>,
    focus: RwSignal<bool>,
    changed_only: RwSignal<bool>,
    seen: RwSignal<HashSet<u64>>,
    changes_generation: Cell<u64>,
    /// One refresh runs at a time; another request runs once it ends.
    refreshing: Cell<bool>,
    again: Cell<bool>,
}

impl Lens {
    pub fn new(scope: Scope, root: PathBuf, status: RwSignal<String>) -> Rc<Self> {
        Rc::new(Self {
            scope,
            root,
            status,
            repo: RefCell::new(None),
            repo_root: RefCell::new(None),
            since: RwSignal::new(Since::Branch),
            branch_base: RwSignal::new(None),
            head: RwSignal::new(None),
            turns: RwSignal::new(Vec::new()),
            turn_files: RwSignal::new(Vec::new()),
            pi: RwSignal::new(PiLines::default()),
            records: Arc::default(),
            authorship: Arc::default(),
            turn_cache: Arc::default(),
            show_formatting: RwSignal::new(false),
            changes: RwSignal::new(Vec::new()),
            files: RefCell::new(HashMap::new()),
            focus: RwSignal::new(false),
            changed_only: RwSignal::new(false),
            seen: RwSignal::new(HashSet::new()),
            changes_generation: Cell::new(0),
            refreshing: Cell::new(false),
            again: Cell::new(false),
        })
    }

    pub fn since(&self) -> ReadSignal<Since> {
        self.since.read_only()
    }
    pub fn changes(&self) -> ReadSignal<Vec<FileChange>> {
        self.changes.read_only()
    }
    pub fn focus(&self) -> ReadSignal<bool> {
        self.focus.read_only()
    }
    pub fn changed_only(&self) -> ReadSignal<bool> {
        self.changed_only.read_only()
    }
    pub fn seen(&self) -> ReadSignal<HashSet<u64>> {
        self.seen.read_only()
    }
    pub fn toggle_focus(&self) {
        self.focus.update(|f| *f = !*f);
    }
    pub fn toggle_changed_only(&self) {
        self.changed_only.update(|f| *f = !*f);
    }
    pub fn turn_files(&self) -> ReadSignal<Vec<TurnFiles>> {
        self.turn_files.read_only()
    }
    pub fn show_formatting(&self) -> ReadSignal<bool> {
        self.show_formatting.read_only()
    }
    pub fn toggle_formatting(&self) {
        self.show_formatting.update(|f| *f = !*f);
    }

    /// Tracked: whether a hunk is out of view, because it was marked seen or
    /// only reformats while formatting is hidden.
    pub fn hidden(&self, comparison: &Comparison, index: usize) -> bool {
        let key = comparison.hunks[index].1;
        self.seen.with(|seen| seen.contains(&key))
            || (!self.show_formatting.get() && comparison.formatting[index])
    }

    /// Tracked: the label the status bar shows for `since`.
    pub fn label(&self, since: Since) -> String {
        match since {
            Since::Branch => self
                .branch_base
                .with(|b| b.as_ref().map(|b| b.branch.clone()))
                .unwrap_or_else(|| "base branch".into()),
            Since::SessionStart => "session start".into(),
            Since::LastTurn => "Pi's last turn".into(),
            Since::Head => "last commit".into(),
        }
    }

    /// Tracked: the revision `since` currently means, if it exists yet.
    pub fn rev(&self) -> Option<Rev> {
        match self.since.get() {
            Since::Branch => self
                .branch_base
                .with(|b| b.as_ref().map(|b| Rev::Commit(b.commit.clone()))),
            Since::Head => self.head.get().map(Rev::Commit),
            Since::SessionStart => self
                .turns
                .with(|t| t.first().map(|t| Rev::Tree(t.before.clone()))),
            Since::LastTurn => self
                .turns
                .with(|t| t.last().map(|t| Rev::Tree(t.before.clone()))),
        }
    }

    pub fn set_since(self: &Rc<Self>, since: Since) {
        self.since.set(since);
        self.refresh();
    }

    /// Follows the active session; a finished turn also regroups the changes.
    pub fn set_turns(self: &Rc<Self>, turns: Vec<Turn>) {
        if self.turns.with_untracked(|t| *t != turns) {
            self.turns.set(turns);
            self.refresh();
        }
    }

    fn workspace_path(&self, path: &Path) -> PathBuf {
        let repo = self.repo.borrow();
        let root = self.repo_root.borrow();
        match (repo.as_ref(), root.as_ref()) {
            (Some(repo), Some(root)) => path
                .strip_prefix(repo.root())
                .map(|relative| root.join(relative))
                .unwrap_or_else(|_| path.to_path_buf()),
            _ => path.to_path_buf(),
        }
    }

    fn relative(&self, path: &Path) -> Option<PathBuf> {
        let root = self.repo_root.borrow();
        path.strip_prefix(root.as_ref()?)
            .ok()
            .map(Path::to_path_buf)
    }

    /// Finds the repository, the base branch and HEAD off the UI thread.
    pub fn start(self: &Rc<Self>) {
        let lens = self.clone();
        let root = self.root.clone();
        let finish = create_ext_action(
            self.scope,
            move |found: Option<(Repo, PathBuf, Option<Base>, Option<String>)>| {
                if let Some((repo, repo_root, base, head)) = found {
                    *lens.repo.borrow_mut() = Some(repo);
                    *lens.repo_root.borrow_mut() = Some(repo_root);
                    lens.branch_base.set(base);
                    lens.head.set(head);
                    lens.refresh();
                }
            },
        );
        std::thread::spawn(move || {
            let found = Repo::discover(&root).ok().flatten().map(|repo| {
                let repo_root = repo
                    .root()
                    .canonicalize()
                    .unwrap_or_else(|_| repo.root().to_path_buf());
                let base = repo.default_base().ok().flatten();
                let head = repo.head().ok().flatten();
                (repo, repo_root, base, head)
            });
            finish(found)
        });
    }

    /// Takes a snapshot of the workspace for a session turn; `done` gets the tree id.
    pub fn snapshot(&self, done: impl FnOnce(Option<String>) + 'static) {
        let Some(repo) = self.repo.borrow().clone() else {
            done(None);
            return;
        };
        let finish = create_ext_action(self.scope, done);
        std::thread::spawn(move || finish(repo.snapshot().ok()));
    }

    /// Re-reads HEAD and the base branch, then recomputes the changed files
    /// and every open file's baseline, off the UI thread.
    pub fn refresh(self: &Rc<Self>) {
        let Some(repo) = self.repo.borrow().clone() else {
            return;
        };
        if self.refreshing.replace(true) {
            self.again.set(true);
            return;
        }
        let generation = self.changes_generation.get() + 1;
        self.changes_generation.set(generation);
        let since = self.since.get_untracked();
        let turns = self.turns.get_untracked();
        let files: Vec<(PathBuf, Option<PathBuf>)> = self
            .files
            .borrow()
            .values()
            .map(|file| (file.path.clone(), self.relative(&file.path)))
            .collect();
        let (records, root) = (self.records.clone(), self.root.clone());
        let (authors, turn_cache) = (self.authorship.clone(), self.turn_cache.clone());
        let lens = self.clone();
        let finish = create_ext_action(self.scope, move |found: Refreshed| {
            lens.refreshing.set(false);
            lens.apply(generation, found);
            if lens.again.replace(false) {
                lens.refresh();
            }
        });
        std::thread::spawn(move || {
            let head = repo.head().ok().flatten();
            let branch_base = repo.default_base().ok().flatten();
            let rev = match since {
                Since::Branch => branch_base.as_ref().map(|b| Rev::Commit(b.commit.clone())),
                Since::Head => head.clone().map(Rev::Commit),
                Since::SessionStart => turns.first().map(|t| Rev::Tree(t.before.clone())),
                Since::LastTurn => turns.last().map(|t| Rev::Tree(t.before.clone())),
            };
            let changes = rev.as_ref().map(|rev| repo.changes(rev));
            let bases = match &rev {
                Some(rev) => files
                    .into_iter()
                    .filter_map(|(path, relative)| {
                        let relative = relative?;
                        let base = repo.read(rev, &relative);
                        let authorship = authorship(&authors, &repo, head.as_deref(), &relative);
                        Some((path, (base, authorship)))
                    })
                    .collect(),
                None => Vec::new(),
            };
            let turn_files = turns
                .iter()
                .enumerate()
                .filter_map(|(index, turn)| {
                    let key = (turn.before.clone(), turn.after.clone()?);
                    let cached = turn_cache.lock().ok()?.get(&key).cloned();
                    let files = match cached {
                        Some(files) => files,
                        None => {
                            let (before, after) =
                                (Rev::Tree(key.0.clone()), Rev::Tree(key.1.clone()));
                            let files = repo.changes_between(&before, &after).ok()?;
                            turn_cache.lock().ok()?.insert(key, files.clone());
                            files
                        }
                    };
                    Some(TurnFiles {
                        number: index + 1,
                        prompt: turn.prompt.clone(),
                        files,
                    })
                })
                .collect();
            finish(Refreshed {
                head,
                branch_base,
                changes,
                bases,
                turn_files,
                pi: records.read(&root),
            })
        });
    }

    fn apply(self: &Rc<Self>, generation: u64, found: Refreshed) {
        if self.head.with_untracked(|h| *h != found.head) {
            self.head.set(found.head);
            self.forget_old_blame();
        }
        if self.branch_base.with_untracked(|b| *b != found.branch_base) {
            self.branch_base.set(found.branch_base);
        }
        if self.changes_generation.get() != generation {
            return;
        }
        if self.pi.with_untracked(|pi| !Arc::ptr_eq(pi, &found.pi)) {
            self.pi.set(found.pi);
        }
        let turn_files: Vec<TurnFiles> = found
            .turn_files
            .into_iter()
            .map(|mut turn| {
                turn.files = self.in_workspace(turn.files);
                turn
            })
            .filter(|turn| !turn.files.is_empty())
            .collect();
        if self.turn_files.with_untracked(|t| *t != turn_files) {
            self.turn_files.set(turn_files);
        }
        let Some(changes) = found.changes else {
            // Nothing to compare with yet, such as a session that has not run.
            self.changes.set(Vec::new());
            for file in self.files.borrow().values() {
                *file.base.borrow_mut() = None;
                file.generation.fetch_add(1, Ordering::Relaxed);
                file.comparison.set(None);
            }
            return;
        };
        match changes {
            Ok(changes) => {
                let changes = self.in_workspace(changes);
                if self.changes.with_untracked(|c| *c != changes) {
                    self.changes.set(changes);
                }
            }
            Err(err) => self.status.set(format!("Changes: {err:#}")),
        }
        for (path, (base, authorship)) in found.bases {
            if let Some(file) = self.file(&path) {
                *file.authorship.borrow_mut() = authorship;
                self.set_base(&file, base);
            }
        }
    }

    /// Workspace paths use `canonicalize` form; git reports plain ones. The
    /// repository may also hold more than this workspace shows.
    fn in_workspace(&self, changes: Vec<FileChange>) -> Vec<FileChange> {
        changes
            .into_iter()
            .map(|mut change| {
                change.path = self.workspace_path(&change.path);
                change
            })
            .filter(|change| change.path.starts_with(&self.root))
            .collect()
    }

    fn set_base(self: &Rc<Self>, file: &Rc<FileLens>, base: anyhow::Result<Option<String>>) {
        match base {
            Ok(base) => {
                *file.base.borrow_mut() = Some(Arc::new(base));
                self.rediff(file);
            }
            // Unreadable is not the same as new: show no marks rather than
            // offer to revert the whole file.
            Err(err) => {
                *file.base.borrow_mut() = None;
                file.generation.fetch_add(1, Ordering::Relaxed);
                file.comparison.set(None);
                self.status.set(format!("Changes: {err:#}"));
            }
        }
    }

    fn rev_untracked(&self) -> Option<Rev> {
        Effect::untrack(|| self.rev())
    }

    fn load_base(self: &Rc<Self>, file: &Rc<FileLens>, repo: &Repo, rev: &Rev) {
        let Some(relative) = self.relative(&file.path) else {
            return;
        };
        let generation = self.changes_generation.get();
        let lens = self.clone();
        let target = file.clone();
        let finish = create_ext_action(self.scope, move |(base, authorship): Loaded| {
            if lens.changes_generation.get() == generation {
                *target.authorship.borrow_mut() = authorship;
                lens.set_base(&target, base);
            }
        });
        let (repo, rev) = (repo.clone(), rev.clone());
        let (authors, head) = (self.authorship.clone(), self.head.get_untracked());
        std::thread::spawn(move || {
            let authorship = authorship(&authors, &repo, head.as_deref(), &relative);
            finish((repo.read(&rev, &relative), authorship))
        });
    }

    /// Starts tracking an open file; its diff follows edits.
    pub fn track(
        self: &Rc<Self>,
        path: &Path,
        text: Rc<TextDocument>,
        revision: ReadSignal<u64>,
    ) -> Rc<FileLens> {
        if let Some(file) = self.files.borrow().get(path) {
            return file.clone();
        }
        let scope = self.scope.create_child();
        let file = self.scope.enter(|| {
            Rc::new(FileLens {
                path: path.to_path_buf(),
                text: text.clone(),
                revision,
                base: RefCell::new(None),
                authorship: RefCell::new(None),
                comparison: RwSignal::new(None),
                generation: Arc::default(),
                structural: Arc::default(),
                scope,
            })
        });
        self.files
            .borrow_mut()
            .insert(path.to_path_buf(), file.clone());
        // Weak, so the effect does not keep a closed file alive.
        let (lens, target) = (Rc::downgrade(self), Rc::downgrade(&file));
        let pending = Rc::new(Cell::new(0u64));
        scope.enter(|| {
            Effect::new(move |_| {
                revision.track();
                let current = pending.get() + 1;
                pending.set(current);
                let (lens, target, pending) = (lens.clone(), target.clone(), pending.clone());
                exec_after(REFRESH_DELAY, move |_| {
                    if pending.get() == current
                        && let (Some(lens), Some(target)) = (lens.upgrade(), target.upgrade())
                    {
                        lens.rediff(&target);
                    }
                });
            })
        });
        if let (Some(repo), Some(rev)) = (self.repo.borrow().clone(), self.rev_untracked()) {
            self.load_base(&file, &repo, &rev);
        }
        file
    }

    pub fn forget(&self, path: &Path) {
        if let Some(file) = self.files.borrow_mut().remove(path) {
            file.scope.dispose();
        }
    }

    pub fn file(&self, path: &Path) -> Option<Rc<FileLens>> {
        self.files.borrow().get(path).cloned()
    }

    fn rediff(self: &Rc<Self>, file: &Rc<FileLens>) {
        let Some(base) = file.base.borrow().clone() else {
            return;
        };
        let text = file.text.text().to_string();
        let revision = file.revision.get_untracked();
        let generation = file.generation.fetch_add(1, Ordering::Relaxed) + 1;
        let target = file.clone();
        let finish = create_ext_action(self.scope, move |comparison: Comparison| {
            if target.generation.load(Ordering::Relaxed) == generation {
                target.comparison.set(Some(Rc::new(comparison)));
            }
        });
        let path = file.path.clone();
        let context = Context {
            pi: self.pi.with_untracked(|pi| pi.get(&path).cloned()),
            authorship: file.authorship.borrow().clone(),
        };
        let (cache, latest) = (file.structural.clone(), file.generation.clone());
        std::thread::spawn(move || {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let structural = || {
                let old = base.as_deref()?;
                let mut hasher = DefaultHasher::new();
                (old, &text).hash(&mut hasher);
                let key = hasher.finish();
                // One difftastic run per file at a time, skipped once stale.
                let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
                if let Some((cached, changed)) = cache.as_ref()
                    && *cached == key
                {
                    return changed.clone();
                }
                if latest.load(Ordering::Relaxed) != generation {
                    return None;
                }
                let changed = structure::compare(&name, old, &text);
                *cache = Some((key, changed.clone()));
                changed
            };
            let comparison =
                Comparison::new(&path, base.clone(), &text, revision, &context, structural);
            finish(comparison)
        });
    }

    /// Clears blame from other commits once HEAD moves.
    fn forget_old_blame(&self) {
        let head = self.head.get_untracked().unwrap_or_default();
        if let Ok(mut cache) = self.authorship.lock() {
            cache.retain(|(at, _), _| *at == head);
        }
    }

    pub fn mark_seen(&self, key: u64) {
        self.seen.update(|seen| {
            seen.insert(key);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compare(base: &str, text: &str) -> Comparison {
        Comparison::new(
            Path::new("a.rs"),
            Arc::new(Some(base.into())),
            text,
            0,
            &Context::default(),
            || None,
        )
    }

    #[test]
    fn identical_changes_in_one_file_have_their_own_keys() {
        let c = compare("a\nx\nb\nx\nc\n", "a\ny\nb\ny\nc\n");
        assert_eq!(c.hunks.len(), 2);
        assert_ne!(c.hunks[0].1, c.hunks[1].1);
        assert_eq!(c.removed(&c.hunks[1].0), ["x"]);
        // The same edit computed again keeps its keys.
        let again = compare("a\nx\nb\nx\nc\n", "a\ny\nb\ny\nc\n");
        assert_eq!(c.hunks, again.hunks);
    }

    #[test]
    fn a_removal_after_the_last_line_sits_on_the_last_line() {
        let c = compare("a\nb\nc", "a\nb");
        let (hunk, _) = &c.hunks[0];
        let last = c.diff.current.len() - 1;
        assert_eq!(c.line_of(hunk), last);
        assert_eq!(c.at_line(last), Some(0));
    }
}
