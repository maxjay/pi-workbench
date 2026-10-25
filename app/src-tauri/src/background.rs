//! Keeping the app running without its window, as tasks only run while it
//! does: the tray icon and its menu, closing to the tray, and starting at
//! sign-in. The page says how many tasks wait on you and whether they're
//! paused; the tray's Pause all goes back to the page, which asks wb-agent.

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_autostart::ManagerExt;

/// What starting at sign-in passes, so the app opens to the tray alone.
pub const HIDDEN: &str = "--background";

const TRAY: &str = "main";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub start_at_sign_in: bool,
    /// Closing the window keeps the app in the tray, rather than quitting.
    pub close_to_tray: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            start_at_sign_in: true,
            close_to_tray: true,
        }
    }
}

#[derive(Default)]
pub struct Background {
    settings: Mutex<Settings>,
    path: Mutex<Option<PathBuf>>,
}

impl Background {
    pub fn close_to_tray(&self) -> bool {
        self.settings.lock().expect("not poisoned").close_to_tray
    }
}

/// Reads the settings, or keeps the defaults on first start, and makes
/// starting at sign-in match them.
pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let state = app.state::<Background>();
    let path = app.path().app_config_dir()?.join("background.json");
    let settings = match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => {
            let settings = Settings::default();
            let _ = save(&path, &settings);
            settings
        }
    };
    apply(app, &settings);
    *state.settings.lock().expect("not poisoned") = settings;
    *state.path.lock().expect("not poisoned") = Some(path);

    let menu = menu(app, 0, false)?;
    TrayIconBuilder::with_id(TRAY)
        .tooltip("Pi Workbench")
        .icon(
            app.default_window_icon()
                .cloned()
                .expect("the app has an icon"),
        )
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" | "waiting" => show(app),
            "pause" => {
                let _ = app.emit("tray", "pauseAll");
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// The tray's menu: open the window, how many tasks wait on you, pause or
/// resume them all, and quit.
fn menu(app: &AppHandle, waiting: u32, paused: bool) -> tauri::Result<Menu<Wry>> {
    let open = MenuItem::with_id(app, "open", "Open Pi Workbench", true, None::<&str>)?;
    let pause = MenuItem::with_id(
        app,
        "pause",
        if paused {
            "Resume tasks"
        } else {
            "Pause all tasks"
        },
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let line = || PredefinedMenuItem::separator(app);
    if waiting == 0 {
        return Menu::with_items(app, &[&open, &line()?, &pause, &line()?, &quit]);
    }
    let text = if waiting == 1 {
        "1 task needs you".to_string()
    } else {
        format!("{waiting} tasks need you")
    };
    let waiting = MenuItem::with_id(app, "waiting", text, true, None::<&str>)?;
    Menu::with_items(app, &[&open, &waiting, &line()?, &pause, &line()?, &quit])
}

/// Brings the window back, from the tray or a second start.
pub fn show(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn apply(app: &AppHandle, settings: &Settings) {
    let launch = app.autolaunch();
    let result = if settings.start_at_sign_in {
        launch.enable()
    } else {
        launch.disable()
    };
    if let Err(err) = result {
        eprintln!("pi-workbench: couldn't change starting at sign-in: {err}");
    }
}

fn save(path: &PathBuf, settings: &Settings) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|err| err.to_string())?;
    }
    let text = serde_json::to_string_pretty(settings).map_err(|err| err.to_string())?;
    std::fs::write(path, text).map_err(|err| format!("couldn't save {}: {err}", path.display()))
}

#[tauri::command]
pub fn background_get(state: tauri::State<Background>) -> Settings {
    state.settings.lock().expect("not poisoned").clone()
}

#[tauri::command]
pub fn background_set(
    app: AppHandle,
    state: tauri::State<Background>,
    settings: Settings,
) -> Result<(), String> {
    if let Some(path) = state.path.lock().expect("not poisoned").as_ref() {
        save(path, &settings)?;
    }
    apply(&app, &settings);
    *state.settings.lock().expect("not poisoned") = settings;
    Ok(())
}

/// What the tray says: how many tasks wait on you, and whether they're paused.
#[tauri::command]
pub fn tray_update(app: AppHandle, waiting: u32, paused: bool) -> Result<(), String> {
    let tray = app.tray_by_id(TRAY).ok_or("no tray icon")?;
    let menu = menu(&app, waiting, paused).map_err(|err| err.to_string())?;
    tray.set_menu(Some(menu)).map_err(|err| err.to_string())?;
    let tooltip = match waiting {
        0 => "Pi Workbench".to_string(),
        1 => "Pi Workbench · 1 task needs you".to_string(),
        n => format!("Pi Workbench · {n} tasks need you"),
    };
    tray.set_tooltip(Some(tooltip))
        .map_err(|err| err.to_string())
}

/// A run that failed or needs you. Windows notifications for these come
/// later; for now the app highlights them, and this does nothing.
#[tauri::command]
pub fn notify_run(_task: String, _state: String) {}
