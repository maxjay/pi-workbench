//! What wbd did, for the Log page: each call from the page, each thing asked of dova, each
//! command it ran, with how long it took and why it failed. The latest are kept, in memory.
//!
//! Some things are asked every few seconds, such as where a repo stands. Those are quiet:
//! one is written only if it fails, takes long, or is still running when the log is read.

use std::collections::VecDeque;
use std::fmt::Display;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use protocol::{LogEntry, LogKind};
use serde_json::Value;

/// How many are kept. The oldest go first.
const KEPT: usize = 2000;
/// What is written of one thing is cut to this many characters.
const LONGEST: usize = 2000;
/// A value given to a call is cut to this many.
const VALUE: usize = 120;
/// A quiet thing that takes longer than this is written after all.
const SLOW: Duration = Duration::from_secs(1);

struct Log {
    /// Each with the `rev` it last changed at.
    entries: VecDeque<(u64, LogEntry)>,
    /// The quiet things that run and are not written yet.
    unwritten: Vec<(Instant, LogEntry)>,
    next: u64,
    rev: u64,
}

static LOG: Mutex<Log> = Mutex::new(Log {
    entries: VecDeque::new(),
    unwritten: Vec::new(),
    next: 1,
    rev: 0,
});

/// A failure somewhere else must not take the log with it.
fn log() -> MutexGuard<'static, Log> {
    LOG.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Log {
    /// Puts an entry in where it started, with a new rev: the page takes it as new or changed.
    fn write(&mut self, entry: LogEntry) {
        self.rev += 1;
        let after = self.entries.partition_point(|(_, e)| e.id < entry.id);
        self.entries.insert(after, (self.rev, entry));
        if self.entries.len() > KEPT {
            self.entries.pop_front();
        }
    }

    fn entry(&mut self, kind: LogKind, what: &str, place: Option<&str>) -> LogEntry {
        self.next += 1;
        LogEntry {
            id: self.next - 1,
            at: now(),
            kind,
            what: cut(what, LONGEST),
            place: place.map(short),
            running: true,
            took: None,
            failed: None,
        }
    }
}

/// wbd's clock, in milliseconds since 1970.
pub fn now() -> u64 {
    let since = SystemTime::now().duration_since(UNIX_EPOCH);
    since.map_or(0, |since| since.as_millis() as u64)
}

/// Something that started and has not ended. Dropped without an end, it is written as given up on.
pub struct Running {
    id: u64,
    since: Instant,
    ended: bool,
}

/// Writes that something started.
pub fn begin(kind: LogKind, what: &str, place: Option<&str>) -> Running {
    let mut log = log();
    let entry = log.entry(kind, what, place);
    let id = entry.id;
    log.write(entry);
    started(id)
}

/// The same for something asked every few seconds: see the top of this file.
pub fn begin_quiet(kind: LogKind, what: &str, place: Option<&str>) -> Running {
    let mut log = log();
    let entry = log.entry(kind, what, place);
    let id = entry.id;
    log.unwritten.push((Instant::now(), entry));
    started(id)
}

fn started(id: u64) -> Running {
    Running {
        id,
        since: Instant::now(),
        ended: false,
    }
}

impl Running {
    pub fn done(mut self) {
        self.finish(None);
    }

    pub fn failed(mut self, why: &str) {
        self.finish(Some(why));
    }

    /// Ends it the way `result` went.
    pub fn end<T, E: Display>(self, result: &Result<T, E>) {
        match result {
            Ok(_) => self.done(),
            Err(err) => self.failed(&err.to_string()),
        }
    }

    fn finish(&mut self, failed: Option<&str>) {
        self.ended = true;
        let took = self.since.elapsed();
        let mut log = log();
        let unwritten = log.unwritten.iter().position(|(_, e)| e.id == self.id);
        let mut entry = match unwritten {
            Some(at) => {
                let (_, entry) = log.unwritten.swap_remove(at);
                // Quick and fine: nothing worth reading.
                if failed.is_none() && took < SLOW {
                    return;
                }
                entry
            }
            None => {
                let written = log.entries.iter().rposition(|(_, e)| e.id == self.id);
                // It ran for so long that it is no longer kept.
                let Some((_, entry)) = written.and_then(|at| log.entries.remove(at)) else {
                    return;
                };
                entry
            }
        };
        entry.running = false;
        entry.took = Some(took.as_millis() as u64);
        entry.failed = failed.map(|why| cut(why, LONGEST));
        log.write(entry);
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
        self.finish(Some(why));
    }
}

/// Writes one thing that is only said: nothing to wait for.
pub fn note(kind: LogKind, what: &str) {
    let mut log = log();
    let mut entry = log.entry(kind, what, None);
    entry.running = false;
    log.write(entry);
}

/// The entries that are new or changed since `after`, the oldest first, and the rev now.
pub fn read(after: u64) -> (Vec<LogEntry>, u64) {
    let mut log = log();
    // A quiet thing that still runs is written now.
    let mut at = 0;
    while at < log.unwritten.len() {
        if log.unwritten[at].0.elapsed() < SLOW {
            at += 1;
            continue;
        }
        let (_, entry) = log.unwritten.swap_remove(at);
        log.write(entry);
    }
    let changed = log.entries.iter().filter(|(rev, _)| *rev > after);
    (changed.map(|(_, entry)| entry.clone()).collect(), log.rev)
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

    /// The log is one for the whole process, so each test looks only at its own entries.
    fn mine(what: &str) -> Vec<LogEntry> {
        let all = read(0).0.into_iter();
        all.filter(|entry| entry.what.starts_with(what)).collect()
    }

    #[test]
    fn what_ran_is_written_when_it_starts_and_again_when_it_ends() {
        let running = begin(LogKind::Git, "test-one fetch", Some("/repo"));
        let (before, rev) = (mine("test-one"), read(0).1);
        assert!(before[0].running && before[0].took.is_none());
        running.failed("no such remote");
        let after = mine("test-one");
        assert_eq!(after.len(), 1);
        assert!(!after[0].running && after[0].took.is_some());
        assert_eq!(after[0].failed.as_deref(), Some("no such remote"));
        // Asked for what changed since, the ended one comes again.
        assert!(read(rev).0.iter().any(|entry| entry.id == after[0].id));
    }

    #[test]
    fn something_dropped_is_given_up_on() {
        drop(begin(LogKind::Call, "test-two", None));
        let failed = mine("test-two")[0].failed.clone();
        assert_eq!(failed.as_deref(), Some("given up on before it ended"));
    }

    #[test]
    fn a_quiet_thing_is_written_only_if_it_fails() {
        begin_quiet(LogKind::Git, "test-three fine", None).done();
        begin_quiet(LogKind::Git, "test-three broken", None).failed("not a repo");
        let written = mine("test-three");
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].what, "test-three broken");
    }

    #[test]
    fn what_a_call_was_given_is_short() {
        let params = json!({ "dir": "/repo", "pr": 12, "text": "x".repeat(500), "team": null });
        let given = given(&params, "dir");
        assert!(given.starts_with("pr=12 text=\"xxx"));
        assert!(given.ends_with("… (382 more)"));
        assert_eq!(cut("añb", 2), "añ… (1 more)");
    }
}
