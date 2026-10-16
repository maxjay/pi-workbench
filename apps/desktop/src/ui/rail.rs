//! The sessions rail: the sessions you opened, by the folder they work in.
//! Sessions from before wait, Pi not running, until picked; closing one
//! takes it off the list and leaves it in Pi's store. Picking one anywhere
//! brings its folder forward in the window.
use super::{color, session::status_color};
use crate::{
    panes::Tab,
    session::{Session, Status},
    window::Window,
    workspace::{MenuItem, Workbench},
};
use floem::{prelude::*, style::CursorStyle};
use pi_client::sessions::SessionSummary;
use std::{
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, SystemTime},
};

/// Past sessions offered to open, per menu.
const PAST: usize = 12;

/// A folder's open and closed marks, drawn rather than typed: not every
/// interface font has the arrows.
pub(super) const CHEVRON_OPEN: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="M2 3.5 L5 6.5 L8 3.5" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/></svg>"#;
const CHEVRON_CLOSED: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="M3.5 2 L6.5 5 L3.5 8" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round" stroke-linejoin="round"/></svg>"#;

fn age(modified: SystemTime) -> String {
    let seconds = SystemTime::now()
        .duration_since(modified)
        .map(|d| d.as_secs())
        .unwrap_or_default();
    match seconds {
        0..60 => "now".into(),
        60..3600 => format!("{}m", seconds / 60),
        3600..86400 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86400),
    }
}

fn folder_name(folder: &Path) -> String {
    folder.file_name().map_or_else(
        || folder.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Runs `act` once the click that asked for it is over: it may rebuild the
/// rail, the clicked row included.
fn after_click(act: impl FnOnce() + 'static) {
    floem::action::exec_after(Duration::ZERO, move |_| act());
}

/// `items`, then a loaded folder's past sessions to open, newest first.
fn with_past_sessions(state: &Rc<Workbench>, mut items: Vec<MenuItem>) -> Vec<MenuItem> {
    let past: Vec<SessionSummary> = state.sessions.recent().into_iter().take(PAST).collect();
    if !past.is_empty() {
        items.push(MenuItem::separator());
        for summary in past {
            let state = state.clone();
            let hint = age(summary.modified);
            let title = summary.title.clone();
            items.push(
                MenuItem::new(title, move || {
                    let session = state.sessions.resume(&summary);
                    state.show_session(session.key);
                })
                .hint(hint),
            );
        }
    }
    items
}

fn close_mark(close: impl Fn() + 'static) -> impl IntoView {
    floem::views::tooltip(
        Label::new("×")
            .style(|s| {
                s.color(color::muted())
                    .padding_horiz(4)
                    .cursor(CursorStyle::Pointer)
                    .hover(|s| s.color(color::text()))
            })
            .on_event_stop(el::Click, move |_, _| close()),
        || Label::new("Close; it stays in Pi's store").style(|s| s.padding(4).font_size(11)),
    )
}

fn open_row(app: Rc<Window>, state: Rc<Workbench>, session: Rc<Session>) -> impl IntoView {
    let key = session.key;
    let title = session.title();
    let running = session.running();
    let active = state.sessions.active_key();
    let status_session = session.clone();
    let folder = state.root().to_path_buf();
    let shown = {
        let (app, folder) = (app.clone(), folder.clone());
        move || app.active().with(|shown| *shown == folder)
    };
    let row_shown = shown.clone();
    let (select_state, close_state) = (state.clone(), state);
    Stack::horizontal((
        // Waiting since last time: a ring, until Pi is started on it.
        Empty::new().style(move |s| {
            let s = s.size(8, 8).border_radius(4).flex_shrink(0.0_f32);
            let status = status_session.status();
            if running.get() || status != Status::Idle {
                s.background(status_color(status))
            } else {
                s.border(1).border_color(color::muted())
            }
        }),
        Label::derived(move || title.get()).style(move |s| {
            s.color(if running.get() {
                color::text()
            } else {
                color::muted()
            })
            .flex_grow(1.0_f32)
            .min_width(0)
            .text_ellipsis()
        }),
        close_mark(move || {
            let state = close_state.clone();
            after_click(move || state.close_session(key));
        }),
    ))
    .style(move |s| {
        let selected = row_shown() && active.get() == Some(key);
        s.width_full()
            .height(32)
            .gap(8)
            .padding_left(26)
            .padding_right(8)
            .items_center()
            .cursor(CursorStyle::Pointer)
            .border_left(2)
            .border_color(Color::TRANSPARENT)
            .hover(|s| s.background(color::border().with_alpha(0.5)))
            .apply_if(selected, |s| {
                s.background(color::border()).border_color(color::accent())
            })
    })
    // A session of the folder in front can be dragged into a pane, or onto
    // its edge to split it.
    .on_event_cont(el::PointerDown, {
        let state = select_state.clone();
        move |_, event| {
            if shown()
                && event
                    .button
                    .is_none_or(|b| b == floem::ui_events::pointer::PointerButton::Primary)
            {
                state.press_tab(None, Tab::Session(key))
            }
        }
    })
    .on_event_stop(el::Click, move |_, _| {
        let (app, folder) = (app.clone(), folder.clone());
        after_click(move || app.show_session(&folder, key));
    })
}

/// A session open in a folder not loaded yet, as it was left.
fn stored_row(app: Rc<Window>, folder: PathBuf, summary: SessionSummary) -> impl IntoView {
    let title = summary.title.clone();
    let (forget_app, forget_folder, forget_path) =
        (app.clone(), folder.clone(), summary.path.clone());
    Stack::horizontal((
        Empty::new().style(|s| {
            s.size(8, 8)
                .border_radius(4)
                .flex_shrink(0.0_f32)
                .border(1)
                .border_color(color::muted())
        }),
        Label::new(title).style(|s| {
            s.color(color::muted())
                .flex_grow(1.0_f32)
                .min_width(0)
                .text_ellipsis()
        }),
        close_mark(move || {
            let (app, folder, path) = (
                forget_app.clone(),
                forget_folder.clone(),
                forget_path.clone(),
            );
            after_click(move || app.forget(&folder, &path));
        }),
    ))
    .style(|s| {
        s.width_full()
            .height(32)
            .gap(8)
            .padding_left(26)
            .padding_right(8)
            .items_center()
            .cursor(CursorStyle::Pointer)
            .hover(|s| s.background(color::border().with_alpha(0.5)))
    })
    .on_event_stop(el::Click, move |_, _| {
        let (app, folder, summary) = (app.clone(), folder.clone(), summary.clone());
        after_click(move || app.resume(&folder, summary));
    })
}

/// A folder's sessions: those open in it, or, before it loads, those it
/// had open.
fn folder_sessions(app: Rc<Window>, folder: PathBuf) -> impl IntoView {
    let (open_app, open_folder) = (app.clone(), folder.clone());
    let open = dyn_stack(
        move || {
            open_app
                .workspace_tracked(&open_folder)
                .map(|state| {
                    state
                        .sessions
                        .open()
                        .get()
                        .into_iter()
                        .map(|session| (state.clone(), session))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        },
        |(_, session)| session.key,
        {
            let app = app.clone();
            move |(state, session)| open_row(app.clone(), state, session)
        },
    )
    .style(|s| s.flex_col().width_full());
    let (stored_app, stored_folder) = (app.clone(), folder.clone());
    let stored = dyn_stack(
        move || {
            if stored_app.workspace_tracked(&stored_folder).is_some() {
                Vec::new()
            } else {
                stored_app.stored(&stored_folder)
            }
        },
        |summary| summary.path.clone(),
        move |summary| stored_row(app.clone(), folder.clone(), summary),
    )
    .style(|s| s.flex_col().width_full());
    Stack::vertical((open, stored)).style(|s| s.width_full())
}

/// A folder's menu: a new session there, its past sessions once it has
/// loaded, and closing it.
fn folder_menu(app: Rc<Window>, folder: PathBuf) {
    let Some(shown) = app.shown() else {
        return;
    };
    let (new_app, new_folder) = (app.clone(), folder.clone());
    let mut items = vec![MenuItem::new("New session here", move || {
        let (app, folder) = (new_app.clone(), new_folder.clone());
        after_click(move || {
            app.go(&folder, Some(String::new()));
        });
    })];
    if let Some(state) = app.workspace(&folder) {
        items = with_past_sessions(&state, items);
    }
    items.push(MenuItem::separator());
    let (close_app, close_folder) = (app.clone(), folder.clone());
    items.push(MenuItem::new("Close folder", move || {
        let (app, folder) = (close_app.clone(), close_folder.clone());
        after_click(move || app.close_folder(&folder));
    }));
    shown.open_menu(shown.pointer(), items);
}

fn folder_group(app: Rc<Window>, folder: PathBuf) -> impl IntoView {
    let name = folder_name(&folder);
    let expanded = {
        let (app, folder) = (app.clone(), folder.clone());
        move || app.collapsed.with(|folded| !folded.contains(&folder))
    };
    let shown = {
        let (app, folder) = (app.clone(), folder.clone());
        move || app.active().with(|shown| *shown == folder)
    };
    let chevron_open = expanded.clone();
    let (menu_app, menu_folder) = (app.clone(), folder.clone());
    let (toggle_app, toggle_folder) = (app.clone(), folder.clone());
    let header = Stack::horizontal((
        floem::views::svg(move || {
            if chevron_open() {
                CHEVRON_OPEN
            } else {
                CHEVRON_CLOSED
            }
            .to_string()
        })
        .style(|s| s.size(10, 10).color(color::muted()).flex_shrink(0.0_f32)),
        Label::new(name).style(move |s| {
            s.color(if shown() {
                color::text()
            } else {
                color::muted()
            })
            .font_bold()
            .flex_grow(1.0_f32)
            .min_width(0)
            .text_ellipsis()
        }),
        floem::views::tooltip(
            Label::new("+")
                .style(|s| {
                    s.color(color::accent())
                        .font_size(14)
                        .padding_horiz(6)
                        .border_radius(5)
                        .cursor(CursorStyle::Pointer)
                        .hover(|s| s.background(color::border()))
                })
                .on_event_stop(el::Click, move |_, _| {
                    folder_menu(menu_app.clone(), menu_folder.clone())
                }),
            || {
                Label::new("New session here, or close the folder")
                    .style(|s| s.padding(4).font_size(11))
            },
        ),
    ))
    .style(|s| {
        s.width_full()
            .height(30)
            .gap(6)
            .padding_left(12)
            .padding_right(8)
            .items_center()
            .cursor(CursorStyle::Pointer)
            .hover(|s| s.background(color::border().with_alpha(0.5)))
    })
    .on_event_stop(el::Click, move |_, _| {
        let folder = toggle_folder.clone();
        toggle_app.collapsed.update(|folded| {
            if !folded.remove(&folder) {
                folded.insert(folder);
            }
        });
    });
    let (body_app, body_folder) = (app, folder);
    Stack::vertical((
        header,
        dyn_container(expanded, move |open| {
            if open {
                folder_sessions(body_app.clone(), body_folder.clone()).into_any()
            } else {
                Empty::new().into_any()
            }
        })
        .style(|s| s.width_full()),
    ))
    .style(|s| s.width_full().padding_bottom(4))
}

pub(super) fn rail(state: Rc<Workbench>, app: Rc<Window>) -> impl IntoView {
    let new_state = state.clone();
    let folders_app = app.clone();
    Stack::vertical((
        Stack::horizontal((
            Label::new("Sessions").style(|s| {
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
                .on_event_stop(el::Click, move |_, _| {
                    // A new session, a past one of this folder, or another
                    // folder.
                    let state = new_state.clone();
                    let (fresh, folder) = (state.clone(), state.clone());
                    let items = vec![
                        MenuItem::new("New session", move || fresh.new_session()),
                        MenuItem::new("Open folder…", move || folder.pick_folder()),
                    ];
                    let items = with_past_sessions(&state, items);
                    state.open_menu(state.pointer(), items);
                }),
        ))
        .style(|s| {
            s.width_full()
                .height(42)
                .padding_left(14)
                .padding_right(8)
                .items_center()
                .flex_shrink(0.0_f32)
        }),
        dyn_stack(
            move || folders_app.folders(),
            |folder| folder.clone(),
            move |folder| folder_group(app.clone(), folder),
        )
        .style(|s| s.flex_col().width_full())
        .scroll()
        .style(|s| s.width_full().flex_grow(1.0_f32).min_height(0)),
    ))
    .style(|s| {
        s.width(240)
            .height_full()
            .flex_shrink(0.0_f32)
            .background(color::panel())
            .border_right(1)
            .border_color(color::border())
    })
}
