//! The tabs, on Chromium's UI thread: made on the app's word, each named by
//! its DevTools target id once it knows it, and shown where the app says.
//! A popup a page opens is a tab too, which the app hears about.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

use cef::*;
use serde::Deserialize;
use serde_json::json;

use crate::{handlers, say, window};

/// The workbench window every tab is drawn in.
static PARENT: AtomicU64 = AtomicU64::new(0);

pub fn set_parent(parent: u64) {
    PARENT.store(parent, Ordering::Relaxed);
}

pub fn parent() -> sys::cef_window_handle_t {
    window::handle(PARENT.load(Ordering::Relaxed))
}

struct Tab {
    browser: Browser,
    /// What it's drawn in, for one of ours: not one DevTools made.
    frame: Option<sys::cef_window_handle_t>,
    /// Its DevTools target id, once it has said.
    target: Option<String>,
    /// The app's request it answers, if the app asked for it.
    ask: Option<u64>,
    /// A popup's size, as the page asked for it.
    popup: Option<(i32, i32)>,
}

/// A browser about to be made: known just before it exists.
struct Coming {
    /// The app's request it answers, if it's one the app asked for.
    ask: Option<u64>,
    /// A popup's size, if it's a popup.
    popup: Option<(i32, i32)>,
    frame: sys::cef_window_handle_t,
}

#[derive(Default)]
struct State {
    tabs: HashMap<i32, Tab>,
    coming: Option<Coming>,
    /// Kept so the observers keep listening.
    observers: Vec<Registration>,
    quitting: bool,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::default();
}

#[derive(Deserialize)]
struct Command {
    open: Option<Open>,
    place: Option<Place>,
    scale: Option<f64>,
    quit: Option<bool>,
}

#[derive(Deserialize)]
struct Open {
    ask: u64,
    url: String,
}

/// Where a tab's page area is, in the window's CSS pixels.
#[derive(Deserialize)]
struct Place {
    tab: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    visible: bool,
}

wrap_task! {
    struct Run {
        line: String,
    }

    impl Task {
        fn execute(&self) {
            run(&self.line);
        }
    }
}

/// Runs one of the app's lines on the UI thread.
pub fn post(line: String) {
    let mut task = Run::new(line);
    post_task(ThreadId::UI, Some(&mut task));
}

fn run(line: &str) {
    let command = match serde_json::from_str::<Command>(line) {
        Ok(command) => command,
        Err(err) => return eprintln!("wb-browser: a line it doesn't understand: {err}"),
    };
    if let Some(open) = command.open {
        create(open);
    }
    if let Some(place) = command.place {
        show(&place, command.scale.unwrap_or(1.0));
    }
    if command.quit == Some(true) {
        quit();
    }
}

fn create(open: Open) {
    let frame = window::frame(parent());
    let info = WindowInfo {
        runtime_style: RuntimeStyle::CHROME,
        ..Default::default()
    }
    .set_as_child(frame, &window::UNPLACED);
    let info = window::hidden(info);
    let mut client = handlers::client();
    STATE.with(|s| {
        s.borrow_mut().coming = Some(Coming {
            ask: Some(open.ask),
            popup: None,
            frame,
        })
    });
    let made = browser_host_create_browser_sync(
        Some(&info),
        Some(&mut client),
        Some(&CefString::from(open.url.as_str())),
        Some(&BrowserSettings::default()),
        None,
        None,
    );
    let unused = STATE.with(|s| s.borrow_mut().coming.take());
    if made.is_none() {
        if let Some(coming) = unused {
            window::close(coming.frame);
        }
        say(json!({ "failed": { "ask": open.ask } }));
    }
}

/// A popup is about to be made, this size if the page said: what to draw
/// it in.
pub fn popup_coming(size: Option<(i32, i32)>) -> sys::cef_window_handle_t {
    let frame = window::frame(parent());
    STATE.with(|s| {
        s.borrow_mut().coming = Some(Coming {
            ask: None,
            popup: Some(size.unwrap_or((0, 0))),
            frame,
        })
    });
    frame
}

/// A browser exists: ours, or a page's popup. It's hidden until placed, and
/// asked for its target id.
pub fn created(browser: &mut Browser) {
    let id = browser.identifier();
    let Some(host) = browser.host() else { return };
    let coming = STATE.with(|s| s.borrow_mut().coming.take());
    if let Some(coming) = &coming {
        window::hide(coming.frame, host.window_handle());
    }
    let mut observer = handlers::observer();
    let registration = host.add_dev_tools_message_observer(Some(&mut observer));
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        state.tabs.insert(
            id,
            Tab {
                browser: browser.clone(),
                frame: coming.as_ref().map(|c| c.frame),
                target: None,
                ask: coming.as_ref().and_then(|c| c.ask),
                popup: coming.as_ref().and_then(|c| c.popup),
            },
        );
        state.observers.extend(registration);
    });
    host.execute_dev_tools_method(id, Some(&CefString::from("Target.getTargetInfo")), None);
}

/// A browser said its target id: the app hears what it is.
pub fn named(browser: i32, target: String) {
    STATE.with(|s| {
        let mut state = s.borrow_mut();
        let Some(tab) = state.tabs.get_mut(&browser) else {
            return;
        };
        if tab.target.is_some() {
            return;
        }
        tab.target = Some(target.clone());
        match (tab.ask, tab.popup) {
            (Some(ask), _) => say(json!({ "opened": { "ask": ask, "tab": target } })),
            (None, Some((width, height))) => {
                say(json!({ "popup": { "tab": target, "width": width, "height": height } }))
            }
            (None, None) => say(json!({ "created": { "tab": target } })),
        }
    });
}

pub fn closed(browser: i32) {
    let empty = STATE.with(|s| {
        let mut state = s.borrow_mut();
        if let Some(tab) = state.tabs.remove(&browser) {
            if let Some(target) = tab.target {
                say(json!({ "closed": { "tab": target } }));
            }
            if let Some(frame) = tab.frame {
                window::close(frame);
            }
        }
        state.quitting && state.tabs.is_empty()
    });
    if empty {
        quit_message_loop();
    }
}

fn show(place: &Place, scale: f64) {
    STATE.with(|s| {
        let state = s.borrow();
        let Some(tab) = state
            .tabs
            .values()
            .find(|t| t.target.as_deref() == Some(&place.tab))
        else {
            return;
        };
        let (Some(frame), Some(host)) = (tab.frame, tab.browser.host()) else {
            return;
        };
        let handle = host.window_handle();
        if !place.visible || place.width <= 0 || place.height <= 0 {
            return window::hide(frame, handle);
        }
        let px = |v: i32| (f64::from(v) * scale).round() as i32;
        window::place(
            frame,
            handle,
            px(place.x),
            px(place.y),
            px(place.width),
            px(place.height),
        );
        host.set_focus(1);
    });
}

/// Closes every tab, then stops once they've all gone.
fn quit() {
    let browsers: Vec<Browser> = STATE.with(|s| {
        let mut state = s.borrow_mut();
        state.quitting = true;
        state.tabs.values().map(|tab| tab.browser.clone()).collect()
    });
    if browsers.is_empty() {
        return quit_message_loop();
    }
    for browser in browsers {
        if let Some(host) = browser.host() {
            host.close_browser(1);
        }
    }
}
