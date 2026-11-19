//! The app: a window for the page, and wbd behind it. The app doesn't read
//! what the page and wbd say to each other. It only carries the lines.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod daemon;
#[cfg(windows)]
mod icon;
mod launch;
#[cfg(windows)]
mod wsl;

use daemon::Daemon;
use tauri::{AppHandle, State};
#[cfg(windows)]
use tauri::{Manager, WindowEvent};

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

/// Opens a link in your own browser. Only https links: the page shows
/// what agents and reviewers wrote, and nothing else they name is run.
#[tauri::command(async)]
fn open_url(url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("only https links are opened".into());
    }
    let opener = if cfg!(windows) {
        "explorer.exe"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener)
        .arg(&url)
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("couldn't open the link: {err}"))
}

fn main() {
    tauri::Builder::default()
        .manage(Daemon::default())
        .invoke_handler(tauri::generate_handler![
            daemon_start,
            daemon_send,
            open_url
        ])
        .setup(|app| {
            #[cfg(windows)]
            if let Some(window) = app.get_webview_window("main") {
                icon::sharpen(&window);
                let moved = window.clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::ScaleFactorChanged { .. } = event {
                        icon::sharpen(&moved);
                    }
                });
            }
            #[cfg(not(windows))]
            let _ = app;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("the window opens");
}
