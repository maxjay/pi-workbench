//! The app: a window for the UI, and wb-agent behind it. The app doesn't
//! read what they say to each other; it only carries the lines. It keeps
//! running in the tray when the window closes, so tasks run on time.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent;
mod background;
mod launch;
#[cfg(windows)]
mod wsl;

use agent::Agent;
use background::Background;
use tauri::{AppHandle, Manager, State, WindowEvent};

/// Starts wb-agent, or starts it again.
#[tauri::command(async)]
fn agent_start(app: AppHandle, agent: State<Agent>) -> Result<(), String> {
    agent.start(&app)
}

/// Sends wb-agent one line.
#[tauri::command(async)]
fn agent_send(agent: State<Agent>, line: String) -> Result<(), String> {
    agent.send(&line)
}

fn main() {
    tauri::Builder::default()
        // One app at a time: starting it again brings the running one back,
        // rather than running every task twice.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            background::show(app)
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![background::HIDDEN]),
        ))
        .manage(Agent::default())
        .manage(Background::default())
        .setup(|app| {
            background::setup(app.handle())?;
            // Started at sign-in, it stays in the tray until you open it.
            if !std::env::args().any(|arg| arg == background::HIDDEN) {
                background::show(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && window.app_handle().state::<Background>().close_to_tray()
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            agent_start,
            agent_send,
            background::background_get,
            background::background_set,
            background::tray_update,
            background::notify_run,
        ])
        .run(tauri::generate_context!())
        .expect("the window opens");
}
