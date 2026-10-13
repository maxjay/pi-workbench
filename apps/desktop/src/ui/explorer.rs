use super::{BORDER, MUTED, PANEL, TEXT, command_button};
use crate::{commands::Command, workspace::Workbench};
use floem::{prelude::*, style::CursorStyle};
use std::rc::Rc;

pub(super) fn explorer(state: Rc<Workbench>) -> impl IntoView {
    let rows = state.clone();
    let make_row = state.clone();
    let layout = state.layout();
    let files = virtual_stack(
        move || {
            rows.explorer
                .rows()
                .into_iter()
                .collect::<floem::imbl::Vector<_>>()
        },
        |row| row.entry.path.clone(),
        move |row| {
            let action_state = make_row.clone();
            let render_state = make_row.clone();
            let path = row.entry.path;
            let render_path = path.clone();
            let title = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let directory = row.entry.directory;
            Label::derived(move || {
                let marker = if directory {
                    if render_state.explorer.is_expanded(&render_path) {
                        "− "
                    } else {
                        "+ "
                    }
                } else {
                    "  "
                };
                format!("{marker}{title}")
            })
            .style(move |s| {
                s.height(28)
                    .width_full()
                    .padding_left((12 + row.depth * 14) as f64)
                    .padding_right(8)
                    .items_center()
                    .text_ellipsis()
                    .cursor(CursorStyle::Pointer)
                    .hover(|s| s.background(BORDER))
                    .color(if directory { TEXT } else { MUTED })
            })
            .on_event_stop(el::Click, move |_, _| {
                if directory {
                    action_state.explorer.toggle(path.clone());
                } else {
                    action_state.open(path.clone(), action_state.focused().get_untracked());
                }
            })
        },
    )
    .style(|s| s.flex_col().width_full())
    .scroll()
    .style(|s| s.size_full().min_height(0));
    Stack::vertical((
        Stack::horizontal((
            Label::new("FILES").style(|s| s.color(MUTED).font_size(11).flex_grow(1.0_f32)),
            command_button(state.clone(), "↔", Command::MoveExplorer),
            command_button(state, "Refresh", Command::Refresh),
        ))
        .style(|s| {
            s.width_full()
                .height(42)
                .padding_horiz(10)
                .gap(6)
                .items_center()
        }),
        files,
    ))
    .style(move |s| {
        s.width(layout.with(|l| l.explorer_width))
            .height_full()
            .min_height(0)
            .flex_shrink(0.0_f32)
            .background(PANEL)
            .apply_if(!layout.with(|l| l.explorer_visible), |s| s.hide())
    })
}
