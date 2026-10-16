//! Native window composition and styling. User actions are implemented in workspace.rs.
pub(crate) mod color;
mod editor;
mod explorer;
mod finder;
mod lens_doc;
mod overlays;
mod pet;
#[cfg(windows)]
mod pet_window;
mod rail;
mod session;
mod terminal;

use crate::{
    commands::{self, Command},
    lens::Since,
    panes::{Node, Tab},
    workspace::{Integrations, Workbench},
};
use editor::pane;
use explorer::explorer;
use floem::{
    context::{EventCallbackConfig, Phases},
    event::EventPropagation,
    prelude::*,
    style::CursorStyle,
    window::WindowId,
};
use overlays::confirmation;
use pi_client::Launch;
use std::{cell::Cell, path::PathBuf, rc::Rc};

/// CSS-style family list; the first installed font wins.
pub(crate) const CODE_FONT: &str =
    "'Cascadia Code', 'JetBrains Mono', Consolas, 'DejaVu Sans Mono', monospace";
pub(crate) const CODE_SIZE: usize = 14;

/// How the lens marks who added a line.
fn origin_color(origin: crate::origins::Origin) -> Color {
    match origin {
        crate::origins::Origin::Pi => color::accent(),
        crate::origins::Origin::You => color::info(),
        crate::origins::Origin::Others => color::warn(),
    }
}
const CAPTURE: EventCallbackConfig = EventCallbackConfig {
    phases: Phases::CAPTURE,
};

/// Syntax colours; `true` draws the token in italics.
/// Editor colours from the theme, shared by every editor.
fn editor_colors(style: floem::views::EditorCustomStyle) -> floem::views::EditorCustomStyle {
    floem::views::editor::text::default_dark_color(style)
        .cursor_color(color::caret())
        .selection_color(color::selection())
        .current_line_color(color::current_line())
        .gutter_current_color(color::current_line())
        .gutter_dim_color(color::gutter_text())
        .gutter_accent_color(color::text())
        .indent_guide_color(color::border())
}

/// The editor's own background and text colour, which its custom style
/// cannot set; they go on the view, and on its gutter.
fn editor_surface(s: floem::style::Style) -> floem::style::Style {
    let background = color::editor_bg();
    s.background(background)
        .color(color::text())
        .class(floem::views::editor::gutter::GutterClass, move |s| {
            s.background(background)
        })
}

/// Editor styling shared by every code view, at a pane's zoom.
fn code_styling(zoom: f64) -> floem::views::editor::text::SimpleStyling {
    let size = (crate::settings::code_size() as f64 * zoom)
        .round()
        .max(6.0) as usize;
    floem::views::editor::text::SimpleStyling::builder()
        .font_family(floem::text::FamilyOwned::parse_list(&crate::settings::code_font()).collect())
        .font_size(size)
        .line_height(1.6)
        .build()
}

fn button(title: &'static str, action: impl Fn() + 'static) -> impl IntoView {
    Button::new(title).action(action).style(|s| {
        s.padding_horiz(10)
            .padding_vert(5)
            .border(0)
            .border_radius(6)
            .background(Color::TRANSPARENT)
            .color(color::muted())
            .hover(|s| s.background(color::border()).color(color::text()))
    })
}

/// A toolbar button that stays highlighted while its feature is on.
/// A little picture of the window with its side bar, for the explorer toggle.
fn sidebar_icon() -> impl IntoView {
    Container::new(Empty::new().style(|s| {
        s.absolute()
            .inset_left(3)
            .inset_top(0)
            .width(1)
            .height_full()
            .background(color::muted())
    }))
    .style(|s| {
        s.size(14, 11)
            .border(1)
            .border_color(color::muted())
            .border_radius(2)
    })
}

/// The window's top row: what is open, and one search box for files,
/// commands, Pi's commands and themes.
fn title_bar(state: Rc<Workbench>) -> impl IntoView {
    let root = state
        .root()
        .file_name()
        .unwrap_or(state.root().as_os_str())
        .to_string_lossy()
        .into_owned();
    let (search_state, explorer_state) = (state.clone(), state.clone());
    let layout = state.layout();
    let search = Stack::horizontal((
        Label::new("Search files, commands and Pi's /commands").style(|s| {
            s.color(color::muted())
                .flex_grow(1.0_f32)
                .min_width(0)
                .text_ellipsis()
        }),
        Label::new("Ctrl+P").style(|s| s.color(color::muted()).font_size(11)),
    ))
    .style(|s| {
        s.width(460)
            .max_width_pct(60.0)
            .height(26)
            .padding_horiz(12)
            .gap(10)
            .items_center()
            .border(1)
            .border_color(color::border())
            .border_radius(6)
            .background(color::bg())
            .cursor(CursorStyle::Pointer)
            .hover(|s| s.border_color(color::accent()))
    })
    .on_event_stop(el::Click, move |_, _| {
        search_state.open_finder(crate::workspace::Finder::Files)
    });
    let explorer_toggle = floem::views::tooltip(
        Container::new(sidebar_icon())
            .style(move |s| {
                s.padding(6)
                    .border_radius(5)
                    .cursor(CursorStyle::Pointer)
                    .hover(|s| s.background(color::border()))
                    .apply_if(layout.with(|l| l.explorer_visible), |s| {
                        s.background(color::highlight())
                    })
            })
            .on_event_stop(el::Click, move |_, _| explorer_state.toggle_explorer()),
        || Label::new("Explorer  (Ctrl+B)").style(|s| s.padding(4).font_size(11)),
    );
    Stack::horizontal((
        Stack::horizontal((
            Label::new("π").style(|s| {
                s.color(color::accent())
                    .font_size(18)
                    .font_family(CODE_FONT.to_string())
            }),
            Label::new(root).style(|s| s.color(color::text()).font_bold().text_ellipsis()),
        ))
        .style(|s| {
            s.gap(8)
                .items_center()
                .flex_basis(0)
                .flex_grow(1.0_f32)
                .min_width(0)
        }),
        search,
        Stack::horizontal((explorer_toggle,)).style(|s| {
            s.flex_basis(0)
                .flex_grow(1.0_f32)
                .justify_end()
                .items_center()
        }),
    ))
    .style(|s| {
        s.width_full()
            .height(40)
            .padding_horiz(14)
            .gap(12)
            .items_center()
            .flex_shrink(0.0_f32)
            .background(color::panel())
            .border_bottom(1)
            .border_color(color::border())
    })
}

/// The explorer's resize handle.
fn divider(state: Rc<Workbench>) -> impl IntoView {
    let dragging = RwSignal::new(None::<(f64, f64)>);
    let layout = state.layout();
    Empty::new()
        .style(move |s| {
            s.width(4)
                .height_full()
                .flex_shrink(0.0_f32)
                .background(color::border())
                .cursor(CursorStyle::ColResize)
                .hover(|s| s.background(color::accent()))
                .apply_if(layout.with(|l| !l.explorer_visible), |s| s.hide())
        })
        .on_event_stop(el::PointerDown, move |cx, event| {
            if let Some(id) = event.pointer.pointer_id {
                cx.request_pointer_capture(id);
                let x = event.state.logical_point().x;
                dragging.set(Some((x, layout.with_untracked(|l| l.explorer_width))));
            }
        })
        .on_event_stop(el::PointerMove, move |_, event| {
            if let Some((start_x, start_width)) = dragging.get_untracked() {
                let x = event.current.logical_point().x;
                let direction = if layout.with_untracked(|l| l.explorer_right) {
                    -1.0
                } else {
                    1.0
                };
                state.resize_explorer(start_width + direction * (x - start_x));
            }
        })
        .on_event_stop(el::PointerUp, move |_, _| dragging.set(None))
        .on_event_stop(el::LostPointerCapture, move |_, _| dragging.set(None))
}

/// The line between two panes of a split; dragging it moves the space
/// between them.
fn split_divider(
    sizes: RwSignal<Vec<f64>>,
    index: usize,
    horizontal: bool,
    container: Rc<Cell<Option<floem::ViewId>>>,
) -> impl IntoView {
    let dragging = RwSignal::new(None::<(f64, Vec<f64>, f64)>);
    let along = move |point: floem::kurbo::Point| if horizontal { point.x } else { point.y };
    Empty::new()
        .style(move |s| {
            let s = s
                .flex_shrink(0.0_f32)
                .background(color::border())
                .hover(|s| s.background(color::accent()));
            if horizontal {
                s.width(4).height_full().cursor(CursorStyle::ColResize)
            } else {
                s.height(4).width_full().cursor(CursorStyle::RowResize)
            }
        })
        .on_event_stop(el::PointerDown, move |cx, event| {
            if let Some(id) = event.pointer.pointer_id {
                cx.request_pointer_capture(id);
                let length = container
                    .get()
                    .and_then(|view| view.get_size())
                    .map_or(
                        1.0,
                        |size| if horizontal { size.width } else { size.height },
                    );
                dragging.set(Some((
                    along(event.state.logical_point()),
                    sizes.get_untracked(),
                    length.max(1.0),
                )));
            }
        })
        .on_event_stop(el::PointerMove, move |_, event| {
            let Some((start, shares, length)) = dragging.get_untracked() else {
                return;
            };
            if index + 1 >= shares.len() {
                return;
            }
            let delta = (along(event.current.logical_point()) - start) / length;
            let pair = shares[index] + shares[index + 1];
            let least = (pair / 2.0).min(0.06);
            let first = (shares[index] + delta).clamp(least, pair - least);
            let mut next = shares;
            next[index] = first;
            next[index + 1] = pair - first;
            sizes.set(next);
        })
        .on_event_stop(el::PointerUp, move |_, _| dragging.set(None))
        .on_event_stop(el::LostPointerCapture, move |_, _| dragging.set(None))
}

/// One node of the pane tree: a pane, or a split of further nodes.
fn node_view(state: Rc<Workbench>, node: Node) -> floem::AnyView {
    match node {
        Node::Leaf(id) => match state.panes.pane(id) {
            Some(found) => pane(state, found).into_any(),
            None => Empty::new().into_any(),
        },
        Node::Split {
            horizontal,
            children,
            sizes,
        } => {
            let container: Rc<Cell<Option<floem::ViewId>>> = Rc::default();
            let count = children.len();
            let mut views = Vec::new();
            for (index, child) in children.into_iter().enumerate() {
                if index > 0 {
                    views.push(
                        split_divider(sizes, index - 1, horizontal, container.clone()).into_any(),
                    );
                }
                views.push(
                    Container::new(node_view(state.clone(), child))
                        .style(move |s| {
                            let share = sizes
                                .with(|v| v.get(index).copied())
                                .unwrap_or(1.0 / count as f64);
                            s.flex_basis(0)
                                .flex_grow(share as f32)
                                .min_width(0)
                                .min_height(0)
                                .apply_if(horizontal, |s| s.height_full())
                                .apply_if(!horizontal, |s| s.width_full())
                        })
                        .into_any(),
                );
            }
            let stack = Stack::from_iter(views).style(move |s| {
                s.size_full()
                    .min_width(0)
                    .min_height(0)
                    .flex_direction(if horizontal {
                        floem::taffy::FlexDirection::Row
                    } else {
                        floem::taffy::FlexDirection::Column
                    })
            });
            container.set(Some(stack.view_id()));
            stack.into_any()
        }
    }
}

/// The dragged tab, following the pointer.
fn drag_ghost(state: Rc<Workbench>) -> impl IntoView {
    let dragging = state.dragging();
    let point = state.drag_point();
    let title_state = state.clone();
    Label::derived(move || {
        dragging
            .get()
            .map(|(_, tab)| match tab {
                Tab::File(path) => path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                Tab::Session(key) => title_state
                    .sessions
                    .find(key)
                    .map(|s| format!("π  {}", s.title().get()))
                    .unwrap_or_default(),
                Tab::Terminal(key) => title_state
                    .terminal(key)
                    .map(|slot| format!("›_  Terminal {}", slot.number))
                    .unwrap_or_default(),
            })
            .unwrap_or_default()
    })
    .style(move |s| {
        let at = point.get();
        s.absolute()
            .inset_left(at.x + 12.0)
            .inset_top(at.y + 10.0)
            .padding_horiz(10)
            .padding_vert(5)
            .border(1)
            .border_color(color::accent())
            .border_radius(6)
            .background(color::panel())
            .color(color::text())
            .z_index(40)
            .apply_if(dragging.with(Option::is_none), |s| s.hide())
    })
}

/// A popup menu: what a pane's + adds, or what can be done to a tab.
fn popup_menu(state: Rc<Workbench>) -> impl IntoView {
    let menu = state.menu();
    let window: Rc<Cell<Option<floem::ViewId>>> = Rc::default();
    let overlay_view = window.clone();
    let row_state = state.clone();
    let panel = dyn_container(
        move || menu.get(),
        move |open| {
            let Some(open) = open else {
                return Empty::new().into_any();
            };
            let count = open.items.len();
            let rows = open.items.into_iter().map(|item| match item.action {
                None => Empty::new()
                    .style(|s| {
                        s.width_full()
                            .height(1)
                            .margin_vert(4)
                            .background(color::border())
                    })
                    .into_any(),
                Some(action) => {
                    let state = row_state.clone();
                    Stack::horizontal((
                        Label::new(item.title).style(|s| s.flex_grow(1.0_f32)),
                        Label::new(item.hint).style(|s| s.color(color::muted()).font_size(11)),
                    ))
                    .style(|s| {
                        s.width_full()
                            .gap(18)
                            .padding_horiz(12)
                            .padding_vert(6)
                            .items_center()
                            .border_radius(5)
                            .cursor(floem::style::CursorStyle::Pointer)
                            .hover(|s| s.background(color::highlight()))
                    })
                    .on_event_stop(el::Click, move |_, _| {
                        state.dismiss_menu();
                        action();
                    })
                    .into_any()
                }
            });
            // Kept inside the window.
            let size = window
                .get()
                .map(|id| id.root().get_size().unwrap_or_default())
                .unwrap_or_default();
            let (width, height) = (260.0, count as f64 * 30.0 + 10.0);
            let x = open.at.x.min(size.width - width - 6.0).max(6.0);
            let y = if open.at.y + height > size.height {
                (open.at.y - height).max(6.0)
            } else {
                open.at.y
            };
            Stack::vertical_from_iter(rows)
                .style(move |s| {
                    s.absolute()
                        .inset_left(x)
                        .inset_top(y)
                        .width(width)
                        .padding(4)
                        .background(color::panel())
                        .color(color::text())
                        .border(1)
                        .border_color(color::border())
                        .border_radius(8)
                        .box_shadow_blur(12)
                        .box_shadow_color(Color::BLACK.with_alpha(0.3))
                })
                .into_any()
        },
    )
    .style(|s| s.absolute().inset(0));
    let dismiss = state.clone();
    let overlay = Stack::new((
        // A click anywhere else closes the menu.
        Empty::new()
            .style(|s| s.absolute().inset(0))
            .on_event_stop(el::PointerDown, move |_, _| dismiss.dismiss_menu()),
        panel,
    ))
    .style(move |s| {
        s.absolute()
            .inset(0)
            .z_index(45)
            .apply_if(menu.with(Option::is_none), |s| s.hide())
    });
    overlay_view.set(Some(overlay.view_id()));
    overlay
}

/// Every pane, or only the maximised one.
fn pane_tree(state: Rc<Workbench>) -> impl IntoView {
    let tree = state.panes.tree();
    let maximized = state.panes.maximized();
    // Rebuilt only when the arrangement really changes.
    let shape = floem::reactive::Memo::new(move |_| (tree.get(), maximized.get()));
    dyn_container(
        move || shape.get(),
        move |(tree, maximized)| match maximized.and_then(|id| state.panes.pane(id)) {
            Some(only) => pane(state.clone(), only).into_any(),
            None => node_view(state.clone(), tree),
        },
    )
    .style(|s| s.size_full().min_width(0).min_height(0))
}

pub fn workbench(window: WindowId, root: PathBuf, layout_path: PathBuf) -> impl IntoView {
    let (settings, settings_error) = crate::settings::current();
    let integrations = Integrations {
        pi: Launch::from_settings(settings.pi.command(), &settings.pi.args),
    };
    let state = Workbench::new(root, layout_path, integrations, move || {
        floem::close_window(window)
    });
    state.start_with_session();
    state.restore();
    state.pets.start();
    // The whole window drawn larger or smaller, as last left; set again only
    // when the zoom itself changes, not with every other layout change.
    let scale_layout = state.layout();
    let ui_scale = floem::reactive::Memo::new(move |_| scale_layout.with(|l| l.ui_scale));
    floem::reactive::Effect::new(move |_| floem::action::set_window_scale(ui_scale.get()));
    // Focusing a pane any way (keyboard, opening a file) gives it the zoom
    // keys back from the status bar.
    let (focus_state, focused_pane) = (state.clone(), state.focused());
    floem::reactive::Effect::new(move |_| {
        focused_pane.track();
        let zoom_window = focus_state.zoom_window();
        if zoom_window.get_untracked() {
            zoom_window.set(false);
        }
    });
    pet::follow_popping(state.clone(), window);
    let shake = pet::shake(&state);
    if let Some(err) = settings_error.or_else(color::follow_settings) {
        state.report(err);
    }
    state.sessions.refresh_recent();
    state.lens.start();
    let (synced, changed, tree) = (
        state.documents.clone(),
        state.lens.clone(),
        state.explorer.clone(),
    );
    // Changes from outside (a terminal, a formatter) fold in like Pi's.
    if let Some(watcher) =
        crate::watcher::watch(floem::reactive::Scope::current(), state.root(), move || {
            synced.sync_from_disk("disk");
            changed.refresh();
            tree.refresh();
        })
    {
        state.keep(watcher);
    }
    let layout = state.layout();
    let status = state.status();
    let focused = state.focused();
    let close_state = state.clone();
    let keyboard_state = state.clone();
    let pointer_state = state.clone();
    let body = Stack::horizontal((
        explorer(state.clone()),
        divider(state.clone()),
        pane_tree(state.clone()),
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
    let (dot_sessions, label_sessions) = (state.sessions.clone(), state.sessions.clone());
    let position_state = state.clone();
    let lens_open = RwSignal::new(false);
    let (summary_lens, menu_state) = (state.lens.clone(), state.clone());
    let lens_summary = Label::derived(move || {
        let label = summary_lens.label(summary_lens.since().get());
        summary_lens.changes().with(|changes| {
            let added: usize = changes.iter().map(|c| c.added).sum();
            let removed: usize = changes.iter().map(|c| c.removed).sum();
            match changes.len() {
                0 => format!("No changes since {label}"),
                1 => format!("Changes since {label}  ·  1 file  +{added} −{removed}"),
                n => format!("Changes since {label}  ·  {n} files  +{added} −{removed}"),
            }
        })
    })
    .style(|s| {
        s.color(color::accent())
            .padding_horiz(8)
            .padding_vert(2)
            .border_radius(5)
            .cursor(floem::style::CursorStyle::Pointer)
            .hover(|s| s.background(color::border()))
            .flex_shrink(0.0_f32)
    })
    .on_event_stop(el::Click, move |_, _| {
        lens_open.update(|open| *open = !*open)
    });
    // Who added the changed lines, as the editors colour them.
    let legend_changes = state.lens.changes();
    let legend = Stack::horizontal(
        [
            ("Pi", crate::origins::Origin::Pi),
            ("you", crate::origins::Origin::You),
            ("others", crate::origins::Origin::Others),
        ]
        .map(|(name, origin)| {
            Stack::horizontal((
                Empty::new().style(move |s| {
                    s.size(8, 8)
                        .border_radius(2)
                        .background(origin_color(origin))
                }),
                Label::new(name).style(|s| s.color(color::muted()).font_size(11)),
            ))
            .style(|s| s.gap(4).items_center())
        }),
    )
    .style(move |s| {
        s.gap(10)
            .items_center()
            .flex_shrink(0.0_f32)
            .apply_if(legend_changes.with(Vec::is_empty), |s| s.hide())
    });
    // Focus fades everything away from the changes; it lives by the lens.
    let (focus_lens, toggle_lens) = (state.lens.clone(), state.lens.clone());
    let focus_chip = Label::new("Focus")
        .style(move |s| {
            let on = focus_lens.focus().get();
            s.padding_horiz(8)
                .padding_vert(2)
                .border_radius(5)
                .cursor(CursorStyle::Pointer)
                .color(if on { color::accent() } else { color::muted() })
                .hover(|s| s.background(color::border()))
                .apply_if(on, |s| s.background(color::highlight()))
        })
        .on_event_stop(el::Click, move |_, _| toggle_lens.toggle_focus());
    // Where the pet went, when it is not in the window; a click brings it
    // back.
    let pet_chip = {
        let (pets, front) = (state.pets.clone(), state.sessions.active_key());
        let (out, hidden) = (pets.out, pets.hidden);
        let away = move || {
            if hidden.get() {
                Some("Glitch is hidden")
            } else if out.get().is_some() && out.get() == front.get() {
                Some("Glitch is out")
            } else {
                None
            }
        };
        let label_away = away;
        Label::derived(move || label_away().unwrap_or_default().to_string())
            .style(move |s| {
                s.color(color::accent())
                    .padding_horiz(6)
                    .border_radius(5)
                    .cursor(floem::style::CursorStyle::Pointer)
                    .hover(|s| s.background(color::border()))
                    .apply_if(away().is_none(), |s| s.hide())
            })
            .on_event_stop(el::Click, move |_, _| pets.find())
    };
    // The whole window's zoom, shown when it is not the usual; a click puts
    // it back.
    let window_zoom = {
        let (layout, zoom_state) = (state.layout(), state.clone());
        Label::derived(move || {
            format!(
                "Window {}%",
                (layout.with(|l| l.ui_scale) * 100.0).round() as i64
            )
        })
        .style(move |s| {
            s.color(color::muted())
                .padding_horiz(6)
                .border_radius(5)
                .cursor(floem::style::CursorStyle::Pointer)
                .hover(|s| s.background(color::border()).color(color::text()))
                .apply_if(layout.with(|l| l.ui_scale) == 1.0, |s| s.hide())
        })
        .on_event_stop(el::Click, move |_, _| {
            zoom_state.zoom_window().set(true);
            zoom_state.zoom_focused(0);
        })
    };
    let (footer_zoom, footer_state) = (state.zoom_window(), state.clone());
    let footer = Stack::horizontal((
        window_zoom,
        lens_summary,
        focus_chip,
        legend,
        pet_chip,
        Label::derived(move || status.get()).style(|s| {
            s.color(color::muted())
                .flex_grow(1.0_f32)
                .min_width(0)
                .text_ellipsis()
        }),
        Empty::new().style(move |s| {
            let status = dot_sessions.active().map(|s| s.status());
            s.size(7, 7)
                .border_radius(4)
                .background(status.map_or(color::muted(), session::status_color))
        }),
        Label::derived(move || {
            label_sessions
                .active()
                .map_or("No session".into(), |session| {
                    let model = session.model().get().map(|m| m.name).unwrap_or_default();
                    format!("Pi {}  {model}", session::status_label(session.status()))
                })
        })
        .style(|s| s.color(color::muted())),
        Label::derived(move || {
            let Some(pane) = position_state.panes.pane(focused.get()) else {
                return String::new();
            };
            if !matches!(pane.active.get(), Some(Tab::File(_))) {
                return String::new();
            }
            let (line, col) = (pane.cursor_line.get(), pane.cursor_col.get());
            format!("Ln {}, Col {}  ·  UTF-8", line + 1, col + 1)
        })
        .style(|s| s.color(color::muted()).margin_left(8)),
    ))
    .style(move |s| {
        // Clicked, it takes the zoom keys, and shows it.
        let zooming = footer_zoom.get();
        s.width_full()
            .height(28)
            .padding_horiz(12)
            .gap(10)
            .font_size(11)
            .items_center()
            .flex_shrink(0.0_f32)
            .background(color::panel())
            .border_top(1)
            .border_color(if zooming {
                color::accent()
            } else {
                color::border()
            })
    })
    .on_event_cont(el::PointerDown, move |_, _| {
        footer_state.zoom_window().set(true)
    });
    Stack::new((
        Stack::vertical((
            title_bar(state.clone()),
            Stack::horizontal((rail::rail(state.clone()), body))
                .style(|s| s.size_full().min_width(0).min_height(0)),
            footer,
        ))
        .style(|s| s.size_full()),
        lens_menu(menu_state, lens_open),
        pet::docked(state.clone(), window),
        popup_menu(state.clone()),
        drag_ghost(state.clone()),
        finder::finder(state.clone()),
        confirmation(state),
    ))
    .style(move |s| {
        // A /kill shakes everything.
        let (x, y) = shake.get();
        s.size_full()
            .background(color::bg())
            .color(color::text())
            .font_size(13)
            .translate_x(x)
            .translate_y(y)
    })
    // Tab drags are followed here, over the whole window.
    .on_event_cont_with_config(el::PointerMove, CAPTURE, {
        let state = pointer_state.clone();
        move |_, event| {
            let held = !event.current.buttons.is_empty();
            let at = event.current.logical_point();
            state.pointer_moved(at, held);
            state.pets.pointer_at(at);
        }
    })
    .on_event_cont_with_config(el::PointerDown, CAPTURE, {
        let state = pointer_state.clone();
        move |_, event| state.note_pointer(event.state.logical_point())
    })
    .on_event_cont_with_config(el::PointerUp, CAPTURE, move |_, _| {
        pointer_state.pointer_released();
    })
    .on_event_cont(el::WindowCloseRequested, move |cx, _| {
        cx.prevent_default();
        close_state.request_quit();
    })
    .on_event_with_config(el::KeyDown, CAPTURE, move |cx, event| {
        if event.key == Key::Named(NamedKey::Escape) {
            if keyboard_state.menu().with_untracked(Option::is_some) {
                keyboard_state.dismiss_menu();
                return EventPropagation::Stop;
            }
            keyboard_state.dismiss_finder();
            keyboard_state.dismiss_confirmation();
            return EventPropagation::Continue;
        }
        if let Some(command) = commands::for_key(&event.key, event.modifiers) {
            // While the finder has the keyboard, only finder keys apply.
            let finding = keyboard_state.finder().get_untracked().is_some();
            if finding && !matches!(command, Command::QuickOpen | Command::FindInFiles) {
                return EventPropagation::Continue;
            }
            // The shell gets Ctrl keys such as Ctrl+P and Ctrl+W, except these.
            let in_terminal = keyboard_state.keys_in_terminal();
            if in_terminal
                && !matches!(
                    command,
                    Command::Terminal
                        | Command::NewTerminal
                        | Command::Palette
                        | Command::NextSession
                        | Command::PreviousSession
                        | Command::Session(_)
                        | Command::FocusPane(_)
                        | Command::MoveTab(_)
                        | Command::MaximizePane
                        | Command::NextTab
                        | Command::PreviousTab
                        | Command::ZoomIn
                        | Command::ZoomOut
                        | Command::ZoomReset
                )
            {
                return EventPropagation::Continue;
            }
            commands::execute(&keyboard_state, command);
            cx.prevent_default();
            return EventPropagation::Stop;
        }
        EventPropagation::Continue
    })
}

/// What the changes lens compares against, chosen from the status bar.
fn lens_menu(state: Rc<Workbench>, open: RwSignal<bool>) -> impl IntoView {
    let since = state.lens.since();
    let items = Since::ALL.map(|choice| {
        let (label_state, pick_state) = (state.clone(), state.clone());
        Label::derived(move || {
            let mark = if since.get() == choice {
                "●  "
            } else {
                "    "
            };
            format!("{mark}{}", label_state.lens.label(choice))
        })
        .style(|s| {
            s.width_full()
                .padding_horiz(12)
                .padding_vert(7)
                .color(color::text())
                .cursor(floem::style::CursorStyle::Pointer)
                .hover(|s| s.background(color::border()))
        })
        .on_event_stop(el::Click, move |_, _| {
            pick_state.lens.set_since(choice);
            open.set(false);
        })
        .into_any()
    });
    Stack::vertical((
        Label::new("Compare everything with").style(|s| {
            s.color(color::muted())
                .font_size(11)
                .padding_horiz(12)
                .padding_vert(6)
        }),
        Stack::vertical_from_iter(items).style(|s| s.width_full()),
    ))
    .style(move |s| {
        s.absolute()
            .inset_bottom(32)
            .inset_left(12)
            .width(260)
            .padding_vert(4)
            .background(color::panel())
            .border(1)
            .border_color(color::border())
            .border_radius(8)
            .z_index(20)
            .apply_if(!open.get(), |s| s.hide())
    })
}
