use super::{ACCENT, BG, BORDER, CAPTURE, MUTED, PANEL};
use crate::{
    documents::OpenDocument,
    workspace::{Pane, Workbench},
};
use floem::reactive::Scope;
use floem::{
    prelude::*, reactive::Effect, style::CursorStyle, views::editor::text_document::TextDocument,
};
use std::rc::Rc;

pub(super) fn pane(state: Rc<Workbench>, pane: Pane) -> impl IntoView {
    let active = state.active(pane);
    let docs = state.documents.items();
    let focused = state.focused();
    let layout = state.layout();
    let focus_state = state.clone();
    let tabs_state = state.clone();
    // Only the primary pane hosts the conversation.
    let conversation = state.conversation();
    let showing_conversation = move || pane == Pane::Primary && conversation.get();
    let conversation_state = state.clone();
    let conversation_tab = Label::new("π  Conversation")
        .style(move |s| {
            s.padding_horiz(14)
                .height(36)
                .items_center()
                .flex_shrink(0.0_f32)
                .cursor(CursorStyle::Pointer)
                .border_right(1)
                .border_bottom(2)
                .border_color(BORDER)
                .apply_if(showing_conversation(), |s| {
                    s.background(BG).border_color(ACCENT)
                })
                .apply_if(pane != Pane::Primary, |s| s.hide())
        })
        .on_event_stop(el::Click, move |_, _| {
            conversation_state.show_conversation()
        });
    let tabs = dyn_stack(
        move || docs.get(),
        |doc| doc.path().clone(),
        move |doc| {
            let title = doc
                .path()
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let dirty_doc = doc.clone();
            let selected_path = doc.path().clone();
            let select_path = doc.path().clone();
            let close_path = doc.path().clone();
            let close_state = tabs_state.clone();
            let select_state = tabs_state.clone();
            Stack::horizontal((
                Label::derived(move || {
                    format!("{title}{}", if dirty_doc.dirty() { " •" } else { "" })
                })
                .style(|s| {
                    s.padding_horiz(10)
                        .height_full()
                        .items_center()
                        .cursor(CursorStyle::Pointer)
                })
                .on_event_stop(el::Click, move |_, _| {
                    select_state.select_document(pane, select_path.clone());
                }),
                Label::new("×")
                    .style(|s| {
                        s.padding_right(8)
                            .height_full()
                            .items_center()
                            .color(MUTED)
                            .cursor(CursorStyle::Pointer)
                    })
                    .on_event_stop(el::Click, move |_, _| {
                        close_state.close_file(close_path.clone())
                    }),
            ))
            .style(move |s| {
                s.height(36)
                    .flex_shrink(0.0_f32)
                    .border_right(1)
                    .border_bottom(2)
                    .border_color(BORDER)
                    .apply_if(
                        !showing_conversation() && active.get().as_ref() == Some(&selected_path),
                        |s| s.background(BG).border_color(ACCENT),
                    )
            })
        },
    )
    .style(|s| s.flex_row().height(36));
    let tabs = Stack::horizontal((conversation_tab, tabs))
        .style(|s| s.height(36))
        .scroll()
        .style(|s| {
            s.width_full()
                .height(36)
                .flex_shrink(0.0_f32)
                .background(PANEL)
        });

    let editor = text_editor("")
        .styling(super::code_styling())
        .editor_style(super::editor_colors)
        .editor_style(|s| s.scroll_beyond_last_line(false))
        .style(|s| s.size_full().min_width(0).min_height(0));
    let handle = editor.editor().clone();
    let styled = editor.editor().clone();
    // Lines Pi just changed stay highlighted until the next edit.
    Effect::new(move |_| {
        let path = active.get();
        let incoming = docs.with(|docs| {
            docs.iter()
                .find(|doc| Some(doc.path()) == path.as_ref())
                .and_then(|doc| doc.incoming().get())
        });
        styled.update_styling(match incoming {
            Some(diff) => super::diff::incoming_styling(diff),
            None => Rc::new(super::code_styling()),
        });
    });
    let empty = Rc::new(TextDocument::new(Scope::current(), ""));
    let held_document = std::cell::RefCell::new(None::<Rc<OpenDocument>>);
    Effect::new(move |_| {
        let path = active.get();
        let documents = docs.get();
        Effect::untrack(|| {
            if let Some(doc) = documents
                .iter()
                .find(|doc| Some(doc.path()) == path.as_ref())
            {
                let changed = held_document
                    .borrow()
                    .as_ref()
                    .is_none_or(|held| !Rc::ptr_eq(held, doc));
                if changed {
                    handle.cursor.update(|cursor| {
                        cursor.set_insert(floem::views::editor::core::selection::Selection::caret(
                            0,
                            floem::views::editor::core::cursor::CursorAffinity::Forward,
                        ))
                    });
                    handle.update_doc(doc.text(), None);
                    handle.scroll_to.set(Some(floem::kurbo::Vec2::ZERO));
                    *held_document.borrow_mut() = Some(doc.clone());
                }
            } else {
                handle.update_doc(empty.clone(), None);
                *held_document.borrow_mut() = None;
            }
        });
    });
    let empty_hint = Label::new(
        "Select a file to begin\n\nBoth panes can share the same document.",
    )
    .style(move |s| {
        s.absolute()
            .inset_left(32)
            .inset_top(55)
            .color(MUTED)
            .line_height(1.8)
            .apply_if(active.get().is_some(), |s| s.hide())
    });
    let editor = editor.style(move |s| s.apply_if(active.get().is_none(), |s| s.hide()));
    let files = Stack::new((editor, empty_hint)).style(move |s| {
        s.size_full()
            .min_height(0)
            .min_width(0)
            .apply_if(showing_conversation(), |s| s.hide())
    });
    let chat = if pane == Pane::Primary {
        super::session::conversation(state.clone())
            .style(move |s| s.apply_if(!showing_conversation(), |s| s.hide()))
            .into_any()
    } else {
        Empty::new().into_any()
    };
    let body = Stack::new((files, chat)).style(|s| s.size_full().min_height(0).min_width(0));
    let title_state = state.clone();
    Stack::vertical((
        tabs,
        Label::derived(move || {
            active
                .get()
                .map(|p| {
                    p.strip_prefix(title_state.root())
                        .unwrap_or(&p)
                        .display()
                        .to_string()
                })
                .unwrap_or_else(|| format!("Pane {}", pane.index() + 1))
        })
        .style(move |s| {
            s.apply_if(showing_conversation(), |s| s.hide())
                .color(MUTED)
                .font_size(11)
                .height(28)
                .padding_horiz(12)
                .items_center()
                .width_full()
                .text_ellipsis()
                .flex_shrink(0.0_f32)
        }),
        body,
    ))
    .style(move |s| {
        let config = layout.get();
        // Flex weights below 1 leave space unused, so a lone pane takes it all.
        let weight = if !config.split {
            1.0
        } else if pane == Pane::Primary {
            config.split_fraction
        } else {
            1.0 - config.split_fraction
        };
        s.flex_basis(0)
            .flex_grow(weight as f32)
            .height_full()
            .min_width(0)
            .min_height(0)
            .border_top(1)
            .border_color(if focused.get() == pane {
                ACCENT
            } else {
                BORDER
            })
            .apply_if(pane == Pane::Secondary && !config.split, |s| s.hide())
    })
    .on_event_cont_with_config(el::PointerDown, CAPTURE, move |_, _| {
        focus_state.focus_pane(pane)
    })
}
