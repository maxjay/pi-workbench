//! The browser behind web tabs. wb-agent, in WSL, reaches its DevTools
//! port through the app: a tunnel line from wb-agent names a stream, which
//! the app opens to the port and carries both ways as more lines. The
//! browser is wb-browser, the CEF program beside the app, which draws each
//! web tab's page in the window where the page says, and makes the tabs
//! wb-agent asks for: its host lines go to wb-browser and back. Shortcuts
//! pressed in a page come to the page as `browser-key`. In development any
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
use tauri::{AppHandle, Emitter, Manager};

use crate::agent::Agent;

#[derive(Default)]
pub struct Browser {
    /// Where the DevTools port is, once there's a browser.
    address: Mutex<Option<String>>,
    streams: Mutex<HashMap<u32, TcpStream>>,
    host: Mutex<Option<Host>>,
    /// wb-browser has said it's running, for a wb-agent started later.
    ready: std::sync::atomic::AtomicBool,
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
    // It needs the window's handle, which exists once the window is up:
    // asked for on the main thread, again until it's there.
    let app = app.clone();
    thread::spawn(move || {
        for _ in 0..600 {
            let (tx, rx) = std::sync::mpsc::channel();
            let on_main = app.clone();
            let asked = app.run_on_main_thread(move || {
                let _ = tx.send(parent(&on_main).ok());
            });
            if asked.is_ok()
                && let Ok(Some(parent)) = rx.recv()
            {
                if let Err(err) = app.state::<Browser>().start(&app, parent) {
                    eprintln!("pi-workbench: no browser: {err}");
                }
                return;
            }
            thread::sleep(std::time::Duration::from_millis(500));
        }
        eprintln!("pi-workbench: no browser: the window never came up");
    });
}

impl Browser {
    fn start(&self, app: &AppHandle, parent: isize) -> Result<(), String> {
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
        command.arg(format!("--parent={parent}"));
        if let Ok(extensions) = std::env::var("WB_BROWSER_EXTENSIONS") {
            command.arg(format!("--extensions={extensions}"));
        }
        // CEF's library is beside it.
        #[cfg(target_os = "linux")]
        if let Some(dir) = beside("wb-browser").and_then(|exe| Some(exe.parent()?.to_path_buf())) {
            command.env("LD_LIBRARY_PATH", dir);
        }
        let mut child = command
            .spawn()
            .map_err(|err| format!("couldn't start wb-browser: {err}"))?;
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let app = app.clone();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                app.state::<Browser>().heard(&app, &line);
            }
        });
        *self.address.lock().expect("not poisoned") = Some(format!("127.0.0.1:{port}"));
        *self.host.lock().expect("not poisoned") = Some(Host { child, stdin });
        Ok(())
    }

    /// A line from wb-browser: a shortcut for the page, or news for wb-agent.
    fn heard(&self, app: &AppHandle, line: &str) {
        let Ok(Value::Object(mut said)) = serde_json::from_str::<Value>(line) else {
            return eprintln!("wb-browser: {line}");
        };
        if let Some(key) = said.get("key") {
            let _ = app.emit("browser-key", key.clone());
            return;
        }
        if said.contains_key("ready") {
            self.ready.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        let mut news = serde_json::Map::new();
        news.insert("type".into(), "host".into());
        news.append(&mut said);
        let _ = app.state::<Agent>().send(&Value::Object(news).to_string());
    }

    /// Tells a newly started wb-agent that wb-browser makes the tabs.
    pub fn announce(&self, app: &AppHandle) {
        if self.ready.load(std::sync::atomic::Ordering::SeqCst) {
            let _ = app.state::<Agent>().send(r#"{"type":"host","ready":true}"#);
        }
    }

    /// A host line from wb-agent, for wb-browser.
    pub fn host(&self, line: &str) {
        let Ok(Value::Object(mut call)) = serde_json::from_str::<Value>(line) else {
            return;
        };
        call.remove("type");
        if let Some(host) = self.host.lock().expect("not poisoned").as_mut() {
            let _ = writeln!(host.stdin, "{}", Value::Object(call));
            let _ = host.stdin.flush();
        }
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

/// The workbench window's native handle, which wb-browser draws tabs in.
fn parent(app: &AppHandle) -> Result<isize, String> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let window = app
        .get_webview_window("main")
        .ok_or("no window to draw tabs in")?;
    let handle = window.window_handle().map_err(|err| err.to_string())?;
    match handle.as_raw() {
        RawWindowHandle::Win32(h) => Ok(h.hwnd.get()),
        RawWindowHandle::Xlib(h) => Ok(h.window as isize),
        RawWindowHandle::Xcb(h) => Ok(h.window.get() as isize),
        other => Err(format!("can't draw tabs in a {other:?} window")),
    }
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
    let started = parent(&app).and_then(|handle| browser.start(&app, handle));
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
