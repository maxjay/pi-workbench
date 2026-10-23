//! Scheduled tasks: a prompt Pi runs in a folder on a schedule, and the
//! sessions each run left in Pi's store, kept in one file for the app.
//!
//! A schedule is kept as `manual`, `every 30m`, `every 2h`, `daily 09:00`,
//! `weekdays 09:00` or `mon,thu 18:30`, in local wall-clock time. People
//! set it with controls, or describe it and have Pi fill those in, and
//! check what Pi chose before it is kept.
use anyhow::{Context, Result, bail, ensure};
pub use jiff::{Timestamp, tz::TimeZone};
use jiff::{ToSpan, civil::Date};
use serde::{Deserialize, Serialize};
use std::{
    fmt, fs,
    io::Write,
    path::{Path, PathBuf},
};

/// Runs kept per task; older ones drop off the list, their sessions stay in
/// Pi's store.
pub const KEPT_RUNS: usize = 30;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Schedule {
    /// Only when asked: Run now.
    Manual,
    Every {
        minutes: u32,
    },
    /// At a time of day, on the given days (Monday first); every day if all.
    At {
        days: [bool; 7],
        hour: i8,
        minute: i8,
    },
}

const DAY_NAMES: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
const WEEKDAYS: [bool; 7] = [true, true, true, true, true, false, false];
const WEEKENDS: [bool; 7] = [false, false, false, false, false, true, true];

/// A time of day as `HH:MM`, 24-hour.
pub fn parse_time(text: &str) -> Result<(i8, i8)> {
    let text = text.trim();
    let (hour, minute) = text
        .split_once(':')
        .with_context(|| format!("\u{201c}{text}\u{201d} is not a time like 09:00"))?;
    let hour: i8 = hour
        .parse()
        .ok()
        .filter(|h| (0..24).contains(h))
        .with_context(|| format!("\u{201c}{text}\u{201d} is not a time like 09:00"))?;
    let minute: i8 = minute
        .parse()
        .ok()
        .filter(|m| (0..60).contains(m))
        .with_context(|| format!("\u{201c}{text}\u{201d} is not a time like 09:00"))?;
    Ok((hour, minute))
}

impl Schedule {
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim().to_lowercase();
        let words: Vec<&str> = text
            .split_whitespace()
            .filter(|word| *word != "at")
            .collect();
        match words.as_slice() {
            [] | ["manual"] => Ok(Self::Manual),
            ["every", amount @ ..] if !amount.is_empty() && amount.len() <= 2 => {
                let amount = amount.concat();
                let (number, unit) = amount.split_at(
                    amount
                        .find(|c: char| !c.is_ascii_digit())
                        .unwrap_or(amount.len()),
                );
                let number: u32 = number
                    .parse()
                    .ok()
                    .filter(|n| *n > 0)
                    .context("Say how often, like every 30m or every 2h")?;
                let minutes = match unit {
                    "m" | "min" | "mins" | "minutes" => number,
                    "h" | "hour" | "hours" => number.saturating_mul(60),
                    _ => bail!("Say how often, like every 30m or every 2h"),
                };
                ensure!(
                    minutes <= 7 * 24 * 60,
                    "At most once a week with every; use a day and time"
                );
                Ok(Self::Every { minutes })
            }
            [days, time] => {
                let (hour, minute) = parse_time(time)?;
                let days = match *days {
                    "daily" => [true; 7],
                    "weekdays" => WEEKDAYS,
                    "weekends" => WEEKENDS,
                    list => {
                        let mut days = [false; 7];
                        for name in list.split(',').filter(|n| !n.is_empty()) {
                            let index = DAY_NAMES
                                .iter()
                                .position(|day| name.starts_with(day))
                                .with_context(|| format!("\u{201c}{name}\u{201d} is not a day"))?;
                            days[index] = true;
                        }
                        ensure!(days.contains(&true), "Name a day, like mon or weekdays");
                        days
                    }
                };
                Ok(Self::At { days, hour, minute })
            }
            _ => bail!("Try manual, every 30m, daily 09:00, weekdays 09:00 or mon,thu 18:30"),
        }
    }

    /// Every so many minutes or hours, as set with the controls.
    pub fn every(amount: &str, hours: bool) -> Result<Self> {
        let amount: u32 = amount
            .trim()
            .parse()
            .ok()
            .filter(|n| *n > 0)
            .context("Say how often, as a whole number")?;
        let minutes = if hours {
            amount.saturating_mul(60)
        } else {
            amount
        };
        ensure!(
            minutes <= 7 * 24 * 60,
            "At most once a week; pick days and a time instead"
        );
        Ok(Self::Every { minutes })
    }

    /// At a time on chosen days, as set with the controls.
    pub fn at(days: [bool; 7], time: &str) -> Result<Self> {
        ensure!(days.contains(&true), "Pick at least one day");
        let (hour, minute) = parse_time(time)?;
        Ok(Self::At { days, hour, minute })
    }

    /// Reads a schedule out of Pi's answer to `describe_prompt`: the first
    /// line that is one, quotes and code marks aside. Pi's answer is only
    /// ever used through this, so a schedule is always one that parses.
    pub fn from_answer(answer: &str) -> Option<Self> {
        answer
            .lines()
            .map(|line| line.trim().trim_matches(['`', '"', '\'', '.', ' ']))
            .filter(|line| !line.is_empty())
            .find_map(|line| Self::parse(line).ok())
    }

    /// In words: `Every weekday at 08:30`, `Mon and Thu at 18:30`,
    /// `Every 2 hours`, `Only when run by hand`.
    pub fn describe(&self) -> String {
        match self {
            Self::Manual => "Only when run by hand".into(),
            Self::Every { minutes: 1 } => "Every minute".into(),
            Self::Every { minutes: 60 } => "Every hour".into(),
            Self::Every { minutes } if minutes % 60 == 0 => format!("Every {} hours", minutes / 60),
            Self::Every { minutes } => format!("Every {minutes} minutes"),
            Self::At { days, hour, minute } => {
                const NAMES: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
                let days = match *days {
                    [true, true, true, true, true, true, true] => "Every day".to_string(),
                    WEEKDAYS => "Every weekday".to_string(),
                    WEEKENDS => "Weekends".to_string(),
                    _ => {
                        let names: Vec<&str> = NAMES
                            .iter()
                            .zip(days)
                            .filter(|(_, on)| **on)
                            .map(|(name, _)| *name)
                            .collect();
                        match names.as_slice() {
                            [one] => format!("Every {one}"),
                            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
                            [] => String::new(),
                        }
                    }
                };
                format!("{days} at {hour:02}:{minute:02}")
            }
        }
    }

    /// The first time after `after` it is due; `None` for manual tasks.
    pub fn next_after(&self, after: Timestamp, tz: &TimeZone) -> Option<Timestamp> {
        match *self {
            Self::Manual => None,
            Self::Every { minutes } => after.checked_add(i64::from(minutes).minutes()).ok(),
            Self::At { days, hour, minute } => {
                let start: Date = after.to_zoned(tz.clone()).date();
                (0..=8).find_map(|offset| {
                    let date = start.checked_add(offset.days()).ok()?;
                    let weekday = date.weekday().to_monday_zero_offset() as usize;
                    if !days[weekday] {
                        return None;
                    }
                    let at = date
                        .at(hour, minute, 0, 0)
                        .to_zoned(tz.clone())
                        .ok()?
                        .timestamp();
                    (at > after).then_some(at)
                })
            }
        }
    }
}

impl fmt::Display for Schedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manual => f.write_str("manual"),
            Self::Every { minutes } if minutes % 60 == 0 => write!(f, "every {}h", minutes / 60),
            Self::Every { minutes } => write!(f, "every {minutes}m"),
            Self::At { days, hour, minute } => {
                let days = match days {
                    [true, true, true, true, true, true, true] => "daily".to_string(),
                    [true, true, true, true, true, false, false] => "weekdays".to_string(),
                    [false, false, false, false, false, true, true] => "weekends".to_string(),
                    _ => DAY_NAMES
                        .iter()
                        .zip(days)
                        .filter(|(_, on)| **on)
                        .map(|(name, _)| *name)
                        .collect::<Vec<_>>()
                        .join(","),
                };
                write!(f, "{days} {hour:02}:{minute:02}")
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    #[default]
    Running,
    Done,
    /// Stopped from the workbench.
    Stopped,
    /// Pi reported an error, or could not be started.
    Failed,
    /// The workbench closed while it ran.
    Interrupted,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Run {
    pub id: u64,
    /// Seconds since the Unix epoch.
    pub started: i64,
    /// Its session in Pi's store, once Pi has said where.
    pub session: Option<PathBuf>,
    pub outcome: Outcome,
    /// Why it failed, when it never got a session.
    pub error: Option<String>,
    /// Looked at since it finished.
    pub seen: bool,
}

impl Run {
    pub fn finished(&self) -> bool {
        self.outcome != Outcome::Running
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Task {
    pub id: u64,
    pub name: String,
    pub folder: PathBuf,
    pub prompt: String,
    /// As written; see `Schedule::parse`.
    pub schedule: String,
    pub paused: bool,
    /// When scheduling last settled: made, edited, resumed or run. The
    /// next run is the schedule's first time after it.
    pub anchor: i64,
    /// Newest last.
    pub runs: Vec<Run>,
}

impl Task {
    pub fn schedule(&self) -> Option<Schedule> {
        Schedule::parse(&self.schedule).ok()
    }

    /// When it runs next, if it is scheduled and not paused.
    pub fn next_run(&self, tz: &TimeZone) -> Option<Timestamp> {
        if self.paused {
            return None;
        }
        let anchor = Timestamp::from_second(self.anchor).ok()?;
        self.schedule()?.next_after(anchor, tz)
    }

    /// Whether it is due at `now`. A task that missed several times while
    /// the workbench was closed is due once, not once for each.
    pub fn due(&self, now: Timestamp, tz: &TimeZone) -> bool {
        self.next_run(tz).is_some_and(|next| next <= now)
    }

    pub fn running(&self) -> bool {
        self.runs.iter().any(|run| !run.finished())
    }

    pub fn run(&self, id: u64) -> Option<&Run> {
        self.runs.iter().find(|run| run.id == id)
    }

    pub fn run_mut(&mut self, id: u64) -> Option<&mut Run> {
        self.runs.iter_mut().find(|run| run.id == id)
    }

    /// Starts a run record at `now`, dropping the oldest beyond `KEPT_RUNS`.
    pub fn begin_run(&mut self, id: u64, now: i64) {
        self.anchor = now;
        self.runs.push(Run {
            id,
            started: now,
            ..Run::default()
        });
        let extra = self.runs.len().saturating_sub(KEPT_RUNS);
        self.runs.drain(..extra);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskFile {
    pub version: u32,
    pub tasks: Vec<Task>,
    /// The last id handed out, for tasks and runs alike.
    pub last_id: u64,
}

impl TaskFile {
    pub fn next_id(&mut self) -> u64 {
        self.last_id += 1;
        self.last_id
    }

    pub fn task(&self, id: u64) -> Option<&Task> {
        self.tasks.iter().find(|task| task.id == id)
    }

    pub fn task_mut(&mut self, id: u64) -> Option<&mut Task> {
        self.tasks.iter_mut().find(|task| task.id == id)
    }

    /// Runs still marked running when the file is read again were cut off
    /// when the workbench closed.
    pub fn interrupt_running(&mut self) {
        for run in self.tasks.iter_mut().flat_map(|task| &mut task.runs) {
            if run.outcome == Outcome::Running {
                run.outcome = Outcome::Interrupted;
            }
        }
    }
}

/// Where the tasks are kept.
pub fn path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "pi-workbench", "Pi Workbench")
        .context("No application data directory")?;
    Ok(dirs.data_local_dir().join("tasks.json"))
}

/// Reads the tasks. A missing file is none.
pub fn load(path: &Path) -> Result<TaskFile> {
    if !path.exists() {
        return Ok(TaskFile {
            version: 1,
            ..TaskFile::default()
        });
    }
    let bytes = fs::read(path)?;
    ensure!(
        bytes.len() <= 8 * 1024 * 1024,
        "The tasks file is too large"
    );
    let file: TaskFile = serde_json::from_slice(&bytes).context("Invalid tasks file")?;
    ensure!(
        file.version == 1,
        "Unsupported tasks file version {}",
        file.version
    );
    Ok(file)
}

pub fn save(path: &Path, file: &TaskFile) -> Result<()> {
    let parent = path.parent().context("The tasks file has no parent")?;
    fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&serde_json::to_vec_pretty(file)?)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path)?;
    Ok(())
}

/// What Pi is asked to turn a description of when a task runs into a
/// schedule, answered on its own line. `now` is the local date and time,
/// for descriptions such as "this evening".
pub fn describe_prompt(description: &str, now: &str) -> String {
    format!(
        "Turn this description of when a task should run into a schedule. \
         Answer with the schedule alone, on one line, in exactly one of these forms:\n\
         manual\n\
         every <N>m   (minutes, such as every 30m)\n\
         every <N>h   (hours, such as every 2h)\n\
         daily HH:MM\n\
         weekdays HH:MM\n\
         weekends HH:MM\n\
         <days> HH:MM   (days from mon,tue,wed,thu,fri,sat,sun joined by commas, such as mon,thu 18:30)\n\
         Times are 24-hour local time. If no time of day is given, choose a sensible one. \
         It is now {now}.\n\n\
         Description: {description}"
    )
}

/// A time that stays right however long it is kept, for a run's session
/// title: `Tue 20 Oct 03:00`.
pub fn stamp(at: Timestamp, tz: &TimeZone) -> String {
    at.to_zoned(tz.clone())
        .strftime("%a %-d %b %H:%M")
        .to_string()
}

/// A time as the task list shows it: `Today 03:00`, `Yesterday 18:30`,
/// `Mon 20 Oct 09:00`, or with the year when it is not this one.
pub fn describe(at: Timestamp, now: Timestamp, tz: &TimeZone) -> String {
    let (at, now) = (at.to_zoned(tz.clone()), now.to_zoned(tz.clone()));
    let time = at.strftime("%H:%M");
    let (day, today) = (at.date(), now.date());
    if day == today {
        format!("Today {time}")
    } else if today.yesterday().is_ok_and(|yesterday| yesterday == day) {
        format!("Yesterday {time}")
    } else if today.tomorrow().is_ok_and(|tomorrow| tomorrow == day) {
        format!("Tomorrow {time}")
    } else if day.year() == today.year() {
        format!("{} {time}", at.strftime("%a %-d %b"))
    } else {
        format!("{} {time}", at.strftime("%a %-d %b %Y"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> Timestamp {
        text.parse().unwrap()
    }

    #[test]
    fn schedules_read_as_written_and_print_the_same_way() {
        for (text, printed) in [
            ("", "manual"),
            ("Manual", "manual"),
            ("every 30m", "every 30m"),
            ("every 2h", "every 2h"),
            ("every 90 minutes", "every 90m"),
            ("daily 9:00", "daily 09:00"),
            ("daily at 03:05", "daily 03:05"),
            ("weekdays 09:00", "weekdays 09:00"),
            ("Mon,Thursday 18:30", "mon,thu 18:30"),
            ("sat,sun 10:00", "weekends 10:00"),
        ] {
            let schedule = Schedule::parse(text).unwrap();
            assert_eq!(schedule.to_string(), printed, "{text}");
            assert_eq!(Schedule::parse(printed).unwrap(), schedule, "{printed}");
        }
        for wrong in [
            "every",
            "every 0m",
            "every 3d",
            "daily",
            "daily 25:00",
            "daily 9",
            "funday 09:00",
            "soon",
        ] {
            assert!(Schedule::parse(wrong).is_err(), "{wrong}");
        }
    }

    #[test]
    fn controls_make_schedules_that_read_in_words() {
        let weekdays = [true, true, true, true, true, false, false];
        let at = |days, time| Schedule::at(days, time).unwrap().describe();
        assert_eq!(at([true; 7], "9:00"), "Every day at 09:00");
        assert_eq!(at(weekdays, "08:30"), "Every weekday at 08:30");
        assert_eq!(
            at([false, false, false, false, false, true, true], "10:00"),
            "Weekends at 10:00"
        );
        assert_eq!(
            at([false, false, true, false, false, false, false], "07:05"),
            "Every Wed at 07:05"
        );
        assert_eq!(
            at([true, false, true, false, true, false, false], "18:30"),
            "Mon, Wed and Fri at 18:30"
        );
        assert!(Schedule::at([false; 7], "09:00").is_err(), "no day");
        assert!(Schedule::at(weekdays, "9am").is_err(), "not HH:MM");
        assert_eq!(
            Schedule::every("30", false).unwrap().describe(),
            "Every 30 minutes"
        );
        assert_eq!(Schedule::every("1", true).unwrap().describe(), "Every hour");
        assert_eq!(
            Schedule::every("3", true).unwrap(),
            Schedule::Every { minutes: 180 }
        );
        assert!(Schedule::every("0", false).is_err());
        assert!(Schedule::every("200", true).is_err(), "over a week");
        assert_eq!(Schedule::Manual.describe(), "Only when run by hand");
    }

    #[test]
    fn only_a_schedule_is_taken_from_pi() {
        assert_eq!(
            Schedule::from_answer("`weekdays 09:00`"),
            Some(Schedule::parse("weekdays 09:00").unwrap())
        );
        assert_eq!(
            Schedule::from_answer("Sure! Here it is:\n\nmon,thu 18:30\n"),
            Some(Schedule::parse("mon,thu 18:30").unwrap())
        );
        assert_eq!(
            Schedule::from_answer("\"every 2h\"."),
            Some(Schedule::Every { minutes: 120 })
        );
        assert_eq!(Schedule::from_answer("I am not sure what you mean."), None);
        assert_eq!(Schedule::from_answer(""), None, "nothing is not manual");
        let prompt = describe_prompt("every weekday morning", "Fri 23 Oct 21:50");
        assert!(prompt.ends_with("Description: every weekday morning"));
        assert!(prompt.contains("It is now Fri 23 Oct 21:50."));
    }

    #[test]
    fn the_next_time_follows_the_schedule() {
        let utc = TimeZone::UTC;
        // 2026-10-20 is a Tuesday.
        let tuesday_noon = at("2026-10-20T12:00:00Z");
        let next = |text: &str| {
            Schedule::parse(text)
                .unwrap()
                .next_after(tuesday_noon, &utc)
        };
        assert_eq!(next("manual"), None);
        assert_eq!(next("every 45m"), Some(at("2026-10-20T12:45:00Z")));
        assert_eq!(next("daily 13:00"), Some(at("2026-10-20T13:00:00Z")));
        assert_eq!(
            next("daily 12:00"),
            Some(at("2026-10-21T12:00:00Z")),
            "not the same instant"
        );
        assert_eq!(next("daily 09:00"), Some(at("2026-10-21T09:00:00Z")));
        assert_eq!(next("mon 09:00"), Some(at("2026-10-26T09:00:00Z")));
        assert_eq!(
            next("tue 09:00"),
            Some(at("2026-10-27T09:00:00Z")),
            "a week on"
        );
        assert_eq!(next("weekends 08:00"), Some(at("2026-10-24T08:00:00Z")));
    }

    #[test]
    fn times_of_day_are_local() {
        let plus_two = TimeZone::fixed(jiff::tz::offset(2));
        let next = Schedule::parse("daily 09:00")
            .unwrap()
            .next_after(at("2026-10-20T12:00:00Z"), &plus_two);
        assert_eq!(next, Some(at("2026-10-21T07:00:00Z")));
    }

    #[test]
    fn a_task_that_missed_runs_is_due_once() {
        let utc = TimeZone::UTC;
        let mut task = Task {
            schedule: "daily 03:00".into(),
            anchor: at("2026-10-17T12:00:00Z").as_second(),
            ..Task::default()
        };
        let now = at("2026-10-20T12:00:00Z");
        assert!(task.due(now, &utc), "three missed days");
        task.begin_run(1, now.as_second());
        assert!(!task.due(now, &utc));
        assert_eq!(task.next_run(&utc), Some(at("2026-10-21T03:00:00Z")));
        task.paused = true;
        assert!(!task.due(at("2026-10-22T12:00:00Z"), &utc));
        assert_eq!(task.next_run(&utc), None);
    }

    #[test]
    fn runs_are_capped_and_cut_off_runs_are_interrupted() {
        let mut file = TaskFile {
            version: 1,
            ..TaskFile::default()
        };
        let id = file.next_id();
        file.tasks.push(Task {
            id,
            ..Task::default()
        });
        for n in 0..(KEPT_RUNS as i64 + 5) {
            let run = file.next_id();
            let task = file.task_mut(id).unwrap();
            if let Some(last) = task.runs.last_mut() {
                last.outcome = Outcome::Done;
            }
            task.begin_run(run, n);
        }
        let task = file.task(id).unwrap();
        assert_eq!(task.runs.len(), KEPT_RUNS);
        assert_eq!(task.runs[0].started, 5, "the oldest go");
        assert!(task.running());
        file.interrupt_running();
        assert_eq!(
            file.task(id).unwrap().runs.last().unwrap().outcome,
            Outcome::Interrupted
        );
    }

    #[test]
    fn tasks_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.json");
        assert_eq!(load(&path).unwrap().tasks.len(), 0);
        let mut file = load(&path).unwrap();
        let id = file.next_id();
        file.tasks.push(Task {
            id,
            name: "Nightly deps".into(),
            folder: "/work/api".into(),
            prompt: "Check for outdated dependencies".into(),
            schedule: "daily 03:00".into(),
            ..Task::default()
        });
        save(&path, &file).unwrap();
        assert_eq!(load(&path).unwrap(), file);
    }

    #[test]
    fn times_read_relative_to_today() {
        let utc = TimeZone::UTC;
        let now = at("2026-10-20T12:00:00Z");
        assert_eq!(
            describe(at("2026-10-20T03:00:00Z"), now, &utc),
            "Today 03:00"
        );
        assert_eq!(
            describe(at("2026-10-19T18:30:00Z"), now, &utc),
            "Yesterday 18:30"
        );
        assert_eq!(
            describe(at("2026-10-21T03:00:00Z"), now, &utc),
            "Tomorrow 03:00"
        );
        assert_eq!(
            describe(at("2026-10-05T09:00:00Z"), now, &utc),
            "Mon 5 Oct 09:00"
        );
        assert_eq!(
            describe(at("2025-12-31T09:00:00Z"), now, &utc),
            "Wed 31 Dec 2025 09:00"
        );
    }
}
