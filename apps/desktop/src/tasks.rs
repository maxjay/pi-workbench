//! Scheduled tasks in the window: the list, what is picked in the tasks
//! view, and the runs. A run is an ordinary Pi session in its task's
//! folder, started without taking the front, so the code view can review
//! what it changed like any other session.
//!
//! Tasks run while the workbench is open: one due while it was closed runs
//! once when it opens, not once for each time it missed.
use crate::{lens::Since, session::Session, window::Window, workspace::Workbench};
use floem::{
    action::exec_after,
    ext_event::create_ext_action,
    prelude::*,
    reactive::{Effect, Scope},
};
use pi_client::sessions;
use std::{
    cell::Cell,
    collections::HashMap,
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    time::Duration,
};
use workbench_core::tasks::{self, Outcome, Run, Schedule, Task, TaskFile, TimeZone, Timestamp};

/// How often due tasks are looked for.
const TICK: Duration = Duration::from_secs(30);

/// What the window shows: the code, or the tasks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Code,
    Tasks,
}

/// What the tasks view has open on its right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    /// A task being written, not saved yet.
    New,
    Task(u64),
    Run {
        task: u64,
        run: u64,
    },
}

/// How a run is doing, as its row shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunState {
    Working,
    /// Pi is waiting for an answer.
    NeedsYou,
    Finished(Outcome),
}

/// A task as written in the tasks view, before it is saved.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub name: String,
    pub folder: String,
    pub prompt: String,
    pub schedule: Schedule,
    pub continuing: bool,
}

pub struct Tasks {
    scope: Scope,
    me: Weak<Self>,
    window: Weak<Window>,
    path: Option<PathBuf>,
    file: RwSignal<TaskFile>,
    pub mode: RwSignal<Mode>,
    pub picked: RwSignal<Option<Pick>>,
    /// Runs with a session open in this window, by run id: the session's
    /// key, or `None` while it is being started or opened.
    live: RwSignal<HashMap<u64, Option<u64>>>,
    /// Why the tasks could not be read or saved, if they could not.
    problem: RwSignal<Option<String>>,
    tz: TimeZone,
    /// Ticks so far: a clock the list can track to keep "next run" and
    /// "Today" current.
    clock: RwSignal<u64>,
    save_failed: Cell<bool>,
}

impl Tasks {
    pub fn new(scope: Scope, window: Weak<Window>) -> Rc<Self> {
        let path = tasks::path().ok();
        let (mut file, problem) = match path.as_deref().map(tasks::load) {
            Some(Ok(file)) => (file, None),
            Some(Err(err)) => (TaskFile::default(), Some(format!("{err:#}"))),
            None => (TaskFile::default(), Some("No place to keep tasks".into())),
        };
        file.version = 1;
        file.interrupt_running();
        Rc::new_cyclic(|me| Self {
            scope,
            me: me.clone(),
            window,
            // A file that could not be read is not written over.
            path: path.filter(|_| problem.is_none()),
            file: RwSignal::new(file),
            mode: RwSignal::new(Mode::Code),
            picked: RwSignal::new(None),
            live: RwSignal::new(HashMap::new()),
            problem: RwSignal::new(problem),
            tz: TimeZone::system(),
            clock: RwSignal::new(0),
            save_failed: Cell::new(false),
        })
    }

    /// Starts looking for due tasks: now, for those missed while closed,
    /// and then every little while.
    pub fn start(&self) {
        self.save();
        self.tick();
    }

    fn tick(&self) {
        self.clock.update(|n| *n += 1);
        self.sweep();
        let now = Timestamp::now();
        let due: Vec<u64> = self.file.with_untracked(|file| {
            file.tasks
                .iter()
                .filter(|task| !task.running() && task.due(now, &self.tz))
                .map(|task| task.id)
                .collect()
        });
        for id in due {
            if self.claim(id) {
                self.run_now(id);
            }
        }
        let me = self.me.clone();
        exec_after(TICK, move |_| {
            if let Some(me) = me.upgrade() {
                me.tick();
            }
        });
    }

    /// Whether this window may run a due task: another workbench sharing the
    /// tasks file may have run it already, which the file then says.
    fn claim(&self, id: u64) -> bool {
        let Some(path) = &self.path else {
            return true;
        };
        let Some(theirs) = tasks::load(path)
            .ok()
            .and_then(|file| file.task(id).cloned())
        else {
            return true;
        };
        let ours = self
            .file
            .with_untracked(|file| file.task(id).map(|t| t.anchor));
        if ours.is_some_and(|anchor| theirs.anchor > anchor) {
            self.file.update(|file| {
                if let Some(task) = file.task_mut(id) {
                    *task = theirs;
                }
            });
            return false;
        }
        true
    }

    /// Runs whose session went away, closed or stopped with the folder,
    /// are over.
    fn sweep(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let live = self.live.get_untracked();
        let gone: Vec<(u64, u64)> = self.file.with_untracked(|file| {
            file.tasks
                .iter()
                .flat_map(|task| task.runs.iter().map(move |run| (task.id, run)))
                .filter(|(_, run)| !run.finished())
                .filter(|(_, run)| match live.get(&run.id) {
                    Some(Some(key)) => find_session(&window, *key).is_none(),
                    Some(None) => false,
                    None => true,
                })
                .map(|(task, run)| (task, run.id))
                .collect()
        });
        for (task, run) in gone {
            self.finish(task, run, Outcome::Interrupted, None);
        }
    }

    /// Tracked: the tasks, in the order they were made.
    pub fn list(&self) -> Vec<Task> {
        self.file.with(|file| file.tasks.clone())
    }

    /// Tracked.
    pub fn task(&self, id: u64) -> Option<Task> {
        self.file.with(|file| file.task(id).cloned())
    }

    pub fn task_untracked(&self, id: u64) -> Option<Task> {
        self.file.with_untracked(|file| file.task(id).cloned())
    }

    /// Tracked: why tasks cannot be kept, if they cannot.
    pub fn problem(&self) -> Option<String> {
        self.problem.get()
    }

    pub fn time_zone(&self) -> &TimeZone {
        &self.tz
    }

    /// Tracked: now, as of the last tick, so what shows the time keeps up.
    pub fn now(&self) -> Timestamp {
        self.clock.track();
        Timestamp::now()
    }

    /// Tracked: finished runs not looked at yet.
    pub fn unread(&self) -> usize {
        self.file.with(|file| {
            file.tasks
                .iter()
                .flat_map(|task| &task.runs)
                .filter(|run| run.finished() && !run.seen)
                .count()
        })
    }

    fn change<T>(&self, act: impl FnOnce(&mut TaskFile) -> T) -> T {
        let mut result = None;
        self.file.update(|file| result = Some(act(file)));
        self.save();
        result.expect("set in update")
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        match self.file.with_untracked(|file| tasks::save(path, file)) {
            Ok(()) => {
                if self.save_failed.replace(false) {
                    self.problem.set(None);
                }
            }
            Err(err) => {
                self.save_failed.set(true);
                self.problem
                    .set(Some(format!("Could not save tasks: {err:#}")));
            }
        }
    }

    /// Checks a draft; the folder comes back whole and the schedule's days
    /// in order.
    pub fn check(draft: &Draft) -> Result<(PathBuf, Schedule), String> {
        let schedule = draft
            .schedule
            .clone()
            .checked()
            .map_err(|err| format!("{err:#}"))?;
        if draft.prompt.trim().is_empty() {
            return Err("Say what Pi should do".into());
        }
        let folder = PathBuf::from(draft.folder.trim());
        let folder = folder
            .canonicalize()
            .ok()
            .filter(|folder| folder.is_dir())
            .ok_or_else(|| format!("\u{201c}{}\u{201d} is not a folder", draft.folder.trim()))?;
        Ok((folder, schedule))
    }

    /// Saves a draft as a new task, or over an existing one. A changed
    /// schedule starts counting from now.
    pub fn save_draft(&self, id: Option<u64>, draft: &Draft) -> Result<u64, String> {
        let (folder, schedule) = Self::check(draft)?;
        let now = Timestamp::now().as_second();
        let name = match draft.name.trim() {
            "" => crate::session::shorten(draft.prompt.trim()),
            name => name.to_string(),
        };
        let id = self.change(|file| {
            let id = id.filter(|id| file.task(*id).is_some()).unwrap_or_else(|| {
                let id = file.next_id();
                file.tasks.push(Task {
                    id,
                    anchor: now,
                    ..Task::default()
                });
                id
            });
            let task = file.task_mut(id).expect("made above");
            if task.schedule != schedule {
                task.anchor = now;
            }
            task.name = name;
            task.folder = folder;
            task.prompt = draft.prompt.trim().to_string();
            task.continuing = draft.continuing;
            task.schedule = schedule;
            id
        });
        self.picked.set(Some(Pick::Task(id)));
        Ok(id)
    }

    /// Pausing keeps a task but stops its schedule; resuming counts from now.
    pub fn set_paused(&self, id: u64, paused: bool) {
        let now = Timestamp::now().as_second();
        self.change(|file| {
            if let Some(task) = file.task_mut(id) {
                task.paused = paused;
                if !paused {
                    task.anchor = now;
                }
            }
        });
    }

    /// Takes a task off the list; its runs' sessions stay in Pi's store.
    pub fn delete(&self, id: u64) {
        self.change(|file| file.tasks.retain(|task| task.id != id));
        let picked = self.picked.get_untracked();
        if matches!(picked, Some(Pick::Task(t) | Pick::Run { task: t, .. }) if t == id) {
            self.picked.set(None);
        }
    }

    /// Starts a run of the task now, in a new session in its folder.
    pub fn run_now(&self, id: u64) {
        let (Some(window), Some(task)) = (self.window.upgrade(), self.task_untracked(id)) else {
            return;
        };
        let now = Timestamp::now();
        let run = self.change(|file| {
            let run = file.next_id();
            if let Some(task) = file.task_mut(id) {
                task.begin_run(run, now.as_second());
            }
            run
        });
        self.live.update(|live| {
            live.insert(run, None);
        });
        // A continuing task carries on its conversation: open in the
        // window, or opened from Pi's store.
        if task.continuing
            && let Some(path) = task.conversation().map(Path::to_path_buf)
        {
            if let Some((_, session)) = find_by_file(&window, &path) {
                self.ask(id, run, &session, &task.prompt);
                return;
            }
            let folder = sessions::cwd_of(&path)
                .filter(|cwd| cwd.is_dir())
                .unwrap_or_else(|| task.folder.clone());
            let state = match window.open(&folder) {
                Ok(state) => state,
                Err(err) => {
                    self.finish(id, run, Outcome::Failed, Some(format!("{err:#}")));
                    return;
                }
            };
            let me = self.me.clone();
            let finish = create_ext_action(self.scope, move |summary| match summary {
                Some(summary) => state.when_restored(move |state| {
                    let session = state.sessions.open_behind(&summary);
                    if let Some(me) = me.upgrade() {
                        me.ask(id, run, &session, &task.prompt);
                    }
                }),
                // Gone from Pi's store: the conversation starts again.
                None => {
                    if let Some(me) = me.upgrade() {
                        me.start_conversation(&window, id, run, &task, now);
                    }
                }
            });
            std::thread::spawn(move || finish(sessions::summarize(&path)));
            return;
        }
        self.start_conversation(&window, id, run, &task, now);
    }

    /// A run in a session of its own, named after the task (and the time,
    /// unless the task keeps one conversation).
    fn start_conversation(&self, window: &Window, id: u64, run: u64, task: &Task, now: Timestamp) {
        let state = match window.open(&task.folder) {
            Ok(state) => state,
            Err(err) => {
                self.finish(id, run, Outcome::Failed, Some(format!("{err:#}")));
                return;
            }
        };
        let title = if task.continuing {
            task.name.clone()
        } else {
            format!("{} \u{b7} {}", task.name, tasks::stamp(now, &self.tz))
        };
        let (me, prompt) = (self.me.clone(), task.prompt.clone());
        state.when_restored(move |state| {
            let Some(me) = me.upgrade() else { return };
            let session = state.sessions.create_behind();
            session.rename(&title);
            me.ask(id, run, &session, &prompt);
        });
    }

    /// Gives a run's prompt to its session: as a follow-up for when Pi is
    /// done, if it is busy in that conversation.
    fn ask(&self, task: u64, run: u64, session: &Rc<Session>, prompt: &str) {
        let busy = session.working().get_untracked();
        if session.submit(prompt.to_string(), busy) {
            self.follow(task, run, session, prompt.to_string());
        } else {
            self.finish(task, run, Outcome::Failed, Some("Pi was not asked".into()));
        }
    }

    /// Keeps the run's record up to date with its session: the file Pi
    /// keeps it in, and how it ended.
    fn follow(&self, task: u64, run: u64, session: &Rc<Session>, prompt: String) {
        self.live.update(|live| {
            live.insert(run, Some(session.key));
        });
        let (weak, me) = (Rc::downgrade(session), self.me.clone());
        let scope = self.scope.create_child();
        scope.enter(|| {
            Effect::new(move |was_working: Option<bool>| {
                let (Some(session), Some(me)) = (weak.upgrade(), me.upgrade()) else {
                    return false;
                };
                let file = session.file().get();
                let working = session.working().get();
                Effect::untrack(|| {
                    if let Some(file) = file {
                        me.note_session(task, run, file);
                    }
                    if was_working == Some(true) && !working {
                        let outcome = if session.ended_in_error() {
                            Outcome::Failed
                        } else if session.was_stopped() {
                            Outcome::Stopped
                        } else {
                            Outcome::Done
                        };
                        me.note_turn(task, run, session.prompt_index(&prompt));
                        me.finish(task, run, outcome, None);
                        // Its work is done; not from inside itself.
                        exec_after(Duration::ZERO, move |_| scope.dispose());
                    }
                });
                working
            });
        });
    }

    fn note_turn(&self, task: u64, run: u64, turn: Option<usize>) {
        self.change(|file| {
            if let Some(run) = file.task_mut(task).and_then(|t| t.run_mut(run)) {
                run.turn = turn;
            }
        });
    }

    fn note_session(&self, task: u64, run: u64, path: PathBuf) {
        let known = self.file.with_untracked(|file| {
            file.task(task)
                .and_then(|t| t.run(run))
                .is_none_or(|r| r.session.as_ref() == Some(&path))
        });
        if !known {
            self.change(|file| {
                if let Some(run) = file.task_mut(task).and_then(|t| t.run_mut(run)) {
                    run.session = Some(path);
                }
            });
        }
    }

    fn finish(&self, task: u64, run: u64, outcome: Outcome, error: Option<String>) {
        // Seen already if it is the one open in the tasks view.
        let watched = self.mode.get_untracked() == Mode::Tasks
            && self.picked.get_untracked() == Some(Pick::Run { task, run });
        self.change(|file| {
            if let Some(record) = file.task_mut(task).and_then(|t| t.run_mut(run)) {
                record.outcome = outcome;
                record.error = error;
                record.seen = watched;
            }
        });
        if outcome == Outcome::Failed || outcome == Outcome::Interrupted {
            // A session that never started has nothing to show.
            self.live.update(|live| {
                if live.get(&run) == Some(&None) {
                    live.remove(&run);
                }
            });
        }
    }

    /// Tracked: how a run is doing; its session's status while it runs.
    pub fn run_state(&self, run: &Run) -> RunState {
        if !run.finished()
            && let Some((_, session)) = self.session(run.id)
        {
            return match session.status() {
                crate::session::Status::NeedsInput => RunState::NeedsYou,
                _ => RunState::Working,
            };
        }
        match run.outcome {
            Outcome::Running => RunState::Working,
            outcome => RunState::Finished(outcome),
        }
    }

    /// Tracked: a run's session, if it is open in this window, with the
    /// workspace it is in.
    pub fn session(&self, run: u64) -> Option<(Rc<Workbench>, Rc<Session>)> {
        let key = self.live.with(|live| live.get(&run).copied().flatten())?;
        let window = self.window.upgrade()?;
        find_session(&window, key)
    }

    pub fn session_untracked(&self, run: u64) -> Option<(Rc<Workbench>, Rc<Session>)> {
        Effect::untrack(|| self.session(run))
    }

    /// Tracked: whether a run's session is still being started or opened.
    pub fn opening(&self, run: u64) -> bool {
        self.live.with(|live| live.get(&run) == Some(&None))
    }

    /// Opens a run in the tasks view: its session, opened from Pi's store
    /// if it is not open, with Pi started on it to carry on the
    /// conversation.
    pub fn pick_run(&self, task: u64, run: u64) {
        self.picked.set(Some(Pick::Run { task, run }));
        let Some((folder, record)) = self.file.with_untracked(|file| {
            file.task(task)
                .and_then(|t| Some((t.folder.clone(), t.run(run)?.clone())))
        }) else {
            return;
        };
        if record.finished() && !record.seen {
            self.change(|file| {
                if let Some(run) = file.task_mut(task).and_then(|t| t.run_mut(run)) {
                    run.seen = true;
                }
            });
        }
        if self.live.with_untracked(|live| live.contains_key(&run)) {
            return;
        }
        let (Some(window), Some(path)) = (self.window.upgrade(), record.session) else {
            return;
        };
        // The session may have moved to another folder since; it says.
        let folder = sessions::cwd_of(&path)
            .filter(|cwd| cwd.is_dir())
            .unwrap_or(folder);
        let state = match window.open(&folder) {
            Ok(state) => state,
            Err(err) => {
                self.problem.set(Some(format!("{err:#}")));
                return;
            }
        };
        self.live.update(|live| {
            live.insert(run, None);
        });
        let live = self.live;
        let finish = create_ext_action(self.scope, move |summary| {
            let Some(summary) = summary else {
                live.update(|live| {
                    live.remove(&run);
                });
                return;
            };
            state.when_restored(move |state| {
                let session = state.sessions.open_behind(&summary);
                live.update(|live| {
                    live.insert(run, Some(session.key));
                });
            });
        });
        std::thread::spawn(move || finish(sessions::summarize(&path)));
    }

    /// Goes to the code view on a run's session, the changes lens reading
    /// from where the run began when this window ran it.
    pub fn review(&self, run: u64) {
        let (Some(window), Some((state, session))) = (self.window.upgrade(), self.session(run))
        else {
            return;
        };
        self.mode.set(Mode::Code);
        window.show_session(state.root(), session.key);
        if !session.turns().get_untracked().is_empty() {
            state.lens.set_since(Since::SessionStart);
        }
    }

    /// Has Pi turn a description of when to run into a schedule, for the
    /// controls to show, to be checked and changed before it is saved.
    /// `done` gets the schedule, or why there is none.
    pub fn fill_schedule(
        &self,
        description: &str,
        folder: &str,
        done: impl FnOnce(Result<Schedule, String>) + 'static,
    ) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let launch = window.launch().clone();
        let folder = Some(PathBuf::from(folder.trim()))
            .filter(|folder| folder.is_dir())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        let now = tasks::stamp(Timestamp::now(), &self.tz);
        let question = tasks::describe_prompt(description.trim(), &now);
        let finish = create_ext_action(self.scope, done);
        std::thread::spawn(move || {
            let answer =
                pi_client::ask_aside(&launch, &folder, None, &question, Duration::from_secs(90));
            finish(match answer {
                Ok(answer) => Schedule::from_answer(&answer).map_err(|err| format!("{err:#}")),
                Err(err) => Err(format!("{err:#}")),
            })
        });
    }

    /// The folder a new task starts in: the one in front.
    pub fn default_folder(&self) -> String {
        self.window
            .upgrade()
            .map(|window| window.active().get_untracked())
            .map(|folder| folder.display().to_string())
            .unwrap_or_default()
    }
}

/// Tracked: a session open in any of the window's folders, by key.
fn find_session(window: &Window, key: u64) -> Option<(Rc<Workbench>, Rc<Session>)> {
    window.workspaces().with(|open| {
        open.iter().find_map(|state| {
            state
                .sessions
                .find_tracked(key)
                .map(|session| (state.clone(), session))
        })
    })
}

/// A session open in any of the window's folders, by its file.
fn find_by_file(window: &Window, path: &Path) -> Option<(Rc<Workbench>, Rc<Session>)> {
    window.workspaces().with_untracked(|open| {
        open.iter().find_map(|state| {
            state
                .sessions
                .open()
                .with_untracked(|sessions| {
                    sessions
                        .iter()
                        .find(|s| s.file().get_untracked().as_deref() == Some(path))
                        .cloned()
                })
                .map(|session| (state.clone(), session))
        })
    })
}

/// A folder's name, for a task's row.
pub fn folder_name(folder: &Path) -> String {
    folder.file_name().map_or_else(
        || folder.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use workbench_core::tasks::Day;

    #[test]
    fn a_draft_needs_a_prompt_a_folder_and_a_schedule() {
        let dir = tempfile::tempdir().unwrap();
        let good = Draft {
            name: String::new(),
            folder: dir.path().display().to_string(),
            prompt: "Check the dependencies".into(),
            schedule: Schedule::At {
                days: vec![Day::Fri, Day::Mon],
                hour: 9,
                minute: 0,
            },
            continuing: false,
        };
        let (folder, schedule) = Tasks::check(&good).unwrap();
        assert_eq!(folder, dir.path().canonicalize().unwrap());
        assert_eq!(
            schedule,
            Schedule::At {
                days: vec![Day::Mon, Day::Fri],
                hour: 9,
                minute: 0
            },
            "days in order"
        );
        let without = |change: fn(&mut Draft)| {
            let mut draft = good.clone();
            change(&mut draft);
            Tasks::check(&draft).unwrap_err()
        };
        assert_eq!(without(|d| d.prompt = "  ".into()), "Say what Pi should do");
        assert!(without(|d| d.folder = "/no/such/folder".into()).contains("is not a folder"));
        assert_eq!(
            without(|d| d.schedule = Schedule::Every { minutes: 0 }),
            "Say how often, as a whole number"
        );
    }
}
