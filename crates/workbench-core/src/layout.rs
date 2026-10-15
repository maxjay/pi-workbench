use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub version: u32,
    pub explorer_visible: bool,
    pub explorer_right: bool,
    pub explorer_width: f64,
    pub split: bool,
    pub split_fraction: f64,
    pub open_files: Vec<PathBuf>,
    pub active: [Option<PathBuf>; 2],
    /// The panes and their tabs. Older layouts have only the fields above,
    /// and are turned into panes when loaded.
    pub panes: Option<SavedNode>,
    /// The focused pane, by its position among the panes in reading order.
    pub focused: usize,
}

/// A tab as saved: a file relative to the workspace, a Pi session by its
/// file in Pi's store, or a terminal, which starts afresh.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedTab {
    File(PathBuf),
    Session(PathBuf),
    Terminal,
}

/// The pane tree as saved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedNode {
    Pane {
        tabs: Vec<SavedTab>,
        active: Option<usize>,
    },
    Split {
        /// Side by side when true, stacked when false.
        horizontal: bool,
        sizes: Vec<f64>,
        children: Vec<SavedNode>,
    },
}

/// No more panes than this are restored, so a damaged file stays usable.
pub const MAX_PANES: usize = 16;
const MAX_DEPTH: usize = 8;

impl SavedNode {
    /// Panes in reading order.
    pub fn panes(&self) -> Vec<&SavedNode> {
        match self {
            SavedNode::Pane { .. } => vec![self],
            SavedNode::Split { children, .. } => {
                children.iter().flat_map(SavedNode::panes).collect()
            }
        }
    }

    /// Drops what cannot be restored and repairs what can: unsafe paths,
    /// splits of one, sizes that are not positive numbers, too many panes.
    fn normalize(self, depth: usize, budget: &mut usize) -> Option<SavedNode> {
        match self {
            SavedNode::Pane { mut tabs, active } => {
                if *budget == 0 {
                    return None;
                }
                *budget -= 1;
                tabs.retain(|tab| match tab {
                    SavedTab::File(path) => safe_relative(path),
                    SavedTab::Session(path) => {
                        path.is_absolute() && path.extension().is_some_and(|e| e == "jsonl")
                    }
                    SavedTab::Terminal => true,
                });
                let mut seen = std::collections::HashSet::new();
                tabs.retain(|tab| *tab == SavedTab::Terminal || seen.insert(tab.clone()));
                tabs.truncate(crate::MAX_OPEN_DOCUMENTS);
                let active = active.filter(|&i| i < tabs.len());
                Some(SavedNode::Pane { tabs, active })
            }
            SavedNode::Split {
                horizontal,
                sizes,
                children,
            } => {
                if depth >= MAX_DEPTH {
                    return None;
                }
                let kept: Vec<(SavedNode, f64)> = children
                    .into_iter()
                    .zip(sizes.into_iter().chain(std::iter::repeat(1.0)))
                    .filter_map(|(child, size)| {
                        let size = if size.is_finite() && size > 0.0 {
                            size
                        } else {
                            1.0
                        };
                        Some((child.normalize(depth + 1, budget)?, size))
                    })
                    .collect();
                match kept.len() {
                    0 => None,
                    1 => kept.into_iter().next().map(|(child, _)| child),
                    _ => {
                        let total: f64 = kept.iter().map(|(_, size)| size).sum();
                        let (children, sizes) = kept
                            .into_iter()
                            .map(|(child, size)| (child, size / total))
                            .unzip();
                        Some(SavedNode::Split {
                            horizontal,
                            sizes,
                            children,
                        })
                    }
                }
            }
        }
    }
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            version: 1,
            explorer_visible: true,
            explorer_right: false,
            explorer_width: 240.0,
            split: false,
            split_fraction: 0.5,
            open_files: Vec::new(),
            active: [None, None],
            panes: None,
            focused: 0,
        }
    }
}

impl Layout {
    pub fn normalize(&mut self) {
        self.explorer_width = if self.explorer_width.is_finite() {
            self.explorer_width.clamp(160.0, 480.0)
        } else {
            240.0
        };
        self.split_fraction = if self.split_fraction.is_finite() {
            self.split_fraction.clamp(0.2, 0.8)
        } else {
            0.5
        };
        self.open_files.retain(|path| safe_relative(path));
        self.open_files.truncate(crate::MAX_OPEN_DOCUMENTS);
        let mut seen = std::collections::HashSet::new();
        self.open_files.retain(|path| seen.insert(path.clone()));
        for active in &mut self.active {
            if active
                .as_ref()
                .is_some_and(|p| !self.open_files.contains(p))
            {
                *active = None;
            }
        }
        let mut budget = MAX_PANES;
        self.panes = self
            .panes
            .take()
            .and_then(|node| node.normalize(0, &mut budget));
        let count = self.panes.as_ref().map_or(1, |node| node.panes().len());
        if self.focused >= count {
            self.focused = 0;
        }
    }

    /// The panes to restore: the saved tree, or one made from an older
    /// layout's open files and its two side-by-side panes.
    pub fn pane_tree(&self) -> SavedNode {
        if let Some(panes) = &self.panes {
            return panes.clone();
        }
        let files: Vec<SavedTab> = self
            .open_files
            .iter()
            .cloned()
            .map(SavedTab::File)
            .collect();
        let pane = |active: &Option<PathBuf>| SavedNode::Pane {
            active: active
                .as_ref()
                .and_then(|a| self.open_files.iter().position(|f| f == a)),
            tabs: files.clone(),
        };
        if self.split {
            SavedNode::Split {
                horizontal: true,
                sizes: vec![self.split_fraction, 1.0 - self.split_fraction],
                children: vec![pane(&self.active[0]), pane(&self.active[1])],
            }
        } else {
            pane(&self.active[0])
        }
    }
}

fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
}

pub fn state_path(root: &Path) -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "pi-workbench", "Pi Workbench")
        .context("No application data directory")?;
    // Stable FNV-1a, unlike the implementation-defined DefaultHasher.
    let hash = root
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |h, byte| {
            (h ^ *byte as u64).wrapping_mul(0x100000001b3)
        });
    Ok(dirs
        .data_local_dir()
        .join("workspaces")
        .join(format!("{hash:016x}.json")))
}

pub fn load(path: &Path) -> Result<Layout> {
    if !path.exists() {
        return Ok(Layout::default());
    }
    let bytes = fs::read(path)?;
    ensure!(bytes.len() <= 1024 * 1024, "Layout file is too large");
    let mut layout: Layout = serde_json::from_slice(&bytes).context("Invalid saved layout")?;
    ensure!(
        layout.version == 1,
        "Unsupported layout version {}",
        layout.version
    );
    layout.normalize();
    Ok(layout)
}

pub fn save(path: &Path, layout: &Layout) -> Result<()> {
    let parent = path.parent().context("Layout has no parent")?;
    fs::create_dir_all(parent)?;
    let mut layout = layout.clone();
    layout.normalize();
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&serde_json::to_vec_pretty(&layout)?)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_restores_panes_and_sidebar() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("workspace.json");
        let layout = Layout {
            explorer_right: true,
            split_fraction: 0.65,
            open_files: vec!["src/main.rs".into()],
            active: [Some("src/main.rs".into()), Some("src/main.rs".into())],
            ..Default::default()
        };
        save(&path, &layout).unwrap();
        let restored = load(&path).unwrap();
        assert!(restored.explorer_right);
        assert_eq!(restored.split_fraction, 0.65);
        assert_eq!(restored.active, layout.active);
    }

    #[test]
    fn invalid_paths_and_sizes_are_sanitized() {
        let mut layout = Layout {
            explorer_width: 1e9,
            split_fraction: f64::NAN,
            open_files: vec![
                "../secret".into(),
                "/secret".into(),
                "src/main.rs".into(),
                "src/main.rs".into(),
            ],
            active: [Some("../secret".into()), None],
            ..Default::default()
        };
        layout.normalize();
        assert_eq!(layout.explorer_width, 480.0);
        assert_eq!(layout.split_fraction, 0.5);
        assert_eq!(layout.open_files, vec![PathBuf::from("src/main.rs")]);
        assert_eq!(layout.active, [None, None]);
    }

    #[test]
    fn pane_trees_round_trip_and_are_repaired() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("panes.json");
        let tree = SavedNode::Split {
            horizontal: true,
            sizes: vec![3.0, f64::NAN, 1.0],
            children: vec![
                SavedNode::Pane {
                    tabs: vec![
                        SavedTab::File("src/a.rs".into()),
                        SavedTab::File("../escape".into()),
                        SavedTab::File("src/a.rs".into()),
                        SavedTab::Terminal,
                        SavedTab::Terminal,
                    ],
                    active: Some(9),
                },
                // A split of one collapses into its child.
                SavedNode::Split {
                    horizontal: false,
                    sizes: vec![1.0],
                    children: vec![SavedNode::Pane {
                        tabs: vec![SavedTab::Session("relative.jsonl".into())],
                        active: Some(0),
                    }],
                },
                SavedNode::Split {
                    horizontal: false,
                    sizes: vec![],
                    children: vec![],
                },
            ],
        };
        let layout = Layout {
            panes: Some(tree),
            focused: 7,
            ..Default::default()
        };
        save(&path, &layout).unwrap();
        let restored = load(&path).unwrap();
        let Some(SavedNode::Split {
            sizes, children, ..
        }) = &restored.panes
        else {
            panic!("expected a split, got {:?}", restored.panes);
        };
        assert_eq!(children.len(), 2);
        // NaN counts as an even share before the shares are scaled to 1.
        assert_eq!(sizes, &[0.75, 0.25]);
        assert_eq!(
            children[0],
            SavedNode::Pane {
                tabs: vec![
                    SavedTab::File("src/a.rs".into()),
                    SavedTab::Terminal,
                    SavedTab::Terminal
                ],
                active: None,
            }
        );
        assert_eq!(
            children[1],
            SavedNode::Pane {
                tabs: vec![],
                active: None
            }
        );
        assert_eq!(restored.focused, 0);
    }

    #[test]
    fn older_layouts_become_panes() {
        let layout = Layout {
            split: true,
            split_fraction: 0.6,
            open_files: vec!["a.rs".into(), "b.rs".into()],
            active: [Some("b.rs".into()), Some("a.rs".into())],
            ..Default::default()
        };
        let SavedNode::Split {
            sizes, children, ..
        } = layout.pane_tree()
        else {
            panic!("expected a split");
        };
        assert_eq!(sizes, vec![0.6, 0.4]);
        assert!(
            matches!(&children[0], SavedNode::Pane { active: Some(1), tabs } if tabs.len() == 2)
        );
        assert!(matches!(
            &children[1],
            SavedNode::Pane {
                active: Some(0),
                ..
            }
        ));
    }

    #[test]
    fn future_layout_is_not_silently_reinterpreted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layout.json");
        fs::write(&path, r#"{"version":99}"#).unwrap();
        assert!(load(&path).is_err());
    }
}
