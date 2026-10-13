use crate::{
    commands::{self, COMMANDS, Command},
    state::{Confirm, Workbench},
};
use floem::{
    context::{EventCallbackConfig, Phases},
    event::EventPropagation,
    prelude::*,
    reactive::Effect,
    style::CursorStyle,
    views::editor::{
        text::{SimpleStyling, default_dark_color},
        text_document::TextDocument,
    },
    window::WindowId,
};
use std::{path::PathBuf, rc::Rc};

const BG: Color = Color::from_rgb8(21, 24, 30);
const PANEL: Color = Color::from_rgb8(28, 32, 40);
const BORDER: Color = Color::from_rgb8(47, 54, 66);
const TEXT: Color = Color::from_rgb8(218, 224, 233);
const MUTED: Color = Color::from_rgb8(143, 156, 176);
const ACCENT: Color = Color::from_rgb8(128, 190, 175);
const CAPTURE: EventCallbackConfig = EventCallbackConfig {
    phases: Phases::CAPTURE,
};

fn button(title: &'static str, action: impl Fn() + 'static) -> impl IntoView {
    Button::new(title).action(action).style(|s| {
        s.padding_horiz(10)
            .padding_vert(6)
            .border(1)
            .border_color(BORDER)
            .border_radius(4)
            .background(PANEL)
            .color(TEXT)
            .hover(|s| s.background(BORDER))
    })
}

fn command_button(state: Rc<Workbench>, title: &'static str, command: Command) -> impl IntoView {
    button(title, move || commands::execute(&state, command))
}

fn toolbar(state: Rc<Workbench>) -> impl IntoView {
    let palette = state.palette;
    let root = state
        .root
        .file_name()
        .unwrap_or(state.root.as_os_str())
        .to_string_lossy()
        .into_owned();
    Stack::horizontal((
        Label::new("PI WORKBENCH").style(|s| s.color(ACCENT).font_size(12).padding_right(12)),
        Label::new(root).style(|s| {
            s.color(TEXT)
                .flex_grow(1.0_f32)
                .min_width(0)
                .text_ellipsis()
        }),
        command_button(state.clone(), "Open folder", Command::OpenFolder),
        command_button(state.clone(), "Save", Command::Save),
        command_button(state.clone(), "Split", Command::Split),
        command_button(state.clone(), "Explorer", Command::Explorer),
        button("Commands", move || palette.update(|show| *show = !*show)),
    ))
    .style(|s| {
        s.width_full()
            .height(52)
            .padding_horiz(14)
            .gap(8)
            .items_center()
            .flex_shrink(0.0_f32)
            .background(PANEL)
            .border_bottom(1)
            .border_color(BORDER)
    })
}

fn explorer(state: Rc<Workbench>) -> impl IntoView {
    let rows = state.clone();
    let make_row = state.clone();
    let layout = state.layout;
    let files = virtual_stack(
        move || rows.rows().into_iter().collect::<floem::imbl::Vector<_>>(),
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
                    if render_state.expanded.with(|e| e.contains(&render_path)) {
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
                    action_state.toggle_directory(path.clone());
                } else {
                    action_state.open(path.clone(), action_state.focused.get_untracked());
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

fn pane(state: Rc<Workbench>, index: usize) -> impl IntoView {
    let active = state.active[index];
    let docs = state.docs;
    let focused = state.focused;
    let layout = state.layout;
    let tabs_state = state.clone();
    let tabs = dyn_stack(
        move || docs.get(),
        |doc| doc.path.clone(),
        move |doc| {
            let title = doc
                .path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let dirty_doc = doc.clone();
            let selected_path = doc.path.clone();
            let select_path = doc.path.clone();
            let close_path = doc.path.clone();
            let close_state = tabs_state.clone();
            let requested = tabs_state.requested[index];
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
                    active.set(Some(select_path.clone()));
                    requested.set(Some(select_path.clone()));
                    focused.set(index);
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
                    .apply_if(active.get().as_ref() == Some(&selected_path), |s| {
                        s.background(BG).border_color(ACCENT)
                    })
            })
        },
    )
    .style(|s| s.flex_row().height(36))
    .scroll()
    .style(|s| {
        s.width_full()
            .height(36)
            .flex_shrink(0.0_f32)
            .background(PANEL)
    });

    let editor = text_editor("")
        .styling(SimpleStyling::new())
        .editor_style(default_dark_color)
        .editor_style(|s| s.scroll_beyond_last_line(false))
        .style(|s| s.size_full().min_width(0).min_height(0));
    let handle = editor.editor().clone();
    let empty = Rc::new(TextDocument::new(state.scope, ""));
    let held_document = std::cell::RefCell::new(None::<Rc<crate::state::OpenDocument>>);
    Effect::new(move |_| {
        let path = active.get();
        let documents = docs.get();
        Effect::untrack(|| {
            if let Some(doc) = documents
                .iter()
                .find(|doc| Some(&doc.path) == path.as_ref())
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
                    handle.update_doc(doc.text.clone(), None);
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
    let body = Stack::new((editor, empty_hint)).style(|s| s.size_full().min_height(0).min_width(0));
    let title_state = state.clone();
    Stack::vertical((
        tabs,
        Label::derived(move || {
            active
                .get()
                .map(|p| {
                    p.strip_prefix(&title_state.root)
                        .unwrap_or(&p)
                        .display()
                        .to_string()
                })
                .unwrap_or_else(|| format!("Pane {}", index + 1))
        })
        .style(|s| {
            s.color(MUTED)
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
        let weight = if index == 0 {
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
            .border_color(if focused.get() == index {
                ACCENT
            } else {
                BORDER
            })
            .apply_if(index == 1 && !config.split, |s| s.hide())
    })
    .on_event_cont_with_config(el::PointerDown, CAPTURE, move |_, _| focused.set(index))
}

fn divider(state: Rc<Workbench>, sidebar: bool) -> impl IntoView {
    let dragging = RwSignal::new(None::<(f64, f64)>);
    let layout = state.layout;
    let width = state.window_width;
    Empty::new()
        .style(move |s| {
            s.width(5)
                .height_full()
                .flex_shrink(0.0_f32)
                .background(BORDER)
                .cursor(CursorStyle::ColResize)
                .hover(|s| s.background(ACCENT))
                .apply_if(
                    layout.with(|l| {
                        if sidebar {
                            !l.explorer_visible
                        } else {
                            !l.split
                        }
                    }),
                    |s| s.hide(),
                )
        })
        .on_event_stop(el::PointerDown, move |cx, event| {
            if let Some(id) = event.pointer.pointer_id {
                cx.request_pointer_capture(id);
                let x = (cx.world_transform.inverse() * event.state.logical_point()).x;
                dragging.set(Some((
                    x,
                    layout.with_untracked(|l| {
                        if sidebar {
                            l.explorer_width
                        } else {
                            l.split_fraction
                        }
                    }),
                )));
            }
        })
        .on_event_stop(el::PointerMove, move |cx, event| {
            if let Some((start_x, start_value)) = dragging.get_untracked() {
                let x = (cx.world_transform.inverse() * event.current.logical_point()).x;
                layout.update(|l| {
                    if sidebar {
                        let direction = if l.explorer_right { -1.0 } else { 1.0 };
                        l.explorer_width =
                            (start_value + direction * (x - start_x)).clamp(160.0, 480.0);
                    } else {
                        let available = (width.get_untracked()
                            - if l.explorer_visible {
                                l.explorer_width + 5.0
                            } else {
                                0.0
                            }
                            - 5.0)
                            .max(200.0);
                        l.split_fraction =
                            (start_value + (x - start_x) / available).clamp(0.2, 0.8);
                    }
                });
            }
        })
        .on_event_stop(el::PointerUp, move |_, _| dragging.set(None))
        .on_event_stop(el::LostPointerCapture, move |_, _| dragging.set(None))
}

fn command_palette(state: Rc<Workbench>) -> impl IntoView {
    let shown = state.palette;
    let query = state.palette_query;
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
                .filter(|c| c.title.to_lowercase().contains(&query.get().to_lowercase()))
                .map(|c| c.id)
                .collect::<Vec<_>>()
        },
        |command| *command,
        move |command| {
            let spec = COMMANDS.iter().find(|c| c.id == command).unwrap();
            let state = state.clone();
            Label::new(format!("{}    {}", spec.title, spec.shortcut))
                .button()
                .action(move || {
                    shown.set(false);
                    commands::execute(&state, command);
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
            button("Close", move || shown.set(false)),
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
        move || shown.set(false),
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

fn confirmation(state: Rc<Workbench>) -> impl IntoView {
    let confirm = state.confirm;
    let docs = state.docs;
    let save_state = state.clone();
    let safe_state = state.clone();
    let discard_state = state.clone();
    let cancel = button("Cancel", move || confirm.set(None)).into_any();
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
                button("Save all", move || save_state.save_all()).style(move |s| {
                    s.apply_if(matches!(confirm.get(), Some(Confirm::Reload(_))), |s| {
                        s.hide()
                    })
                }),
                button("Close when saved", move || match confirm.get_untracked() {
                    Some(Confirm::Quit) => safe_state.request_quit(),
                    Some(Confirm::Close(path))
                        if safe_state
                            .document(&path)
                            .is_none_or(|d| !d.dirty() && !d.saving.get_untracked()) =>
                    {
                        safe_state.remove_file(&path);
                        confirm.set(None);
                    }
                    _ => {}
                })
                .style(move |s| {
                    s.apply_if(matches!(confirm.get(), Some(Confirm::Reload(_))), |s| {
                        s.hide()
                    })
                }),
                button("Discard", move || {
                    if docs.with_untracked(|docs| docs.iter().any(|d| d.saving.get_untracked())) {
                        discard_state
                            .status
                            .set("Wait for saving to finish.".into());
                        return;
                    }
                    match confirm.get_untracked() {
                        Some(Confirm::Quit) => discard_state.finish_quit(),
                        Some(Confirm::Close(path)) => discard_state.remove_file(&path),
                        Some(Confirm::Reload(path)) => discard_state.reload(path),
                        None => {}
                    }
                    confirm.set(None);
                }),
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
        move || confirm.set(None),
    )
}

pub fn workbench(window: WindowId, root: PathBuf, layout_path: PathBuf) -> impl IntoView {
    let state = Workbench::new(window, root, layout_path);
    state.restore();
    let layout = state.layout;
    let status = state.status;
    let focused = state.focused;
    let width = state.window_width;
    let close_state = state.clone();
    let keyboard_state = state.clone();
    let editors = Stack::horizontal((
        pane(state.clone(), 0),
        divider(state.clone(), false),
        pane(state.clone(), 1),
    ))
    .style(|s| s.size_full().min_width(0).min_height(0));
    let body = Stack::horizontal((
        explorer(state.clone()),
        divider(state.clone(), true),
        editors,
    ))
    .style(move |s| {
        s.size_full().min_width(0).min_height(0).flex_direction(
            if layout.with(|l| l.explorer_right) {
                floem::taffy::FlexDirection::RowReverse
            } else {
                floem::taffy::FlexDirection::Row
            },
        )
    });
    let footer = Stack::horizontal((
        Label::derived(move || status.get())
            .style(|s| s.flex_grow(1.0_f32).min_width(0).text_ellipsis()),
        Label::derived(move || format!("Pane {}  ·  UTF-8", focused.get() + 1))
            .style(|s| s.color(ACCENT)),
    ))
    .style(|s| {
        s.width_full()
            .height(28)
            .padding_horiz(12)
            .gap(10)
            .font_size(11)
            .items_center()
            .flex_shrink(0.0_f32)
            .background(PANEL)
            .border_top(1)
            .border_color(BORDER)
    });
    Stack::new((
        Stack::vertical((toolbar(state.clone()), body, footer)).style(|s| s.size_full()),
        command_palette(state.clone()),
        confirmation(state),
    ))
    .style(|s| s.size_full().background(BG).color(TEXT).font_size(13))
    .on_event_cont(el::WindowCloseRequested, move |cx, _| {
        cx.prevent_default();
        close_state.request_quit();
    })
    .on_event_cont(el::WindowResized, move |_, size| width.set(size.width))
    .on_event_with_config(el::KeyDown, CAPTURE, move |cx, event| {
        let modifier = if cfg!(target_os = "macos") {
            Modifiers::META
        } else {
            Modifiers::CONTROL
        };
        if event.key == Key::Named(NamedKey::Escape) {
            keyboard_state.palette.set(false);
            keyboard_state.confirm.set(None);
            return EventPropagation::Continue;
        }
        if event.modifiers.contains(modifier)
            && let Key::Character(key) = &event.key
        {
            let command = match key.to_lowercase().as_str() {
                "s" => Some(Command::Save),
                "q" => Some(Command::Quit),
                "w" => Some(Command::CloseFile),
                "b" => Some(Command::Explorer),
                "\\" => Some(Command::Split),
                "p" if event.modifiers.contains(Modifiers::SHIFT) => {
                    keyboard_state.palette.update(|show| *show = !*show);
                    cx.prevent_default();
                    return EventPropagation::Stop;
                }
                _ => None,
            };
            if let Some(command) = command {
                commands::execute(&keyboard_state, command);
                cx.prevent_default();
                return EventPropagation::Stop;
            }
        }
        EventPropagation::Continue
    })
}
