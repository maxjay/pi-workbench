//! Terminals: shells in pseudo-terminals, in a codespace's folder. What
//! each prints goes to the workbench as it comes, gathered up every
//! [`FLUSH`], and the last of it is kept so a terminal can be drawn again
//! and Pi can read it. Pi reaches them through `terminal.ts`, a tool that
//! asks over one socket, named in `WB_TERMINALS`.

use std::ffi::CStr;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use protocol::{Event, FromAgent, Terminal};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

use crate::codespaces::Codespaces;
use crate::error::Error;
use crate::session::Out;

const EXTENSION: &str = include_str!("terminal.ts");

/// How often what terminals printed goes to the workbench.
const FLUSH: Duration = Duration::from_millis(30);
/// How often each is checked for what runs in it.
const LOOK: Duration = Duration::from_millis(700);
/// How much of what a terminal printed is kept, in bytes.
const KEPT: usize = 256 * 1024;

#[derive(Clone)]
pub struct Terminals {
    all: Arc<Mutex<Vec<Arc<Term>>>>,
    next: Arc<AtomicU64>,
    out: Out,
    spaces: Arc<Codespaces>,
}

struct Term {
    id: String,
    cwd: PathBuf,
    shell: String,
    master: File,
    pid: i32,
    /// What it printed lately, and what hasn't gone to the workbench yet.
    kept: Mutex<String>,
    pending: Mutex<String>,
    /// Everything it ever printed, counted, so Pi can read what came after.
    printed: AtomicU64,
    now: Mutex<Now>,
}

#[derive(Clone, PartialEq)]
struct Now {
    title: String,
    running: bool,
}

impl Terminals {
    pub fn new(out: Out, spaces: Arc<Codespaces>) -> Self {
        Terminals {
            all: Arc::default(),
            next: Arc::new(AtomicU64::new(1)),
            out,
            spaces,
        }
    }

    /// Sends what terminals print, and notices what runs in them.
    pub fn start(&self) {
        let terminals = self.clone();
        tokio::spawn(async move {
            let mut flush = tokio::time::interval(FLUSH);
            let mut look = tokio::time::interval(LOOK);
            loop {
                tokio::select! {
                    _ = flush.tick() => terminals.flush(),
                    _ = look.tick() => terminals.look(),
                }
            }
        });
    }

    pub fn list(&self, dir: &Path) -> Vec<Terminal> {
        self.terms()
            .iter()
            .filter(|term| term.cwd.starts_with(dir))
            .map(|term| term.view())
            .collect()
    }

    fn terms(&self) -> Vec<Arc<Term>> {
        self.all.lock().expect("not poisoned").clone()
    }

    fn get(&self, id: &str) -> Result<Arc<Term>, Error> {
        self.terms()
            .into_iter()
            .find(|term| term.id == id)
            .ok_or_else(|| Error::NotFound(format!("no terminal {id}")))
    }

    /// Starts the shell in `cwd`.
    pub fn open(&self, cwd: &Path, cols: u16, rows: u16) -> Result<Terminal, Error> {
        if !cwd.is_dir() {
            return Err(Error::NotFound(format!("no folder at {}", cwd.display())));
        }
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|shell| !shell.is_empty())
            .unwrap_or_else(|| "/bin/bash".into());
        let (master, slave) = pty(cols, rows).map_err(|err| Error::io("a terminal", err))?;
        let mut command = std::process::Command::new(&shell);
        command
            .current_dir(cwd)
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .stdin(Stdio::from(
                slave.try_clone().map_err(|e| Error::io(cwd, e))?,
            ))
            .stdout(Stdio::from(
                slave.try_clone().map_err(|e| Error::io(cwd, e))?,
            ))
            .stderr(Stdio::from(slave));
        // The shell leads a session of its own, with the terminal as its own.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command
            .spawn()
            .map_err(|err| Error::io(PathBuf::from(&shell), err))?;
        drop(command);
        let reader = master.try_clone().map_err(|err| Error::io(cwd, err))?;
        let name = Path::new(&shell)
            .file_name()
            .map_or_else(|| shell.clone(), |name| name.to_string_lossy().into_owned());
        let term = Arc::new(Term {
            id: format!("t{}", self.next.fetch_add(1, Ordering::SeqCst)),
            cwd: cwd.to_path_buf(),
            pid: child.id() as i32,
            master,
            kept: Mutex::default(),
            pending: Mutex::default(),
            printed: AtomicU64::new(0),
            now: Mutex::new(Now {
                title: name.clone(),
                running: false,
            }),
            shell: name,
        });
        self.all.lock().expect("not poisoned").push(term.clone());

        // Reads what it prints until the shell and all it started are gone.
        std::thread::spawn({
            let term = term.clone();
            move || read(&term, reader)
        });
        // A shell that exits closes its terminal, as in an editor.
        std::thread::spawn({
            let terminals = self.clone();
            let id = term.id.clone();
            move || {
                let _ = child.wait();
                std::thread::sleep(FLUSH * 2);
                terminals.remove(&id);
            }
        });
        self.changed();
        Ok(term.view())
    }

    pub fn read(&self, id: &str) -> Result<String, Error> {
        let term = self.get(id)?;
        let kept = term.kept.lock().expect("not poisoned").clone();
        Ok(kept)
    }

    pub fn input(&self, id: &str, data: &str) -> Result<(), Error> {
        let term = self.get(id)?;
        (&term.master)
            .write_all(data.as_bytes())
            .map_err(|err| Error::io("a terminal", err))
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), Error> {
        let term = self.get(id)?;
        set_size(&term.master, cols, rows).map_err(|err| Error::io("a terminal", err))
    }

    /// Hangs up on the shell, and on everything it started.
    pub fn close(&self, id: &str) -> Result<(), Error> {
        let term = self.get(id)?;
        unsafe {
            libc::kill(-term.pid, libc::SIGHUP);
        }
        self.remove(id);
        Ok(())
    }

    fn remove(&self, id: &str) {
        let mut all = self.all.lock().expect("not poisoned");
        let before = all.len();
        all.retain(|term| term.id != id);
        let gone = all.len() < before;
        drop(all);
        if gone {
            self.changed();
        }
    }

    /// Hangs up on every shell.
    pub fn shutdown(&self) {
        for term in self.all.lock().expect("not poisoned").drain(..) {
            unsafe {
                libc::kill(-term.pid, libc::SIGHUP);
            }
        }
    }

    fn changed(&self) {
        let list = self.terms().iter().map(|term| term.view()).collect();
        let _ = self.out.send(FromAgent::Event(Event::Terminals { list }));
    }

    fn flush(&self) {
        for term in self.terms() {
            let data = std::mem::take(&mut *term.pending.lock().expect("not poisoned"));
            if !data.is_empty() {
                let _ = self.out.send(FromAgent::Event(Event::Terminal {
                    id: term.id.clone(),
                    data,
                }));
            }
        }
    }

    /// What runs in each terminal: whatever leads its foreground, when
    /// that isn't the shell.
    fn look(&self) {
        let mut changed = false;
        for term in self.terms() {
            let leader = unsafe { libc::tcgetpgrp(term.master.as_raw_fd()) };
            let running = leader > 0 && leader != term.pid;
            let title = if running {
                command_line(leader).unwrap_or_else(|| term.shell.clone())
            } else {
                term.shell.clone()
            };
            let now = Now { title, running };
            let mut was = term.now.lock().expect("not poisoned");
            if *was != now {
                *was = now;
                changed = true;
            }
        }
        if changed {
            self.changed();
        }
    }

    // ---- Pi's terminal tool.

    /// Writes the tool where Pi can load it, and says where.
    pub fn install(dir: &Path) -> std::io::Result<PathBuf> {
        let path = dir.join("terminal.ts");
        std::fs::create_dir_all(dir)?;
        if std::fs::read_to_string(&path).ok().as_deref() != Some(EXTENSION) {
            std::fs::write(&path, EXTENSION)?;
        }
        Ok(path)
    }

    /// Listens for the tool on its socket, and says where that is.
    pub fn serve(&self) -> std::io::Result<PathBuf> {
        let path = std::env::temp_dir()
            .join(format!("wb-agent-{}", std::process::id()))
            .join("terminals.sock");
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        let terminals = self.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(terminals.clone().asked(stream));
            }
        });
        Ok(path)
    }

    async fn asked(self, stream: UnixStream) {
        let (read, mut write) = stream.into_split();
        let mut lines = BufReader::new(read).lines();
        let Ok(Some(line)) = lines.next_line().await else {
            return;
        };
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(asked) => match self.tool(&asked).await {
                Ok(text) => json!({ "text": text }),
                Err(error) => json!({ "error": error }),
            },
            Err(err) => json!({ "error": format!("couldn't read the request: {err}") }),
        };
        let mut line = reply.to_string();
        line.push('\n');
        let _ = write.write_all(line.as_bytes()).await;
    }

    /// The folder whose terminals a session sees: its codespace's, or its own.
    fn scope(&self, cwd: &Path) -> PathBuf {
        self.spaces
            .holding(cwd)
            .map_or_else(|| cwd.to_path_buf(), |space| PathBuf::from(space.path))
    }

    async fn tool(&self, asked: &Value) -> Result<String, String> {
        let cwd = PathBuf::from(asked["cwd"].as_str().unwrap_or("/"));
        let scope = self.scope(&cwd);
        let seen = |id: &str| -> Result<Arc<Term>, String> {
            self.terms()
                .into_iter()
                .find(|term| term.id == id && term.cwd.starts_with(&scope))
                .ok_or_else(|| {
                    format!("There's no terminal {id} here. List them to see which there are.")
                })
        };
        let id = asked["id"].as_str().unwrap_or_default();
        match asked["action"].as_str().unwrap_or_default() {
            "list" => {
                let list = self.list(&scope);
                if list.is_empty() {
                    return Ok("No terminals are open here. Open one to run something.".into());
                }
                Ok(list
                    .iter()
                    .map(|t| {
                        let state = if t.running {
                            "running"
                        } else {
                            "idle, at its prompt"
                        };
                        format!("{}: {} ({state}) in {}", t.id, t.title, t.cwd)
                    })
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            "read" => {
                let term = seen(id)?;
                let lines = asked["lines"].as_u64().unwrap_or(60).clamp(1, 500) as usize;
                let kept = term.kept.lock().expect("not poisoned").clone();
                Ok(last_lines(&plain(&kept), lines))
            }
            "open" => {
                let term = self.open(&scope, 120, 30).map_err(|err| err.to_string())?;
                Ok(format!("Opened terminal {} in {}.", term.id, term.cwd))
            }
            "run" => {
                let term = seen(id)?;
                let command = asked["command"].as_str().unwrap_or_default().trim();
                if command.is_empty() {
                    return Err("Give the command to run.".into());
                }
                let busy = term.now.lock().expect("not poisoned").clone();
                if busy.running {
                    return Err(format!(
                        "{} is running `{}`. Wait for it and read it, or open another terminal.",
                        term.id, busy.title
                    ));
                }
                let wait = Duration::from_secs(asked["wait"].as_u64().unwrap_or(20).clamp(1, 600));
                let from = term.printed.load(Ordering::SeqCst);
                (&term.master)
                    .write_all(format!("{command}\r").as_bytes())
                    .map_err(|err| err.to_string())?;
                let finished = settle(&term, wait).await;
                let printed = term.printed.load(Ordering::SeqCst);
                let kept = term.kept.lock().expect("not poisoned").clone();
                // What came after the command was typed, of what's kept.
                let start = kept.len().saturating_sub((printed - from) as usize);
                let start = (start..=kept.len())
                    .find(|&at| kept.is_char_boundary(at))
                    .unwrap_or(kept.len());
                let mut text = last_lines(&plain(&kept[start..]), 200);
                if !finished {
                    text.push_str("\n(Still running. Read the terminal again later.)");
                }
                Ok(text)
            }
            other => Err(format!(
                "No such action: {other}. Use list, read, run or open."
            )),
        }
    }
}

impl Term {
    fn view(&self) -> Terminal {
        let now = self.now.lock().expect("not poisoned").clone();
        Terminal {
            id: self.id.clone(),
            cwd: self.cwd.display().to_string(),
            title: now.title,
            running: now.running,
            exited: false,
        }
    }
}

/// Waits for a command to finish: back at the prompt, and quiet for a
/// moment. False if it's still going after `wait`.
async fn settle(term: &Term, wait: Duration) -> bool {
    let start = Instant::now();
    let mut seen = term.printed.load(Ordering::SeqCst);
    let mut quiet = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_millis(150)).await;
        let printed = term.printed.load(Ordering::SeqCst);
        if printed != seen {
            seen = printed;
            quiet = Instant::now();
        }
        let leader = unsafe { libc::tcgetpgrp(term.master.as_raw_fd()) };
        let at_prompt = leader <= 0 || leader == term.pid;
        if at_prompt && quiet.elapsed() > Duration::from_millis(600) {
            return true;
        }
        if start.elapsed() > wait {
            return false;
        }
    }
}

/// Reads what a terminal prints, keeping the last of it, until it closes.
fn read(term: &Term, mut master: File) {
    let mut buffer = [0u8; 16 * 1024];
    let mut carry: Vec<u8> = Vec::new();
    loop {
        let n = match master.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        carry.extend_from_slice(&buffer[..n]);
        let whole = utf8_whole(&carry);
        let text = String::from_utf8_lossy(&carry[..whole]).into_owned();
        carry.drain(..whole);
        if text.is_empty() {
            continue;
        }
        term.printed.fetch_add(text.len() as u64, Ordering::SeqCst);
        term.pending.lock().expect("not poisoned").push_str(&text);
        let mut kept = term.kept.lock().expect("not poisoned");
        kept.push_str(&text);
        if kept.len() > KEPT {
            let mut cut = kept.len() - KEPT;
            while !kept.is_char_boundary(cut) {
                cut += 1;
            }
            kept.drain(..cut);
        }
    }
}

/// How much of `bytes` to send now: all of it, but for a character cut off
/// at the end, which waits for the rest.
fn utf8_whole(bytes: &[u8]) -> usize {
    match std::str::from_utf8(bytes) {
        Ok(_) => bytes.len(),
        Err(err) if err.error_len().is_none() => err.valid_up_to(),
        // Not UTF-8 at all: send it, shown as it can be.
        Err(_) => bytes.len(),
    }
}

/// A pseudo-terminal of the given size: its master and its slave.
fn pty(cols: u16, rows: u16) -> std::io::Result<(File, OwnedFd)> {
    let last = std::io::Error::last_os_error;
    unsafe {
        let master = libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC);
        if master < 0 {
            return Err(last());
        }
        let master = File::from_raw_fd(master);
        if libc::grantpt(master.as_raw_fd()) != 0 || libc::unlockpt(master.as_raw_fd()) != 0 {
            return Err(last());
        }
        let mut name = [0 as libc::c_char; 128];
        if libc::ptsname_r(master.as_raw_fd(), name.as_mut_ptr(), name.len()) != 0 {
            return Err(last());
        }
        let slave = libc::open(
            CStr::from_ptr(name.as_ptr()).as_ptr(),
            libc::O_RDWR | libc::O_NOCTTY | libc::O_CLOEXEC,
        );
        if slave < 0 {
            return Err(last());
        }
        set_size(&master, cols, rows)?;
        Ok((master, OwnedFd::from_raw_fd(slave)))
    }
}

fn set_size(master: &File, cols: u16, rows: u16) -> std::io::Result<()> {
    let size = libc::winsize {
        ws_row: rows.max(1),
        ws_col: cols.max(1),
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    if unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ, &size) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// A process's command line, shortened, as a terminal's title.
fn command_line(pid: i32) -> Option<String> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let words: Vec<String> = raw
        .split(|&b| b == 0)
        .filter(|word| !word.is_empty())
        .map(|word| String::from_utf8_lossy(word).into_owned())
        .collect();
    let first = words.first()?;
    // `/usr/bin/python3 -m http.server` reads better as `python3 -m http.server`.
    let program = Path::new(first)
        .file_name()
        .map_or_else(|| first.clone(), |name| name.to_string_lossy().into_owned());
    let line = std::iter::once(program)
        .chain(words.into_iter().skip(1))
        .collect::<Vec<_>>()
        .join(" ");
    Some(if line.chars().count() > 40 {
        format!("{}…", line.chars().take(39).collect::<String>())
    } else {
        line
    })
}

/// What a terminal showed, as plain text: escape sequences dropped, lines
/// a carriage return wrote over kept as they ended up.
pub fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut line = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                // CSI: parameters, then one final byte.
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC: to BEL or ESC \.
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\r' if chars.peek() == Some(&'\n') => {}
            '\r' => line.clear(),
            '\n' => {
                out.push_str(line.trim_end());
                out.push('\n');
                line.clear();
            }
            '\x08' => {
                line.pop();
            }
            c if c.is_control() && c != '\t' => {}
            c => line.push(c),
        }
    }
    out.push_str(&line);
    out
}

fn last_lines(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.trim_end().lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_from_a_terminal() {
        let shown =
            "\x1b]0;title\x07\x1b[1;32mok\x1b[0m done\r\nloading 10%\rloading 100%\nab\x08c\n";
        assert_eq!(plain(shown), "ok done\nloading 100%\nac\n");
        assert_eq!(last_lines("a\nb\nc\n", 2), "b\nc");
    }

    #[test]
    fn waits_for_the_rest_of_a_character() {
        let pound = "£".as_bytes();
        assert_eq!(utf8_whole(&[b'a', pound[0]]), 1);
        assert_eq!(utf8_whole(&[b'a', pound[0], pound[1]]), 3);
    }

    #[tokio::test]
    async fn runs_a_shell_and_reads_it() {
        let (out, mut events) = tokio::sync::mpsc::unbounded_channel();
        let data = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let spaces = Arc::new(Codespaces::new(data.path()));
        let terminals = Terminals::new(out, spaces);
        terminals.start();
        let term = terminals.open(folder.path(), 80, 24).unwrap();
        assert_eq!(terminals.list(folder.path()).len(), 1);

        let asked = json!({ "cwd": folder.path(), "action": "run", "id": term.id, "command": "echo hi-$((40+2))", "wait": 10 });
        let text = terminals.tool(&asked).await.unwrap();
        assert!(text.contains("hi-42"), "{text}");
        let read = terminals
            .tool(&json!({ "cwd": folder.path(), "action": "read", "id": term.id }))
            .await
            .unwrap();
        assert!(read.contains("hi-42"));
        let elsewhere = tempfile::tempdir().unwrap();
        assert!(
            terminals
                .tool(&json!({ "cwd": elsewhere.path(), "action": "read", "id": term.id }))
                .await
                .is_err()
        );

        let mut printed = String::new();
        while let Ok(Some(FromAgent::Event(event))) =
            tokio::time::timeout(Duration::from_millis(300), events.recv()).await
        {
            if let Event::Terminal { data, .. } = event {
                printed.push_str(&data);
            }
        }
        assert!(printed.contains("hi-42"));

        terminals.close(&term.id).unwrap();
        assert!(terminals.list(folder.path()).is_empty());
    }
}
