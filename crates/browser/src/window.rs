//! Each tab's native window, a child of the workbench's: placed, shown and
//! hidden. On Windows it's an HWND; on Linux, where it's tried out, an X11
//! window.
//!
//! A tab's browser is made in a frame: on Windows the workbench's window
//! itself, on Linux a window of its own inside it, as Chromium can't make
//! its window straight in GTK's, whose visual isn't the screen's default.

use cef::{Rect, WindowInfo, sys::cef_window_handle_t};

/// A tab's size until it's placed: a page out of sight, like one Pi uses,
/// still lays out and draws at it.
pub const UNPLACED: Rect = Rect {
    x: 0,
    y: 0,
    width: 1280,
    height: 800,
};

#[cfg(windows)]
mod platform {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    use super::*;

    pub fn frame(parent: cef_window_handle_t) -> cef_window_handle_t {
        parent
    }

    /// Made without being shown: it shows once placed.
    pub fn hidden(info: WindowInfo) -> WindowInfo {
        WindowInfo {
            style: info.style & !WS_VISIBLE,
            ..info
        }
    }

    pub fn place(
        _frame: cef_window_handle_t,
        browser: cef_window_handle_t,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) {
        unsafe {
            SetWindowPos(
                browser,
                HWND_TOP,
                x,
                y,
                width,
                height,
                SWP_SHOWWINDOW | SWP_NOACTIVATE,
            );
        }
    }

    pub fn hide(_frame: cef_window_handle_t, browser: cef_window_handle_t) {
        unsafe {
            ShowWindow(browser, SW_HIDE);
        }
    }

    pub fn close(_frame: cef_window_handle_t) {}
}

#[cfg(target_os = "linux")]
mod platform {
    use std::cell::OnceCell;

    use x11_dl::xlib::{
        CWBorderPixel, CWColormap, Display, InputOutput, XSetWindowAttributes, Xlib,
    };

    use super::*;

    thread_local! {
        static X: OnceCell<Option<(Xlib, *mut Display)>> = const { OnceCell::new() };
    }

    fn with<T: Default>(work: impl FnOnce(&Xlib, *mut Display) -> T) -> T {
        X.with(|x| {
            let x = x.get_or_init(|| {
                let xlib = Xlib::open().ok()?;
                let display = unsafe { (xlib.XOpenDisplay)(std::ptr::null()) };
                (!display.is_null()).then_some((xlib, display))
            });
            let Some((xlib, display)) = x else {
                return T::default();
            };
            let done = work(xlib, *display);
            unsafe { (xlib.XFlush)(*display) };
            done
        })
    }

    /// A window of the screen's default visual, unmapped until placed.
    pub fn frame(parent: cef_window_handle_t) -> cef_window_handle_t {
        with(|xlib, display| unsafe {
            let screen = (xlib.XDefaultScreen)(display);
            let visual = (xlib.XDefaultVisual)(display, screen);
            let mut attributes: XSetWindowAttributes = std::mem::zeroed();
            attributes.colormap = (xlib.XDefaultColormap)(display, screen);
            (xlib.XCreateWindow)(
                display,
                parent,
                0,
                0,
                UNPLACED.width as u32,
                UNPLACED.height as u32,
                0,
                (xlib.XDefaultDepth)(display, screen),
                InputOutput as u32,
                visual,
                CWColormap | CWBorderPixel,
                &mut attributes,
            )
        })
    }

    pub fn hidden(info: WindowInfo) -> WindowInfo {
        info
    }

    /// The frame goes where the tab does; the browser fills it.
    pub fn place(
        frame: cef_window_handle_t,
        browser: cef_window_handle_t,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) {
        let (width, height) = (width.max(1) as u32, height.max(1) as u32);
        with(|xlib, display| unsafe {
            (xlib.XMoveResizeWindow)(display, frame, x, y, width, height);
            (xlib.XMoveResizeWindow)(display, browser, 0, 0, width, height);
            (xlib.XMapWindow)(display, browser);
            (xlib.XMapRaised)(display, frame);
        });
    }

    pub fn hide(frame: cef_window_handle_t, _browser: cef_window_handle_t) {
        with(|xlib, display| unsafe {
            (xlib.XUnmapWindow)(display, frame);
        });
    }

    /// Once its browser has gone.
    pub fn close(frame: cef_window_handle_t) {
        with(|xlib, display| unsafe {
            (xlib.XDestroyWindow)(display, frame);
        });
    }
}

pub use platform::{close, frame, hidden, hide, place};
