use super::{ACCENT, BG, BORDER, CAPTURE, MUTED, PANEL, TEXT, button};
use crate::{
    commands::{self, COMMANDS},
    workspace::{Confirm, Workbench},
};
use floem::{event::EventPropagation, prelude::*, reactive::Effect};
use std::rc::Rc;

pub(super) fn command_palette(state: Rc<Workbench>) -> impl IntoView {
    let shown = state.palette();
    let close_state = state.clone();
    let dismiss_state = state.clone();
    let query = RwSignal::new(String::new());
    let input = TextInput::new(query)
        .placeholder("Filter commands…")
        .style(|s| s.width_full().padding(10).background(BG).color(TEXT));
    let input_id = input.view_id();
    Effect::new(move |_| {
        if shown.get() {
            floem::action::exec_after_animation_frame(move |_| {
                if shown.get_untracked() {
                    input_id.request_focus();
                }
            });
        }
    });
    let commands = dyn_stack(
        move || {
            COMMANDS
                .iter()
                .filter(|c| {
                    c.title()
                        .to_lowercase()
                        .contains(&query.get().to_lowercase())
                })
                .copied()
                .collect::<Vec<_>>()
        },
        |command| *command,
        move |command| {
            let state = state.clone();
            Label::new(format!(
                "{}    {}",
                command.title(),
                command.shortcut_label()
            ))
            .button()
            .action(move || {
                commands::execute(&state, command);
                state.dismiss_palette();
            })
            .style(|s| {
                s.width_full()
                    .padding(10)
                    .background(PANEL)
                    .color(TEXT)
                    .hover(|s| s.background(BORDER))
            })
        },
    )
    .style(|s| s.flex_col().width_full())
    .scroll()
    .style(|s| s.width_full().max_height(360));
    floating_layer(
        Stack::vertical((
            Label::new("Commands").style(|s| s.color(ACCENT)),
            input,
            commands,
            button("Close", move || close_state.dismiss_palette()),
        ))
        .style(move |s| {
            s.absolute()
                .inset_top(70)
                .inset_left(80)
                .width(570)
                .padding(16)
                .gap(10)
                .background(PANEL)
                .border(1)
                .border_color(BORDER)
                .border_radius(8)
                .z_index(20)
                .apply_if(!shown.get(), |s| s.hide())
        }),
        move || shown.get(),
        move || dismiss_state.dismiss_palette(),
    )
}

fn floating_layer(
    content: impl IntoView + 'static,
    visible: impl Fn() -> bool + 'static,
    dismiss: impl Fn() + 'static,
) -> impl IntoView {
    let backdrop = Empty::new()
        .style(|s| s.absolute().inset(0).background(css::BLACK.with_alpha(0.3)))
        .on_event_stop(el::PointerDown, |_, _| {})
        .on_event_stop(el::Click, |_, _| {});
    Overlay::new(
        Stack::new((backdrop, content))
            .style(move |s| {
                s.fixed()
                    .inset(0)
                    .size_full()
                    .apply_if(!visible(), |s| s.hide())
            })
            .on_event_with_config(el::KeyDown, CAPTURE, move |cx, event| {
                if event.key == Key::Named(NamedKey::Escape) {
                    dismiss();
                    cx.prevent_default();
                    EventPropagation::Stop
                } else {
                    EventPropagation::Continue
                }
            }),
    )
}

pub(super) fn confirmation(state: Rc<Workbench>) -> impl IntoView {
    let confirm = state.confirmation();
    let cancel_state = state.clone();
    let dismiss_state = state.clone();
    let save_state = state.clone();
    let safe_state = state.clone();
    let discard_state = state.clone();
    let cancel = button("Cancel", move || cancel_state.dismiss_confirmation()).into_any();
    let cancel_id = cancel.view_id();
    Effect::new(move |_| {
        if confirm.get().is_some() {
            floem::action::exec_after_animation_frame(move |_| {
                if confirm.get_untracked().is_some() {
                    cancel_id.request_focus();
                }
            });
        }
    });
    let title = Label::derived(move || match confirm.get() {
        Some(Confirm::Quit) => {
            "There are unsaved changes. Save all, then close when ready.".to_owned()
        }
        Some(Confirm::Close(path)) => format!(
            "Close {}? It has unsaved changes.",
            path.file_name().unwrap_or_default().to_string_lossy()
        ),
        Some(Confirm::Reload(_)) => "Reload from disk and discard unsaved edits?".to_owned(),
        None => String::new(),
    });
    floating_layer(
        Stack::vertical((
            title,
            Label::new("Save errors appear in the status bar. Cancel keeps your edits open.")
                .style(|s| s.color(MUTED).font_size(12)),
            Stack::horizontal((
                button("Save all", move || save_state.documents.save_all()).style(move |s| {
                    s.apply_if(matches!(confirm.get(), Some(Confirm::Reload(_))), |s| {
                        s.hide()
                    })
                }),
                button("Close when saved", move || safe_state.close_when_saved()).style(move |s| {
                    s.apply_if(matches!(confirm.get(), Some(Confirm::Reload(_))), |s| {
                        s.hide()
                    })
                }),
                button("Discard", move || discard_state.discard_confirmed()),
                cancel,
            ))
            .style(|s| s.gap(8)),
        ))
        .style(move |s| {
            s.absolute()
                .inset_top(130)
                .inset_left(60)
                .max_width(700)
                .padding(24)
                .gap(16)
                .background(PANEL)
                .border(1)
                .border_color(ACCENT)
                .border_radius(8)
                .z_index(30)
                .apply_if(confirm.get().is_none(), |s| s.hide())
        }),
        move || confirm.get().is_some(),
        move || dismiss_state.dismiss_confirmation(),
    )
}
