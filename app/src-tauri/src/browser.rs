//! The browser behind web tabs. wb-agent, in WSL, reaches its DevTools
//! port through the app: a tunnel line from wb-agent names a stream, which
//! the app opens to the port and carries both ways as more lines. The
//! browser is wb-browser, the CEF program beside the app, which draws each
//! web tab's page over the window where the page says. In development any
//! Chromium will do, at `WB_BROWSER_CDP` (`host:port`).

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;
use std::thread;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};

use crate::agent::Agent;

#[derive(Default)]
pub struct Browser {
    /// Where the DevTools port is, once there's a browser.
    address: Mutex<Option<String>>,
    streams: Mutex<HashMap<u32, TcpStream>>,
    host: Mutex<Option<Host>>,
}

struct Host {
    child: Child,
    stdin: ChildStdin,
}

/// Where the page wants a web tab's view: its page area, in the window's
/// CSS pixels.
#[derive(Clone, Debug, Deserialize, serde::Serialize)]
pub struct Place {
    tab: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    visible: bool,
}

/// Starts wb-browser, if the app carries one; or uses the Chromium at
/// `WB_BROWSER_CDP`.
pub fn setup(app: &AppHandle) {
    let browser = app.state::<Browser>();
    if let Ok(address) = std::env::var("WB_BROWSER_CDP") {
        *browser.address.lock().expect("not poisoned") = Some(address);
        return;
    }
    if let Err(err) = browser.start(app) {
        eprintln!("pi-workbench: no browser: {err}");
    }
}

impl Browser {
    fn start(&self, app: &AppHandle) -> Result<(), String> {
        let exe = beside("wb-browser").ok_or("wb-browser isn't installed beside the app")?;
        let profile = profile_dir(app)?;
        let port = free_port().map_err(|err| format!("no port for the browser: {err}"))?;
        let mut command = Command::new(exe);
        command
            .arg(format!("--port={port}"))
            .arg(format!("--profile={}", profile.display()))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        #[cfg(windows)]
        if let Some(window) = app.get_webview_window("main")
            && let Ok(hwnd) = window.hwnd()
        {
            command.arg(format!("--parent={}", hwnd.0 as isize));
        }
        let mut child = command
            .spawn()
            .map_err(|err| format!("couldn't start wb-browser: {err}"))?;
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                eprintln!("wb-browser: {line}");
            }
        });
        *self.address.lock().expect("not poisoned") = Some(format!("127.0.0.1:{port}"));
        *self.host.lock().expect("not poisoned") = Some(Host { child, stdin });
        Ok(())
    }

    fn stop(&self) {
        if let Some(mut host) = self.host.lock().expect("not poisoned").take() {
            let _ = writeln!(host.stdin, "{}", json!({ "quit": true }));
            let _ = host.child.wait();
        }
    }

    /// A tunnel line from wb-agent: open a stream, carry bytes, or close it.
    pub fn tunnel(&self, app: &AppHandle, line: &str) {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let stream = message["stream"].as_u64().unwrap_or_default() as u32;
        match message["op"].as_str().unwrap_or_default() {
            "open" => self.open(app, stream),
            "data" => {
                let bytes = message["data"]
                    .as_str()
                    .and_then(|d| STANDARD.decode(d).ok());
                let mut streams = self.streams.lock().expect("not poisoned");
                let failed = match (streams.get_mut(&stream), bytes) {
                    (Some(socket), Some(bytes)) => socket.write_all(&bytes).is_err(),
                    _ => false,
                };
                if failed && let Some(socket) = streams.remove(&stream) {
                    let _ = socket.shutdown(Shutdown::Both);
                }
            }
            "close" => {
                if let Some(socket) = self.streams.lock().expect("not poisoned").remove(&stream) {
                    let _ = socket.shutdown(Shutdown::Both);
                }
            }
            _ => {}
        }
    }

    fn open(&self, app: &AppHandle, stream: u32) {
        let address = self.address.lock().expect("not poisoned").clone();
        let socket = address.and_then(|address| TcpStream::connect(address).ok());
        let Some(socket) = socket else {
            send(app, stream, json!({ "op": "close" }));
            return;
        };
        let _ = socket.set_nodelay(true);
        let Ok(mut reading) = socket.try_clone() else {
            send(app, stream, json!({ "op": "close" }));
            return;
        };
        self.streams
            .lock()
            .expect("not poisoned")
            .insert(stream, socket);
        let app = app.clone();
        thread::spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match reading.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => send(
                        &app,
                        stream,
                        json!({ "op": "data", "data": STANDARD.encode(&buf[..n]) }),
                    ),
                }
            }
            let browser = app.state::<Browser>();
            if browser
                .streams
                .lock()
                .expect("not poisoned")
                .remove(&stream)
                .is_some()
            {
                send(&app, stream, json!({ "op": "close" }));
            }
        });
    }
}

/// A line to wb-agent about one of its streams.
fn send(app: &AppHandle, stream: u32, op: Value) {
    let mut line = json!({ "type": "tunnel", "stream": stream });
    if let (Some(line), Some(op)) = (line.as_object_mut(), op.as_object()) {
        line.extend(op.clone());
    }
    let _ = app.state::<Agent>().send(&line.to_string());
}

fn beside(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let path = exe
        .parent()?
        .join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    path.is_file().then_some(path)
}

fn profile_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_local_data_dir()
        .map_err(|err| err.to_string())?
        .join("browser");
    std::fs::create_dir_all(&dir)
        .map_err(|err| format!("couldn't make {}: {err}", dir.display()))?;
    Ok(dir)
}

fn free_port() -> std::io::Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// Puts a web tab's view where the page says, or hides it.
#[tauri::command]
pub fn browser_place(app: AppHandle, browser: tauri::State<Browser>, place: Place) {
    let scale = app
        .get_webview_window("main")
        .and_then(|window| window.scale_factor().ok())
        .unwrap_or(1.0);
    let mut host = browser.host.lock().expect("not poisoned");
    if let Some(host) = host.as_mut() {
        let _ = writeln!(host.stdin, "{}", json!({ "place": place, "scale": scale }));
        let _ = host.stdin.flush();
    }
}

/// Opens the Downloads folder, where the browser puts what it downloads.
#[tauri::command(async)]
pub fn open_downloads(app: AppHandle) -> Result<(), String> {
    let dir = app.path().download_dir().map_err(|err| err.to_string())?;
    let opener = if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    Command::new(opener)
        .arg(dir)
        .spawn()
        .map(drop)
        .map_err(|err| format!("couldn't open the Downloads folder: {err}"))
}

/// Copies the profile of the Chromium you use into the browser's own, with
/// the browser stopped meanwhile: extensions, sign-ins and history.
#[tauri::command(async)]
pub fn browser_import_profile(
    app: AppHandle,
    browser: tauri::State<Browser>,
) -> Result<String, String> {
    let from = chromium_profile().ok_or("No Chromium profile found to bring in")?;
    if from.join("lockfile").exists() || from.join("SingletonLock").exists() {
        return Err("Chromium is still open. Close it, then bring the profile in.".into());
    }
    let to = profile_dir(&app)?;
    browser.stop();
    let copied = copy_tree(&from, &to);
    let started = browser.start(&app);
    let files = copied.map_err(|err| format!("couldn't copy the profile: {err}"))?;
    started?;
    Ok(format!("Brought in your Chromium profile: {files} files."))
}

fn chromium_profile() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        PathBuf::from(std::env::var_os("LOCALAPPDATA")?)
            .join("Chromium")
            .join("User Data")
    } else {
        PathBuf::from(std::env::var_os("HOME")?)
            .join(".config")
            .join("chromium")
    };
    base.is_dir().then_some(base)
}

/// Copies every file under `from` into `to`, skipping caches.
fn copy_tree(from: &Path, to: &Path) -> std::io::Result<u64> {
    let mut copied = 0;
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        let skip = [
            "Cache",
            "Code Cache",
            "GPUCache",
            "ShaderCache",
            "GrShaderCache",
            "Crashpad",
        ];
        if skip.iter().any(|s| name == *s) {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type()?.is_dir() {
            copied += copy_tree(&entry.path(), &target)?;
        } else if std::fs::copy(entry.path(), &target).is_ok() {
            copied += 1;
        }
    }
    Ok(copied)
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.stop();
    }
}
