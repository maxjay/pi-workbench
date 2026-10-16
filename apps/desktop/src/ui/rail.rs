//! The sessions rail: Pi sessions by the folder they work in, open ones to
//! switch between and recent ones to resume. Picking one anywhere brings its
//! folder forward in the window.
use super::{color, session::status_color};
use crate::{panes::Tab, session::Session, window::Window, workspace::Workbench};
use floem::{prelude::*, style::CursorStyle};
use pi_client::sessions::SessionSummary;
use std::{
    path::{Path, PathBuf},
    rc::Rc,
    time::SystemTime,
};

/// Recent sessions shown per folder before "Show all".
const RECENT: usize = 5;

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

fn open_row(
    app: Rc<Window>,
    state: Rc<Workbench>,
    session: Rc<Session>,
    index: usize,
) -> impl IntoView {
    let key = session.key;
    let title = session.title();
    let active = state.sessions.active_key();
    let status_session = session.clone();
    let folder = state.root().to_path_buf();
    let shown = {
        let (app, folder) = (app.clone(), folder.clone());
        move || app.active().with(|shown| *shown == folder)
    };
    let number_shown = shown.clone();
    let row_shown = shown.clone();
    let (select_state, close_state) = (state.clone(), state);
    Stack::horizontal((
        Empty::new().style(move |s| {
            s.size(8, 8)
                .border_radius(4)
                .flex_shrink(0.0_f32)
                .background(status_color(status_session.status()))
        }),
        Label::derived(move || title.get()).style(|s| {
            s.color(color::text())
                .flex_grow(1.0_f32)
                .min_width(0)
                .text_ellipsis()
        }),
        // Ctrl+1..9 pick the front folder's sessions.
        Label::derived(move || {
            if index < 9 && number_shown() {
                format!("{}", index + 1)
            } else {
                String::new()
            }
        })
        .style(|s| s.color(color::muted()).font_size(10).flex_shrink(0.0_f32)),
        Label::new("×")
            .style(|s| {
                s.color(color::muted())
                    .padding_horiz(4)
                    .cursor(CursorStyle::Pointer)
                    .hover(|s| s.color(color::text()))
            })
            .on_event_stop(el::Click, move |_, _| close_state.close_session(key)),
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
    .on_event_stop(el::Click, move |_, _| app.show_session(&folder, key))
}

fn recent_row(app: Rc<Window>, folder: PathBuf, summary: SessionSummary) -> impl IntoView {
    let when = age(summary.modified);
    let title = summary.title.clone();
    Stack::horizontal((
        Label::new(title).style(|s| {
            s.color(color::muted())
                .flex_grow(1.0_f32)
                .min_width(0)
                .text_ellipsis()
        }),
        Label::new(when).style(|s| s.color(color::muted()).font_size(10).flex_shrink(0.0_f32)),
    ))
    .style(|s| {
        s.width_full()
            .height(28)
            .gap(8)
            .padding_left(26)
            .padding_right(14)
            .items_center()
            .cursor(CursorStyle::Pointer)
            .hover(|s| {
                s.background(color::border().with_alpha(0.5))
                    .color(color::text())
            })
    })
    .on_event_stop(el::Click, move |_, _| app.resume(&folder, summary.clone()))
}

/// A folder's open sessions, then its recent ones (a few, or all).
fn folder_sessions(app: Rc<Window>, folder: PathBuf) -> impl IntoView {
    let show_all = RwSignal::new(false);
    let (open_app, open_folder) = (app.clone(), folder.clone());
    let open = dyn_stack(
        move || {
            open_app
                .workspace_tracked(&open_folder)
                .map(|state| {
                    let sessions = state.sessions.open().get();
                    sessions
                        .into_iter()
                        .enumerate()
                        .map(|(index, session)| (index, state.clone(), session))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        },
        |(index, _, session)| (*index, session.key),
        {
            let app = app.clone();
            move |(index, state, session)| open_row(app.clone(), state, session, index)
        },
    )
    .style(|s| s.flex_col().width_full());
    // An open folder knows its own; others are read when first shown.
    // Without sessions open anywhere in the window, which can be one Pi
    // moved from here.
    let recent = {
        let (app, folder) = (app.clone(), folder.clone());
        move || {
            let open = app.open_files();
            match app.workspace_tracked(&folder) {
                Some(state) => state.sessions.recent(),
                None => app.stored(&folder).unwrap_or_default(),
            }
            .into_iter()
            .filter(|summary| !open.contains(&summary.path))
            .collect::<Vec<_>>()
        }
    };
    let (count, hide_more) = (recent.clone(), recent.clone());
    let (rows_app, rows_folder) = (app.clone(), folder.clone());
    let recent_rows = dyn_stack(
        move || {
            let all = recent();
            let shown = if show_all.get() { all.len() } else { RECENT };
            all.into_iter().take(shown).collect::<Vec<_>>()
        },
        |summary| summary.path.clone(),
        move |summary| recent_row(rows_app.clone(), rows_folder.clone(), summary),
    )
    .style(|s| s.flex_col().width_full());
    let more = Label::derived(move || {
        if show_all.get() {
            "Show fewer".to_string()
        } else {
            format!("Show all {}", count().len())
        }
    })
    .style(move |s| {
        s.color(color::accent())
            .font_size(11)
            .padding_left(26)
            .padding_vert(4)
            .cursor(CursorStyle::Pointer)
            .apply_if(hide_more().len() <= RECENT, |s| s.hide())
    })
    .on_event_stop(el::Click, move |_, _| show_all.update(|all| *all = !*all));
    Stack::vertical((open, recent_rows, more)).style(|s| s.width_full())
}

fn folder_group(app: Rc<Window>, folder: PathBuf) -> impl IntoView {
    let name = folder_name(&folder);
    let expanded = {
        let (app, folder) = (app.clone(), folder.clone());
        move || app.expanded.with(|open| open.contains(&folder))
    };
    let shown = {
        let (app, folder) = (app.clone(), folder.clone());
        move || app.active().with(|shown| *shown == folder)
    };
    let (when_app, when_folder) = (app.clone(), folder.clone());
    let chevron_open = expanded.clone();
    let name_shown = shown.clone();
    let (new_app, new_folder) = (app.clone(), folder.clone());
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
            s.color(if name_shown() {
                color::text()
            } else {
                color::muted()
            })
            .font_bold()
            .flex_grow(1.0_f32)
            .min_width(0)
            .text_ellipsis()
        }),
        Label::derived(move || {
            when_app
                .last_used(&when_folder)
                .map(age)
                .unwrap_or_default()
        })
        .style(|s| s.color(color::muted()).font_size(10).flex_shrink(0.0_f32)),
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
                    new_app.go(&new_folder, Some(String::new()));
                }),
            || Label::new("New session here").style(|s| s.padding(4).font_size(11)),
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
        let opening = !toggle_app
            .expanded
            .with_untracked(|open| open.contains(&folder));
        if opening
            && toggle_app.workspace(&folder).is_none()
            && toggle_app.stored(&folder).is_none()
        {
            toggle_app.load_stored(folder.clone());
        }
        toggle_app.expanded.update(|open| {
            if !open.remove(&folder) {
                open.insert(folder);
            }
        });
    });
    let body_app = app.clone();
    let body_folder = folder.clone();
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
                    new_state.new_session();
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
