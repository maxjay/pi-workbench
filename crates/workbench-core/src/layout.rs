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
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            version: 1,
            explorer_visible: true,
            explorer_right: false,
            explorer_width: 240.0,
            split: true,
            split_fraction: 0.5,
            open_files: Vec::new(),
            active: [None, None],
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
        self.open_files.truncate(64);
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
    fn future_layout_is_not_silently_reinterpreted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("layout.json");
        fs::write(&path, r#"{"version":99}"#).unwrap();
        assert!(load(&path).is_err());
    }
}
