//! The tasks view: scheduled tasks and their runs on the left, and on the
//! right the task's settings or a run's conversation, carried on in place.
use super::{
    button, color,
    session::{pill, session_view, status_color},
};
use crate::{
    session::Status,
    tasks::{Draft, Mode, Pick, RunState, Tasks, folder_name},
    window::Window,
};
use floem::{
    prelude::*,
    reactive::{Effect, Memo},
    style::CursorStyle,
    views::{
        PlaceholderTextClass, TextInput, TextInputEnter,
        editor::text::{SimpleStyling, WrapMethod},
    },
};
use std::rc::Rc;
use workbench_core::tasks::{self, Day, Outcome, Run, Schedule, Task, Timestamp};

/// The Code | Tasks switch in the title bar, with how many runs finished
/// unseen.
pub(super) fn mode_switch(tasks: Rc<Tasks>) -> impl IntoView {
    let segment = |label: &'static str, mode: Mode, tasks: Rc<Tasks>| {
        let (style_tasks, unread_tasks) = (tasks.clone(), tasks.clone());
        Label::derived(move || match (mode, unread_tasks.unread()) {
            (Mode::Tasks, unread) if unread > 0 => format!("{label}  {unread}"),
            _ => label.to_string(),
        })
        .style(move |s| {
            let on = style_tasks.mode.get() == mode;
            s.padding_horiz(10)
                .padding_vert(3)
                .border_radius(5)
                .font_size(12)
                .cursor(CursorStyle::Pointer)
                .color(if on { color::text() } else { color::muted() })
                .hover(|s| s.color(color::text()))
                .apply_if(on, |s| s.background(color::highlight()))
        })
        .on_event_stop(el::Click, move |_, _| tasks.mode.set(mode))
    };
    Stack::horizontal((
        segment("Code", Mode::Code, tasks.clone()),
        segment("Tasks", Mode::Tasks, tasks),
    ))
    .style(|s| {
        s.gap(2)
            .padding(2)
            .items_center()
            .border(1)
            .border_color(color::border())
            .border_radius(7)
            .flex_shrink(0.0_f32)
    })
}

fn run_label(state: RunState) -> &'static str {
    match state {
        RunState::Working => "working",
        RunState::NeedsYou => "needs you",
        RunState::Finished(Outcome::Running | Outcome::Done) => "done",
        RunState::Finished(Outcome::Stopped) => "stopped",
        RunState::Finished(Outcome::Failed) => "failed",
        RunState::Finished(Outcome::Interrupted) => "interrupted",
    }
}

fn run_color(state: RunState) -> Color {
    match state {
        RunState::Working => status_color(Status::Working),
        RunState::NeedsYou => status_color(Status::NeedsInput),
        RunState::Finished(Outcome::Running | Outcome::Done) => color::added_text(),
        RunState::Finished(Outcome::Failed | Outcome::Interrupted) => color::removed_text(),
        RunState::Finished(Outcome::Stopped) => color::muted(),
    }
}

/// Under a task's name: its schedule, when it runs next, and where.
fn task_summary(tasks: &Tasks, task: &Task) -> String {
    let now = tasks.now();
    let when = match (&task.schedule, task.next_run(tasks.time_zone())) {
        _ if task.paused => "paused".to_string(),
        (Schedule::Manual, _) => "by hand".to_string(),
        (_, Some(next)) if next <= now => "due now".to_string(),
        (schedule, Some(next)) => format!(
            "{}  \u{b7}  next {}",
            schedule.describe(),
            tasks::describe(next, now, tasks.time_zone())
        ),
        (schedule, None) => schedule.describe(),
    };
    format!("{when}  \u{b7}  {}", folder_name(&task.folder))
}

fn row_style(s: floem::style::Style, selected: bool) -> floem::style::Style {
    s.width_full()
        .cursor(CursorStyle::Pointer)
        .border_left(2)
        .border_color(Color::TRANSPARENT)
        .hover(|s| s.background(color::border().with_alpha(0.5)))
        .apply_if(selected, |s| {
            s.background(color::border()).border_color(color::accent())
        })
}

fn run_row(tasks: Rc<Tasks>, task: u64, run: Run) -> impl IntoView {
    let id = run.id;
    let started = Timestamp::from_second(run.started).unwrap_or_default();
    let (dot_tasks, label_tasks, text_tasks, pick_tasks, style_tasks) = (
        tasks.clone(),
        tasks.clone(),
        tasks.clone(),
        tasks.clone(),
        tasks,
    );
    let (dot_run, label_run, text_run) = (run.clone(), run.clone(), run);
    Stack::horizontal((
        Empty::new().style(move |s| {
            s.size(7, 7)
                .border_radius(4)
                .flex_shrink(0.0_f32)
                .background(run_color(dot_tasks.run_state(&dot_run)))
        }),
        Label::derived(move || {
            let now = label_tasks.now();
            tasks::describe(started, now, label_tasks.time_zone())
        })
        .style(move |s| {
            let unread = label_run.finished() && !label_run.seen;
            s.flex_grow(1.0_f32)
                .min_width(0)
                .text_ellipsis()
                .color(if unread {
                    color::text()
                } else {
                    color::muted()
                })
                .apply_if(unread, |s| s.font_bold())
        }),
        Label::derived(move || run_label(text_tasks.run_state(&text_run)).to_string())
            .style(|s| s.color(color::muted()).font_size(11).flex_shrink(0.0_f32)),
    ))
    .style(move |s| {
        let selected = style_tasks.picked.get() == Some(Pick::Run { task, run: id });
        row_style(s, selected)
            .height(28)
            .gap(8)
            .padding_left(28)
            .padding_right(10)
            .items_center()
    })
    .on_event_stop(el::Click, move |_, _| pick_tasks.pick_run(task, id))
}

fn task_group(tasks: Rc<Tasks>, id: u64) -> impl IntoView {
    let (name_tasks, summary_tasks, runs_tasks, style_tasks, pick_tasks, row_tasks) = (
        tasks.clone(),
        tasks.clone(),
        tasks.clone(),
        tasks.clone(),
        tasks.clone(),
        tasks,
    );
    let header = Stack::vertical((
        Label::derived(move || name_tasks.task(id).map(|t| t.name).unwrap_or_default()).style(
            |s| {
                s.color(color::text())
                    .font_bold()
                    .min_width(0)
                    .text_ellipsis()
            },
        ),
        Label::derived(move || {
            summary_tasks
                .task(id)
                .map(|task| task_summary(&summary_tasks, &task))
                .unwrap_or_default()
        })
        .style(|s| {
            s.color(color::muted())
                .font_size(11)
                .min_width(0)
                .text_ellipsis()
        }),
    ))
    .style(move |s| {
        let selected = style_tasks.picked.get() == Some(Pick::Task(id));
        row_style(s, selected)
            .gap(2)
            .padding_vert(7)
            .padding_left(14)
            .padding_right(10)
    })
    .on_event_stop(el::Click, move |_, _| {
        pick_tasks.picked.set(Some(Pick::Task(id)))
    });
    let runs = dyn_stack(
        move || {
            runs_tasks
                .task(id)
                .map(|task| task.runs.into_iter().rev().collect::<Vec<_>>())
                .unwrap_or_default()
        },
        // A run's row is made again when its record changes.
        |run| (run.id, run.outcome, run.seen),
        move |run| run_row(row_tasks.clone(), id, run),
    )
    .style(|s| s.flex_col().width_full());
    Stack::vertical((header, runs)).style(|s| s.width_full().padding_bottom(6))
}

fn task_list(tasks: Rc<Tasks>) -> impl IntoView {
    let (new_tasks, problem_tasks, empty_tasks, list_tasks, group_tasks) = (
        tasks.clone(),
        tasks.clone(),
        tasks.clone(),
        tasks.clone(),
        tasks,
    );
    Stack::vertical((
        Stack::horizontal((
            Label::new("Tasks").style(|s| {
                s.color(color::muted())
                    .font_size(11)
                    .font_bold()
                    .flex_grow(1.0_f32)
            }),
            Label::new("+ New")
                .style(|s| {
                    s.color(color::accent())
                        .font_size(12)
                        .padding_horiz(8)
                        .padding_vert(3)
                        .border_radius(6)
                        .cursor(CursorStyle::Pointer)
                        .hover(|s| s.background(color::border()))
                })
                .on_event_stop(el::Click, move |_, _| new_tasks.picked.set(Some(Pick::New))),
        ))
        .style(|s| {
            s.width_full()
                .height(42)
                .padding_left(14)
                .padding_right(8)
                .items_center()
                .flex_shrink(0.0_f32)
        }),
        Label::derived(move || problem_tasks.problem().unwrap_or_default()).style({
            let tasks = empty_tasks.clone();
            move |s| {
                s.color(color::removed_text())
                    .font_size(11)
                    .padding_horiz(14)
                    .padding_bottom(8)
                    .text_wrap()
                    .apply_if(tasks.problem().is_none(), |s| s.hide())
            }
        }),
        Label::new("Tasks run a prompt in a folder on a schedule, while the workbench is open. Each run is a session you can carry on.")
            .style(move |s| {
                s.color(color::muted())
                    .font_size(12)
                    .padding_horiz(14)
                    .line_height(1.5)
                    .text_wrap()
                    .apply_if(!empty_tasks.list().is_empty(), |s| s.hide())
            }),
        dyn_stack(
            move || {
                list_tasks
                    .list()
                    .into_iter()
                    .map(|task| task.id)
                    .collect::<Vec<_>>()
            },
            |id| *id,
            move |id| task_group(group_tasks.clone(), id),
        )
        .style(|s| s.flex_col().width_full())
        .scroll()
        .style(|s| s.width_full().flex_grow(1.0_f32).min_height(0)),
    ))
    .style(|s| {
        s.width(280)
            .height_full()
            .flex_shrink(0.0_f32)
            .background(color::panel())
            .border_right(1)
            .border_color(color::border())
    })
}

fn field_label(text: &'static str) -> impl IntoView {
    Label::new(text).style(|s| {
        s.color(color::muted())
            .font_size(11)
            .font_bold()
            .margin_top(10)
    })
}

fn text_field(value: RwSignal<String>, placeholder: &'static str) -> impl IntoView {
    TextInput::new(value).placeholder(placeholder).style(|s| {
        s.width_full()
            .padding_horiz(10)
            .padding_vert(7)
            .border(1)
            .border_color(color::border())
            .border_radius(6)
            .background(color::bg())
            .color(color::text())
            .focus(|s| s.border_color(color::accent()))
            .class(PlaceholderTextClass, |s| s.color(color::muted()))
    })
}

fn accent_button(title: &'static str, action: impl Fn() + 'static) -> impl IntoView {
    button(title, action).style(|s| {
        s.background(color::accent())
            .color(color::on_accent())
            .font_bold()
            .hover(|s| {
                s.background(color::accent().with_alpha(0.85))
                    .color(color::on_accent())
            })
    })
}

/// Which kind of schedule the controls are set to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Manual,
    Every,
    At,
}

/// The schedule as set by hand: what is saved is what these say.
#[derive(Clone, Copy)]
struct Controls {
    kind: RwSignal<Kind>,
    amount: RwSignal<String>,
    hours: RwSignal<bool>,
    days: RwSignal<[bool; 7]>,
    hour: RwSignal<String>,
    minute: RwSignal<String>,
}

impl Controls {
    fn new(schedule: &Schedule) -> Self {
        let controls = Self {
            kind: RwSignal::new(Kind::At),
            amount: RwSignal::new("30".into()),
            hours: RwSignal::new(false),
            days: RwSignal::new([true; 7]),
            hour: RwSignal::new("09".into()),
            minute: RwSignal::new("00".into()),
        };
        controls.set(schedule);
        controls
    }

    /// Shows a schedule, as Pi filled it in or as it was saved.
    fn set(&self, schedule: &Schedule) {
        match schedule {
            Schedule::Manual => self.kind.set(Kind::Manual),
            &Schedule::Every { minutes } => {
                self.kind.set(Kind::Every);
                let hours = minutes % 60 == 0;
                self.hours.set(hours);
                self.amount
                    .set(if hours { minutes / 60 } else { minutes }.to_string());
            }
            Schedule::At { days, hour, minute } => {
                self.kind.set(Kind::At);
                self.days.set(Day::ALL.map(|day| days.contains(&day)));
                self.hour.set(format!("{hour:02}"));
                self.minute.set(format!("{minute:02}"));
            }
        }
    }

    /// Tracked: the schedule they make, or what is wrong.
    fn schedule(&self) -> Result<Schedule, String> {
        let number = |field: RwSignal<String>, what: &str| {
            field
                .with(|text| text.trim().parse::<u32>())
                .map_err(|_| format!("The {what} is a whole number"))
        };
        let schedule = match self.kind.get() {
            Kind::Manual => Schedule::Manual,
            Kind::Every => {
                let amount = number(self.amount, "gap")?;
                Schedule::Every {
                    minutes: if self.hours.get() {
                        amount.saturating_mul(60)
                    } else {
                        amount
                    },
                }
            }
            Kind::At => Schedule::At {
                days: Day::ALL
                    .into_iter()
                    .zip(self.days.get())
                    .filter(|(_, on)| *on)
                    .map(|(day, _)| day)
                    .collect(),
                hour: u8::try_from(number(self.hour, "hour")?).unwrap_or(u8::MAX),
                minute: u8::try_from(number(self.minute, "minute")?).unwrap_or(u8::MAX),
            },
        };
        schedule.checked().map_err(|err| format!("{err:#}"))
    }
}

/// A small toggle, highlighted while it is on.
fn chip(
    label: impl Fn() -> String + 'static,
    on: impl Fn() -> bool + 'static,
    click: impl Fn() + 'static,
) -> impl IntoView {
    Label::derived(label)
        .style(move |s| {
            let on = on();
            s.padding_horiz(9)
                .padding_vert(4)
                .border(1)
                .border_radius(6)
                .font_size(12)
                .cursor(CursorStyle::Pointer)
                .border_color(if on { color::accent() } else { color::border() })
                .color(if on { color::text() } else { color::muted() })
                .hover(|s| s.color(color::text()))
                .apply_if(on, |s| s.background(color::highlight()))
        })
        .on_event_stop(el::Click, move |_, _| click())
}

fn small_field(value: RwSignal<String>, width: f64) -> impl IntoView {
    TextInput::new(value).style(move |s| {
        s.width(width)
            .padding_horiz(8)
            .padding_vert(4)
            .border(1)
            .border_color(color::border())
            .border_radius(6)
            .background(color::bg())
            .color(color::text())
            .focus(|s| s.border_color(color::accent()))
    })
}

/// When a task runs: described in words for Pi to fill in, and the
/// controls that say exactly when, to check and change.
fn schedule_editor(
    tasks: Rc<Tasks>,
    controls: Controls,
    folder: RwSignal<String>,
) -> impl IntoView {
    const DAYS: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];
    let description = RwSignal::new(String::new());
    let asking = RwSignal::new(false);
    // What Pi made of the description, or why it could not; `true` if wrong.
    let note = RwSignal::new(None::<(String, bool)>);
    let fill = Rc::new(move || {
        let text = description.get_untracked();
        if asking.get_untracked() || text.trim().is_empty() {
            return;
        }
        asking.set(true);
        note.set(None);
        tasks.fill_schedule(&text, &folder.get_untracked(), move |result| {
            asking.set(false);
            match result {
                Ok(schedule) => {
                    controls.set(&schedule);
                    note.set(Some((
                        format!(
                            "Pi set this to \u{201c}{}\u{201d}. Check it below and change anything it got wrong.",
                            schedule.describe()
                        ),
                        false,
                    )));
                }
                Err(err) => note.set(Some((err, true))),
            }
        });
    });
    let enter = fill.clone();
    let describe = Stack::horizontal((
        TextInput::new(description)
            .placeholder("Describe it, like \u{201c}every weekday morning at 9\u{201d}")
            .style(|s| {
                s.flex_grow(1.0_f32)
                    .min_width(0)
                    .padding_horiz(10)
                    .padding_vert(7)
                    .border(1)
                    .border_color(color::border())
                    .border_radius(6)
                    .background(color::bg())
                    .color(color::text())
                    .focus(|s| s.border_color(color::accent()))
                    .class(PlaceholderTextClass, |s| s.color(color::muted()))
            })
            .on_event_stop(TextInputEnter::listener(), move |_, _| enter()),
        Label::derived(move || {
            if asking.get() {
                "Asking Pi\u{2026}"
            } else {
                "Fill in"
            }
            .to_string()
        })
        .style(move |s| {
            s.padding_horiz(10)
                .padding_vert(7)
                .border_radius(6)
                .flex_shrink(0.0_f32)
                .color(color::accent())
                .cursor(CursorStyle::Pointer)
                .hover(|s| s.background(color::border()))
                .apply_if(asking.get(), |s| s.color(color::muted()))
        })
        .on_event_stop(el::Click, move |_, _| fill()),
    ))
    .style(|s| s.width_full().gap(8).items_center());
    let note_view = Label::derived(move || note.get().map(|(text, _)| text).unwrap_or_default())
        .style(move |s| {
            let wrong = note.get().is_some_and(|(_, wrong)| wrong);
            s.font_size(11)
                .text_wrap()
                .color(if wrong {
                    color::removed_text()
                } else {
                    color::accent()
                })
                .apply_if(note.get().is_none(), |s| s.hide())
        });
    let kind = controls.kind;
    let kinds = Stack::horizontal(
        [
            ("By hand", Kind::Manual),
            ("Every", Kind::Every),
            ("At a time", Kind::At),
        ]
        .map(|(label, choice)| {
            chip(
                move || label.to_string(),
                move || kind.get() == choice,
                move || kind.set(choice),
            )
        }),
    )
    .style(|s| s.gap(6).items_center());
    let (amount, hours) = (controls.amount, controls.hours);
    let every = Stack::horizontal((
        Label::new("Every").style(|s| s.color(color::muted())),
        small_field(amount, 70.0),
        chip(
            || "minutes".into(),
            move || !hours.get(),
            move || hours.set(false),
        ),
        chip(
            || "hours".into(),
            move || hours.get(),
            move || hours.set(true),
        ),
    ))
    .style(move |s| {
        s.gap(8)
            .items_center()
            .apply_if(kind.get() != Kind::Every, |s| s.hide())
    });
    let (days, hour, minute) = (controls.days, controls.hour, controls.minute);
    let preset = |label: &'static str, set: [bool; 7]| {
        chip(
            move || label.to_string(),
            move || days.get() == set,
            move || days.set(set),
        )
    };
    let at = Stack::vertical((
        Stack::horizontal((
            Stack::horizontal(std::array::from_fn::<_, 7, _>(|index| {
                chip(
                    move || DAYS[index].to_string(),
                    move || days.get()[index],
                    move || days.update(|days| days[index] = !days[index]),
                )
            }))
            .style(|s| s.gap(4)),
            Label::new("at").style(|s| s.color(color::muted()).margin_left(6)),
            small_field(hour, 44.0),
            Label::new(":").style(|s| s.color(color::muted())),
            small_field(minute, 44.0),
        ))
        .style(|s| s.gap(6).items_center()),
        Stack::horizontal((
            preset("Every day", [true; 7]),
            preset("Weekdays", [true, true, true, true, true, false, false]),
            preset("Weekends", [false, false, false, false, false, true, true]),
        ))
        .style(|s| s.gap(6).items_center()),
    ))
    .style(move |s| s.gap(8).apply_if(kind.get() != Kind::At, |s| s.hide()));
    Stack::vertical((describe, note_view, kinds, every, at)).style(|s| s.width_full().gap(10))
}

/// A task's settings, or a new task's.
fn task_form(tasks: Rc<Tasks>, id: Option<u64>) -> impl IntoView {
    let task = id.and_then(|id| tasks.task_untracked(id));
    let draft = task.as_ref().map_or_else(
        || Draft {
            folder: tasks.default_folder(),
            schedule: Schedule::At {
                days: Day::ALL.to_vec(),
                hour: 9,
                minute: 0,
            },
            ..Draft::default()
        },
        |task| Draft {
            name: task.name.clone(),
            folder: task.folder.display().to_string(),
            prompt: task.prompt.clone(),
            schedule: task.schedule.clone(),
            continuing: task.continuing,
        },
    );
    let name = RwSignal::new(draft.name);
    let folder = RwSignal::new(draft.folder);
    let controls = Controls::new(&draft.schedule);
    let continuing = RwSignal::new(draft.continuing);
    let error = RwSignal::new(None::<String>);
    let prompt = text_editor(draft.prompt)
        .styling(SimpleStyling::builder().font_size(13).build())
        .editor_style(super::editor_colors)
        .editor_style(|s| {
            s.wrap_method(WrapMethod::EditorWidth)
                .hide_gutter(true)
                .current_line_color(color::editor_bg())
        })
        .style(|s| {
            super::editor_surface(
                s.width_full()
                    .height(150)
                    .border(1)
                    .border_color(color::border())
                    .border_radius(6),
            )
        });
    let prompt_doc = prompt.editor().doc();
    // The schedule is what the controls say, or why they say none.
    let read = Rc::new(move || {
        let schedule = Effect::untrack(|| controls.schedule())?;
        Ok::<_, String>(Draft {
            name: name.get_untracked(),
            folder: folder.get_untracked(),
            prompt: prompt_doc.text().to_string(),
            schedule,
            continuing: continuing.get_untracked(),
        })
    });
    // In words, with when it would run next; or what is wrong.
    let hint_tasks = tasks.clone();
    let hint = Label::derived(move || match controls.schedule() {
        Err(err) => err,
        Ok(Schedule::Manual) => "Only when run by hand, with Run now".into(),
        Ok(schedule) => {
            let now = hint_tasks.now();
            let next = schedule
                .next_after(now, hint_tasks.time_zone())
                .map(|next| tasks::describe(next, now, hint_tasks.time_zone()))
                .unwrap_or_default();
            format!("{}  \u{b7}  next {next}", schedule.describe())
        }
    })
    .style(move |s| {
        let wrong = controls.schedule().is_err();
        s.font_size(12).color(if wrong {
            color::removed_text()
        } else {
            color::text()
        })
    });
    let (save_tasks, save_read) = (tasks.clone(), read.clone());
    let save = move || match save_read().and_then(|draft| save_tasks.save_draft(id, &draft)) {
        Ok(_) => error.set(None),
        Err(err) => error.set(Some(err)),
    };
    let run_save = save.clone();
    let (run_tasks, pause_tasks, label_tasks, delete_tasks) =
        (tasks.clone(), tasks.clone(), tasks.clone(), tasks.clone());
    let paused = move || {
        id.and_then(|id| label_tasks.task(id))
            .is_some_and(|t| t.paused)
    };
    let pause_label = paused.clone();
    let confirm_delete = RwSignal::new(false);
    let existing = id.is_some();
    let actions = Stack::horizontal((
        accent_button("Save", save),
        button("Run now", move || {
            // What is written is what runs.
            run_save();
            if error.get_untracked().is_none()
                && let Some(Pick::Task(id)) = run_tasks.picked.get_untracked()
            {
                run_tasks.run_now(id);
            }
        }),
        Label::derived(move || if pause_label() { "Resume" } else { "Pause" }.to_string())
            .style(move |s| {
                s.padding_horiz(10)
                    .padding_vert(5)
                    .border_radius(6)
                    .color(color::muted())
                    .cursor(CursorStyle::Pointer)
                    .hover(|s| s.background(color::border()).color(color::text()))
                    .apply_if(!existing, |s| s.hide())
            })
            .on_event_stop(el::Click, move |_, _| {
                if let Some(id) = id {
                    pause_tasks.set_paused(id, !paused());
                }
            }),
        Empty::new().style(|s| s.flex_grow(1.0_f32)),
        Label::derived(move || {
            if confirm_delete.get() {
                "Delete it? Its sessions stay in Pi"
            } else {
                "Delete"
            }
            .to_string()
        })
        .style(move |s| {
            s.padding_horiz(10)
                .padding_vert(5)
                .border_radius(6)
                .color(color::removed_text())
                .cursor(CursorStyle::Pointer)
                .hover(|s| s.background(color::border()))
                .apply_if(!existing, |s| s.hide())
        })
        .on_event_stop(el::Click, move |_, _| {
            if !confirm_delete.get_untracked() {
                confirm_delete.set(true);
            } else if let Some(id) = id {
                delete_tasks.delete(id);
            }
        }),
    ))
    .style(|s| s.width_full().gap(8).items_center().margin_top(16));
    Stack::vertical((
        Label::new(if existing { "Task" } else { "New task" })
            .style(|s| s.font_size(16).font_bold().color(color::text())),
        field_label("Name"),
        text_field(name, "Named after the prompt if left empty"),
        field_label("Folder"),
        text_field(folder, "Where Pi works"),
        field_label("When"),
        schedule_editor(tasks.clone(), controls, folder),
        hint,
        field_label("Each run"),
        Stack::horizontal((
            chip(
                || "New conversation".into(),
                move || !continuing.get(),
                move || continuing.set(false),
            ),
            chip(
                || "Continue one conversation".into(),
                move || continuing.get(),
                move || continuing.set(true),
            ),
        ))
        .style(|s| s.gap(6).items_center()),
        Label::derived(move || {
            if continuing.get() {
                "Every run adds to the same conversation, so you can ask about earlier runs too. If Pi is busy in it when a run comes due, the run waits its turn."
            } else {
                "Every run starts a conversation of its own."
            }
            .to_string()
        })
        .style(|s| s.color(color::muted()).font_size(11).text_wrap()),
        field_label("What Pi should do"),
        prompt,
        Label::derived(move || error.get().unwrap_or_default()).style(move |s| {
            s.color(color::removed_text())
                .font_size(12)
                .margin_top(8)
                .apply_if(error.get().is_none(), |s| s.hide())
        }),
        actions,
        Label::new("Tasks run while the workbench is open; one that came due while it was closed runs once when it opens.")
            .style(|s| {
                s.color(color::muted())
                    .font_size(11)
                    .margin_top(16)
                    .line_height(1.5)
                    .text_wrap()
            }),
    ))
    .style(|s| {
        s.width_full()
            .max_width(640)
            .gap(6)
            .padding(28)
            .margin_horiz(floem::unit::LengthAuto::Auto)
    })
    .scroll()
    .style(|s| s.size_full().min_width(0))
}

/// A run: what it was and how it went, above its conversation.
fn run_view(tasks: Rc<Tasks>, task: u64, run: u64) -> impl IntoView {
    let record = move |tasks: &Tasks| {
        tasks
            .task(task)
            .and_then(|t| Some((t.clone(), t.run(run)?.clone())))
    };
    let (title_tasks, pill_tasks, color_tasks, review_tasks, review_style, body_tasks) = (
        tasks.clone(),
        tasks.clone(),
        tasks.clone(),
        tasks.clone(),
        tasks.clone(),
        tasks,
    );
    let header = Stack::horizontal((
        Label::derived(move || {
            let Some((task, record)) = record(&title_tasks) else {
                return String::new();
            };
            let started = Timestamp::from_second(record.started).unwrap_or_default();
            format!(
                "{}  \u{b7}  {}  \u{b7}  {}",
                task.name,
                tasks::describe(started, title_tasks.now(), title_tasks.time_zone()),
                folder_name(&task.folder)
            )
        })
        .style(|s| s.color(color::text()).min_width(0).text_ellipsis()),
        pill(
            move || {
                record(&pill_tasks)
                    .map(|(_, run)| run_label(pill_tasks.run_state(&run)))
                    .unwrap_or_default()
                    .to_string()
            },
            move || {
                record(&color_tasks).map_or(color::muted(), |(_, run)| {
                    run_color(color_tasks.run_state(&run))
                })
            },
            1.0,
        ),
        Empty::new().style(|s| s.flex_grow(1.0_f32)),
        button("Review in code view", move || review_tasks.review(run)).style(move |s| {
            s.color(color::accent())
                .apply_if(review_style.session(run).is_none(), |s| s.hide())
        }),
    ))
    .style(|s| {
        s.width_full()
            .height(38)
            .padding_horiz(14)
            .gap(10)
            .items_center()
            .flex_shrink(0.0_f32)
            .border_bottom(1)
            .border_color(color::border())
            .background(color::panel())
    });
    let visible = Memo::new(|_| true);
    // Made again only when the run's session itself changes, not whenever
    // another one opens somewhere.
    let key_tasks = body_tasks.clone();
    let key = Memo::new(move |_| key_tasks.session(run).map(|(_, session)| session.key));
    let body = dyn_container(
        move || key.get(),
        move |_| match body_tasks.session_untracked(run) {
            Some((state, session)) => {
                // In a conversation the task keeps, where this run starts.
                let jump = body_tasks
                    .task_untracked(task)
                    .filter(|task| task.continuing)
                    .and_then(|task| task.run(run).and_then(|run| run.turn));
                session_view(state, session, visible, 1.0, jump).into_any()
            }
            None => {
                let tasks = body_tasks.clone();
                Label::derived(move || {
                    if tasks.opening(run) {
                        return "Opening the conversation\u{2026}".to_string();
                    }
                    match record(&tasks) {
                        Some((_, run)) if run.error.is_some() => run.error.unwrap_or_default(),
                        Some((_, run)) if run.session.is_none() => {
                            "This run left no conversation".to_string()
                        }
                        _ => "The conversation could not be opened".to_string(),
                    }
                })
                .style(|s| s.color(color::muted()).padding(28).text_wrap())
                .into_any()
            }
        },
    )
    .style(|s| s.size_full().min_height(0));
    Stack::vertical((header, body)).style(|s| s.size_full().min_width(0))
}

/// Whatever is picked, on the right.
fn detail(tasks: Rc<Tasks>) -> impl IntoView {
    let picked_tasks = tasks.clone();
    dyn_container(
        move || picked_tasks.picked.get(),
        move |picked| match picked {
            None => Stack::vertical((
                Label::new(
                    "Pick a task to change it, or a run to read and carry on its conversation.",
                )
                .style(|s| s.color(color::muted()).text_wrap()),
                accent_button("New task", {
                    let tasks = tasks.clone();
                    move || tasks.picked.set(Some(Pick::New))
                }),
            ))
            .style(|s| {
                s.gap(14)
                    .padding(28)
                    .padding_top(80)
                    .items_center()
                    .width_full()
            })
            .into_any(),
            Some(Pick::New) => task_form(tasks.clone(), None).into_any(),
            Some(Pick::Task(id)) => task_form(tasks.clone(), Some(id)).into_any(),
            Some(Pick::Run { task, run }) => run_view(tasks.clone(), task, run).into_any(),
        },
    )
    .style(|s| s.size_full().min_width(0).min_height(0))
}

/// The tasks view, in place of the sessions rail, explorer and panes.
pub(super) fn tasks_view(app: Rc<Window>) -> impl IntoView {
    let tasks = app.tasks.clone();
    Stack::horizontal((task_list(tasks.clone()), detail(tasks))).style(|s| {
        s.size_full()
            .min_width(0)
            .min_height(0)
            .background(color::bg())
    })
}
