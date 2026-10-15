use super::{CAPTURE, button, color};
use crate::workspace::{Confirm, Workbench};
use floem::{event::EventPropagation, prelude::*, reactive::Effect};
use std::rc::Rc;

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
                .style(|s| s.color(color::muted()).font_size(12)),
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
                .background(color::panel())
                .border(1)
                .border_color(color::accent())
                .border_radius(8)
                .z_index(30)
                .apply_if(confirm.get().is_none(), |s| s.hide())
        }),
        move || confirm.get().is_some(),
        move || dismiss_state.dismiss_confirmation(),
    )
}
