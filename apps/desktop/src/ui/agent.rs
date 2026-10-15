//! The Pi panel: conversation, tool cards with live previews, and a prompt box.
use super::{
    ACCENT, ADDED, ADDED_TEXT, BG, BORDER, CODE_FONT, INFO, MUTED, PANEL, REMOVED, REMOVED_TEXT,
    SURFACE, TEXT, WARN, button,
};
use crate::{
    agent::{Entry, EntryKind, ToolState},
    workspace::Workbench,
};
use floem::{
    prelude::*,
    reactive::Effect,
    views::{PlaceholderTextClass, TextInput, TextInputEnter},
};
use std::rc::Rc;

/// Preview lines kept on screen; streaming shows the newest.
const PREVIEW_LINES: usize = 14;

fn pill(text: impl Fn() -> String + 'static, color: impl Fn() -> Color + 'static) -> impl IntoView {
    Label::derived(text).style(move |s| {
        let color = color();
        s.font_size(10)
            .font_bold()
            .padding_horiz(7)
            .padding_vert(2)
            .border_radius(9)
            .color(color)
            .background(color.with_alpha(0.14))
            .flex_shrink(0.0_f32)
    })
}

fn state_label(tool: &str, state: Option<ToolState>) -> (&'static str, Color) {
    let file_tool = matches!(tool, "edit" | "write");
    match state {
        Some(ToolState::Streaming) => ("writing", WARN),
        Some(ToolState::Running) => ("running", INFO),
        Some(ToolState::Failed) => ("failed", REMOVED_TEXT),
        _ if file_tool => ("applied", ACCENT),
        _ => ("done", ACCENT),
    }
}

/// Diff-style lines for edits and writes; plain output for other tools.
fn preview(body: RwSignal<String>) -> impl IntoView {
    let lines = move || {
        body.with(|body| {
            let all: Vec<&str> = body.lines().collect();
            let skipped = all.len().saturating_sub(PREVIEW_LINES);
            let mut shown: Vec<(usize, String)> = all[skipped..]
                .iter()
                .enumerate()
                .map(|(i, line)| (i + 1, line.to_string()))
                .collect();
            if skipped > 0 {
                shown.insert(0, (0, format!("··· {skipped} earlier lines")));
            }
            shown
        })
    };
    dyn_stack(
        lines,
        |line| line.clone(),
        |(index, line)| {
            let (background, color) = match line.chars().next() {
                _ if index == 0 => (Color::TRANSPARENT, MUTED),
                Some('+') => (ADDED, ADDED_TEXT),
                Some('-') => (REMOVED, REMOVED_TEXT),
                _ => (Color::TRANSPARENT, MUTED),
            };
            Label::new(line).style(move |s| {
                s.width_full()
                    .padding_horiz(10)
                    .padding_vert(1)
                    .background(background)
                    .color(color)
                    .text_clip()
            })
        },
    )
    .style(move |s| {
        s.flex_col()
            .width_full()
            .padding_vert(6)
            .font_size(12)
            .font_family(CODE_FONT.to_string())
            .border_top(1)
            .border_color(BORDER)
            .apply_if(body.with(String::is_empty), |s| s.hide())
    })
}

fn tool_card(entry: &Entry) -> impl IntoView {
    let (title, body, state) = (entry.title, entry.body, entry.state);
    let (pill_tool, color_tool) = (entry.tool.clone(), entry.tool.clone());
    Stack::vertical((
        Stack::horizontal((
            pill(
                move || state_label(&pill_tool, state.get()).0.to_string(),
                move || state_label(&color_tool, state.get()).1,
            ),
            Label::new(entry.tool.clone()).style(|s| s.color(MUTED).flex_shrink(0.0_f32)),
            Label::derived(move || title.get()).style(|s| {
                s.color(TEXT)
                    .min_width(0)
                    .flex_grow(1.0_f32)
                    .text_ellipsis()
            }),
        ))
        .style(|s| {
            s.width_full()
                .gap(8)
                .padding_horiz(10)
                .padding_vert(8)
                .items_center()
                .font_family(CODE_FONT.to_string())
                .font_size(12)
        }),
        preview(body),
    ))
    .style(move |s| {
        let failed = state.get() == Some(ToolState::Failed);
        s.width_full()
            .background(SURFACE)
            .border(1)
            .border_radius(8)
            .border_color(if failed {
                REMOVED_TEXT.with_alpha(0.5)
            } else {
                BORDER
            })
    })
}

fn entry_view(entry: Rc<Entry>) -> impl IntoView {
    let (title, body) = (entry.title, entry.body);
    let content = match entry.kind {
        EntryKind::User => Stack::horizontal((Label::derived(move || title.get()).style(|s| {
            s.text_wrap()
                .min_width(0)
                .max_width_pct(88.0)
                .padding_horiz(12)
                .padding_vert(8)
                .border_radius(12)
                .background(ACCENT.with_alpha(0.16))
                .color(TEXT)
        }),))
        .style(|s| s.width_full().justify_end())
        .into_any(),
        EntryKind::Assistant => Stack::vertical((
            Label::new("Pi").style(|s| s.color(ACCENT).font_bold().font_size(11)),
            Label::derived(move || body.get())
                .style(|s| s.text_wrap().min_width(0).color(TEXT).line_height(1.5)),
        ))
        .style(|s| s.width_full().gap(4))
        .into_any(),
        EntryKind::Tool => tool_card(&entry).into_any(),
        EntryKind::Notice => Label::derived(move || title.get())
            .style(|s| {
                s.text_wrap()
                    .min_width(0)
                    .width_full()
                    .padding(10)
                    .border_radius(8)
                    .border_left(3)
                    .border_color(REMOVED_TEXT)
                    .background(REMOVED)
                    .color(TEXT)
            })
            .into_any(),
    };
    content.style(|s| s.width_full().margin_vert(6))
}

fn empty_state() -> impl IntoView {
    Stack::vertical((
        Label::new("π").style(|s| {
            s.font_size(36)
                .color(ACCENT)
                .font_family(CODE_FONT.to_string())
        }),
        Label::new("Ask Pi to change something").style(|s| s.color(TEXT).font_bold()),
        Label::new(
            "Edits stream into your open files as Pi writes them.\nUnsaved work is merged, never overwritten.",
        )
        .style(|s| s.color(MUTED).text_wrap().line_height(1.5)),
    ))
    .style(|s| s.width_full().gap(8).padding(24).items_center())
}

pub(super) fn agent_panel(state: Rc<Workbench>) -> impl IntoView {
    let agent = state.agent.clone();
    let entries = agent.entries();
    let working = agent.working();
    let visible = agent.visible();
    let draft = RwSignal::new(String::new());

    let (stop_agent, hide_agent) = (agent.clone(), agent.clone());
    let header = Stack::horizontal((
        Label::new("Pi").style(|s| s.color(TEXT).font_bold().font_size(14)),
        pill(
            move || if working.get() { "working" } else { "ready" }.to_string(),
            move || if working.get() { WARN } else { ACCENT },
        ),
        Empty::new().style(|s| s.flex_grow(1.0_f32)),
        button("Stop", move || stop_agent.stop())
            .style(move |s| s.color(REMOVED_TEXT).apply_if(!working.get(), |s| s.hide())),
        button("×", move || hide_agent.toggle()),
    ))
    .style(|s| {
        s.width_full()
            .height(48)
            .padding_left(16)
            .padding_right(8)
            .gap(8)
            .items_center()
            .flex_shrink(0.0_f32)
            .border_bottom(1)
            .border_color(BORDER)
    });

    let log = Stack::vertical((
        empty_state().style(move |s| s.apply_if(!entries.with(Vec::is_empty), |s| s.hide())),
        dyn_stack(move || entries.get(), |entry| entry.id, entry_view)
            .style(|s| s.flex_col().width_full()),
    ))
    .style(|s| s.width_full().padding_horiz(14).padding_vert(8))
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

    let send_agent = agent.clone();
    // A refused prompt (Pi still working) keeps the draft.
    let send = move || {
        if send_agent.send(draft.get_untracked()) {
            draft.set(String::new());
        }
    };
    let enter = send.clone();
    let input = TextInput::new(draft)
        .placeholder("Ask Pi…")
        .on_event_stop(TextInputEnter::listener(), move |_, _| enter())
        .style(|s| {
            s.flex_grow(1.0_f32)
                .min_width(0)
                .padding_horiz(12)
                .padding_vert(9)
                .border(1)
                .border_color(BORDER)
                .border_radius(8)
                .background(BG)
                .color(TEXT)
                .focus(|s| s.border_color(ACCENT))
                .class(PlaceholderTextClass, |s| s.color(MUTED))
        });
    let input_id = input.view_id();
    // Opening the panel puts the cursor in the prompt box.
    Effect::new(move |_| {
        if visible.get() {
            floem::action::exec_after_animation_frame(move |_| input_id.request_focus());
        }
    });
    let composer = Stack::vertical((
        Stack::horizontal((
            input,
            button("Send", send).style(move |s| {
                s.background(ACCENT)
                    .color(BG)
                    .font_bold()
                    .padding_horiz(14)
                    .padding_vert(9)
                    .border_radius(8)
                    .hover(|s| s.background(ACCENT.with_alpha(0.85)).color(BG))
                    .apply_if(working.get(), |s| {
                        s.background(BORDER)
                            .color(MUTED)
                            .hover(|s| s.background(BORDER))
                    })
            }),
        ))
        .style(|s| s.width_full().gap(8).items_center()),
        Label::new("Enter to send  ·  Ctrl+J to hide  ·  open files are saved first")
            .style(|s| s.color(MUTED).font_size(11)),
    ))
    .style(|s| {
        s.width_full()
            .gap(6)
            .padding(12)
            .flex_shrink(0.0_f32)
            .border_top(1)
            .border_color(BORDER)
    });

    Stack::vertical((header, log, composer)).style(move |s| {
        s.width(400)
            .height_full()
            .flex_shrink(0.0_f32)
            .background(PANEL)
            .border_left(1)
            .border_color(BORDER)
            .apply_if(!visible.get(), |s| s.hide())
    })
}
