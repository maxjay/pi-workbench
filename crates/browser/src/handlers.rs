//! What CEF calls back: the app's start-up, each browser's life, its keys,
//! and DevTools' answers. The work is in `tabs`.

use cef::*;
use serde_json::{Value, json};

use crate::{say, tabs};

pub fn app(extensions: Option<String>) -> App {
    WbApp::new(extensions.unwrap_or_default())
}

pub fn client() -> Client {
    WbClient::new()
}

pub fn observer() -> DevToolsMessageObserver {
    Observer::new()
}

wrap_app! {
    struct WbApp {
        extensions: String,
    }

    impl App {
        fn on_before_command_line_processing(
            &self,
            process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            // The browser process's own switches; helpers get theirs from it.
            let browser = process_type.is_none_or(|t| t.to_string().is_empty());
            let Some(line) = command_line else { return };
            if browser && !self.extensions.is_empty() {
                line.append_switch_with_value(
                    Some(&CefString::from("load-extension")),
                    Some(&CefString::from(self.extensions.as_str())),
                );
            }
        }

        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(Process::new())
        }
    }
}

wrap_browser_process_handler! {
    struct Process {}

    impl BrowserProcessHandler {
        fn on_context_initialized(&self) {
            say(json!({ "ready": true }));
        }

        /// Windows Chromium opens itself get the same callbacks as ours.
        fn default_client(&self) -> Option<Client> {
            Some(client())
        }
    }
}

wrap_client! {
    struct WbClient {}

    impl Client {
        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(Life::new())
        }

        fn keyboard_handler(&self) -> Option<KeyboardHandler> {
            Some(Keys::new())
        }
    }
}

wrap_life_span_handler! {
    struct Life {}

    impl LifeSpanHandler {
        /// A page's popup is a tab of ours too, a child of the workbench's
        /// window like the rest, and keeps its opener.
        fn on_before_popup(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            _popup_id: i32,
            _target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            _target_disposition: WindowOpenDisposition,
            _user_gesture: i32,
            popup_features: Option<&PopupFeatures>,
            window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut i32>,
        ) -> i32 {
            let size = popup_features
                .filter(|f| f.width_set != 0 && f.height_set != 0)
                .map(|f| (f.width, f.height));
            let frame = tabs::popup_coming(size);
            if let Some(info) = window_info {
                let child = info
                    .clone()
                    .set_as_child(frame, &crate::window::UNPLACED);
                *info = crate::window::hidden(WindowInfo {
                    runtime_style: RuntimeStyle::CHROME,
                    ..child
                });
            }
            0
        }

        fn on_after_created(&self, browser: Option<&mut Browser>) {
            if let Some(browser) = browser {
                tabs::created(browser);
            }
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            if let Some(browser) = browser {
                tabs::closed(browser.identifier());
            }
        }
    }
}

/// What a key event looks like on this system, as CEF passes it on.
type OsEvent = <EventHandle as Pointee>::Target;

pub trait Pointee {
    type Target;
}

impl<T> Pointee for *mut T {
    type Target = T;
}

/// Shortcuts that belong to the workbench, not the page: Ctrl K, L, T, N, W
/// and comma. They go to the app instead.
const CONTROL: u32 = 1 << 2;

wrap_keyboard_handler! {
    struct Keys {}

    impl KeyboardHandler {
        fn on_pre_key_event(
            &self,
            _browser: Option<&mut Browser>,
            event: Option<&KeyEvent>,
            _os_event: Option<&mut OsEvent>,
            _is_keyboard_shortcut: Option<&mut i32>,
        ) -> i32 {
            let Some(event) = event else { return 0 };
            if event.type_ != KeyEventType::RAWKEYDOWN || event.modifiers & CONTROL == 0 {
                return 0;
            }
            let key = match event.windows_key_code {
                0x4B => "k",
                0x4C => "l",
                0x54 => "t",
                0x4E => "n",
                0x57 => "w",
                0xBC => ",",
                _ => return 0,
            };
            say(json!({ "key": key }));
            1
        }
    }
}

wrap_dev_tools_message_observer! {
    struct Observer {}

    impl DevToolsMessageObserver {
        /// The answer to asking a browser its target id, by the browser's id.
        fn on_dev_tools_method_result(
            &self,
            _browser: Option<&mut Browser>,
            message_id: i32,
            success: i32,
            result: Option<&[u8]>,
        ) {
            if success == 0 {
                return;
            }
            let Some(result) = result.and_then(|r| serde_json::from_slice::<Value>(r).ok()) else {
                return;
            };
            if let Some(target) = result["targetInfo"]["targetId"].as_str() {
                tabs::named(message_id, target.to_string());
            }
        }
    }
}
