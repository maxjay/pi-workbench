//! The window's own frame, where the system draws none: WSLg's Wayland
//! takes on decorating windows and then draws nothing, leaving no title bar
//! to move, resize or close the window by. There the workbench's top bar
//! moves it (double-click maximizes), its edges resize it, and buttons at
//! the top right minimize, maximize and close it.
use super::color;
use crate::workspace::Workbench;
use floem::{
    action::{drag_resize_window, drag_window, minimize_window, toggle_window_maximized},
    prelude::*,
    style::CursorStyle,
    window::ResizeDirection,
};
use std::{
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
};

static OWN_FRAME: AtomicBool = AtomicBool::new(false);

/// Whether the workbench draws its own frame.
pub fn own_frame() -> bool {
    OWN_FRAME.load(Ordering::Relaxed)
}

/// Draws the frame from now on; decided once, at start-up, on WSL only.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn use_own_frame() {
    OWN_FRAME.store(true, Ordering::Relaxed);
}

/// Makes `view` a place to move the window from, and to maximize it with a
/// double-click, when the workbench draws its own frame.
pub fn drag_area<V: IntoView + 'static>(view: V) -> impl IntoView {
    view.into_view()
        .on_event_cont(el::PointerDown, |_, event| {
            let primary = event
                .button
                .is_none_or(|b| b == floem::ui_events::pointer::PointerButton::Primary);
            if own_frame() && primary {
                drag_window();
            }
        })
        .on_event_cont(el::DoubleClick, |_, _| {
            if own_frame() {
                toggle_window_maximized();
            }
        })
}

const MINIMIZE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="M2 5.5 H8" fill="none" stroke="currentColor" stroke-width="1"/></svg>"#;
const MAXIMIZE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect x="2" y="2" width="6" height="6" fill="none" stroke="currentColor" stroke-width="1"/></svg>"#;
const CLOSE: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><path d="M2.5 2.5 L7.5 7.5 M7.5 2.5 L2.5 7.5" fill="none" stroke="currentColor" stroke-width="1"/></svg>"#;

fn control(icon: &'static str, closes: bool, act: impl Fn() + 'static) -> impl IntoView {
    Container::new(
        floem::views::svg(icon.to_string()).style(|s| s.size(12, 12).color(color::muted())),
    )
    .style(move |s| {
        s.width(38)
            .height(28)
            .items_center()
            .justify_center()
            .border_radius(5)
            .cursor(CursorStyle::Pointer)
            .apply_if(closes, |s| {
                s.hover(|s| s.background(Color::from_rgb8(0xC4, 0x2B, 0x1C)))
            })
            .apply_if(!closes, |s| s.hover(|s| s.background(color::border())))
    })
    .on_event_stop(el::Click, move |_, _| act())
}

/// Minimize, maximize and close, when the workbench draws its own frame;
/// nothing otherwise.
pub fn window_controls(state: Rc<Workbench>) -> impl IntoView {
    Stack::horizontal((
        control(MINIMIZE, false, minimize_window),
        control(MAXIMIZE, false, toggle_window_maximized),
        // Closing asks about unsaved files, as the system's close does.
        control(CLOSE, true, move || state.quit()),
    ))
    .style(|s| {
        s.gap(2)
            .margin_left(6)
            .items_center()
            .apply_if(!own_frame(), |s| s.hide())
    })
}

fn edge(direction: ResizeDirection, cursor: CursorStyle) -> impl IntoView {
    const EDGE: f64 = 5.0;
    const CORNER: f64 = 12.0;
    Empty::new()
        .style(move |s| {
            let s = s.absolute().cursor(cursor);
            match direction {
                ResizeDirection::North => s
                    .inset_top(0)
                    .inset_left(CORNER)
                    .inset_right(CORNER)
                    .height(EDGE),
                ResizeDirection::South => s
                    .inset_bottom(0)
                    .inset_left(CORNER)
                    .inset_right(CORNER)
                    .height(EDGE),
                ResizeDirection::West => s
                    .inset_left(0)
                    .inset_top(CORNER)
                    .inset_bottom(CORNER)
                    .width(EDGE),
                ResizeDirection::East => s
                    .inset_right(0)
                    .inset_top(CORNER)
                    .inset_bottom(CORNER)
                    .width(EDGE),
                ResizeDirection::NorthWest => s.inset_left(0).inset_top(0).size(CORNER, CORNER),
                ResizeDirection::NorthEast => s.inset_right(0).inset_top(0).size(CORNER, CORNER),
                ResizeDirection::SouthWest => s.inset_left(0).inset_bottom(0).size(CORNER, CORNER),
                ResizeDirection::SouthEast => s.inset_right(0).inset_bottom(0).size(CORNER, CORNER),
            }
        })
        .on_event_stop(el::PointerDown, move |_, event| {
            if event
                .button
                .is_none_or(|b| b == floem::ui_events::pointer::PointerButton::Primary)
            {
                drag_resize_window(direction);
            }
        })
}

/// `content`, with thin strips along the window's edges and corners to
/// resize it by when the workbench draws its own frame. Each strip covers
/// only its edge; the content beneath takes the pointer everywhere else.
pub fn with_resize_edges(content: impl IntoView + 'static) -> floem::AnyView {
    if !own_frame() {
        return content.into_any();
    }
    use ResizeDirection::*;
    Stack::new((
        content,
        edge(North, CursorStyle::NResize),
        edge(South, CursorStyle::SResize),
        edge(West, CursorStyle::WResize),
        edge(East, CursorStyle::EResize),
        edge(NorthWest, CursorStyle::NwResize),
        edge(NorthEast, CursorStyle::NeResize),
        edge(SouthWest, CursorStyle::SwResize),
        edge(SouthEast, CursorStyle::SeResize),
    ))
    .style(|s| s.size_full())
    .into_any()
}
