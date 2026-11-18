//! The app: a window for the page, and wbd behind it. The app doesn't read
//! what the page and wbd say to each other. It only carries the lines.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod daemon;
mod launch;

use daemon::Daemon;
use tauri::{AppHandle, State};

/// Starts wbd, or starts it again.
#[tauri::command(async)]
fn daemon_start(app: AppHandle, daemon: State<Daemon>) -> Result<(), String> {
    daemon.start(&app)
}

/// Sends wbd one line.
#[tauri::command(async)]
fn daemon_send(daemon: State<Daemon>, line: String) -> Result<(), String> {
    daemon.send(&line)
}

fn main() {
    tauri::Builder::default()
        .manage(Daemon::default())
        .invoke_handler(tauri::generate_handler![daemon_start, daemon_send])
        .run(tauri::generate_context!())
        .expect("the window opens");
}
