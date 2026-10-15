//! The Pi panel: activity log with live tool previews, and a prompt box.
use super::{ACCENT, ADDED, BG, BORDER, MUTED, PANEL, REMOVED, TEXT, button};
use crate::{
    agent::{Entry, EntryKind, ToolState},
    workspace::Workbench,
};
use floem::{
    prelude::*,
    views::{PlaceholderTextClass, TextInput, TextInputEnter},
};
use std::rc::Rc;

fn entry_view(entry: Rc<Entry>) -> impl IntoView {
    let (title, body, state) = (entry.title, entry.body, entry.state);
    let kind = entry.kind;
    let marker = move || match (kind, state.get()) {
        (EntryKind::User, _) => "you",
        (EntryKind::Assistant, _) => "pi",
        (EntryKind::Notice, _) => "!",
        (EntryKind::Tool, Some(ToolState::Streaming)) => "…",
        (EntryKind::Tool, Some(ToolState::Running)) => "run",
        (EntryKind::Tool, Some(ToolState::Failed)) => "err",
        (EntryKind::Tool, _) => "ok",
    };
    let color = move || match (kind, state.get()) {
        (EntryKind::Notice, _) | (_, Some(ToolState::Failed)) => REMOVED,
        (EntryKind::User, _) => ACCENT,
        _ => MUTED,
    };
    Stack::vertical((
        Stack::horizontal((
            Label::derived(marker).style(move |s| s.color(color()).width(28).flex_shrink(0.0_f32)),
            Label::derived(move || title.get()).style(move |s| {
                s.min_width(0)
                    .text_wrap()
                    .apply_if(kind != EntryKind::Assistant, |s| s.color(TEXT))
                    .apply_if(kind == EntryKind::Assistant, |s| s.hide())
            }),
        )),
        Label::derived(move || body.get()).style(move |s| {
            let tool = kind == EntryKind::Tool;
            s.text_wrap()
                .min_width(0)
                .margin_left(if tool { 28 } else { 0 })
                .padding(if tool { 8 } else { 0 })
                .apply_if(tool, |s| {
                    s.background(BG)
                        .border(1)
                        .border_color(BORDER)
                        .border_radius(4)
                        .font_size(12)
                        .font_family("monospace".to_string())
                })
                .apply_if(body.with(String::is_empty), |s| s.hide())
        }),
    ))
    .style(|s| s.width_full().gap(4).padding_vert(6))
}

pub(super) fn agent_panel(state: Rc<Workbench>) -> impl IntoView {
    let agent = state.agent.clone();
    let entries = agent.entries();
    let working = agent.working();
    let visible = agent.visible();
    let draft = RwSignal::new(String::new());
    let (send_agent, enter_agent, stop_agent) = (agent.clone(), agent.clone(), agent);
    let send = move || {
        send_agent.send(draft.get_untracked());
        draft.set(String::new());
    };
    let header = Stack::horizontal((
        Label::new("PI").style(|s| s.color(ACCENT).font_size(12)),
        Label::derived(move || if working.get() { "working…" } else { "idle" }.to_string())
            .style(|s| s.color(MUTED).flex_grow(1.0_f32)),
        button("Stop", move || stop_agent.stop()),
    ))
    .style(|s| {
        s.width_full()
            .height(40)
            .padding_horiz(12)
            .gap(10)
            .items_center()
            .flex_shrink(0.0_f32)
            .border_bottom(1)
            .border_color(BORDER)
    });
    let log = dyn_stack(move || entries.get(), |entry| entry.id, entry_view)
        .style(|s| s.flex_col().width_full().padding_horiz(12))
        .scroll()
        // Follow the newest output.
        .scroll_to_percent(move || {
            entries.with(|entries| {
                if let Some(last) = entries.last() {
                    last.body.track();
                }
            });
            100.0
        })
        .style(|s| s.width_full().flex_grow(1.0_f32).min_height(0));
    let input = Stack::horizontal((
        TextInput::new(draft)
            .placeholder("Ask Pi… (Enter to send)")
            .on_event_stop(TextInputEnter::listener(), move |_, _| {
                enter_agent.send(draft.get_untracked());
                draft.set(String::new());
            })
            .style(|s| {
                s.flex_grow(1.0_f32)
                    .min_width(0)
                    .padding(8)
                    .border(1)
                    .border_color(BORDER)
                    .border_radius(4)
                    .background(BG)
                    .class(PlaceholderTextClass, |s| s.color(MUTED))
            }),
        button("Send", send),
    ))
    .style(move |s| {
        s.width_full()
            .gap(8)
            .padding(12)
            .items_center()
            .flex_shrink(0.0_f32)
            .border_top(1)
            .border_color(BORDER)
            .apply_if(working.get(), |s| s.border_top_color(ADDED))
    });
    Stack::vertical((header, log, input)).style(move |s| {
        s.width(380)
            .height_full()
            .flex_shrink(0.0_f32)
            .background(PANEL)
            .border_left(1)
            .border_color(BORDER)
            .apply_if(!visible.get(), |s| s.hide())
    })
}
