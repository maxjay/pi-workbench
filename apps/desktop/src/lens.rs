//! The changes lens: what the workspace is compared against, which files
//! differ, and each open file's diff. Every view reads from here.
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
    sync::Arc,
    time::Duration,
};
use workbench_core::{
    diff::{Hunk, LineDiff},
    git::{Base, FileChange, Repo, Rev},
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

/// One open file under the lens.
pub struct FileLens {
    pub path: PathBuf,
    text: Rc<TextDocument>,
    /// Baseline text for the current revision; `None` inside means the file is new.
    base: RefCell<Option<Arc<Option<String>>>>,
    diff: RwSignal<Option<Rc<LineDiff>>>,
    generation: Cell<u64>,
}

impl FileLens {
    pub fn diff(&self) -> ReadSignal<Option<Rc<LineDiff>>> {
        self.diff.read_only()
    }
    pub fn base_text(&self) -> Option<Arc<Option<String>>> {
        self.base.borrow().clone()
    }
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
    /// Snapshots the active session took before each prompt, oldest first.
    turns: RwSignal<Vec<String>>,
    changes: RwSignal<Vec<FileChange>>,
    files: RefCell<HashMap<PathBuf, Rc<FileLens>>>,
    focus: RwSignal<bool>,
    changed_only: RwSignal<bool>,
    seen: RwSignal<HashSet<u64>>,
    changes_generation: Cell<u64>,
}

fn hunk_key(path: &Path, hunk_text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hunk_text.hash(&mut hasher);
    hasher.finish()
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
            changes: RwSignal::new(Vec::new()),
            files: RefCell::new(HashMap::new()),
            focus: RwSignal::new(false),
            changed_only: RwSignal::new(false),
            seen: RwSignal::new(HashSet::new()),
            changes_generation: Cell::new(0),
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
            Since::SessionStart => self.turns.with(|t| t.first().cloned()).map(Rev::Tree),
            Since::LastTurn => self.turns.with(|t| t.last().cloned()).map(Rev::Tree),
        }
    }

    pub fn set_since(self: &Rc<Self>, since: Since) {
        self.since.set(since);
        self.refresh();
    }

    pub fn set_turns(self: &Rc<Self>, turns: Vec<String>) {
        if self.turns.with_untracked(|t| *t != turns) {
            self.turns.set(turns);
            if matches!(
                self.since.get_untracked(),
                Since::SessionStart | Since::LastTurn
            ) {
                self.refresh();
            }
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

    /// Recomputes the changed files and every open file's baseline.
    pub fn refresh(self: &Rc<Self>) {
        let Some(repo) = self.repo.borrow().clone() else {
            return;
        };
        let rev = self.rev_untracked();
        let generation = self.changes_generation.get() + 1;
        self.changes_generation.set(generation);
        for file in self.files.borrow().values() {
            *file.base.borrow_mut() = None;
        }
        let Some(rev) = rev else {
            self.changes.set(Vec::new());
            for file in self.files.borrow().values() {
                file.diff.set(None);
            }
            return;
        };
        let lens = self.clone();
        let finish = create_ext_action(
            self.scope,
            move |result: anyhow::Result<Vec<FileChange>>| {
                if lens.changes_generation.get() != generation {
                    return;
                }
                match result {
                    Ok(changes) => {
                        // Workspace paths use `canonicalize` form; git reports plain ones.
                        let changes = changes
                            .into_iter()
                            .map(|mut change| {
                                change.path = lens.workspace_path(&change.path);
                                change
                            })
                            .collect();
                        lens.changes.set(changes)
                    }
                    Err(err) => lens.status.set(format!("Changes: {err:#}")),
                }
            },
        );
        let changes_rev = rev.clone();
        let changes_repo = repo.clone();
        std::thread::spawn(move || finish(changes_repo.changes(&changes_rev)));
        let files: Vec<_> = self.files.borrow().values().cloned().collect();
        for file in files {
            self.load_base(&file, &repo, &rev);
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
        let finish = create_ext_action(self.scope, move |base: Option<String>| {
            if lens.changes_generation.get() == generation {
                *target.base.borrow_mut() = Some(Arc::new(base));
                lens.rediff(&target);
            }
        });
        let (repo, rev) = (repo.clone(), rev.clone());
        std::thread::spawn(move || finish(repo.read(&rev, &relative).ok().flatten()));
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
        let file = self.scope.enter(|| {
            Rc::new(FileLens {
                path: path.to_path_buf(),
                text: text.clone(),
                base: RefCell::new(None),
                diff: RwSignal::new(None),
                generation: Cell::new(0),
            })
        });
        self.files
            .borrow_mut()
            .insert(path.to_path_buf(), file.clone());
        let (lens, target) = (self.clone(), file.clone());
        let pending = Rc::new(Cell::new(0u64));
        self.scope.enter(|| {
            Effect::new(move |_| {
                revision.track();
                let current = pending.get() + 1;
                pending.set(current);
                let (lens, target, pending, text) =
                    (lens.clone(), target.clone(), pending.clone(), text.clone());
                exec_after(REFRESH_DELAY, move |_| {
                    if pending.get() == current {
                        lens.rediff_with(&target, text.text().to_string());
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
        self.files.borrow_mut().remove(path);
    }

    pub fn file(&self, path: &Path) -> Option<Rc<FileLens>> {
        self.files.borrow().get(path).cloned()
    }

    fn rediff(self: &Rc<Self>, file: &Rc<FileLens>) {
        self.rediff_with(file, file.text.text().to_string());
    }

    fn rediff_with(self: &Rc<Self>, file: &Rc<FileLens>, text: String) {
        let Some(base) = file.base_text() else { return };
        let generation = file.generation.get() + 1;
        file.generation.set(generation);
        let target = file.clone();
        let finish = create_ext_action(self.scope, move |diff: LineDiff| {
            if target.generation.get() == generation {
                target.diff.set(Some(Rc::new(diff)));
            }
        });
        std::thread::spawn(move || {
            finish(match base.as_ref() {
                Some(base) => LineDiff::compute(base, &text),
                None => LineDiff::all_added(&text),
            })
        });
    }

    /// Identifies a hunk by its content, so "seen" survives unrelated edits.
    pub fn hunk_key(&self, file: &FileLens, hunk: &Hunk, current: &str) -> u64 {
        let lines: Vec<&str> = current.split('\n').collect();
        let text = lines
            .get(hunk.current.clone())
            .unwrap_or_default()
            .join("\n");
        let base = file
            .base_text()
            .and_then(|b| (*b).clone())
            .unwrap_or_default();
        let base_lines: Vec<&str> = base.split('\n').collect();
        let removed = base_lines
            .get(hunk.base.clone())
            .unwrap_or_default()
            .join("\n");
        hunk_key(&file.path, &format!("{removed}\u{0}{text}"))
    }

    pub fn mark_seen(&self, key: u64) {
        self.seen.update(|seen| {
            seen.insert(key);
        });
    }
}
