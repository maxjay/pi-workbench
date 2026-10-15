//! The settings in effect, re-read when the file changes.
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::SystemTime,
};
use workbench_core::settings::{self, Settings};

struct Loaded {
    settings: Arc<Settings>,
    modified: Option<SystemTime>,
    error: Option<String>,
}

static CURRENT: Mutex<Option<Loaded>> = Mutex::new(None);

fn modified() -> Option<SystemTime> {
    std::fs::metadata(settings::path().ok()?)
        .and_then(|m| m.modified())
        .ok()
}

/// The settings, re-read if the file changed since last time. A broken file
/// keeps the last good settings, and the error is returned once.
pub fn current() -> (Arc<Settings>, Option<String>) {
    // Tests use the defaults, not whatever file the machine has.
    if cfg!(test) {
        return (Arc::default(), None);
    }
    let mut current = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    let stamp = modified();
    let stale = current.as_ref().is_none_or(|l| l.modified != stamp);
    if stale {
        let previous = current.as_ref().map(|l| l.settings.clone());
        *current = Some(match settings::load() {
            Ok(settings) => Loaded {
                settings: Arc::new(settings),
                modified: stamp,
                error: None,
            },
            Err(err) => Loaded {
                settings: previous.unwrap_or_default(),
                modified: stamp,
                error: Some(format!("Settings: {err:#}")),
            },
        });
    }
    let loaded = current.as_mut().expect("loaded above");
    (loaded.settings.clone(), loaded.error.take())
}

pub fn get() -> Arc<Settings> {
    current().0
}

/// The settings file in the same form as opened paths, created if missing.
pub fn file() -> anyhow::Result<PathBuf> {
    let path = settings::ensure_file()?;
    Ok(path.canonicalize().unwrap_or(path))
}

pub fn code_font() -> String {
    get()
        .font_family
        .clone()
        .unwrap_or_else(|| crate::ui::CODE_FONT.to_owned())
}

pub fn code_size() -> usize {
    get()
        .font_size
        .filter(|size| (6.0..=48.0).contains(size))
        .map_or(crate::ui::CODE_SIZE, |size| size.round() as usize)
}
