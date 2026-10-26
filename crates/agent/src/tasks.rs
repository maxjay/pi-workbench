//! Tasks: prompts Pi runs on a schedule, each in a folder of its own. They
//! and their runs are kept in `tasks.json`. A tick now and then starts the
//! runs that are due: a run whose time passed while wb-agent wasn't
//! running, or while tasks were paused, runs once, late. A run still going
//! when the next is due means the next is skipped.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jiff::{Timestamp, Zoned};
use protocol::{
    Entry, Event, FromAgent, Run, RunState, Schedule, Task, TaskInput, TaskList, TaskView, Tone,
    ToolStatus, Trigger,
};
use serde::{Deserialize, Serialize};

use crate::config;
use crate::error::Error;
use crate::pi;
use crate::schedule::{self, LATE};
use crate::session::{Live, Out};
use crate::sessions::Sessions;
use crate::transcript::Transcript;

/// How often wb-agent looks for runs that are due.
const TICK: Duration = Duration::from_secs(20);
/// How often a run is checked on while it goes.
const WATCH: Duration = Duration::from_millis(500);
/// A run that hasn't started after this, with an error to show for it, failed.
const NO_START: Duration = Duration::from_secs(15);
/// Runs kept per task.
const KEPT: usize = 100;

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    paused: bool,
    tasks: Vec<Kept>,
}

/// A task as kept: what it is, its runs, and the bookkeeping for when it's due.
#[derive(Clone, Serialize, Deserialize)]
struct Kept {
    #[serde(flatten)]
    task: Task,
    /// Due times before this don't count: when it was made or its schedule
    /// changed, or just after the last one handled. Seconds since the epoch.
    since: i64,
    /// A continuing task's one session file.
    session: Option<String>,
    /// Newest first.
    runs: Vec<Run>,
}

#[derive(Clone)]
pub struct Tasks {
    path: PathBuf,
    folders: PathBuf,
    saved: Arc<Mutex<Saved>>,
    sessions: Sessions,
    out: Out,
}

fn now_ms() -> u64 {
    Timestamp::now().as_millisecond() as u64
}

impl Tasks {
    pub fn new(data_dir: &Path, sessions: Sessions, out: Out) -> Self {
        let path = data_dir.join("tasks.json");
        let mut saved = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<Saved>(&text).ok())
            .unwrap_or_default();
        // A run that was going when wb-agent last stopped can't be followed now.
        for run in saved.tasks.iter_mut().flat_map(|kept| &mut kept.runs) {
            if going(run) {
                run.state = RunState::Failed;
                run.error = Some("The app stopped while it ran.".into());
                run.finished = Some(now_ms());
                run.live = None;
            }
        }
        Tasks {
            path,
            folders: data_dir.join("tasks"),
            saved: Arc::new(Mutex::new(saved)),
            sessions,
            out,
        }
    }

    pub fn list(&self) -> TaskList {
        let saved = self.saved.lock().expect("not poisoned");
        let now = Zoned::now();
        TaskList {
            paused: saved.paused,
            tasks: saved
                .tasks
                .iter()
                .map(|kept| TaskView {
                    task: kept.task.clone(),
                    runs: kept.runs.clone(),
                    next: since(kept)
                        .and_then(|since| schedule::next_due(&kept.task.schedule, since, &now))
                        .map(|due| due.as_millisecond() as u64),
                })
                .collect(),
        }
    }

    /// Changes what's kept, saves it, and tells the workbench.
    fn change<R>(&self, edit: impl FnOnce(&mut Saved) -> R) -> R {
        let result = {
            let mut saved = self.saved.lock().expect("not poisoned");
            let result = edit(&mut saved);
            if let Ok(text) = serde_json::to_string_pretty(&*saved)
                && let Err(err) = config::write_file(&self.path, &text)
            {
                eprintln!("wb-agent: couldn't save the tasks: {err}");
            }
            result
        };
        let _ = self
            .out
            .send(FromAgent::Event(Event::Tasks { list: self.list() }));
        result
    }

    pub fn save(&self, input: TaskInput) -> Result<TaskList, Error> {
        validate(&input.schedule)?;
        let name = input.name.trim().to_string();
        if name.is_empty() || input.prompt.trim().is_empty() {
            return Err(Error::invalid(
                "the task",
                "it needs a name and something to do",
            ));
        }
        let now = Timestamp::now().as_second();
        let made = self.change(|saved| match &input.id {
            Some(id) => {
                let kept = saved
                    .tasks
                    .iter_mut()
                    .find(|kept| kept.task.id == *id)
                    .ok_or_else(|| Error::NotFound(format!("no task {id}")))?;
                // A new schedule counts from now, so changing the time
                // doesn't set off a run for a time that's already gone.
                if kept.task.schedule != input.schedule {
                    kept.since = now;
                }
                // One conversation stays in one folder.
                if kept.task.continuing != input.continuing
                    || kept.task.codespace != input.codespace
                {
                    kept.session = None;
                }
                kept.task.name = name.clone();
                kept.task.prompt = input.prompt.clone();
                kept.task.schedule = input.schedule.clone();
                kept.task.continuing = input.continuing;
                kept.task.codespace = input.codespace.clone();
                Ok(None)
            }
            None => {
                let taken: BTreeSet<&str> =
                    saved.tasks.iter().map(|k| k.task.id.as_str()).collect();
                let id = unique(&slug(&name), &taken);
                let folder = self.folders.join(&id);
                saved.tasks.push(Kept {
                    task: Task {
                        id: id.clone(),
                        name: name.clone(),
                        prompt: input.prompt.clone(),
                        schedule: input.schedule.clone(),
                        continuing: input.continuing,
                        paused: false,
                        folder: folder.display().to_string(),
                        codespace: input.codespace.clone(),
                    },
                    since: now,
                    session: None,
                    runs: Vec::new(),
                });
                Ok(Some(folder))
            }
        })?;
        if let Some(folder) = made {
            std::fs::create_dir_all(&folder).map_err(|err| Error::io(&folder, err))?;
        }
        Ok(self.list())
    }

    pub fn set_paused(&self, id: &str, paused: bool) -> Result<TaskList, Error> {
        self.change(|saved| {
            let kept = saved
                .tasks
                .iter_mut()
                .find(|kept| kept.task.id == id)
                .ok_or_else(|| Error::NotFound(format!("no task {id}")))?;
            kept.task.paused = paused;
            Ok::<_, Error>(())
        })?;
        Ok(self.list())
    }

    pub async fn pause_all(&self, paused: bool) -> TaskList {
        self.change(|saved| saved.paused = paused);
        if !paused {
            self.tick().await;
        }
        self.list()
    }

    /// Starts every run that's due.
    pub async fn tick(&self) {
        let now = Zoned::now();
        let any_due = {
            let saved = self.saved.lock().expect("not poisoned");
            !saved.paused
                && saved.tasks.iter().any(|kept| {
                    !kept.task.paused
                        && since(kept).is_some_and(|since| {
                            schedule::last_due(&kept.task.schedule, since, &now).is_some()
                        })
                })
        };
        // Nothing to save or say on a quiet tick.
        if !any_due {
            return;
        }
        let due: Vec<(String, Trigger)> = self.change(|saved| {
            let mut due = Vec::new();
            for kept in &mut saved.tasks {
                if kept.task.paused {
                    continue;
                }
                let since =
                    jiff::Timestamp::from_second(kept.since).unwrap_or(Timestamp::UNIX_EPOCH);
                let Some(at) = schedule::last_due(&kept.task.schedule, since, &now) else {
                    continue;
                };
                // Handled, whether it runs or is skipped for one still going.
                kept.since = at.as_second() + 1;
                if kept.runs.first().is_some_and(going) {
                    continue;
                }
                let late = now.timestamp().duration_since(at) > LATE;
                due.push((
                    kept.task.id.clone(),
                    if late {
                        Trigger::Late
                    } else {
                        Trigger::Scheduled
                    },
                ));
            }
            due
        });
        for (id, trigger) in due {
            if let Err(err) = self.run(&id, trigger).await {
                eprintln!("wb-agent: task {id} didn't start: {err}");
            }
        }
    }

    /// Looks for due runs every [`TICK`], from now on.
    pub fn start(&self) {
        let tasks = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(TICK);
            loop {
                tick.tick().await;
                tasks.tick().await;
            }
        });
    }

    /// Runs a task: in its one session if it continues one, or a new one.
    pub async fn run(&self, id: &str, trigger: Trigger) -> Result<TaskList, Error> {
        let (task, file, busy) = {
            let saved = self.saved.lock().expect("not poisoned");
            let kept = saved
                .tasks
                .iter()
                .find(|kept| kept.task.id == id)
                .ok_or_else(|| Error::NotFound(format!("no task {id}")))?;
            (
                kept.task.clone(),
                kept.session.clone(),
                kept.runs.first().is_some_and(going),
            )
        };
        if busy {
            return Err(Error::Pi(format!("{} is already running", task.name)));
        }
        let session = match (task.continuing, &file) {
            (true, Some(file)) if Path::new(file).exists() => self.sessions.open(file).await?,
            _ => {
                let cwd = task
                    .codespace
                    .clone()
                    .unwrap_or_else(|| task.folder.clone());
                self.sessions.start(Some(cwd), None).await?
            }
        };
        let live = self.sessions.get(&session.id)?;
        let run = Run {
            id: format!("r{}", now_ms()),
            started: now_ms(),
            finished: None,
            trigger,
            state: RunState::Running,
            summary: String::new(),
            error: None,
            files: Vec::new(),
            session: session.state.file.clone(),
            live: Some(session.id.clone()),
            from: session.entries.len() as u32,
        };
        let run_id = run.id.clone();
        self.change(|saved| {
            if let Some(kept) = saved.tasks.iter_mut().find(|kept| kept.task.id == id) {
                kept.runs.insert(0, run);
                kept.runs.truncate(KEPT);
            }
        });
        live.edit(|t| t.sent(&task.prompt));
        if let Err(err) = live.send(pi::prompt(&task.prompt)).await {
            self.finish(id, &run_id, |run| {
                run.state = RunState::Failed;
                run.error = Some(err.to_string());
            });
            return Err(err);
        }
        let tasks = self.clone();
        let id = id.to_string();
        tokio::spawn(async move { tasks.watch(&id, &run_id, live).await });
        Ok(self.list())
    }

    /// Stops a task's run.
    pub async fn stop(&self, id: &str) -> Result<TaskList, Error> {
        let live = {
            let saved = self.saved.lock().expect("not poisoned");
            saved
                .tasks
                .iter()
                .find(|kept| kept.task.id == id)
                .and_then(|kept| kept.runs.first())
                .filter(|run| going(run))
                .and_then(|run| run.live.clone())
        };
        if let Some(live) = live {
            self.sessions.get(&live)?.send(pi::command("abort")).await?;
        }
        Ok(self.list())
    }

    /// Follows a run until it ends, keeping its state, as Pi works through it.
    async fn watch(&self, id: &str, run_id: &str, live: Arc<Live>) {
        let begun = std::time::Instant::now();
        let mut started = false;
        let mut asking = false;
        loop {
            tokio::time::sleep(WATCH).await;
            let now = live.look(|t| {
                (
                    t.state.busy,
                    t.state.ask.is_some(),
                    t.state.stopped.clone(),
                    t.state.file.clone(),
                    t.state.queued.is_empty(),
                )
            });
            let (busy, asked, stopped, file, settled) = now;
            started |= busy;
            if asked != asking {
                asking = asked;
                self.update(id, run_id, |run| {
                    run.state = if asked {
                        RunState::Asking
                    } else {
                        RunState::Running
                    };
                    run.session = run.session.clone().or(file.clone());
                });
            }
            if let Some(reason) = stopped {
                self.finish(id, run_id, |run| {
                    run.state = RunState::Failed;
                    run.error = Some(reason);
                });
                return;
            }
            let ended = started && !busy && !asked && settled;
            let never = !started && begun.elapsed() > NO_START;
            if ended || never {
                let from = self.from(id, run_id);
                let outcome = live.look(|t| outcome(t, from));
                self.finish(id, run_id, |run| {
                    run.state = outcome.state;
                    run.summary = outcome.summary;
                    run.error = outcome.error;
                    run.files = outcome.files;
                    run.session = run.session.clone().or(file);
                });
                // A continuing task keeps its session for the next run.
                let file = live.file();
                self.change(|saved| {
                    if let Some(kept) = saved.tasks.iter_mut().find(|kept| kept.task.id == id)
                        && kept.task.continuing
                    {
                        kept.session = file;
                    }
                });
                return;
            }
        }
    }

    fn from(&self, id: &str, run_id: &str) -> usize {
        let saved = self.saved.lock().expect("not poisoned");
        saved
            .tasks
            .iter()
            .find(|kept| kept.task.id == id)
            .and_then(|kept| kept.runs.iter().find(|run| run.id == run_id))
            .map_or(0, |run| run.from as usize)
    }

    fn update(&self, id: &str, run_id: &str, edit: impl FnOnce(&mut Run)) {
        self.change(|saved| {
            if let Some(run) = saved
                .tasks
                .iter_mut()
                .find(|kept| kept.task.id == id)
                .and_then(|kept| kept.runs.iter_mut().find(|run| run.id == run_id))
            {
                edit(run);
            }
        });
    }

    fn finish(&self, id: &str, run_id: &str, edit: impl FnOnce(&mut Run)) {
        self.update(id, run_id, |run| {
            edit(run);
            run.finished = Some(now_ms());
            run.live = None;
        });
        // Windows notifications for a run that failed or needs you come
        // later; for now the workbench highlights it.
    }

    /// A schedule from words, as Pi reads them.
    pub async fn schedule_from(&self, words: &str, dir: &Path) -> Result<Schedule, Error> {
        let prompt = format!(
            "Turn this description of when a task should run into JSON, and reply with the JSON alone. \
             It's one of {{\"kind\":\"at\",\"days\":[0,1,2,3,4],\"time\":\"08:00\"}} (days from 0 for Monday \
             to 6 for Sunday, a 24-hour local time), {{\"kind\":\"every\",\"hours\":2}}, or {{\"kind\":\"manual\"}}.\n\n\
             The description: {words}"
        );
        std::fs::create_dir_all(dir).map_err(|err| Error::io(dir, err))?;
        let run = tokio::process::Command::new(self.sessions.pi())
            .args(["-p", "--no-session", &prompt])
            .current_dir(dir)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output();
        let output = tokio::time::timeout(Duration::from_secs(90), run)
            .await
            .map_err(|_| Error::Pi("Pi took too long to answer".into()))?
            .map_err(|err| Error::Pi(format!("couldn't run Pi: {err}")))?;
        let text = String::from_utf8_lossy(&output.stdout);
        schedule_in(&text).ok_or_else(|| {
            Error::Pi(format!(
                "Pi didn't answer with a schedule: {}",
                text.trim().lines().next().unwrap_or("nothing")
            ))
        })
    }
}

/// When a task's due times start counting.
fn since(kept: &Kept) -> Option<Timestamp> {
    Timestamp::from_second(kept.since).ok()
}

fn going(run: &Run) -> bool {
    matches!(run.state, RunState::Running | RunState::Asking)
}

fn validate(schedule: &Schedule) -> Result<(), Error> {
    match schedule {
        Schedule::At { days, time } => {
            let ok_time = time
                .split_once(':')
                .and_then(|(h, m)| Some((h.parse::<u8>().ok()?, m.parse::<u8>().ok()?)))
                .is_some_and(|(h, m)| h < 24 && m < 60);
            if days.is_empty() || days.iter().any(|day| *day > 6) || !ok_time {
                return Err(Error::invalid("the schedule", "it needs days and a time"));
            }
        }
        Schedule::Every { hours } if !(1..=24).contains(hours) => {
            return Err(Error::invalid("the schedule", "every 1 to 24 hours"));
        }
        _ => {}
    }
    Ok(())
}

/// The schedule in Pi's answer: its first JSON object that is one.
fn schedule_in(text: &str) -> Option<Schedule> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    let schedule: Schedule = serde_json::from_str(text.get(start..=end)?).ok()?;
    validate(&schedule).ok()?;
    Some(schedule)
}

/// A line of markdown as plain text, for a summary: no heading marks,
/// emphasis or code ticks.
fn plain(line: &str) -> String {
    line.trim_start_matches('#')
        .trim()
        .replace("**", "")
        .replace('`', "")
}

/// `Read my emails` → `read-my-emails`.
fn slug(name: &str) -> String {
    let words: Vec<String> = name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect();
    let slug = words.join("-");
    let slug: String = slug.chars().take(40).collect();
    if slug.is_empty() { "task".into() } else { slug }
}

fn unique(base: &str, taken: &BTreeSet<&str>) -> String {
    if !taken.contains(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !taken.contains(id.as_str()))
        .expect("some number is free")
}

struct Outcome {
    state: RunState,
    summary: String,
    error: Option<String>,
    files: Vec<String>,
}

/// How a run ended, from the conversation since it started.
fn outcome(t: &Transcript, from: usize) -> Outcome {
    let run = t.entries.get(from..).unwrap_or_default();
    let first_line = |text: &str| {
        let line = text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("");
        crate::transcript::shorten(&plain(line))
    };
    let mut files: Vec<String> = Vec::new();
    for entry in run {
        if let Entry::Tool(tool) = entry
            && tool.status == ToolStatus::Done
            && matches!(tool.name.as_str(), "write" | "edit")
            && let Some(path) = tool.args["path"].as_str()
        {
            let path = Path::new(&t.state.cwd).join(path).display().to_string();
            files.retain(|known| *known != path);
            files.push(path);
        }
    }
    let error = run.iter().rev().find_map(|entry| match entry {
        Entry::Notice {
            text,
            tone: Tone::Error,
        } => Some(text.clone()),
        _ => None,
    });
    let stopped = run
        .iter()
        .any(|entry| matches!(entry, Entry::Notice { text, .. } if text == "Stopped"));
    let reply = run.iter().rev().find_map(|entry| match entry {
        Entry::Text { text } if !text.trim().is_empty() => Some(text.as_str()),
        _ => None,
    });
    let ended_badly = matches!(
        run.last(),
        Some(Entry::Notice {
            tone: Tone::Error,
            ..
        })
    );
    let state = if stopped {
        RunState::Stopped
    } else if ended_badly || (reply.is_none() && error.is_some()) {
        RunState::Failed
    } else {
        RunState::Done
    };
    let summary = match state {
        RunState::Failed => first_line(error.as_deref().unwrap_or("It failed")),
        RunState::Stopped => "Stopped by you".into(),
        _ => reply.map(first_line).unwrap_or_else(|| "Done".into()),
    };
    Outcome {
        error: if state == RunState::Failed {
            error
        } else {
            None
        },
        state,
        summary,
        files,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pi::decode;

    #[test]
    fn slugs_names() {
        assert_eq!(slug("Read my emails!"), "read-my-emails");
        assert_eq!(slug("   "), "task");
        assert_eq!(
            plain("## I wrote **plan.md** and `x`"),
            "I wrote plan.md and x"
        );
        let taken = BTreeSet::from(["read-my-emails"]);
        assert_eq!(unique("read-my-emails", &taken), "read-my-emails-2");
    }

    #[test]
    fn reads_a_schedule_from_pis_answer() {
        let answer = "Here you go:\n{\"kind\":\"at\",\"days\":[0,1,2,3,4],\"time\":\"07:30\"}\n";
        assert_eq!(
            schedule_in(answer),
            Some(Schedule::At {
                days: vec![0, 1, 2, 3, 4],
                time: "07:30".into()
            })
        );
        assert_eq!(
            schedule_in("{\"kind\":\"at\",\"days\":[9],\"time\":\"07:30\"}"),
            None
        );
        assert_eq!(schedule_in("no idea"), None);
    }

    #[test]
    fn a_runs_outcome_comes_from_its_part_of_the_conversation() {
        let mut t = Transcript::new(Path::new("/tasks/mail"));
        for line in [
            r#"{"type":"message_start","message":{"role":"user","content":"an earlier run"}}"#,
            r#"{"type":"message_end","message":{"role":"assistant","stopReason":"error","errorMessage":"503","content":[]}}"#,
            r#"{"type":"message_start","message":{"role":"user","content":"read my email"}}"#,
            r#"{"type":"message_end","message":{"role":"assistant","stopReason":"toolUse","content":[{"type":"toolCall","id":"c1","name":"write","arguments":{"path":"notes/today.md","content":"x"}}]}}"#,
            r#"{"type":"tool_execution_start","toolCallId":"c1","toolName":"write","args":{"path":"notes/today.md","content":"x"}}"#,
            r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"write","isError":false,"result":{"content":[]}}"#,
            r#"{"type":"message_end","message":{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"\n4 new emails. Two need a reply.\nMore."}]}}"#,
        ] {
            t.apply(decode(line).unwrap());
        }
        let done = outcome(&t, 2);
        assert_eq!(done.state, RunState::Done);
        assert_eq!(done.summary, "4 new emails. Two need a reply.");
        assert_eq!(done.files, ["/tasks/mail/notes/today.md"]);
        let failed = outcome(&t, 0);
        assert_eq!(
            failed.state,
            RunState::Done,
            "a later reply outweighs an earlier error"
        );
        let only_error = outcome(&t, 1);
        assert_eq!(only_error.state, RunState::Done);
        let mut broken = Transcript::new(Path::new("/tasks/mail"));
        broken.apply(decode(r#"{"type":"message_end","message":{"role":"assistant","stopReason":"error","errorMessage":"Outlook answered 503","content":[]}}"#).unwrap());
        let failed = outcome(&broken, 0);
        assert_eq!(failed.state, RunState::Failed);
        assert_eq!(failed.error.as_deref(), Some("Outlook answered 503"));
    }

    #[test]
    fn rejects_schedules_that_cant_run() {
        assert!(
            validate(&Schedule::At {
                days: vec![],
                time: "08:00".into()
            })
            .is_err()
        );
        assert!(
            validate(&Schedule::At {
                days: vec![0],
                time: "25:00".into()
            })
            .is_err()
        );
        assert!(validate(&Schedule::Every { hours: 0 }).is_err());
        assert!(validate(&Schedule::Manual).is_ok());
    }
}
