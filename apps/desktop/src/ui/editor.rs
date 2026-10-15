use super::{
    ACCENT, BG, BORDER, CAPTURE, MUTED, PANEL, TEXT,
    lens_doc::{LensDocument, LensStyling, Overlay},
};
use crate::{
    commands::{self, Command},
    documents::OpenDocument,
    workspace::{JumpTo, Pane, Workbench},
};
use floem::reactive::Scope;
use floem::{
    prelude::*,
    reactive::Effect,
    style::CursorStyle,
    views::editor::{
        core::{cursor::CursorAffinity, selection::Selection},
        text::Document,
        text_document::TextDocument,
    },
};
use std::{path::PathBuf, rc::Rc};

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
    // This pane's view of the lens: ghost lines and folds for the file it shows.
    let overlay = RwSignal::new(Rc::new(Overlay::default()));
    let empty = Rc::new(TextDocument::new(Scope::current(), ""));
    let held_document = std::cell::RefCell::new(None::<Rc<OpenDocument>>);
    // Where each file was left in this pane: cursor offset and scroll position.
    let positions =
        std::cell::RefCell::new(std::collections::HashMap::<PathBuf, (usize, f64)>::new());
    let (bind_handle, bind_lens) = (handle.clone(), state.lens.clone());
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
                    if let Some(previous) = held_document.borrow().as_ref() {
                        let offset = bind_handle.cursor.with_untracked(|c| c.offset());
                        let top = bind_handle.viewport.get_untracked().y0;
                        positions
                            .borrow_mut()
                            .insert(previous.path().clone(), (offset, top));
                    }
                    bind_lens.track(doc.path(), doc.text(), doc.revision());
                    let (offset, top) = positions
                        .borrow()
                        .get(doc.path())
                        .copied()
                        .unwrap_or_default();
                    let lensed = LensDocument::new(doc.text(), overlay);
                    bind_handle.update_doc(Rc::new(lensed), None);
                    let offset = offset.min(doc.text().text().len());
                    bind_handle.cursor.update(|cursor| {
                        cursor.set_insert(Selection::caret(offset, CursorAffinity::Forward))
                    });
                    bind_handle
                        .scroll_to
                        .set(Some(floem::kurbo::Vec2::new(0.0, top)));
                    *held_document.borrow_mut() = Some(doc.clone());
                }
            } else {
                bind_handle.update_doc(empty.clone(), None);
                *held_document.borrow_mut() = None;
            }
        });
    });
    // Restyle when the lens diff, focus, seen changes, or Pi's latest edit change.
    let (styled, lens, syntax) = (handle.clone(), state.lens.clone(), state.syntax.clone());
    let blank = Rc::new(TextDocument::new(Scope::current(), ""));
    Effect::new(move |_| {
        let path = active.get();
        let doc = docs.with(|docs| {
            docs.iter()
                .find(|doc| Some(doc.path()) == path.as_ref())
                .cloned()
        });
        let focus = lens.focus().get();
        let seen = lens.seen().get();
        let (diff, incoming, view, spans, text) = match &doc {
            Some(doc) => {
                let file = lens.file(doc.path());
                let diff = file.as_ref().and_then(|f| f.diff().get());
                let view = match (&diff, &file) {
                    (Some(diff), Some(file)) => {
                        let base = file
                            .base_text()
                            .and_then(|b| (*b).clone())
                            .unwrap_or_default();
                        let text = doc.text().text().to_string();
                        Overlay::new(diff, &base, focus, |hunk| {
                            seen.contains(&lens.hunk_key(file, hunk, &text))
                        })
                    }
                    _ => Overlay::default(),
                };
                let spans = syntax.track(doc.path(), doc.text(), doc.revision()).get();
                (diff, doc.incoming().get(), view, spans, doc.text())
            }
            None => (None, None, Overlay::default(), None, blank.clone()),
        };
        let view = Rc::new(view);
        overlay.set(view.clone());
        styled.update_styling(Rc::new(LensStyling::new(
            super::code_styling(),
            diff,
            incoming,
            view,
            spans,
            text,
        )));
    });
    // Keep the workspace informed of the cursor line, for change navigation.
    let (cursor_handle, cursor_line, cursor_col) = (
        handle.clone(),
        state.cursor_line(pane),
        state.cursor_col(pane),
    );
    Effect::new(move |_| {
        let offset = cursor_handle.cursor.with(|cursor| cursor.offset());
        let line = cursor_handle.line_of_offset(offset);
        cursor_line.set(line);
        cursor_col.set(offset - cursor_handle.offset_of_line(line));
    });
    // Carry out jumps once the file is showing and its diff is ready.
    let (jump_handle, jump_state) = (handle.clone(), state.clone());
    let jump = state.jump();
    Effect::new(move |_| {
        let Some(request) = jump.get() else { return };
        if request.pane != pane || active.get().as_ref() != Some(&request.path) {
            return;
        }
        let ready = jump_state
            .lens
            .file(&request.path)
            .is_some_and(|file| file.diff().get().is_some());
        let line = match request.to {
            JumpTo::Line(line) => Some(line),
            JumpTo::FirstChange if ready => jump_state
                .hunks(&request.path, false)
                .first()
                .map(|hunk| hunk.current.start),
            JumpTo::LastChange if ready => jump_state
                .hunks(&request.path, false)
                .last()
                .map(|hunk| hunk.current.start),
            _ => return,
        };
        jump.set(None);
        if let Some(line) = line {
            let offset = jump_handle.offset_of_line(line);
            jump_handle.cursor.update(|cursor| {
                cursor.set_insert(Selection::caret(offset, CursorAffinity::Forward))
            });
            jump_handle.center_window();
        }
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
    let files = Stack::new((editor, empty_hint, change_bar(state.clone(), pane))).style(move |s| {
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

/// Floats over the change under the cursor with what can be done to it.
fn change_bar(state: Rc<Workbench>, pane: Pane) -> impl IntoView {
    let active = state.active(pane);
    let cursor = state.cursor_line(pane);
    let (lens, seen) = (state.lens.clone(), state.lens.seen());
    let find_state = state.clone();
    let position = move || {
        let path = active.get()?;
        lens.file(&path)?.diff().track();
        seen.track();
        let line = cursor.get();
        let hunks = find_state.hunks(&path, true);
        let at = hunks.iter().position(|hunk| {
            hunk.current.contains(&line) || (hunk.current.is_empty() && hunk.current.start == line)
        })?;
        Some((at + 1, hunks.len()))
    };
    let shown = position.clone();
    let action = |title: &'static str, command: Command| {
        let state = state.clone();
        Label::new(title)
            .style(|s| {
                s.color(TEXT)
                    .font_size(11)
                    .padding_horiz(8)
                    .padding_vert(3)
                    .border_radius(5)
                    .cursor(CursorStyle::Pointer)
                    .hover(|s| s.background(BORDER))
            })
            .on_event_stop(el::Click, move |_, _| commands::execute(&state, command))
    };
    Stack::horizontal((
        Label::derived(move || {
            position()
                .map(|(at, of)| format!("Change {at} of {of}"))
                .unwrap_or_default()
        })
        .style(|s| s.color(MUTED).font_size(11).padding_horiz(6)),
        action("Revert", Command::RevertChange),
        action("Ask Pi", Command::AskAboutChange),
        action("Seen", Command::MarkChangeSeen),
        action("Next ↓", Command::NextChange),
    ))
    .style(move |s| {
        s.absolute()
            .inset_top(8)
            .inset_right(18)
            .items_center()
            .gap(2)
            .padding(3)
            .border(1)
            .border_color(BORDER)
            .border_radius(7)
            .background(PANEL)
            .z_index(5)
            .apply_if(shown().is_none(), |s| s.hide())
    })
}
