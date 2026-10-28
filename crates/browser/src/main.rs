//! wb-browser: the workbench's Chromium, in CEF. Each web tab is a
//! Chrome-style browser (the real extension system, history and downloads,
//! without Chrome's own toolbar) drawn as a child of the workbench's window,
//! where the app says. Its DevTools port is how wb-agent and Pi drive it.
//!
//! It talks to the app in lines of JSON: on stdin, what to open and where
//! each tab goes; on stdout, what opened, closed or was pressed. Tabs are
//! made here, not by DevTools' `Target.createTarget`, which in CEF would
//! open a Chrome window of its own.
//!
//! The same program is also each of Chromium's helper processes.

mod handlers;
mod tabs;
mod window;

use std::io::BufRead;

use cef::*;

/// What the app says on the command line.
struct Options {
    port: i32,
    profile: String,
    parent: u64,
    extensions: Option<String>,
}

fn options() -> Options {
    let mut options = Options {
        port: 9222,
        profile: String::new(),
        parent: 0,
        extensions: None,
    };
    for arg in std::env::args().skip(1) {
        let Some((key, value)) = arg.split_once('=') else {
            continue;
        };
        match key {
            "--port" => options.port = value.parse().unwrap_or(options.port),
            "--profile" => options.profile = value.to_string(),
            "--parent" => options.parent = value.parse().unwrap_or(0),
            "--extensions" => options.extensions = Some(value.to_string()),
            _ => {}
        }
    }
    options
}

fn main() {
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    let args = args::Args::new();
    let Some(line) = args.as_cmd_line() else {
        std::process::exit(1);
    };
    let helper = line.has_switch(Some(&CefString::from("type"))) == 1;
    let options = options();
    let mut app = handlers::app(options.extensions.clone());
    // A helper process does its job here and never comes back.
    let code = execute_process(
        Some(args.as_main_args()),
        Some(&mut app),
        std::ptr::null_mut(),
    );
    if helper {
        std::process::exit(code);
    }

    tabs::set_parent(options.parent);
    forget_windows(&options.profile);
    let settings = Settings {
        no_sandbox: 1,
        remote_debugging_port: options.port,
        root_cache_path: CefString::from(options.profile.as_str()),
        cache_path: CefString::from(options.profile.as_str()),
        persist_session_cookies: 1,
        ..Default::default()
    };
    if initialize(
        Some(args.as_main_args()),
        Some(&settings),
        Some(&mut app),
        std::ptr::null_mut(),
    ) != 1
    {
        eprintln!("wb-browser: Chromium didn't start");
        std::process::exit(1);
    }
    std::thread::spawn(read_commands);
    run_message_loop();
    shutdown();
}

/// Keeping session cookies has Chromium continue where it left off, which
/// would reopen the last run's pages as Chrome windows of their own. The
/// workbench reopens its tabs itself, so Chromium's record of them goes.
fn forget_windows(profile: &str) {
    let Ok(dirs) = std::fs::read_dir(profile) else {
        return;
    };
    for dir in dirs.flatten() {
        let _ = std::fs::remove_dir_all(dir.path().join("Sessions"));
    }
}

/// The app's lines, each run on Chromium's UI thread, until stdin ends;
/// then the browser closes.
fn read_commands() {
    for line in std::io::stdin().lock().lines().map_while(Result::ok) {
        if !line.trim().is_empty() {
            tabs::post(line);
        }
    }
    tabs::post(r#"{"quit":true}"#.into());
}

/// A line for the app.
pub fn say(message: serde_json::Value) {
    println!("{message}");
}
