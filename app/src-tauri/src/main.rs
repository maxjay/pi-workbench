//! The app: a window for the UI, and wb-agent behind it. The app doesn't
//! read what they say to each other; it only carries the lines.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod agent;
mod launch;
#[cfg(windows)]
mod wsl;

use agent::Agent;
use tauri::{AppHandle, State};

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
        .manage(Agent::default())
        .invoke_handler(tauri::generate_handler![agent_start, agent_send])
        .run(tauri::generate_context!())
        .expect("the window opens");
}
