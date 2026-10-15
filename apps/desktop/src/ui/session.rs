//! The conversation: a Pi session's timeline, its questions, and the composer.
use super::{CAPTURE, CODE_FONT, button, color};
use crate::{
    markdown::{self, Block, Inline, Style},
    session::{Entry, EntryKind, Model, Session, Status, ToolState},
    workspace::Workbench,
};
use floem::{
    AnyView,
    event::EventPropagation,
    prelude::*,
    reactive::{Effect, ReadSignal},
    style::CursorStyle,
    text::{Attrs, AttrsList, FamilyOwned, FontStyle, FontWeight},
    views::{
        TextInput, TextInputEnter, dyn_container,
        editor::{
            core::{cursor::CursorAffinity, editor::EditType, selection::Selection},
            text::{Document, SimpleStyling, WrapMethod},
        },
        rich_text,
    },
};
use pi_client::{Answer, AskKind};
use std::{rc::Rc, sync::LazyLock};

/// Preview lines kept on screen; streaming shows the newest.
const PREVIEW_LINES: usize = 14;

static CODE_FAMILY: LazyLock<Vec<FamilyOwned>> =
    LazyLock::new(|| FamilyOwned::parse_list(CODE_FONT).collect());

pub(super) fn status_color(status: Status) -> Color {
    match status {
        Status::Working => color::warn(),
        Status::NeedsInput => color::info(),
        Status::Stopped => color::removed_text(),
        Status::Idle => color::muted(),
    }
}

pub(super) fn status_label(status: Status) -> &'static str {
    match status {
        Status::Working => "working",
        Status::NeedsInput => "needs you",
        Status::Stopped => "stopped",
        Status::Idle => "ready",
    }
}

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

fn rich(inline: &Inline, size: f32, color: Color, bold: bool) -> impl IntoView {
    let base = Attrs::new().color(color).font_size(size).weight(if bold {
        FontWeight::BOLD
    } else {
        FontWeight::NORMAL
    });
    let mut attrs = AttrsList::new(base.clone());
    for (range, style) in &inline.styles {
        let span = match style {
            Style::Strong => base.clone().weight(FontWeight::BOLD),
            Style::Emphasis => base.clone().font_style(FontStyle::Italic),
            Style::Code => base
                .clone()
                .family(&CODE_FAMILY)
                .color(color::accent())
                .font_size(size - 1.0),
            Style::Link => base.clone().color(color::info()),
        };
        attrs.add_span(range.clone(), span);
    }
    let text = inline.text.clone();
    rich_text(text.clone(), attrs.clone(), move || {
        (text.clone(), attrs.clone())
    })
    .style(|s| s.width_full().min_width(0).line_height(1.5))
}

fn block_view(block: Block) -> AnyView {
    match block {
        Block::Heading(inline) => rich(&inline, 15.0, color::text(), true)
            .style(|s| s.padding_top(4))
            .into_any(),
        Block::Paragraph(inline) => rich(&inline, 13.0, color::text(), false).into_any(),
        Block::Item {
            depth,
            marker,
            inline,
        } => Stack::horizontal((
            Label::new(marker)
                .style(|s| s.color(color::muted()).min_width(18).flex_shrink(0.0_f32)),
            rich(&inline, 13.0, color::text(), false),
        ))
        .style(move |s| s.width_full().gap(4).padding_left(depth as f64 * 16.0))
        .into_any(),
        Block::Quote(inline) => rich(&inline, 13.0, color::muted(), false)
            .style(|s| {
                s.padding_left(10)
                    .border_left(3)
                    .border_color(color::border())
            })
            .into_any(),
        Block::Code { language, text } => Stack::vertical((
            Label::new(language.clone()).style(move |s| {
                s.color(color::muted())
                    .font_size(10)
                    .apply_if(language.is_empty(), |s| s.hide())
            }),
            Label::new(text).style(|s| {
                s.font_family(CODE_FONT.to_string())
                    .font_size(12)
                    .color(color::text())
                    .text_wrap()
                    .min_width(0)
            }),
        ))
        .style(|s| {
            s.width_full()
                .gap(4)
                .padding(10)
                .border_radius(6)
                .background(color::bg())
                .border(1)
                .border_color(color::border())
        })
        .into_any(),
        Block::Rule => Empty::new()
            .style(|s| s.width_full().height(1).background(color::border()))
            .into_any(),
    }
}

fn markdown_view(body: RwSignal<String>) -> impl IntoView {
    dyn_stack(
        move || {
            body.with(|body| markdown::parse(body))
                .into_iter()
                .enumerate()
                .collect::<Vec<_>>()
        },
        |item| item.clone(),
        |(_, block)| block_view(block),
    )
    .style(|s| s.flex_col().width_full().gap(8))
}

fn state_label(tool: &str, state: Option<ToolState>) -> (&'static str, Color) {
    let file_tool = matches!(tool, "edit" | "write");
    match state {
        Some(ToolState::Streaming) => ("writing", color::warn()),
        Some(ToolState::Running) => ("running", color::info()),
        Some(ToolState::Failed) => ("failed", color::removed_text()),
        _ if file_tool => ("applied", color::accent()),
        _ => ("done", color::accent()),
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
                _ if index == 0 => (Color::TRANSPARENT, color::muted()),
                Some('+') => (color::added(), color::added_text()),
                Some('-') => (color::removed(), color::removed_text()),
                _ => (Color::TRANSPARENT, color::muted()),
            };
            // Diff lines keep their shape; command output and errors wrap.
            let diff = background != Color::TRANSPARENT;
            Label::new(line).style(move |s| {
                s.width_full()
                    .min_width(0)
                    .padding_horiz(10)
                    .padding_vert(1)
                    .background(background)
                    .color(color)
                    .apply_if(diff, |s| s.text_clip())
                    .apply_if(!diff, |s| s.text_wrap())
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
            .border_color(color::border())
            .apply_if(body.with(String::is_empty), |s| s.hide())
    })
}

fn tool_card(state: Rc<Workbench>, entry: &Entry) -> impl IntoView {
    let (title, body, entry_state) = (entry.title, entry.body, entry.state);
    let tool = entry.tool.clone();
    let (pill_tool, color_tool) = (tool.clone(), tool.clone());
    let file_tool = matches!(tool.as_str(), "edit" | "write");
    Stack::vertical((
        Stack::horizontal((
            pill(
                move || state_label(&pill_tool, entry_state.get()).0.to_string(),
                move || state_label(&color_tool, entry_state.get()).1,
            ),
            Label::new(tool).style(|s| s.color(color::muted()).flex_shrink(0.0_f32)),
            Label::derived(move || title.get()).style(move |s| {
                s.color(color::text())
                    .min_width(0)
                    .flex_grow(1.0_f32)
                    .text_ellipsis()
                    .apply_if(file_tool, |s| {
                        s.cursor(CursorStyle::Pointer)
                            .hover(|s| s.color(color::accent()))
                    })
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
        })
        .on_event_stop(el::Click, move |_, _| {
            let path = title.get_untracked();
            if file_tool && !path.is_empty() {
                // Next to the conversation, so the timeline stays in view.
                state.open_beside(state.root().join(path));
            }
        }),
        preview(body),
    ))
    .style(move |s| {
        let failed = entry_state.get() == Some(ToolState::Failed);
        s.width_full()
            .background(color::surface())
            .border(1)
            .border_radius(8)
            .border_color(if failed {
                color::removed_text().with_alpha(0.5)
            } else {
                color::border()
            })
    })
}

fn ask_card(session: Rc<Session>, entry: Rc<Entry>) -> impl IntoView {
    let Some(ask) = entry.ask.clone() else {
        return Empty::new().into_any();
    };
    let answered = entry.answered;
    let reply = |label: String, answer: Answer| {
        let (session, entry) = (session.clone(), entry.clone());
        button_owned(label.clone(), move || {
            session.answer(&entry, answer.clone(), label.clone())
        })
    };
    let controls: AnyView = match &ask.kind {
        AskKind::Select(options) => Stack::horizontal_from_iter(
            options
                .iter()
                .map(|option| reply(option.clone(), Answer::Value(option.clone())).into_any()),
        )
        .style(|s| s.gap(6).flex_wrap(floem::taffy::FlexWrap::Wrap))
        .into_any(),
        AskKind::Confirm => Stack::horizontal((
            reply("Yes".into(), Answer::Confirmed(true)),
            reply("No".into(), Answer::Confirmed(false)),
        ))
        .style(|s| s.gap(6))
        .into_any(),
        AskKind::Input { prefill, .. } => {
            let text = RwSignal::new(prefill.clone());
            let (submit_session, submit_entry) = (session.clone(), entry.clone());
            let submit = move || {
                let value = text.get_untracked();
                submit_session.answer(&submit_entry, Answer::Value(value.clone()), value);
            };
            let enter = submit.clone();
            Stack::horizontal((
                TextInput::new(text)
                    .on_event_stop(TextInputEnter::listener(), move |_, _| enter())
                    .style(|s| {
                        s.flex_grow(1.0_f32)
                            .padding(6)
                            .border(1)
                            .border_color(color::border())
                            .border_radius(6)
                            .background(color::bg())
                            .color(color::text())
                    }),
                button("Answer", submit),
            ))
            .style(|s| s.gap(6).width_full().items_center())
            .into_any()
        }
    };
    let cancel = {
        let (session, entry) = (session.clone(), entry.clone());
        button("Dismiss", move || {
            session.answer(&entry, Answer::Cancelled, "Dismissed".into())
        })
    };
    Stack::vertical((
        Label::new(ask.title.clone()).style(|s| s.color(color::text()).font_bold()),
        Label::new(ask.message.clone()).style(move |s| {
            s.color(color::muted())
                .text_wrap()
                .apply_if(ask.message.is_empty(), |s| s.hide())
        }),
        Stack::horizontal((controls, cancel))
            .style(move |s| s.gap(6).apply_if(answered.get().is_some(), |s| s.hide())),
        Label::derived(move || format!("Answered: {}", answered.get().unwrap_or_default())).style(
            move |s| {
                s.color(color::muted())
                    .apply_if(answered.get().is_none(), |s| s.hide())
            },
        ),
    ))
    .style(move |s| {
        s.width_full()
            .gap(8)
            .padding(12)
            .border_radius(8)
            .border(1)
            .border_color(if answered.get().is_none() {
                color::info()
            } else {
                color::border()
            })
            .background(color::surface())
    })
    .into_any()
}

fn button_owned(title: String, action: impl Fn() + 'static) -> impl IntoView {
    Button::new(title).action(action).style(|s| {
        s.padding_horiz(10)
            .padding_vert(5)
            .border(1)
            .border_color(color::border())
            .border_radius(6)
            .background(color::bg())
            .color(color::text())
            .hover(|s| s.background(color::border()))
    })
}

fn thinking_view(body: RwSignal<String>) -> impl IntoView {
    let open = RwSignal::new(false);
    Stack::vertical((
        Label::derived(move || {
            let first = body.with(|b| {
                b.lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("")
                    .to_string()
            });
            if open.get() {
                "Thinking ▾".to_string()
            } else {
                format!("Thinking ▸  {}", crate::session::shorten(&first))
            }
        })
        .style(|s| {
            s.color(color::muted())
                .font_size(12)
                .font_style(FontStyle::Italic)
                .cursor(CursorStyle::Pointer)
                .text_ellipsis()
                .min_width(0)
        })
        .on_event_stop(el::Click, move |_, _| open.update(|o| *o = !*o)),
        Label::derived(move || body.get()).style(move |s| {
            s.color(color::muted())
                .font_size(12)
                .text_wrap()
                .min_width(0)
                .padding_left(10)
                .border_left(2)
                .border_color(color::border())
                .apply_if(!open.get(), |s| s.hide())
        }),
    ))
    .style(|s| s.width_full().gap(4))
}

fn entry_view(state: Rc<Workbench>, session: Rc<Session>, entry: Rc<Entry>) -> impl IntoView {
    let (title, body) = (entry.title, entry.body);
    let content = match entry.kind {
        EntryKind::User => {
            let queued = match entry.tool.as_str() {
                "steer" => "steering",
                "follow-up" => "queued for after",
                _ => "",
            };
            Stack::vertical((
                Label::new(queued).style(move |s| {
                    s.color(color::muted())
                        .font_size(10)
                        .apply_if(queued.is_empty(), |s| s.hide())
                }),
                Label::derived(move || title.get()).style(|s| {
                    s.text_wrap()
                        .min_width(0)
                        .padding_horiz(12)
                        .padding_vert(8)
                        .border_radius(12)
                        .background(color::accent().with_alpha(0.16))
                        .color(color::text())
                }),
            ))
            .style(|s| s.width_full().items_end().gap(2).padding_left(80))
            .into_any()
        }
        EntryKind::Assistant => Stack::vertical((
            Label::new("Pi").style(|s| s.color(color::accent()).font_bold().font_size(11)),
            markdown_view(body),
        ))
        .style(|s| s.width_full().gap(6))
        .into_any(),
        EntryKind::Thinking => thinking_view(body).into_any(),
        EntryKind::Tool => tool_card(state, &entry).into_any(),
        EntryKind::Ask => ask_card(session, entry.clone()).into_any(),
        EntryKind::Notice => {
            let error = entry.state.get_untracked() == Some(ToolState::Failed);
            Label::derived(move || title.get())
                .style(move |s| {
                    s.text_wrap()
                        .min_width(0)
                        .width_full()
                        .padding(10)
                        .border_radius(8)
                        .apply_if(error, |s| {
                            s.border_left(3)
                                .border_color(color::removed_text())
                                .background(color::removed())
                                .color(color::text())
                        })
                        .apply_if(!error, |s| s.color(color::muted()).font_size(12))
                })
                .into_any()
        }
    };
    content.style(|s| s.width_full().margin_vert(6))
}

fn model_menu(session: Rc<Session>, open: RwSignal<bool>) -> impl IntoView {
    let models = session.models();
    dyn_stack(
        move || models.get(),
        |model| (model.provider.clone(), model.id.clone()),
        move |model: Model| {
            let (session, label) = (
                session.clone(),
                format!("{}  ·  {}", model.name, model.provider),
            );
            Label::new(label)
                .style(|s| {
                    s.width_full()
                        .padding_horiz(12)
                        .padding_vert(7)
                        .color(color::text())
                        .cursor(CursorStyle::Pointer)
                        .hover(|s| s.background(color::border()))
                })
                .on_event_stop(el::Click, move |_, _| {
                    session.set_model(&model);
                    open.set(false);
                })
        },
    )
    .style(|s| s.flex_col().width_full())
    .scroll()
    .style(move |s| {
        s.absolute()
            .inset_top(44)
            .inset_right(12)
            .width(320)
            .max_height(320)
            .background(color::panel())
            .border(1)
            .border_color(color::border())
            .border_radius(8)
            .z_index(10)
            .apply_if(!open.get(), |s| s.hide())
    })
}

fn composer(session: Rc<Session>, visible: ReadSignal<bool>) -> impl IntoView {
    let working = session.working();
    let request = session.focus_request();
    let editor = text_editor("")
        .styling(SimpleStyling::builder().font_size(13).build())
        .editor_style(super::editor_colors)
        .editor_style(|s| {
            s.wrap_method(WrapMethod::EditorWidth)
                .hide_gutter(true)
                .current_line_color(color::editor_bg())
        })
        .style(|s| super::editor_surface(s.width_full().height(84).min_width(0)));
    let handle = editor.editor().clone();
    handle.update_doc(session.draft.clone(), None);
    // Placeholders belong to the document, so set it on the session's draft.
    session.draft.add_placeholder(handle.id(), "Ask Pi…".into());
    let draft = session.draft.clone();
    let submit = move |follow_up: bool| {
        let text = draft.text().to_string();
        if session.submit(text, follow_up) {
            let len = draft.text().len();
            draft.edit_single(
                Selection::region(0, len, CursorAffinity::Backward),
                "",
                EditType::Other,
            );
        }
    };
    let key_submit = submit.clone();
    let editor = editor.on_event_with_config(el::KeyDown, CAPTURE, move |_, event| {
        // Enter sends; Shift+Enter is a newline; Alt+Enter queues after the run.
        if event.key == Key::Named(NamedKey::Enter) && !event.modifiers.contains(Modifiers::SHIFT) {
            key_submit(event.modifiers.contains(Modifiers::ALT));
            return EventPropagation::Stop;
        }
        EventPropagation::Continue
    });
    // A picked Pi command focuses the composer, after its text.
    let caret_handle = handle.clone();
    let request_focus = handle.editor_view_id;
    Effect::new(move |_| {
        let Some(offset) = request.get() else { return };
        request.set(None);
        floem::action::exec_after_animation_frame(move |_| {
            caret_handle.cursor.update(|cursor| {
                cursor.set_insert(floem::views::editor::core::selection::Selection::caret(
                    offset,
                    floem::views::editor::core::cursor::CursorAffinity::Forward,
                ))
            });
            if let Some(id) = request_focus.try_get_untracked().flatten() {
                id.request_focus();
            }
        });
    });
    // Focus the composer only when it can be seen; a hidden one would take typing.
    let focus = handle.editor_view_id;
    Effect::new(move |_| {
        if visible.get() {
            floem::action::exec_after_animation_frame(move |_| {
                if let Some(id) = focus.try_get_untracked().flatten() {
                    id.request_focus();
                }
            });
        }
    });
    Stack::vertical((
        Stack::horizontal((
            editor,
            button("Send", move || submit(false)).style(move |s| {
                s.background(color::accent())
                    .color(color::bg())
                    .font_bold()
                    .padding_horiz(14)
                    .padding_vert(9)
                    .border_radius(8)
                    .hover(|s| {
                        s.background(color::accent().with_alpha(0.85))
                            .color(color::bg())
                    })
            }),
        ))
        .style(|s| {
            s.width_full()
                .gap(8)
                .items_end()
                .padding(8)
                .border(1)
                .border_color(color::border())
                .border_radius(10)
                .background(color::editor_bg())
        }),
        Label::derived(move || {
            if working.get() {
                "Enter steers Pi now  ·  Alt+Enter queues for after  ·  Shift+Enter new line"
            } else {
                "Enter sends  ·  Shift+Enter new line  ·  open files are saved first"
            }
            .to_string()
        })
        .style(|s| s.color(color::muted()).font_size(11)),
    ))
    .style(|s| s.width_full().gap(6).padding(14).flex_shrink(0.0_f32))
}

fn session_view(state: Rc<Workbench>, session: Rc<Session>) -> impl IntoView {
    let entries = session.entries();
    let (title, stats, model) = (session.title(), session.stats(), session.model());
    let status_session = session.clone();
    let status = move || status_session.status();
    let menu_open = RwSignal::new(false);
    let stop_session = session.clone();
    let working = session.working();
    let header = Stack::horizontal((
        Label::derived(move || title.get()).style(|s| {
            s.color(color::text())
                .font_bold()
                .font_size(14)
                .min_width(0)
                .text_ellipsis()
        }),
        pill(
            {
                let status = status.clone();
                move || status_label(status()).to_string()
            },
            move || status_color(status()),
        ),
        Empty::new().style(|s| s.flex_grow(1.0_f32)),
        Label::derived(move || stats.get()).style(|s| s.color(color::muted()).font_size(11)),
        Label::derived(move || model.get().map_or("Choose model".into(), |m| m.name))
            .style(|s| {
                s.color(color::text())
                    .font_size(12)
                    .padding_horiz(10)
                    .padding_vert(5)
                    .border_radius(6)
                    .cursor(CursorStyle::Pointer)
                    .hover(|s| s.background(color::border()))
            })
            .on_event_stop(el::Click, move |_, _| menu_open.update(|o| *o = !*o)),
        button("Stop", move || stop_session.stop()).style(move |s| {
            s.color(color::removed_text())
                .apply_if(!working.get(), |s| s.hide())
        }),
    ))
    .style(|s| {
        s.width_full()
            .height(44)
            .padding_horiz(18)
            .gap(10)
            .items_center()
            .flex_shrink(0.0_f32)
            .border_bottom(1)
            .border_color(color::border())
    });
    let timeline_state = state.clone();
    let timeline_session = session.clone();
    let empty = Stack::vertical((
        Label::new("π").style(|s| {
            s.font_size(36)
                .color(color::accent())
                .font_family(CODE_FONT.to_string())
        }),
        Label::new("What should Pi work on?").style(|s| s.color(color::text()).font_bold()),
        Label::new(
            "Pi's edits stream into your open files as it writes them.\nUnsaved work is merged, never overwritten.",
        )
        .style(|s| s.color(color::muted()).text_wrap().line_height(1.5)),
    ))
    .style(move |s| {
        s.width_full()
            .gap(8)
            .padding_top(80)
            .items_center()
            .apply_if(!entries.with(Vec::is_empty), |s| s.hide())
    });
    let timeline = Stack::vertical((
        empty,
        dyn_stack(
            move || entries.get(),
            |entry| entry.id,
            move |entry| entry_view(timeline_state.clone(), timeline_session.clone(), entry),
        )
        .style(|s| s.flex_col().width_full()),
    ))
    .style(|s| {
        s.width_full()
            .max_width(860)
            .padding_horiz(24)
            .padding_vert(12)
            .margin_horiz(floem::unit::LengthAuto::Auto)
    })
    .scroll()
    .scroll_to_percent(move || {
        entries.with(|entries| {
            if let Some(last) = entries.last() {
                last.body.track();
            }
        });
        100.0
    })
    .style(|s| s.width_full().flex_grow(1.0_f32).min_height(0));
    let composer = Stack::vertical((composer(session.clone(), state.conversation()),)).style(|s| {
        s.width_full()
            .max_width(860)
            .margin_horiz(floem::unit::LengthAuto::Auto)
    });
    Stack::new((
        Stack::vertical((header, timeline, composer)).style(|s| s.size_full().min_height(0)),
        model_menu(session, menu_open),
    ))
    .style(|s| {
        s.size_full()
            .min_width(0)
            .min_height(0)
            .background(color::bg())
    })
}

/// The active session's conversation, or a prompt to start one.
pub(super) fn conversation(state: Rc<Workbench>) -> impl IntoView {
    let sessions = state.sessions.clone();
    let active = state.sessions.active_key();
    // Rebuild only when the front session changes: rebuilding twice in one
    // frame leaves Floem with a stale layout listener for the removed editor.
    dyn_container(
        move || active.get(),
        move |key| match key.and_then(|key| sessions.find(key)) {
            Some(session) => session_view(state.clone(), session).into_any(),
            None => Label::new("No session open. Press Ctrl+N to start one.")
                .style(|s| s.color(color::muted()).padding(40))
                .into_any(),
        },
    )
    .style(|s| s.size_full().min_width(0).min_height(0))
}
