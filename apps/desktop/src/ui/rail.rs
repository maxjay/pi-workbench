//! The sessions rail: open Pi sessions to tab between, and recent ones to resume.
use super::{ACCENT, BORDER, MUTED, PANEL, TEXT, session::status_color};
use crate::{session::Session, workspace::Workbench};
use floem::{prelude::*, style::CursorStyle};
use pi_client::sessions::SessionSummary;
use std::{rc::Rc, time::SystemTime};

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

fn heading(text: &'static str) -> impl IntoView {
    Label::new(text).style(|s| {
        s.color(MUTED)
            .font_size(10)
            .font_bold()
            .padding_horiz(14)
            .padding_top(14)
            .padding_bottom(6)
    })
}

fn open_row(state: Rc<Workbench>, session: Rc<Session>, index: usize) -> impl IntoView {
    let key = session.key;
    let title = session.title();
    let active = state.sessions.active_key();
    let status_session = session.clone();
    let (select_state, close_state) = (state.clone(), state);
    Stack::horizontal((
        Empty::new().style(move |s| {
            s.size(8, 8)
                .border_radius(4)
                .flex_shrink(0.0_f32)
                .background(status_color(status_session.status()))
        }),
        Label::derived(move || title.get()).style(|s| {
            s.color(TEXT)
                .flex_grow(1.0_f32)
                .min_width(0)
                .text_ellipsis()
        }),
        Label::new(if index < 9 {
            format!("{}", index + 1)
        } else {
            String::new()
        })
        .style(|s| s.color(MUTED).font_size(10).flex_shrink(0.0_f32)),
        Label::new("×")
            .style(|s| {
                s.color(MUTED)
                    .padding_horiz(4)
                    .cursor(CursorStyle::Pointer)
                    .hover(|s| s.color(TEXT))
            })
            .on_event_stop(el::Click, move |_, _| close_state.sessions.close(key)),
    ))
    .style(move |s| {
        let selected = active.get() == Some(key);
        s.width_full()
            .height(34)
            .gap(8)
            .padding_left(14)
            .padding_right(8)
            .items_center()
            .cursor(CursorStyle::Pointer)
            .border_left(2)
            .border_color(Color::TRANSPARENT)
            .hover(|s| s.background(BORDER.with_alpha(0.5)))
            .apply_if(selected, |s| s.background(BORDER).border_color(ACCENT))
    })
    .on_event_stop(el::Click, move |_, _| {
        select_state.sessions.select(key);
        select_state.show_conversation();
    })
}

fn recent_row(state: Rc<Workbench>, summary: SessionSummary) -> impl IntoView {
    let when = age(summary.modified);
    let title = summary.title.clone();
    Stack::horizontal((
        Label::new(title).style(|s| {
            s.color(MUTED)
                .flex_grow(1.0_f32)
                .min_width(0)
                .text_ellipsis()
        }),
        Label::new(when).style(|s| s.color(MUTED).font_size(10).flex_shrink(0.0_f32)),
    ))
    .style(|s| {
        s.width_full()
            .height(30)
            .gap(8)
            .padding_horiz(14)
            .items_center()
            .cursor(CursorStyle::Pointer)
            .hover(|s| s.background(BORDER.with_alpha(0.5)).color(TEXT))
    })
    .on_event_stop(el::Click, move |_, _| {
        state.sessions.resume(&summary);
        state.show_conversation();
    })
}

pub(super) fn rail(state: Rc<Workbench>) -> impl IntoView {
    let open = state.sessions.open();
    let (open_state, recent_state, new_state) = (state.clone(), state.clone(), state.clone());
    let recent_sessions = state.sessions.clone();
    Stack::vertical((
        Stack::horizontal((
            Label::new("SESSIONS")
                .style(|s| s.color(MUTED).font_size(10).font_bold().flex_grow(1.0_f32)),
            Label::new("+ New")
                .style(|s| {
                    s.color(ACCENT)
                        .font_size(12)
                        .padding_horiz(8)
                        .padding_vert(3)
                        .border_radius(6)
                        .cursor(CursorStyle::Pointer)
                        .hover(|s| s.background(BORDER))
                })
                .on_event_stop(el::Click, move |_, _| {
                    new_state.sessions.create();
                    new_state.show_conversation();
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
            move || open.get().into_iter().enumerate().collect::<Vec<_>>(),
            |(index, session)| (*index, session.key),
            move |(index, session)| open_row(open_state.clone(), session, index),
        )
        .style(|s| s.flex_col().width_full()),
        heading("RECENT"),
        dyn_stack(
            move || {
                recent_sessions
                    .recent()
                    .into_iter()
                    .take(40)
                    .collect::<Vec<_>>()
            },
            |summary| summary.path.clone(),
            move |summary| recent_row(recent_state.clone(), summary),
        )
        .style(|s| s.flex_col().width_full())
        .scroll()
        .style(|s| s.width_full().flex_grow(1.0_f32).min_height(0)),
    ))
    .style(|s| {
        s.width(240)
            .height_full()
            .flex_shrink(0.0_f32)
            .background(PANEL)
            .border_right(1)
            .border_color(BORDER)
    })
}
