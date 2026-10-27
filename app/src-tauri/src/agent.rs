//! wb-agent as the app sees it: one child process, lines in and lines out.
//! Each line it prints reaches the page as an `agent-line` event, but for
//! its streams to the browser, which the app carries itself; when it
//! stops, the page gets `agent-exit` with the last thing it said on stderr.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use tauri::{AppHandle, Emitter, Manager};

use crate::browser::Browser;
use crate::launch;

#[derive(Default)]
pub struct Agent {
    running: Mutex<Option<Running>>,
    /// Counts starts, so an old process that ends after a restart doesn't
    /// report the new one as stopped.
    starts: Arc<AtomicU64>,
}

struct Running {
    child: Child,
    stdin: ChildStdin,
}

impl Agent {
    pub fn start(&self, app: &AppHandle) -> Result<(), String> {
        let mut running = self.running.lock().expect("not poisoned");
        if let Some(old) = running.take() {
            old.stop();
        }
        let start = self.starts.fetch_add(1, Ordering::SeqCst) + 1;

        let mut child = launch::command()?
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("couldn't start wb-agent: {err}"))?;
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        let stderr = child.stderr.take().expect("stderr is piped");

        let last_error = Arc::new(Mutex::new(String::new()));
        thread::spawn({
            let last_error = last_error.clone();
            move || log(stderr, &last_error)
        });
        thread::spawn({
            let app = app.clone();
            let starts = self.starts.clone();
            move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if line.starts_with(r#"{"type":"tunnel""#) {
                        app.state::<Browser>().tunnel(&app, &line);
                    } else {
                        let _ = app.emit("agent-line", line);
                    }
                }
                if starts.load(Ordering::SeqCst) == start {
                    let last = last_error.lock().expect("not poisoned");
                    let reason = match last.as_str() {
                        "" => "wb-agent stopped".to_string(),
                        last => format!("wb-agent stopped: {last}"),
                    };
                    let _ = app.emit("agent-exit", reason);
                }
            }
        });

        *running = Some(Running { child, stdin });
        Ok(())
    }

    pub fn send(&self, line: &str) -> Result<(), String> {
        let mut running = self.running.lock().expect("not poisoned");
        let running = running.as_mut().ok_or("wb-agent isn't running")?;
        writeln!(running.stdin, "{line}")
            .and_then(|()| running.stdin.flush())
            .map_err(|err| format!("couldn't reach wb-agent: {err}"))
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        if let Some(running) = self.running.get_mut().ok().and_then(Option::take) {
            running.stop();
        }
    }
}

impl Running {
    fn stop(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Passes wb-agent's stderr on to the app's, keeping the last line.
fn log(stderr: impl Read, last: &Mutex<String>) {
    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
        eprintln!("wb-agent: {line}");
        *last.lock().expect("not poisoned") = line;
    }
}
