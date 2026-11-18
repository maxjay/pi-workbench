//! Terminals: shells in pseudo-terminals, in a workspace's folder. What each
//! prints goes to the page as it comes, gathered up every [`FLUSH`], and the
//! last of it is kept so a terminal can be drawn again.

use std::ffi::CStr;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use protocol::{Event, Terminal};
use tokio::sync::mpsc::UnboundedSender;

use crate::error::Error;

/// How often what terminals printed goes to the page.
const FLUSH: Duration = Duration::from_millis(30);
/// How often each is checked for what runs in it.
const LOOK: Duration = Duration::from_millis(700);
/// How much of what a terminal printed is kept, in bytes.
const KEPT: usize = 256 * 1024;

#[derive(Clone)]
pub struct Terminals {
    all: Arc<Mutex<Vec<Arc<Term>>>>,
    next: Arc<AtomicU64>,
    /// wbd's one writer: lines for the page.
    out: UnboundedSender<String>,
}

struct Term {
    id: String,
    cwd: PathBuf,
    shell: String,
    master: File,
    pid: i32,
    /// What it printed lately, and what hasn't gone to the page yet.
    kept: Mutex<String>,
    pending: Mutex<String>,
    now: Mutex<Now>,
}

#[derive(Clone, PartialEq)]
struct Now {
    title: String,
    running: bool,
}

impl Terminals {
    pub fn new(out: UnboundedSender<String>) -> Self {
        Terminals {
            all: Arc::default(),
            next: Arc::new(AtomicU64::new(1)),
            out,
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

    /// Starts your shell in `cwd`.
    pub fn open(&self, cwd: &Path, cols: u16, rows: u16) -> Result<Terminal, Error> {
        if !cwd.is_dir() {
            return Err(Error::NotFound(format!("no folder at {}", cwd.display())));
        }
        let shell = std::env::var("SHELL")
            .ok()
            .filter(|shell| !shell.is_empty())
            .unwrap_or_else(|| "/bin/bash".into());
        let failed = |err| Error::io(Path::new("a terminal"), err);
        let (master, slave) = pty(cols, rows).map_err(failed)?;
        let mut command = std::process::Command::new(&shell);
        command
            .current_dir(cwd)
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .stdin(Stdio::from(slave.try_clone().map_err(failed)?))
            .stdout(Stdio::from(slave.try_clone().map_err(failed)?))
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
            .map_err(|err| Error::io(Path::new(&shell), err))?;
        drop(command);
        let reader = master.try_clone().map_err(failed)?;
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

    /// The last of what a terminal printed.
    pub fn read(&self, id: &str) -> Result<String, Error> {
        let term = self.get(id)?;
        let kept = term.kept.lock().expect("not poisoned").clone();
        Ok(kept)
    }

    pub fn input(&self, id: &str, data: &str) -> Result<(), Error> {
        let term = self.get(id)?;
        (&term.master)
            .write_all(data.as_bytes())
            .map_err(|err| Error::io(Path::new("a terminal"), err))
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> Result<(), Error> {
        let term = self.get(id)?;
        set_size(&term.master, cols, rows).map_err(|err| Error::io(Path::new("a terminal"), err))
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
        let _ = self.out.send(protocol::event(&Event::Terminals { list }));
    }

    fn flush(&self) {
        for term in self.terms() {
            let data = std::mem::take(&mut *term.pending.lock().expect("not poisoned"));
            if !data.is_empty() {
                let _ = self.out.send(protocol::event(&Event::Terminal {
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
}

impl Term {
    fn view(&self) -> Terminal {
        let now = self.now.lock().expect("not poisoned").clone();
        Terminal {
            id: self.id.clone(),
            cwd: self.cwd.display().to_string(),
            title: now.title,
            running: now.running,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_for_the_rest_of_a_character() {
        let euro = "€".as_bytes();
        assert_eq!(utf8_whole(b"ab"), 2);
        assert_eq!(utf8_whole(&[b"a".as_slice(), &euro[..2]].concat()), 1);
        assert_eq!(utf8_whole(&[0xff, b'a']), 2);
    }

    #[tokio::test]
    async fn runs_a_shell_and_hears_it() {
        let (out, mut lines) = tokio::sync::mpsc::unbounded_channel();
        let terminals = Terminals::new(out);
        terminals.start();
        let dir = tempfile::tempdir().unwrap();
        let term = terminals.open(dir.path(), 80, 24).unwrap();
        terminals.input(&term.id, "echo wb-$((40 + 2))\n").unwrap();

        // What it prints arrives as events, and is kept to read again.
        let heard = async {
            while let Some(line) = lines.recv().await {
                if line.contains("wb-42") {
                    return true;
                }
            }
            false
        };
        assert!(
            tokio::time::timeout(Duration::from_secs(10), heard)
                .await
                .unwrap()
        );
        assert!(terminals.read(&term.id).unwrap().contains("wb-42"));
        assert_eq!(terminals.list(dir.path()).len(), 1);

        terminals.close(&term.id).unwrap();
        assert!(terminals.list(dir.path()).is_empty());
        assert!(matches!(
            terminals.input(&term.id, "x"),
            Err(Error::NotFound(_))
        ));
    }
}
