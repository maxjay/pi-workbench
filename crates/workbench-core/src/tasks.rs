//! Scheduled tasks: a prompt Pi runs in a folder on a schedule, and the
//! sessions each run left in Pi's store, kept in one file for the app.
//!
//! A schedule is data, set with controls in local wall-clock time. Asked
//! to fill it in from a description, Pi answers with the same data as
//! JSON, which is only taken if it is a schedule, and shown in the
//! controls to be checked before it is kept.
use anyhow::{Context, Result, ensure};
pub use jiff::{Timestamp, tz::TimeZone};
use jiff::{ToSpan, civil::Date};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

/// Runs kept per task; older ones drop off the list, their sessions stay in
/// Pi's store.
pub const KEPT_RUNS: usize = 30;

/// The longest gap `Every` allows: a week.
const MAX_EVERY: u32 = 7 * 24 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Day {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

impl Day {
    pub const ALL: [Day; 7] = [
        Day::Mon,
        Day::Tue,
        Day::Wed,
        Day::Thu,
        Day::Fri,
        Day::Sat,
        Day::Sun,
    ];

    fn name(self) -> &'static str {
        ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][self as usize]
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Schedule {
    /// Only when asked: Run now.
    #[default]
    Manual,
    Every {
        minutes: u32,
    },
    /// At a time of day on the given days.
    At {
        days: Vec<Day>,
        hour: u8,
        minute: u8,
    },
}

impl Schedule {
    /// Whether it can be kept: a gap of a minute to a week, or a real time
    /// on at least one day. Days come back in order, once each.
    pub fn checked(self) -> Result<Self> {
        match self {
            Self::Manual => Ok(self),
            Self::Every { minutes } => {
                ensure!(minutes > 0, "Say how often, as a whole number");
                ensure!(
                    minutes <= MAX_EVERY,
                    "At most once a week; pick days and a time instead"
                );
                Ok(self)
            }
            Self::At {
                mut days,
                hour,
                minute,
            } => {
                days.sort();
                days.dedup();
                ensure!(!days.is_empty(), "Pick at least one day");
                ensure!(hour < 24, "The hour is from 0 to 23");
                ensure!(minute < 60, "The minute is from 0 to 59");
                Ok(Self::At { days, hour, minute })
            }
        }
    }

    /// Takes the schedule out of Pi's answer to `describe_prompt`: the JSON
    /// object in it, if that is a schedule that can be kept.
    pub fn from_answer(answer: &str) -> Result<Self> {
        let (start, end) = answer
            .find('{')
            .zip(answer.rfind('}'))
            .filter(|(start, end)| start < end)
            .context("Pi did not answer with a schedule")?;
        let schedule: Self = serde_json::from_str(&answer[start..=end])
            .context("Pi did not answer with a schedule")?;
        schedule.checked()
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
                use Day::*;
                let days = match days.as_slice() {
                    [Mon, Tue, Wed, Thu, Fri, Sat, Sun] => "Every day".to_string(),
                    [Mon, Tue, Wed, Thu, Fri] => "Every weekday".to_string(),
                    [Sat, Sun] => "Weekends".to_string(),
                    [one] => format!("Every {}", one.name()),
                    [rest @ .., last] => format!(
                        "{} and {}",
                        rest.iter().map(|d| d.name()).collect::<Vec<_>>().join(", "),
                        last.name()
                    ),
                    [] => String::new(),
                };
                format!("{days} at {hour:02}:{minute:02}")
            }
        }
    }

    /// The first time after `after` it is due; `None` for manual tasks.
    pub fn next_after(&self, after: Timestamp, tz: &TimeZone) -> Option<Timestamp> {
        match self {
            Self::Manual => None,
            Self::Every { minutes } => after.checked_add(i64::from(*minutes).minutes()).ok(),
            Self::At { days, hour, minute } => {
                let start: Date = after.to_zoned(tz.clone()).date();
                (0..=8).find_map(|offset| {
                    let date = start.checked_add(offset.days()).ok()?;
                    let day = Day::ALL[date.weekday().to_monday_zero_offset() as usize];
                    if !days.contains(&day) {
                        return None;
                    }
                    let at = date
                        .at(*hour as i8, *minute as i8, 0, 0)
                        .to_zoned(tz.clone())
                        .ok()?
                        .timestamp();
                    (at > after).then_some(at)
                })
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
    /// Which of its session's prompts was this run's, counting from 0: in a
    /// task that keeps one conversation, where this run starts.
    pub turn: Option<usize>,
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
    pub schedule: Schedule,
    /// Each run carries on one conversation, rather than starting its own.
    pub continuing: bool,
    pub paused: bool,
    /// When scheduling last settled: made, edited, resumed or run. The
    /// next run is the schedule's first time after it.
    pub anchor: i64,
    /// Newest last.
    pub runs: Vec<Run>,
}

impl Task {
    /// When it runs next, if it is scheduled and not paused.
    pub fn next_run(&self, tz: &TimeZone) -> Option<Timestamp> {
        if self.paused {
            return None;
        }
        let anchor = Timestamp::from_second(self.anchor).ok()?;
        self.schedule.next_after(anchor, tz)
    }

    /// Whether it is due at `now`. A task that missed several times while
    /// the workbench was closed is due once, not once for each.
    pub fn due(&self, now: Timestamp, tz: &TimeZone) -> bool {
        self.next_run(tz).is_some_and(|next| next <= now)
    }

    /// The conversation a continuing task's next run carries on: the
    /// latest run's session, if any run got one.
    pub fn conversation(&self) -> Option<&Path> {
        self.runs
            .iter()
            .rev()
            .find_map(|run| run.session.as_deref())
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
/// schedule: the JSON `Schedule` is read from. `now` is the local date and
/// time, for descriptions such as "this evening".
pub fn describe_prompt(description: &str, now: &str) -> String {
    format!(
        "Turn this description of when a task should run into a schedule, \
         as one JSON object and nothing else, in one of these shapes:\n\
         {{\"kind\": \"manual\"}}  (only when run by hand)\n\
         {{\"kind\": \"every\", \"minutes\": 30}}  (repeating, every 1 to 10080 minutes)\n\
         {{\"kind\": \"at\", \"days\": [\"mon\", \"thu\"], \"hour\": 18, \"minute\": 30}}  \
         (at a time of day on the days listed, from mon tue wed thu fri sat sun)\n\
         Hours are 0 to 23, local time. If no time of day is given, choose a sensible one. \
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

    fn on(days: &[Day], hour: u8, minute: u8) -> Schedule {
        Schedule::At {
            days: days.to_vec(),
            hour,
            minute,
        }
    }

    const WEEKDAYS: [Day; 5] = [Day::Mon, Day::Tue, Day::Wed, Day::Thu, Day::Fri];

    #[test]
    fn schedules_read_in_words() {
        assert_eq!(on(&Day::ALL, 9, 0).describe(), "Every day at 09:00");
        assert_eq!(on(&WEEKDAYS, 8, 30).describe(), "Every weekday at 08:30");
        assert_eq!(
            on(&[Day::Sat, Day::Sun], 10, 0).describe(),
            "Weekends at 10:00"
        );
        assert_eq!(on(&[Day::Wed], 7, 5).describe(), "Every Wed at 07:05");
        assert_eq!(
            on(&[Day::Mon, Day::Wed, Day::Fri], 18, 30).describe(),
            "Mon, Wed and Fri at 18:30"
        );
        assert_eq!(
            Schedule::Every { minutes: 30 }.describe(),
            "Every 30 minutes"
        );
        assert_eq!(Schedule::Every { minutes: 60 }.describe(), "Every hour");
        assert_eq!(Schedule::Every { minutes: 180 }.describe(), "Every 3 hours");
        assert_eq!(Schedule::Manual.describe(), "Only when run by hand");
    }

    #[test]
    fn only_schedules_that_can_run_are_kept() {
        assert_eq!(
            on(&[Day::Fri, Day::Mon, Day::Fri], 9, 0).checked().unwrap(),
            on(&[Day::Mon, Day::Fri], 9, 0),
            "days in order, once each"
        );
        assert!(on(&[], 9, 0).checked().is_err(), "no day");
        assert!(on(&[Day::Mon], 24, 0).checked().is_err());
        assert!(on(&[Day::Mon], 9, 60).checked().is_err());
        assert!(Schedule::Every { minutes: 0 }.checked().is_err());
        assert!(
            Schedule::Every {
                minutes: 8 * 24 * 60
            }
            .checked()
            .is_err(),
            "over a week"
        );
    }

    #[test]
    fn pi_answers_with_a_schedule_as_json() {
        assert_eq!(
            Schedule::from_answer(
                "Here you go:\n```json\n{\"kind\": \"at\", \"days\": [\"thu\", \"mon\"], \"hour\": 18, \"minute\": 30}\n```"
            )
            .unwrap(),
            on(&[Day::Mon, Day::Thu], 18, 30)
        );
        assert_eq!(
            Schedule::from_answer("{\"kind\":\"every\",\"minutes\":120}").unwrap(),
            Schedule::Every { minutes: 120 }
        );
        assert_eq!(
            Schedule::from_answer("{\"kind\":\"manual\"}").unwrap(),
            Schedule::Manual
        );
        for wrong in [
            "",
            "Every weekday at nine.",
            "{\"kind\": \"at\", \"days\": [\"funday\"], \"hour\": 9, \"minute\": 0}",
            "{\"kind\": \"at\", \"days\": [\"mon\"], \"hour\": 25, \"minute\": 0}",
            "{\"kind\": \"sometimes\"}",
        ] {
            assert!(Schedule::from_answer(wrong).is_err(), "{wrong}");
        }
        let prompt = describe_prompt("every weekday morning", "Fri 23 Oct 21:50");
        assert!(prompt.ends_with("Description: every weekday morning"));
        assert!(prompt.contains("It is now Fri 23 Oct 21:50."));
    }

    #[test]
    fn schedules_are_kept_as_data() {
        let json = serde_json::to_string(&on(&[Day::Mon, Day::Thu], 18, 30)).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"at","days":["mon","thu"],"hour":18,"minute":30}"#
        );
        assert_eq!(
            serde_json::to_string(&Schedule::Every { minutes: 30 }).unwrap(),
            r#"{"kind":"every","minutes":30}"#
        );
    }

    #[test]
    fn the_next_time_follows_the_schedule() {
        let utc = TimeZone::UTC;
        // 2026-10-20 is a Tuesday.
        let tuesday_noon = at("2026-10-20T12:00:00Z");
        let next = |schedule: Schedule| schedule.next_after(tuesday_noon, &utc);
        assert_eq!(next(Schedule::Manual), None);
        assert_eq!(
            next(Schedule::Every { minutes: 45 }),
            Some(at("2026-10-20T12:45:00Z"))
        );
        assert_eq!(next(on(&Day::ALL, 13, 0)), Some(at("2026-10-20T13:00:00Z")));
        assert_eq!(
            next(on(&Day::ALL, 12, 0)),
            Some(at("2026-10-21T12:00:00Z")),
            "not the same instant"
        );
        assert_eq!(next(on(&Day::ALL, 9, 0)), Some(at("2026-10-21T09:00:00Z")));
        assert_eq!(
            next(on(&[Day::Mon], 9, 0)),
            Some(at("2026-10-26T09:00:00Z"))
        );
        assert_eq!(
            next(on(&[Day::Tue], 9, 0)),
            Some(at("2026-10-27T09:00:00Z")),
            "a week on"
        );
        assert_eq!(
            next(on(&[Day::Sat, Day::Sun], 8, 0)),
            Some(at("2026-10-24T08:00:00Z"))
        );
    }

    #[test]
    fn times_of_day_are_local() {
        let plus_two = TimeZone::fixed(jiff::tz::offset(2));
        let next = on(&Day::ALL, 9, 0).next_after(at("2026-10-20T12:00:00Z"), &plus_two);
        assert_eq!(next, Some(at("2026-10-21T07:00:00Z")));
    }

    #[test]
    fn a_task_that_missed_runs_is_due_once() {
        let utc = TimeZone::UTC;
        let mut task = Task {
            schedule: on(&Day::ALL, 3, 0),
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
    fn a_continuing_task_carries_on_its_latest_session() {
        let mut task = Task {
            continuing: true,
            ..Task::default()
        };
        assert_eq!(task.conversation(), None, "no run yet");
        task.begin_run(1, 0);
        task.runs[0].session = Some("/pi/one.jsonl".into());
        task.begin_run(2, 1);
        assert_eq!(
            task.conversation(),
            Some(Path::new("/pi/one.jsonl")),
            "the new run has none yet"
        );
        task.runs[1].session = Some("/pi/two.jsonl".into());
        assert_eq!(task.conversation(), Some(Path::new("/pi/two.jsonl")));
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
            schedule: on(&WEEKDAYS, 3, 0),
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
