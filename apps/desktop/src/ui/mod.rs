//! Native window composition and styling. User actions are implemented in workspace.rs.
mod diff;
mod editor;
mod explorer;
mod overlays;
mod rail;
mod session;

use crate::{
    commands::{self, Command},
    workspace::{Integrations, Pane, Workbench},
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
use overlays::{command_palette, confirmation};
use pi_client::Launch;
use std::{path::PathBuf, rc::Rc, sync::Arc};
use workbench_core::baseline::GitHead;

const BG: Color = Color::from_rgb8(21, 24, 30);
const PANEL: Color = Color::from_rgb8(28, 32, 40);
const BORDER: Color = Color::from_rgb8(47, 54, 66);
const TEXT: Color = Color::from_rgb8(218, 224, 233);
const MUTED: Color = Color::from_rgb8(143, 156, 176);
const ACCENT: Color = Color::from_rgb8(128, 190, 175);
/// Raised cards inside panels.
const SURFACE: Color = Color::from_rgb8(34, 39, 49);
const WARN: Color = Color::from_rgb8(229, 184, 100);
const INFO: Color = Color::from_rgb8(122, 162, 247);
/// Line backgrounds in the changes view and tool previews.
const ADDED: Color = Color::from_rgba8(80, 170, 110, 46);
const REMOVED: Color = Color::from_rgba8(210, 90, 90, 46);
const ADDED_TEXT: Color = Color::from_rgb8(134, 207, 150);
const REMOVED_TEXT: Color = Color::from_rgb8(236, 135, 135);
/// CSS-style family list; the first installed font wins.
const CODE_FONT: &str =
    "'Cascadia Code', 'JetBrains Mono', Consolas, 'DejaVu Sans Mono', monospace";
const CODE_SIZE: usize = 14;
const CAPTURE: EventCallbackConfig = EventCallbackConfig {
    phases: Phases::CAPTURE,
};

/// Editor styling shared by every code view.
fn code_styling() -> floem::views::editor::text::SimpleStyling {
    floem::views::editor::text::SimpleStyling::builder()
        .font_family(floem::text::FamilyOwned::parse_list(CODE_FONT).collect())
        .font_size(CODE_SIZE)
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
            .color(MUTED)
            .hover(|s| s.background(BORDER).color(TEXT))
    })
}

/// A toolbar button that stays highlighted while its feature is on.
fn toggle_button(
    state: Rc<Workbench>,
    title: &'static str,
    command: Command,
    on: impl Fn() -> bool + 'static,
) -> impl IntoView {
    button(title, move || commands::execute(&state, command)).style(move |s| {
        s.apply_if(on(), |s| {
            s.background(ACCENT.with_alpha(0.16))
                .color(ACCENT)
                .hover(|s| s.background(ACCENT.with_alpha(0.26)).color(ACCENT))
        })
    })
}

fn command_button(state: Rc<Workbench>, title: &'static str, command: Command) -> impl IntoView {
    button(title, move || commands::execute(&state, command))
}

fn separator() -> impl IntoView {
    Empty::new().style(|s| s.width(1).height(20).margin_horiz(4).background(BORDER))
}

fn toolbar(state: Rc<Workbench>) -> impl IntoView {
    let root = state
        .root()
        .file_name()
        .unwrap_or(state.root().as_os_str())
        .to_string_lossy()
        .into_owned();
    let layout = state.layout();
    let comparing = state.changes.current();
    Stack::horizontal((
        Label::new("π").style(|s| {
            s.color(ACCENT)
                .font_size(20)
                .font_family(CODE_FONT.to_string())
        }),
        Label::new(root).style(|s| {
            s.color(TEXT)
                .font_bold()
                .flex_grow(1.0_f32)
                .min_width(0)
                .text_ellipsis()
        }),
        command_button(state.clone(), "Open folder", Command::OpenFolder),
        command_button(state.clone(), "Save", Command::Save),
        separator(),
        toggle_button(state.clone(), "Explorer", Command::Explorer, move || {
            layout.with(|l| l.explorer_visible)
        }),
        toggle_button(state.clone(), "Split", Command::Split, move || {
            layout.with(|l| l.split)
        }),
        toggle_button(state.clone(), "Changes", Command::Changes, move || {
            comparing.with(Option::is_some)
        }),
        separator(),
        command_button(state, "Commands", Command::Palette),
    ))
    .style(|s| {
        s.width_full()
            .height(48)
            .padding_horiz(14)
            .gap(4)
            .items_center()
            .flex_shrink(0.0_f32)
            .background(PANEL)
            .border_bottom(1)
            .border_color(BORDER)
    })
}

fn divider(state: Rc<Workbench>, sidebar: bool, width: RwSignal<f64>) -> impl IntoView {
    let dragging = RwSignal::new(None::<(f64, f64)>);
    let layout = state.layout();
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
                let layout = layout.get_untracked();
                if sidebar {
                    let direction = if layout.explorer_right { -1.0 } else { 1.0 };
                    state.resize_explorer(start_value + direction * (x - start_x));
                } else {
                    let explorer_width = if layout.explorer_visible {
                        layout.explorer_width + 5.0
                    } else {
                        0.0
                    };
                    let available = (width.get_untracked() - explorer_width - 5.0).max(200.0);
                    state.resize_split(start_value + (x - start_x) / available);
                }
            }
        })
        .on_event_stop(el::PointerUp, move |_, _| dragging.set(None))
        .on_event_stop(el::LostPointerCapture, move |_, _| dragging.set(None))
}

pub fn workbench(window: WindowId, root: PathBuf, layout_path: PathBuf) -> impl IntoView {
    let integrations = Integrations {
        baseline: Arc::new(GitHead),
        pi: Launch::from_env(),
    };
    let state = Workbench::new(root, layout_path, integrations, move || {
        floem::close_window(window)
    });
    state.restore();
    state.sessions.refresh_recent();
    state.sessions.create();
    let layout = state.layout();
    let status = state.status();
    let focused = state.focused();
    let width = RwSignal::new(1280.0);
    let close_state = state.clone();
    let keyboard_state = state.clone();
    let comparing = state.changes.current();
    let editors = Stack::horizontal((
        pane(state.clone(), Pane::Primary),
        divider(state.clone(), false, width),
        pane(state.clone(), Pane::Secondary),
    ))
    .style(move |s| {
        s.size_full()
            .min_width(0)
            .min_height(0)
            .apply_if(comparing.with(Option::is_some), |s| s.hide())
    });
    let body = Stack::horizontal((
        explorer(state.clone()),
        divider(state.clone(), true, width),
        editors,
        diff::changes(state.clone()),
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
    let footer = Stack::horizontal((
        Label::derived(move || status.get()).style(|s| {
            s.color(MUTED)
                .flex_grow(1.0_f32)
                .min_width(0)
                .text_ellipsis()
        }),
        Empty::new().style(move |s| {
            let status = dot_sessions.active().map(|s| s.status());
            s.size(7, 7)
                .border_radius(4)
                .background(status.map_or(MUTED, session::status_color))
        }),
        Label::derived(move || {
            label_sessions
                .active()
                .map_or("No session".into(), |session| {
                    let model = session.model().get().map(|m| m.name).unwrap_or_default();
                    format!("Pi {}  {model}", session::status_label(session.status()))
                })
        })
        .style(|s| s.color(MUTED)),
        Label::derived(move || format!("Pane {}  ·  UTF-8", focused.get().index() + 1))
            .style(|s| s.color(MUTED).margin_left(8)),
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
        Stack::vertical((
            toolbar(state.clone()),
            Stack::horizontal((rail::rail(state.clone()), body))
                .style(|s| s.size_full().min_width(0).min_height(0)),
            footer,
        ))
        .style(|s| s.size_full()),
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
        if event.key == Key::Named(NamedKey::Escape) {
            keyboard_state.dismiss_palette();
            keyboard_state.dismiss_confirmation();
            return EventPropagation::Continue;
        }
        if let Some(command) = commands::for_key(&event.key, event.modifiers) {
            commands::execute(&keyboard_state, command);
            cx.prevent_default();
            return EventPropagation::Stop;
        }
        EventPropagation::Continue
    })
}
