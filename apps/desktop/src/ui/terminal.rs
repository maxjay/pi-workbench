//! The terminal panel under the editors: one shell per window, started the
//! first time the panel opens.
use super::{ACCENT, BG, BORDER, MUTED, PANEL};
use crate::{
    terminal::{Row, Screen, key_bytes},
    workspace::Workbench,
};
use floem::{
    event::{EventPropagation, PointerScrollEventExt},
    prelude::*,
    reactive::Effect,
    text::{Attrs, AttrsList, FamilyOwned, FontStyle, FontWeight, LineHeightValue, TextLayout},
    views::rich_text,
};
use std::rc::Rc;

const PADDING: f64 = 8.0;

/// The look shared by every row, and the size of one cell.
#[derive(Clone)]
struct Font {
    attrs: Attrs<'static>,
    cell_width: f64,
    line_height: f64,
}

fn font() -> Font {
    let size = (crate::settings::code_size() as f32 - 1.0).max(9.0);
    let line_height = (size * 1.4).round();
    let family: &'static [FamilyOwned] =
        Box::leak(FamilyOwned::parse_list(&crate::settings::code_font()).collect());
    let attrs = Attrs::new()
        .family(family)
        .font_size(size)
        .line_height(LineHeightValue::Pt(line_height))
        .color(super::TEXT);
    // Monospace: ten cells measure one more precisely than one.
    let sample = TextLayout::new_with_text("MMMMMMMMMM", AttrsList::new(attrs.clone()), None);
    let cell_width = (sample.size().width / 10.0).max(1.0);
    Font {
        attrs,
        cell_width,
        line_height: f64::from(line_height),
    }
}

fn row_view(row: Row, font: Font) -> impl IntoView {
    let mut list = AttrsList::new(font.attrs.clone());
    for span in &row.spans {
        let mut attrs = font.attrs.clone().color(span.color);
        if span.bold {
            attrs = attrs.weight(FontWeight::BOLD);
        }
        if span.italic {
            attrs = attrs.font_style(FontStyle::Italic);
        }
        list.add_span(span.range.clone(), attrs);
    }
    let text = row.text.clone();
    let line_height = font.line_height;
    rich_text(text.clone(), list.clone(), move || {
        (text.clone(), list.clone())
    })
    .style(move |s| s.height(line_height).min_width(0))
}

/// Cell backgrounds other than the default, drawn behind the text.
fn backgrounds(screen: &Screen, font: &Font) -> Vec<(f64, f64, f64, Color)> {
    let mut boxes = Vec::new();
    for (line, row) in screen.rows.iter().enumerate() {
        for span in &row.spans {
            let Some(color) = span.background else {
                continue;
            };
            let start = row.text[..span.range.start].chars().count();
            let count = row.text[span.range.clone()].chars().count();
            boxes.push((
                start as f64 * font.cell_width,
                line as f64 * font.line_height,
                count as f64 * font.cell_width,
                color,
            ));
        }
    }
    boxes
}

pub(super) fn terminal_panel(state: Rc<Workbench>) -> impl IntoView {
    let visible = state.terminal_visible();
    let terminal = state.terminal();
    let font = font();
    let (cell_width, line_height) = (font.cell_width, font.line_height);

    let screen = move || terminal.get().map(|t| t.screen().get()).unwrap_or_default();
    let rows_font = font.clone();
    let rows = dyn_stack(
        move || {
            screen()
                .rows
                .iter()
                .cloned()
                .enumerate()
                .collect::<Vec<_>>()
        },
        |(index, row)| (*index, row.text.clone(), format!("{:?}", row.spans)),
        move |(_, row)| row_view(row, rows_font.clone()),
    )
    .style(|s| s.flex_col().absolute().inset(0));
    let back_font = font.clone();
    let fills = dyn_stack(
        move || {
            backgrounds(&screen(), &back_font)
                .into_iter()
                .enumerate()
                .collect::<Vec<_>>()
        },
        |(index, (x, y, w, color))| {
            (
                *index,
                x.to_bits(),
                y.to_bits(),
                w.to_bits(),
                format!("{color:?}"),
            )
        },
        move |(_, (x, y, w, color))| {
            Empty::new().style(move |s| {
                s.absolute()
                    .inset_left(x)
                    .inset_top(y)
                    .width(w)
                    .height(line_height)
                    .background(color)
            })
        },
    )
    .style(|s| s.absolute().inset(0));
    let focused = state.terminal_focused();
    let cursor = Empty::new().style(move |s| {
        let at = screen().cursor;
        let (line, column) = at.unwrap_or_default();
        s.absolute()
            .inset_left(column as f64 * cell_width)
            .inset_top(line as f64 * line_height)
            .width(cell_width)
            .height(line_height)
            .apply_if(focused.get(), |s| s.background(ACCENT.with_alpha(0.55)))
            .apply_if(!focused.get(), |s| s.border(1).border_color(ACCENT))
            .apply_if(at.is_none(), |s| s.hide())
    });
    let exited = Label::new("The shell exited. Press Enter to start another.").style(move |s| {
        let gone = terminal.get().is_some_and(|t| t.exited().get());
        s.absolute()
            .inset_bottom(0)
            .inset_left(0)
            .padding(6)
            .color(MUTED)
            .background(PANEL)
            .apply_if(!gone, |s| s.hide())
    });
    let screen_view = Stack::new((fills, rows, cursor, exited))
        .style(|s| s.size_full().min_height(0).min_width(0));

    let key_state = state.clone();
    let wheel_state = state.clone();
    let click_state = state.clone();
    let body = Container::new(screen_view).style(|s| {
        s.size_full()
            .padding(PADDING)
            .min_height(0)
            .min_width(0)
            .keyboard_navigable()
    });
    let body_id = body.view_id();
    // Starts the shell, and keeps its size in step with the panel.
    let size_state = state.clone();
    let resize = move || {
        let Some(size) = body_id.get_size() else {
            return;
        };
        let columns = ((size.width - 2.0 * PADDING) / cell_width).floor().max(2.0) as usize;
        let lines = ((size.height - 2.0 * PADDING) / line_height)
            .floor()
            .max(1.0) as usize;
        size_state.ensure_terminal(columns, lines);
    };
    let layout_resize = resize.clone();
    let open_state = state.clone();
    Effect::new(move |_| {
        if visible.get() {
            let resize = resize.clone();
            floem::action::exec_after_animation_frame(move |_| resize());
            let focus = open_state.clone();
            floem::action::exec_after_animation_frame(move |_| {
                if focus.terminal_visible().get_untracked() {
                    body_id.request_focus();
                }
            });
        }
    });
    let focus_in = state.clone();
    let focus_out = state.clone();
    let body = body
        .on_event_cont(el::UpdatePhaseLayout, move |_, _| layout_resize())
        .on_event_cont(el::FocusGained, move |_, _| {
            focus_in.set_terminal_focused(true)
        })
        .on_event_cont(el::FocusLost, move |_, _| {
            focus_out.set_terminal_focused(false)
        })
        .on_event_cont(el::PointerDown, move |_, _| {
            click_state.set_terminal_focused(true);
            body_id.request_focus();
        })
        .on_event(el::PointerWheel, move |_, event| {
            let delta = event.resolve_to_points(None, None);
            let lines = (-delta.y / line_height).round() as i32;
            if lines != 0
                && let Some(terminal) = wheel_state.terminal().get_untracked()
            {
                terminal.scroll(lines);
            }
            EventPropagation::Stop
        })
        .on_event(el::KeyDown, move |cx, event| {
            let Some(terminal) = key_state.terminal().get_untracked() else {
                return EventPropagation::Continue;
            };
            if terminal.exited().get_untracked() {
                if event.key == Key::Named(NamedKey::Enter) {
                    key_state.restart_terminal();
                }
                return EventPropagation::Stop;
            }
            let ctrl_shift = Modifiers::CONTROL | Modifiers::SHIFT;
            if event.modifiers.contains(ctrl_shift)
                && matches!(&event.key, Key::Character(c) if c.eq_ignore_ascii_case("v"))
            {
                if let Ok(text) = floem::Clipboard::get_contents() {
                    terminal.paste(&text);
                }
                return EventPropagation::Stop;
            }
            match key_bytes(&event.key, event.modifiers, terminal.app_cursor()) {
                Some(bytes) => {
                    terminal.write(bytes);
                    cx.prevent_default();
                    EventPropagation::Stop
                }
                None => EventPropagation::Continue,
            }
        });

    let close_state = state.clone();
    let header = Stack::horizontal((
        Label::new("TERMINAL").style(|s| s.color(MUTED).font_size(11).flex_grow(1.0_f32)),
        super::button("Hide", move || close_state.toggle_terminal()),
    ))
    .style(|s| {
        s.width_full()
            .height(30)
            .padding_horiz(10)
            .items_center()
            .border_bottom(1)
            .border_color(BORDER)
            .flex_shrink(0.0_f32)
    });
    Stack::vertical((header, body)).style(move |s| {
        s.width_full()
            .height(280)
            .flex_shrink(0.0_f32)
            .background(BG)
            .border_top(1)
            .border_color(BORDER)
            .apply_if(!visible.get(), |s| s.hide())
    })
}
