//! A pane: a row of tabs, and whatever the active tab shows — a file, a Pi
//! session, or a terminal. Tabs drag to other panes, or to a pane's edge to
//! split it.
use super::{
    CAPTURE, color,
    lens_doc::{LensDocument, LensStyling, Overlay, Quiet},
};
use crate::{
    commands::{self, Command},
    documents::OpenDocument,
    panes::{Direction, Pane, PaneId, Tab},
    workspace::{DropAt, JumpTo, MenuItem, Workbench},
};
use floem::reactive::Scope;
use floem::style::CustomStylable;
use floem::ui_events::pointer::PointerButton;
use floem::{
    prelude::*,
    reactive::{Effect, Memo},
    style::CursorStyle,
    views::editor::{
        core::{buffer::rope_text::RopeText, cursor::CursorAffinity, selection::Selection},
        text::Document,
        text_document::TextDocument,
    },
};
use std::rc::Rc;

const TAB_HEIGHT: f64 = 34.0;

/// The tab's label, kept up to date: a file's name and unsaved mark, a
/// session's title, or a terminal's number.
fn tab_title(state: &Rc<Workbench>, tab: &Tab) -> impl Fn() -> String + 'static {
    let state = state.clone();
    let tab = tab.clone();
    move || match &tab {
        Tab::File(path) => {
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let dirty = state.documents.get(path).is_some_and(|doc| doc.dirty());
            format!("{name}{}", if dirty { " •" } else { "" })
        }
        Tab::Session(key) => state
            .sessions
            .find(*key)
            .map(|session| format!("π  {}", session.title().get()))
            .unwrap_or_else(|| "π  Session".into()),
        Tab::Terminal(key) => state.terminal(*key).map_or_else(
            || "Terminal".into(),
            |slot| {
                let exited = slot
                    .terminal
                    .get()
                    .is_some_and(|terminal| terminal.exited().get());
                format!(
                    "›_  Terminal {}{}",
                    slot.number,
                    if exited { " (exited)" } else { "" }
                )
            },
        ),
    }
}

/// A menu row that acts on the workbench.
fn item(state: &Rc<Workbench>, title: &str, action: impl Fn(&Rc<Workbench>) + 'static) -> MenuItem {
    let state = state.clone();
    MenuItem::new(title, move || action(&state))
}

/// What can be done to a tab, on right-click.
fn tab_menu(state: &Rc<Workbench>, pane: &Rc<Pane>, tab: &Tab) -> Vec<MenuItem> {
    let id = pane.id;
    let tabs = pane.tabs.get_untracked();
    let at = tabs.iter().position(|t| t == tab).unwrap_or(0);
    let others: Vec<Tab> = tabs.iter().filter(|t| *t != tab).cloned().collect();
    let right: Vec<Tab> = tabs[(at + 1).min(tabs.len())..].to_vec();
    let mut items = Vec::new();
    let closed = tab.clone();
    items.push(
        item(state, "Close", move |s| s.close_tab(id, closed.clone()))
            .hint(Command::CloseFile.shortcut_label()),
    );
    if !others.is_empty() {
        items.push(item(state, "Close others", move |s| {
            s.close_tabs(id, others.clone())
        }));
    }
    if !right.is_empty() {
        items.push(item(state, "Close to the right", move |s| {
            s.close_tabs(id, right.clone())
        }));
    }
    items.push(item(state, "Close all", move |s| {
        s.close_tabs(id, tabs.clone())
    }));
    items.push(MenuItem::separator());
    for (title, direction, command) in [
        ("Split right", Direction::Right, Command::Split),
        ("Split down", Direction::Down, Command::SplitDown),
    ] {
        let tab = tab.clone();
        items.push(
            item(state, title, move |s| {
                s.select_tab(id, tab.clone());
                s.split(direction);
            })
            .hint(command.shortcut_label()),
        );
    }
    match *tab {
        Tab::Session(key) => {
            items.push(MenuItem::separator());
            items.push(item(state, "End session", move |s| s.close_session(key)));
        }
        Tab::Terminal(key) => {
            items.push(MenuItem::separator());
            items.push(item(state, "Restart shell", move |s| {
                s.restart_terminal(key)
            }));
        }
        Tab::File(_) => {}
    }
    items
}

fn tab_view(state: Rc<Workbench>, pane: Rc<Pane>, tab: Tab) -> impl IntoView {
    let id = pane.id;
    let active = pane.active;
    let tabs = pane.tabs;
    let focused = state.focused();
    let target = state.drop_target();
    let selected = tab.clone();
    let (select_state, close_state, menu_state) = (state.clone(), state.clone(), state.clone());
    let (select_tab, close_tab, menu_tab, mark_tab) =
        (tab.clone(), tab.clone(), tab.clone(), tab.clone());
    let menu_pane = pane.clone();
    let reveal_tab = tab.clone();
    let session_dot = match &tab {
        Tab::Session(key) => state.sessions.find(*key),
        _ => None,
    };
    let view = Stack::horizontal((
        Empty::new().style(move |s| {
            let status = session_dot.as_ref().map(|session| session.status());
            s.size(7, 7)
                .border_radius(4)
                .margin_left(10)
                .background(status.map_or(Color::TRANSPARENT, super::session::status_color))
                .apply_if(status.is_none(), |s| s.hide())
        }),
        Label::derived(tab_title(&state, &tab)).style(|s| {
            s.padding_left(10)
                .padding_right(6)
                .height_full()
                .items_center()
                .text_ellipsis()
                .max_width(240)
        }),
        Label::new("×")
            .style(|s| {
                s.padding_right(8)
                    .padding_left(2)
                    .height_full()
                    .items_center()
                    .color(color::muted())
                    .cursor(CursorStyle::Pointer)
                    .hover(|s| s.color(color::text()))
            })
            .on_event_stop(el::PointerDown, |_, _| {})
            .on_event_stop(el::Click, {
                let (state, tab) = (state.clone(), tab.clone());
                move |_, _| state.close_tab(id, tab.clone())
            }),
    ))
    .style(move |s| {
        let showing = active.with(|a| a.as_ref() == Some(&selected));
        // A tab dragged over the bar lands before this tab, or after the last.
        let (before, after) = match target.get() {
            Some(DropAt {
                pane,
                index: Some(index),
                ..
            }) if pane == id => tabs.with(|tabs| {
                let at = tabs.iter().position(|t| *t == mark_tab);
                (
                    at == Some(index),
                    at.is_some_and(|at| at + 1 == index && index == tabs.len()),
                )
            }),
            _ => (false, false),
        };
        // Tab text is a handle, not something to select.
        s.selectable(false)
            .height(TAB_HEIGHT)
            .items_center()
            .flex_shrink(0.0_f32)
            .cursor(CursorStyle::Pointer)
            .color(color::muted())
            .border_right(1)
            .border_bottom(2)
            .border_color(color::border())
            .hover(|s| s.color(color::text()))
            .apply_if(showing, |s| {
                s.background(color::bg())
                    .color(color::text())
                    .border_bottom_color(if focused.get() == id {
                        color::accent()
                    } else {
                        color::border()
                    })
            })
            .apply_if(before, |s| {
                s.border_left(2).border_left_color(color::accent())
            })
            .apply_if(after, |s| {
                s.border_right(2).border_right_color(color::accent())
            })
    })
    // Pressing selects, so a tab picked up to drag is the one showing; the
    // window turns the press into a drag once the pointer moves. The middle
    // button closes, as in a browser.
    .on_event_cont(el::PointerDown, move |_, event| match event.button {
        Some(PointerButton::Auxiliary) => close_state.close_tab(id, close_tab.clone()),
        Some(PointerButton::Secondary) => {}
        _ => {
            select_state.select_tab(id, select_tab.clone());
            select_state.press_tab(Some(id), select_tab.clone());
        }
    })
    .on_event_stop(el::SecondaryClick, move |_, _| {
        let items = tab_menu(&menu_state, &menu_pane, &menu_tab);
        menu_state.open_menu(menu_state.pointer(), items);
    });
    state.register_tab_view(id, tab.clone(), view.view_id());
    // The showing tab scrolls into view, as when a tab arrives by drag.
    let view_id = view.view_id();
    Effect::new(move |_| {
        if active.with(|a| a.as_ref() == Some(&reveal_tab)) {
            // Once laid out: a new tab has no place in the bar yet.
            floem::action::exec_after_animation_frame(move |_| view_id.scroll_to(None));
        }
    });
    view
}

/// A small square icon: a split line down or across the middle, or none.
fn pane_icon(split: Option<bool>) -> impl IntoView {
    let line = Empty::new().style(move |s| {
        let s = s.absolute().background(color::muted());
        match split {
            Some(true) => s.inset_left(5).inset_top(0).width(1).height_full(),
            Some(false) => s.inset_top(4).inset_left(0).height(1).width_full(),
            None => s.hide(),
        }
    });
    Container::new(line).style(|s| {
        s.size(12, 10)
            .border(1)
            .border_color(color::muted())
            .border_radius(2)
    })
}

fn pane_button(
    state: Rc<Workbench>,
    pane: PaneId,
    icon: impl IntoView + 'static,
    tip: &'static str,
    action: impl Fn(&Rc<Workbench>, floem::kurbo::Point) + 'static,
) -> impl IntoView {
    let button = Container::new(icon);
    let button_id = button.view_id();
    floem::views::tooltip(
        button
            .style(|s| {
                s.padding_horiz(6)
                    .padding_vert(6)
                    .border_radius(5)
                    .items_center()
                    .justify_center()
                    .cursor(CursorStyle::Pointer)
                    .hover(|s| s.background(color::border()))
            })
            .on_event_stop(el::Click, move |_, _| {
                state.focus_pane(pane);
                // Below the button, for one that opens a menu.
                let rect = button_id.get_visual_rect();
                action(&state, floem::kurbo::Point::new(rect.x0, rect.y1 + 2.0))
            }),
        move || Label::new(tip).style(|s| s.padding(4).font_size(11)),
    )
}

/// What the + in a pane's tab bar adds there.
fn new_tab_menu(state: &Rc<Workbench>, pane: PaneId) -> Vec<MenuItem> {
    let act = |f: fn(&Rc<Workbench>, PaneId)| {
        let state = state.clone();
        move || {
            state.focus_pane(pane);
            f(&state, pane)
        }
    };
    vec![
        MenuItem::new("New file…", act(|s, _| s.new_file())),
        MenuItem::new(
            "Open file…",
            act(|s, _| commands::execute(s, Command::QuickOpen)),
        )
        .hint(Command::QuickOpen.shortcut_label()),
        MenuItem::separator(),
        MenuItem::new("New Pi session", act(|s, _| s.new_session()))
            .hint(Command::NewSession.shortcut_label()),
        MenuItem::new(
            "New terminal",
            act(|s, pane| {
                s.new_terminal(pane);
            }),
        )
        .hint(Command::NewTerminal.shortcut_label()),
    ]
}

fn tab_bar(state: Rc<Workbench>, pane: Rc<Pane>) -> Stack {
    let id = pane.id;
    let tabs = pane.tabs;
    let tab_state = state.clone();
    let tab_pane = pane.clone();
    let list = dyn_stack(
        move || tabs.get(),
        |tab| tab.clone(),
        move |tab| tab_view(tab_state.clone(), tab_pane.clone(), tab),
    )
    // As wide as its tabs, not the bar, so the bar can scroll.
    .style(|s| s.flex_row().height(TAB_HEIGHT).flex_shrink(0.0_f32))
    .scroll()
    // The wheel scrolls the tabs sideways.
    .custom_style(|s: floem::views::scroll::ScrollCustomStyle| {
        s.vertical_scroll_as_horizontal(true)
            .show_bars_when_idle(false)
    })
    .style(|s| {
        s.flex_grow(1.0_f32)
            .flex_basis(0)
            .min_width(0)
            .height(TAB_HEIGHT)
    });
    let several = {
        let state = state.clone();
        move || state.panes.tree().with(|tree| tree.leaves().len() > 1)
    };
    let maximized = state.panes.maximized();
    let close = pane_button(
        state.clone(),
        id,
        Label::new("×").style(|s| s.font_size(14).color(color::muted())),
        "Close pane",
        move |state, _| state.close_pane(id),
    );
    let zoom = pane.zoom;
    let zoom_chip = pane_button(
        state.clone(),
        id,
        Label::derived(move || format!("{}%", (zoom.get() * 100.0).round() as i64))
            .style(|s| s.font_size(11).color(color::muted())),
        "Reset zoom  (Ctrl+0; Ctrl+= and Ctrl+- or Ctrl+wheel to zoom)",
        move |state, _| state.zoom(id, 0),
    )
    .style(move |s| s.apply_if(zoom.get() == 1.0, |s| s.hide()));
    let actions = Stack::horizontal((
        zoom_chip,
        pane_button(
            state.clone(),
            id,
            Label::new("+").style(|s| s.font_size(14).color(color::muted())),
            "New file, session or terminal here",
            move |state, at| state.open_menu(at, new_tab_menu(state, id)),
        ),
        pane_button(
            state.clone(),
            id,
            pane_icon(Some(true)),
            "Split right  (Ctrl+\\)",
            |state, _| state.split(Direction::Right),
        ),
        pane_button(
            state.clone(),
            id,
            pane_icon(Some(false)),
            "Split down  (Ctrl+Shift+\\)",
            |state, _| state.split(Direction::Down),
        ),
        pane_button(
            state.clone(),
            id,
            pane_icon(None),
            "Maximise or restore  (Ctrl+Shift+M)",
            |state, _| state.toggle_maximized(),
        )
        .style(move |s| s.apply_if(!several() && maximized.get().is_none(), |s| s.hide())),
        close.style({
            let state = state.clone();
            move |s| {
                s.apply_if(state.panes.tree().with(|t| t.leaves().len() <= 1), |s| {
                    s.hide()
                })
            }
        }),
    ))
    .style(|s| {
        s.items_center()
            .padding_horiz(4)
            .gap(1)
            .flex_shrink(0.0_f32)
    });
    Stack::horizontal((list, actions)).style(|s| {
        s.width_full()
            .height(TAB_HEIGHT)
            .flex_shrink(0.0_f32)
            .items_center()
            .background(color::panel())
    })
}

/// The editor for the pane's file tabs.
fn file_area(
    state: Rc<Workbench>,
    pane: Rc<Pane>,
    active: Memo<Option<std::path::PathBuf>>,
) -> impl IntoView {
    let id = pane.id;
    let docs = state.documents.items();
    let editor = text_editor("")
        .styling(super::code_styling(pane.zoom.get_untracked()))
        .editor_style(super::editor_colors)
        .editor_style(|s| s.scroll_beyond_last_line(false))
        .style(|s| super::editor_surface(s.size_full().min_width(0).min_height(0)));
    let handle = editor.editor().clone();
    // This pane's view of the lens: ghost lines and folds for the file it shows.
    let overlay = RwSignal::new(Rc::new(Overlay::default()));
    let empty = Rc::new(TextDocument::new(Scope::current(), ""));
    let held_document = std::cell::RefCell::new(None::<Rc<OpenDocument>>);
    let (bind_handle, bind_lens, bind_pane) = (handle.clone(), state.lens.clone(), pane.clone());
    Effect::new(move |_| {
        let path = active.get();
        let documents = docs.get();
        Effect::untrack(|| {
            bind_pane
                .positions
                .borrow_mut()
                .retain(|path, _| documents.iter().any(|doc| doc.path() == path));
            if let Some(doc) = documents
                .iter()
                .find(|doc| Some(doc.path()) == path.as_ref())
            {
                let changed = held_document
                    .borrow()
                    .as_ref()
                    .is_none_or(|held| !Rc::ptr_eq(held, doc));
                if changed {
                    bind_lens.track(doc.path(), doc.text(), doc.revision());
                    let (line, column, top) = bind_pane
                        .positions
                        .borrow()
                        .get(doc.path())
                        .copied()
                        .unwrap_or_default();
                    let lensed = LensDocument::new(doc.text(), overlay);
                    bind_handle.update_doc(Rc::new(lensed), None);
                    let offset = offset_of(&doc.text().rope_text(), line, column);
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
    let styled_zoom = pane.zoom;
    let blank = Rc::new(TextDocument::new(Scope::current(), ""));
    Effect::new(move |_| {
        let path = active.get();
        let doc = docs.with(|docs| {
            docs.iter()
                .find(|doc| Some(doc.path()) == path.as_ref())
                .cloned()
        });
        // Code colours come from the theme, and size from the pane's zoom.
        super::color::theme().track();
        let zoom = styled_zoom.get();
        let focus = lens.focus().get();
        let seen = lens.seen().get();
        let show_formatting = lens.show_formatting().get();
        let (comparison, incoming, view, spans, text) = match &doc {
            Some(doc) => {
                let comparison = lens
                    .file(doc.path())
                    .and_then(|file| file.comparison().get());
                let view = match &comparison {
                    Some(c) => Overlay::new(&c.diff, c.base_text(), focus, |index| {
                        if seen.contains(&c.hunks[index].1) {
                            Some(Quiet::Seen)
                        } else {
                            (!show_formatting && c.formatting[index]).then_some(Quiet::Formatting)
                        }
                    }),
                    None => Overlay::default(),
                };
                let spans = syntax.track(doc.path(), doc.text(), doc.revision()).get();
                (comparison, doc.incoming().get(), view, spans, doc.text())
            }
            None => (None, None, Overlay::default(), None, blank.clone()),
        };
        let view = Rc::new(view);
        overlay.set(view.clone());
        styled.update_styling(Rc::new(LensStyling::new(
            super::code_styling(zoom),
            comparison,
            incoming,
            view,
            spans,
            text,
        )));
    });
    // Keep the pane's cursor and scroll for change navigation, the footer,
    // and for coming back to the file after the layout changes.
    let (cursor_handle, cursor_pane) = (handle.clone(), pane.clone());
    Effect::new(move |_| {
        let offset = cursor_handle.cursor.with(|cursor| cursor.offset());
        let top = cursor_handle.viewport.get().y0;
        let (line, column) = line_and_column(&cursor_handle.doc().rope_text(), offset);
        cursor_pane.cursor_line.set(line);
        cursor_pane.cursor_col.set(column);
        if let Some(path) = active.get_untracked() {
            cursor_pane
                .positions
                .borrow_mut()
                .insert(path, (line, column, top));
        }
    });
    // Carry out jumps once the file is showing and its diff is ready.
    let (jump_handle, jump_state) = (handle.clone(), state.clone());
    let jump = state.jump();
    Effect::new(move |_| {
        let Some(request) = jump.get() else { return };
        if request.pane != id || active.get().as_ref() != Some(&request.path) {
            return;
        }
        let ready = jump_state
            .lens
            .file(&request.path)
            .is_some_and(|file| file.comparison().get().is_some());
        let line = match request.to {
            JumpTo::Line(line) => Some(line),
            JumpTo::FirstChange if ready => jump_state.change_lines(&request.path).first().copied(),
            JumpTo::LastChange if ready => jump_state.change_lines(&request.path).last().copied(),
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
    let editor = editor.style(move |s| s.apply_if(active.get().is_none(), |s| s.hide()));
    Stack::new((editor, change_bar(state, pane, active)))
        .style(|s| s.size_full().min_height(0).min_width(0))
}

/// Shows where a dragged tab would land.
fn drop_hint(state: Rc<Workbench>, id: PaneId) -> impl IntoView {
    let target = state.drop_target();
    Empty::new().style(move |s| {
        let side = target
            .get()
            .filter(|at| at.pane == id && at.index.is_none())
            .map(|at| at.side);
        let s = s
            .absolute()
            .background(color::accent().with_alpha(0.16))
            .border(2)
            .border_color(color::accent())
            .border_radius(4)
            .z_index(20);
        match side {
            None => s.hide(),
            Some(None) => s.inset(4),
            Some(Some(Direction::Left)) => {
                s.inset_left(0).inset_top(0).height_full().width_pct(50.0)
            }
            Some(Some(Direction::Right)) => {
                s.inset_right(0).inset_top(0).height_full().width_pct(50.0)
            }
            Some(Some(Direction::Up)) => s.inset_top(0).inset_left(0).width_full().height_pct(50.0),
            Some(Some(Direction::Down)) => s
                .inset_bottom(0)
                .inset_left(0)
                .width_full()
                .height_pct(50.0),
        }
    })
}

pub(super) fn pane(state: Rc<Workbench>, pane: Rc<Pane>) -> impl IntoView {
    let id = pane.id;
    let active_tab = pane.active;
    let active = Memo::new(move |_| match active_tab.get() {
        Some(Tab::File(path)) => Some(path),
        _ => None,
    });
    let session_key = Memo::new(move |_| match active_tab.get() {
        Some(Tab::Session(key)) => Some(key),
        _ => None,
    });
    let terminal_key = Memo::new(move |_| match active_tab.get() {
        Some(Tab::Terminal(key)) => Some(key),
        _ => None,
    });
    let focused = state.focused();
    let focus_state = state.clone();

    let files = file_area(state.clone(), pane.clone(), active).style(move |s| {
        s.apply_if(
            session_key.get().is_some() || terminal_key.get().is_some(),
            |s| s.hide(),
        )
    });
    let session_state = state.clone();
    let session_visible = Memo::new(move |_| session_key.get().is_some());
    let zoom = pane.zoom;
    // A new zoom lays the session or terminal out afresh at the new size.
    let session = dyn_container(
        move || (session_key.get(), zoom.get().to_bits()),
        move |(key, zoom)| match key.and_then(|key| session_state.sessions.find(key)) {
            Some(session) => super::session::session_view(
                session_state.clone(),
                session,
                session_visible,
                f64::from_bits(zoom),
            )
            .into_any(),
            None => Empty::new().into_any(),
        },
    )
    .style(move |s| {
        s.size_full()
            .min_width(0)
            .min_height(0)
            .apply_if(session_key.get().is_none(), |s| s.hide())
    });
    let terminal_state = state.clone();
    let terminal_visible = Memo::new(move |_| terminal_key.get().is_some());
    let terminal = dyn_container(
        move || (terminal_key.get(), zoom.get().to_bits()),
        move |(key, zoom)| match key.and_then(|key| terminal_state.terminal(key)) {
            Some(slot) => super::terminal::terminal_view(
                terminal_state.clone(),
                slot,
                terminal_visible,
                f64::from_bits(zoom),
            )
            .into_any(),
            None => Empty::new().into_any(),
        },
    )
    .style(move |s| {
        s.size_full()
            .min_width(0)
            .min_height(0)
            .apply_if(terminal_key.get().is_none(), |s| s.hide())
    });
    let empty_hint = Label::new(
        "Open a file from the explorer or with Ctrl+P, drag a tab or file here,\nor use + above for a new file, Pi session or terminal.",
    )
    .style(move |s| {
        s.absolute()
            .inset_left(32)
            .inset_top(32)
            .color(color::muted())
            .line_height(1.8)
            .apply_if(active_tab.with(Option::is_some), |s| s.hide())
    });
    let title_state = state.clone();
    let path_label = Label::derived(move || {
        active
            .get()
            .map(|p| match p.strip_prefix(title_state.root()) {
                Ok(relative) => relative.display().to_string(),
                // Files outside the workspace, such as the settings file.
                Err(_) => pi_client::sessions::plain_cwd(&p),
            })
            .unwrap_or_default()
    })
    .style(move |s| {
        s.apply_if(active.get().is_none(), |s| s.hide())
            .color(color::muted())
            .font_size(11)
            .height(24)
            .padding_horiz(12)
            .items_center()
            .width_full()
            .text_ellipsis()
            .flex_shrink(0.0_f32)
            .background(color::editor_bg())
    });
    let body = Stack::new((
        files,
        session,
        terminal,
        empty_hint,
        drop_hint(state.clone(), id),
    ))
    .style(|s| s.size_full().min_height(0).min_width(0));
    let bar = tab_bar(state.clone(), pane.clone());
    state.register_pane_view(id, body.view_id(), bar.view_id());
    let wheel_state = state.clone();
    let several = {
        let state = state.clone();
        move || state.panes.tree().with(|tree| tree.leaves().len() > 1)
    };
    Stack::vertical((bar, path_label, body))
        .style(move |s| {
            s.size_full()
                .min_width(0)
                .min_height(0)
                .border_top(2)
                .border_color(if several() && focused.get() == id {
                    color::accent()
                } else {
                    color::panel()
                })
        })
        .on_event_cont_with_config(el::PointerDown, CAPTURE, move |_, _| {
            focus_state.focus_pane(id)
        })
        // Ctrl+wheel zooms the pane under the pointer.
        .on_event_with_config(el::PointerWheel, CAPTURE, move |_, event| {
            if !event.state.modifiers.ctrl() {
                return floem::event::EventPropagation::Continue;
            }
            use floem::event::PointerScrollEventExt;
            let delta = event.resolve_to_points(None, None);
            if delta.y != 0.0 {
                wheel_state.zoom(id, if delta.y > 0.0 { 1 } else { -1 });
            }
            floem::event::EventPropagation::Stop
        })
}

/// The line of `offset` and its column in characters.
fn line_and_column(text: &impl RopeText, offset: usize) -> (usize, usize) {
    let offset = offset.min(text.len());
    let line = text.line_of_offset(offset);
    let start = text.offset_of_line(line);
    (line, text.slice_to_cow(start..offset).chars().count())
}

/// The offset of a line and character column, clamped to the text and kept
/// on a character boundary.
fn offset_of(text: &impl RopeText, line: usize, column: usize) -> usize {
    let line = line.min(text.last_line());
    let start = text.offset_of_line(line);
    let content = text.line_content(line);
    let content = content.trim_end_matches(['\n', '\r']);
    start
        + content
            .char_indices()
            .nth(column)
            .map_or(content.len(), |(index, _)| index)
}

/// Floats over the change under the cursor with what can be done to it.
fn change_bar(
    state: Rc<Workbench>,
    pane: Rc<Pane>,
    active: Memo<Option<std::path::PathBuf>>,
) -> impl IntoView {
    let cursor = pane.cursor_line;
    let lens = state.lens.clone();
    let position = move || {
        let path = active.get()?;
        let comparison = lens.file(&path)?.comparison().get()?;
        let line = cursor.get();
        let at = comparison.at_line(line)?;
        Some((at + 1, comparison.hunks.len()))
    };
    let shown = position.clone();
    let action = |title: &'static str, command: Command| {
        let state = state.clone();
        Label::new(title)
            .style(|s| {
                s.color(color::text())
                    .font_size(11)
                    .padding_horiz(8)
                    .padding_vert(3)
                    .border_radius(5)
                    .cursor(CursorStyle::Pointer)
                    .hover(|s| s.background(color::border()))
            })
            .on_event_stop(el::Click, move |_, _| commands::execute(&state, command))
    };
    Stack::horizontal((
        Label::derived(move || {
            position()
                .map(|(at, of)| format!("Change {at} of {of}"))
                .unwrap_or_default()
        })
        .style(|s| s.color(color::muted()).font_size(11).padding_horiz(6)),
        action("Revert", Command::RevertChange),
        action("Stage", Command::StageChange),
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
            .border_color(color::border())
            .border_radius(7)
            .background(color::panel())
            .z_index(5)
            .apply_if(shown().is_none(), |s| s.hide())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::side_at;

    #[test]
    fn dropping_near_an_edge_splits_toward_it() {
        assert_eq!(side_at(10.0, 200.0, 400.0, 400.0), Some(Direction::Left));
        assert_eq!(side_at(390.0, 200.0, 400.0, 400.0), Some(Direction::Right));
        assert_eq!(side_at(200.0, 380.0, 400.0, 400.0), Some(Direction::Down));
        assert_eq!(side_at(200.0, 200.0, 400.0, 400.0), None);
        // Nearest edge wins in a corner.
        assert_eq!(side_at(5.0, 60.0, 400.0, 400.0), Some(Direction::Left));
        assert_eq!(side_at(0.0, 0.0, 0.0, 0.0), None);
    }
}
