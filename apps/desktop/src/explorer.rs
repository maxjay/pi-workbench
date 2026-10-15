//! Lazy directory loading and the flattened rows consumed by the explorer view.
use floem::reactive::Scope;
use floem::{ext_event::create_ext_action, prelude::*};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
};
use workbench_core::files::{self, Entry};

#[derive(Clone)]
pub struct TreeRow {
    pub entry: Entry,
    pub depth: usize,
}

pub struct Explorer {
    scope: Scope,
    root: PathBuf,
    directories: RwSignal<BTreeMap<PathBuf, Vec<Entry>>>,
    expanded: RwSignal<HashSet<PathBuf>>,
    loading: RefCell<HashSet<PathBuf>>,
    status: RwSignal<String>,
}

impl Explorer {
    pub fn new(scope: Scope, root: PathBuf, status: RwSignal<String>) -> Rc<Self> {
        Rc::new(Self {
            scope,
            root,
            directories: RwSignal::new(BTreeMap::new()),
            expanded: RwSignal::new(HashSet::new()),
            loading: RefCell::new(HashSet::new()),
            status,
        })
    }

    pub fn is_expanded(&self, path: &Path) -> bool {
        self.expanded.with(|expanded| expanded.contains(path))
    }

    pub fn refresh(self: &Rc<Self>) {
        self.read_directory(self.root.clone());
        for path in self.expanded.get_untracked() {
            self.read_directory(path);
        }
    }

    fn read_directory(self: &Rc<Self>, path: PathBuf) {
        // Only one read per directory can be in flight, so older results cannot
        // overwrite a newer read. Repeated refreshes share that pending read.
        if !self.loading.borrow_mut().insert(path.clone()) {
            return;
        }
        let root = self.root.clone();
        let explorer = self.clone();
        let result_path = path.clone();
        let finish = create_ext_action(self.scope, move |result: Result<Vec<Entry>, String>| {
            explorer.loading.borrow_mut().remove(&result_path);
            match result {
                Ok(entries) => explorer.directories.update(|cache| {
                    cache.insert(result_path, entries);
                }),
                Err(err) => explorer.status.set(err),
            }
        });
        std::thread::spawn(move || {
            // Hide what .gitignore and friends hide, as git and the lens do.
            let visible = workbench_text::index::visible_children(&path);
            finish(
                files::list_directory(&root, &path)
                    .map(|entries| {
                        entries
                            .into_iter()
                            .filter(|e| visible.contains(&e.path))
                            .collect()
                    })
                    .map_err(|e| e.to_string()),
            )
        });
    }

    pub fn toggle(self: &Rc<Self>, path: PathBuf) {
        let opening = !self
            .expanded
            .with_untracked(|expanded| expanded.contains(&path));
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
        fn visit(
            path: &Path,
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
        self.directories.with(|cache| {
            self.expanded.with(|expanded| {
                let mut rows = Vec::new();
                visit(&self.root, 0, cache, expanded, &mut rows);
                rows
            })
        })
    }
}
