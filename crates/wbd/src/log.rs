//! What wbd did, written to a file: `wbd.log` among wbd's own files. A line for each call
//! from the page, each thing asked of dova, each command it ran, when it ended: how long it
//! took, or why it failed. The Log page shows the end of this file, and what runs now.
//!
//!   2026-11-20 13:15:58.123  page  pull.open pr=10  ~/storefront  248 ms
//!   2026-11-20 13:16:00.001  dova  api path="…"  ~/storefront  failed after 23 ms: TF401180: …
//!   2026-11-20 13:16:07.512  git   fetch origin main  ~/storefront  still running after 5 s
//!
//! A line that starts with spaces carries on the one before it. Something that never ends has
//! only its `still running` line. Some things are asked every few seconds, such as where a
//! repo stands. Those are quiet: one is written only if it fails or takes long.

use std::fmt::Display;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use protocol::LogRunning;
use serde_json::Value;

/// The file is started again when it is larger than this. The one before is kept beside it.
const MOST: u64 = 5 * 1024 * 1024;
/// How much of the file's end the Log page is given when it has read nothing yet.
const TAIL: u64 = 256 * 1024;
/// What is written of one thing is cut to this many characters.
const LONGEST: usize = 2000;
/// A value given to a call is cut to this many.
const VALUE: usize = 120;
/// A quiet thing that takes longer than this is written after all. The Log page lists what
/// has run for longer than this as running now.
const SLOW: Duration = Duration::from_secs(1);
/// What has run for this long is written as still running, once.
const STILL: Duration = Duration::from_secs(5);

/// Who did it.
#[derive(Clone, Copy)]
pub enum LogKind {
    /// A call from the page.
    Call,
    /// Something asked of dova, or something dova printed.
    Dova,
    /// A git command.
    Git,
    /// Another program: an agent, or one asked for its version.
    Run,
    /// wbd itself.
    Wbd,
}

impl LogKind {
    fn name(self) -> &'static str {
        match self {
            LogKind::Call => "page",
            LogKind::Dova => "dova",
            LogKind::Git => "git",
            LogKind::Run => "run",
            LogKind::Wbd => "wbd",
        }
    }
}

/// Something that started and has not ended.
struct Going {
    id: u64,
    since: Instant,
    kind: LogKind,
    what: String,
    place: Option<String>,
    quiet: bool,
    /// Its `still running` line is written.
    told: bool,
}

struct Log {
    /// None until `open`, and when the file can't be written: then nothing is written.
    file: Option<File>,
    path: PathBuf,
    /// How large the file is.
    size: u64,
    going: Vec<Going>,
    next: u64,
}

static LOG: Mutex<Log> = Mutex::new(Log::new());

/// A failure somewhere else must not take the log with it.
fn log() -> MutexGuard<'static, Log> {
    LOG.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Log {
    const fn new() -> Self {
        Log {
            file: None,
            path: PathBuf::new(),
            size: 0,
            going: Vec::new(),
            next: 1,
        }
    }

    /// Writes to `wbd.log` in `dir` from now on, after what is there.
    fn open(&mut self, dir: &Path) {
        let _ = std::fs::create_dir_all(dir);
        self.path = dir.join("wbd.log");
        self.size = std::fs::metadata(&self.path).map_or(0, |file| file.len());
        if self.size > MOST {
            self.start_again();
        } else {
            self.file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)
                .ok();
        }
    }

    /// Keeps the file as `wbd.before.log` and starts an empty one.
    fn start_again(&mut self) {
        self.file = None;
        let _ = std::fs::rename(&self.path, self.path.with_file_name("wbd.before.log"));
        self.file = File::create(&self.path).ok();
        self.size = 0;
    }

    /// Writes one thing. The lines of `text` after its first carry on, set in.
    fn line(&mut self, kind: LogKind, text: &str) {
        let mut lines = text.lines();
        let first = lines.next().unwrap_or_default();
        let mut written = format!("{}  {:<4}  {first}\n", stamp(), kind.name());
        for more in lines {
            written.push_str("    ");
            written.push_str(more);
            written.push('\n');
        }
        if self.size > MOST {
            self.start_again();
        }
        let Some(file) = &mut self.file else { return };
        if file.write_all(written.as_bytes()).is_ok() {
            self.size += written.len() as u64;
        }
    }

    fn begin(&mut self, kind: LogKind, what: &str, place: Option<&str>, quiet: bool) -> u64 {
        self.next += 1;
        self.going.push(Going {
            id: self.next - 1,
            since: Instant::now(),
            kind,
            what: cut(what, LONGEST),
            place: place.map(short),
            quiet,
            told: false,
        });
        self.next - 1
    }

    fn end(&mut self, id: u64, failed: Option<&str>) {
        let Some(at) = self.going.iter().position(|going| going.id == id) else {
            return;
        };
        let going = self.going.swap_remove(at);
        let took = going.since.elapsed();
        // Quick and fine, and asked every few seconds: nothing worth reading.
        if going.quiet && failed.is_none() && took < SLOW {
            return;
        }
        let outcome = match failed {
            Some(why) => format!("failed after {}: {}", span(took), cut(why, LONGEST)),
            None => span(took),
        };
        self.line(going.kind, &said(&going, &outcome));
    }

    /// Writes what has run for long as still running. So something that never ends is in the
    /// file, even if wbd ends before it does.
    fn tick(&mut self) {
        for at in 0..self.going.len() {
            let going = &mut self.going[at];
            if going.told || going.since.elapsed() < STILL {
                continue;
            }
            going.told = true;
            let outcome = format!("still running after {} s", STILL.as_secs());
            let (kind, text) = (going.kind, said(going, &outcome));
            self.line(kind, &text);
        }
    }

    /// What was written to the file after byte `after`, and where the file ends now. With
    /// nothing read yet, or with a file that was started again since, it is the file's end.
    fn read(&self, after: u64) -> (String, u64) {
        let Ok(mut file) = File::open(&self.path) else {
            return (String::new(), 0);
        };
        let len = file.metadata().map_or(0, |file| file.len());
        let carries_on = after > 0 && after <= len;
        let from = match carries_on {
            true => after,
            false => len.saturating_sub(TAIL),
        };
        let mut bytes = Vec::new();
        if file.seek(SeekFrom::Start(from)).is_err() || file.read_to_end(&mut bytes).is_err() {
            return (String::new(), after);
        }
        let end = from + bytes.len() as u64;
        // Starting in the middle of the file, what is before the first whole line is left out.
        if !carries_on && from > 0 {
            let line = bytes.iter().position(|byte| *byte == b'\n');
            bytes.drain(..line.map_or(bytes.len(), |at| at + 1));
        }
        (String::from_utf8_lossy(&bytes).into_owned(), end)
    }

    /// What has run for more than a moment, the longest first.
    fn running(&self) -> Vec<LogRunning> {
        let slow = self
            .going
            .iter()
            .filter(|going| going.since.elapsed() >= SLOW);
        let mut running: Vec<LogRunning> = slow
            .map(|going| LogRunning {
                who: going.kind.name().to_string(),
                what: going.what.clone(),
                place: going.place.clone(),
                ms: going.since.elapsed().as_millis() as u64,
            })
            .collect();
        running.sort_by_key(|one| std::cmp::Reverse(one.ms));
        running
    }
}

/// What ran, where, and how it went, as one thing to write.
fn said(going: &Going, outcome: &str) -> String {
    match &going.place {
        Some(place) => format!("{}  {place}  {outcome}", going.what),
        None => format!("{}  {outcome}", going.what),
    }
}

/// Starts the file in `dir`, and the clock that writes what is still running. The clock has
/// a thread of its own, so it also writes while wbd's own thread is held up.
pub fn open(dir: &Path) {
    log().open(dir);
    std::thread::spawn(|| {
        loop {
            std::thread::sleep(Duration::from_secs(1));
            log().tick();
        }
    });
}

/// Something that started and has not ended. Dropped without an end, it is written as given up on.
pub struct Running {
    id: u64,
    ended: bool,
}

/// Something started. Its line is written when it ends.
pub fn begin(kind: LogKind, what: &str, place: Option<&str>) -> Running {
    let id = log().begin(kind, what, place, false);
    Running { id, ended: false }
}

/// The same for something asked every few seconds: see the top of this file.
pub fn begin_quiet(kind: LogKind, what: &str, place: Option<&str>) -> Running {
    let id = log().begin(kind, what, place, true);
    Running { id, ended: false }
}

impl Running {
    pub fn done(mut self) {
        self.ended = true;
        log().end(self.id, None);
    }

    pub fn failed(mut self, why: &str) {
        self.ended = true;
        log().end(self.id, Some(why));
    }

    /// Ends it the way `result` went.
    pub fn end<T, E: Display>(self, result: &Result<T, E>) {
        match result {
            Ok(_) => self.done(),
            Err(err) => self.failed(&err.to_string()),
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if self.ended {
            return;
        }
        let why = match std::thread::panicking() {
            true => "wbd failed here. This is a bug in wbd.",
            false => "given up on before it ended",
        };
        log().end(self.id, Some(why));
    }
}

/// Writes one thing that is only said: nothing to wait for.
pub fn note(kind: LogKind, what: &str) {
    log().line(kind, &cut(what, LONGEST));
}

/// For the Log page: what was written after byte `after`, where the file ends now, what
/// runs now, and where the file is.
pub fn read(after: u64) -> (String, u64, Vec<LogRunning>, String) {
    let log = log();
    let (text, end) = log.read(after);
    (text, end, log.running(), short(&log.path.to_string_lossy()))
}

/// What a call was given, on one line: `pr=12 text="Fix the…"`. `place` names the value that
/// is the folder, which is written apart.
pub fn given(params: &Value, place: &str) -> String {
    let Some(fields) = params.as_object() else {
        return String::new();
    };
    let shown = fields
        .iter()
        .filter(|(name, value)| name.as_str() != place && !value.is_null());
    let words: Vec<String> = shown
        .map(|(name, value)| format!("{name}={}", cut(&value.to_string(), VALUE)))
        .collect();
    words.join(" ")
}

/// The time now where you are, to the millisecond.
fn stamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = now.as_secs() as libc::time_t;
    // SAFETY: `tm` is plain numbers, so all zeroes is one, and both pointers are to values
    // that live through the call.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&seconds, &mut tm);
        tm
    };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        now.subsec_millis()
    )
}

/// How long, as it is read: `248 ms`, `1.4 s`, `2 min 5 s`.
fn span(took: Duration) -> String {
    let ms = took.as_millis();
    match ms {
        0..1000 => format!("{ms} ms"),
        1000..60_000 => format!("{:.1} s", took.as_secs_f32()),
        _ => format!("{} min {} s", ms / 60_000, ms % 60_000 / 1000),
    }
}

/// A folder as the log writes it: your home folder is `~`.
fn short(path: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    match path.strip_prefix(&home) {
        Some(rest) if !home.is_empty() && rest.starts_with('/') => format!("~{rest}"),
        _ => path.to_string(),
    }
}

/// `text`, or its start and how much more there was.
fn cut(text: &str, most: usize) -> String {
    match text.char_indices().nth(most) {
        Some((at, _)) => format!("{}… ({} more)", &text[..at], text.len() - at),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A log of its own in a folder of its own, and what its file holds without the times.
    fn opened(dir: &Path) -> Log {
        let mut log = Log::new();
        log.open(dir);
        log
    }

    fn lines(log: &Log) -> Vec<String> {
        let text = std::fs::read_to_string(&log.path).unwrap();
        // A line that carries on has no time in front.
        let untimed = |line: &str| match line.starts_with(' ') {
            true => line.to_string(),
            false => line[25..].to_string(),
        };
        text.lines().map(untimed).collect()
    }

    #[test]
    fn a_thing_is_written_when_it_ends_with_how_it_went() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = opened(dir.path());
        let fine = log.begin(LogKind::Call, "pull.open pr=10", Some("/repo"), false);
        let broken = log.begin(LogKind::Git, "fetch origin main", Some("/repo"), false);
        assert!(lines(&log).is_empty());
        log.end(broken, Some("fatal: no such remote\nhint: add one"));
        log.end(fine, None);
        let written = lines(&log);
        // How long each took is not the same from one run to the next.
        assert!(written[0].starts_with("git   fetch origin main  /repo  failed after "));
        assert!(written[0].ends_with(" ms: fatal: no such remote"));
        assert_eq!(written[1], "    hint: add one");
        assert!(
            written[2].starts_with("page  pull.open pr=10  /repo  ") && written[2].ends_with(" ms")
        );
        assert!(log.going.is_empty());
    }

    #[test]
    fn a_quiet_thing_is_written_only_if_it_fails() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = opened(dir.path());
        let fine = log.begin(LogKind::Git, "status", None, true);
        log.end(fine, None);
        let broken = log.begin(LogKind::Git, "status", None, true);
        log.end(broken, Some("couldn't run git"));
        let written = lines(&log);
        assert_eq!(written.len(), 1);
        assert!(written[0].ends_with(" ms: couldn't run git"));
    }

    #[test]
    fn what_runs_long_is_written_once_as_still_running_and_listed() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = opened(dir.path());
        let id = log.begin(LogKind::Dova, "pr.view", Some("/repo"), false);
        log.tick();
        assert!(lines(&log).is_empty() && log.running().is_empty());
        // As if it started a while ago.
        log.going[0].since = Instant::now() - Duration::from_secs(6);
        log.tick();
        log.tick();
        assert_eq!(
            lines(&log),
            ["dova  pr.view  /repo  still running after 5 s"]
        );
        let running = log.running();
        assert_eq!(
            (running[0].who.as_str(), running[0].what.as_str()),
            ("dova", "pr.view")
        );
        log.end(id, None);
        assert!(log.running().is_empty());
        assert!(lines(&log)[1].starts_with("dova  pr.view  /repo  6."));
    }

    #[test]
    fn the_page_reads_what_is_new_since_it_last_read() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = opened(dir.path());
        log.line(LogKind::Wbd, "one");
        let (first, end) = log.read(0);
        assert!(first.ends_with("wbd   one\n"));
        assert_eq!(log.read(end), (String::new(), end));
        log.line(LogKind::Wbd, "two");
        let (second, after) = log.read(end);
        assert!(second.ends_with("wbd   two\n") && !second.contains("one"));
        // A file that was started again is read from where it starts.
        log.start_again();
        log.line(LogKind::Wbd, "three");
        let (third, _) = log.read(after);
        assert!(third.ends_with("wbd   three\n"));
        assert!(dir.path().join("wbd.before.log").exists());
    }

    #[test]
    fn what_a_call_was_given_is_short() {
        let params = json!({ "dir": "/repo", "pr": 12, "text": "x".repeat(500), "team": null });
        let given = given(&params, "dir");
        assert!(given.starts_with("pr=12 text=\"xxx"));
        assert!(given.ends_with("… (382 more)"));
        assert_eq!(cut("añb", 2), "añ… (1 more)");
        assert_eq!(span(Duration::from_millis(125_000)), "2 min 5 s");
    }
}
