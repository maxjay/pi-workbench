//! The app's log: what the app, wb-agent and wb-browser say along the way,
//! kept in a file, as a Windows app has no console to say it on. It's
//! `pi-workbench.log` in the app's log folder (on Windows,
//! `%LOCALAPPDATA%\dev.maxjay.pi-workbench\logs`), with the run before as
//! `pi-workbench.old.log`. Chromium's own log is `wb-browser.log` beside it.

use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use tauri::{AppHandle, Manager};

static FILE: OnceLock<Mutex<File>> = OnceLock::new();

/// Starts this run's log.
pub fn setup(app: &AppHandle) {
    let Some(dir) = dir(app) else { return };
    let path = dir.join("pi-workbench.log");
    let _ = fs::rename(&path, dir.join("pi-workbench.old.log"));
    if let Ok(file) = File::create(&path) {
        let _ = FILE.set(Mutex::new(file));
    }
    note(&format!(
        "Pi Workbench {} started",
        env!("CARGO_PKG_VERSION")
    ));
}

/// The log folder, made if it isn't there.
pub fn dir(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_log_dir().ok()?;
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// One line for the log, and for the console when there is one.
pub fn note(line: &str) {
    eprintln!("{line}");
    if let Some(file) = FILE.get() {
        let mut file = file.lock().expect("not poisoned");
        let _ = writeln!(file, "{} {line}", time());
    }
}

/// The time of day, in UTC: enough to line things up within a run.
fn time() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let day = secs % 86_400;
    format!("{:02}:{:02}:{:02}", day / 3600, day % 3600 / 60, day % 60)
}
