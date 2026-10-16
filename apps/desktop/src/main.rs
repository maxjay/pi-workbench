// Release builds on Windows are a windowed app: started from the Start menu
// or a shortcut, no console window opens beside it. Debug builds keep the
// console for their logs. Without one, what the app has to say goes to the
// terminal it was started from, if any, and errors to a message box too
// (`report`).
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod commands;
mod documents;
mod explorer;
mod lens;
mod markdown;
mod origins;
mod panes;
mod pet;
mod session;
mod sessions;
mod settings;
mod syntax;
mod terminal;
mod ui;
mod watcher;
mod window;
mod workspace;

use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};

fn workspace_paths(path: &Path) -> Result<(PathBuf, PathBuf)> {
    let root = path
        .canonicalize()
        .context("Cannot open workspace folder")?;
    ensure!(root.is_dir(), "Provide a folder, not a file");
    let layout = workbench_core::layout::state_path(&root)?;
    Ok((root, layout))
}

fn window_config() -> floem::window::WindowConfig {
    let config = floem::window::WindowConfig::default()
        .size(floem::kurbo::Size::new(1280.0, 800.0))
        .title("Pi Workbench");
    match icon() {
        Some(icon) => config.window_icon(icon),
        None => config,
    }
}

/// Glitch, for the title bar and taskbar.
fn icon() -> Option<floem::window::Icon> {
    let image = image::load_from_memory(include_bytes!("../assets/icon/glitch.png"))
        .ok()?
        .into_rgba8();
    let (width, height) = image.dimensions();
    floem::window::RgbaIcon::new(image.into_raw(), width, height)
        .ok()
        .map(floem::window::Icon::from)
}

/// WSLg's GPU-backed OpenGL loses the device while wgpu sets up, which Floem
/// cannot recover from; Mesa's software renderer works. `PI_WORKBENCH_GPU=1`
/// keeps the GPU for setups where it works.
#[cfg(target_os = "linux")]
fn prefer_software_gl_on_wsl() {
    let wsl = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .is_ok_and(|release| release.to_lowercase().contains("microsoft"));
    let chosen = ["LIBGL_ALWAYS_SOFTWARE", "PI_WORKBENCH_GPU"]
        .iter()
        .any(|name| std::env::var_os(name).is_some());
    if wsl && !chosen {
        // SAFETY: runs first in main, before any other thread exists.
        unsafe { std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1") };
    }
    if !wsl {
        return;
    }
    // WSLg's Wayland takes on drawing windows' title bars and then draws
    // none, so the workbench draws its own frame there. Through X11 there is
    // a title bar, but on some setups the window takes no clicks, so X11 is
    // only used when asked for, with `PI_WORKBENCH_X11=1`.
    if std::env::var_os("PI_WORKBENCH_X11").is_some() {
        // SAFETY: as above.
        unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
    } else if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        ui::frame::use_own_frame();
    }
}

/// Says what went wrong where it will be seen: on standard error, and, for
/// the windowed Windows build that may have no terminal, in a message box.
fn report(message: &str) {
    eprintln!("{message}");
    #[cfg(all(windows, not(debug_assertions)))]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
        let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
        let (text, title) = (wide(message), wide("Pi Workbench"));
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                text.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR,
            )
        };
    }
}

/// A crash is written down, with where it happened, and said in a message
/// box too where there may be no terminal to see it.
fn record_crashes() {
    let main = std::thread::current().id();
    let usual = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        let when = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let details = format!(
            "--- at {when} (Unix time), thread {}\n{info}\n\n{}\n",
            thread.name().unwrap_or("unnamed"),
            std::backtrace::Backtrace::force_capture()
        );
        // Added to, so an earlier crash's details are kept.
        let log = workbench_core::layout::crash_log_path()
            .ok()
            .filter(|path| {
                use std::io::Write;
                path.parent()
                    .is_some_and(|dir| std::fs::create_dir_all(dir).is_ok())
                    && std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(path)
                        .and_then(|mut file| file.write_all(details.as_bytes()))
                        .is_ok()
            });
        // Only a crash on the main thread stops the app; one in a helper
        // thread is just written down. The box is shown from a thread of its
        // own, so its message loop cannot call back into the crashing UI.
        if thread.id() == main {
            let place = log.map_or_else(String::new, |path| {
                format!("\n\nThe details are in {}", path.display())
            });
            let message = format!("Pi Workbench stopped: {info}{place}");
            let _ = std::thread::spawn(move || report(&message)).join();
        }
        usual(info);
    }));
}

fn main() -> std::process::ExitCode {
    record_crashes();
    // Started from a terminal, the windowed Windows build writes to it
    // (`--help`, errors); from a shortcut there is none, and this does
    // nothing.
    #[cfg(all(windows, not(debug_assertions)))]
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(
            windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
        );
    }
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            report(&format!("Error: {err:#}"));
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    #[cfg(target_os = "linux")]
    prefer_software_gl_on_wsl();
    let mut args = std::env::args_os().skip(1);
    let folder = args.next().map(PathBuf::from);
    if folder
        .as_deref()
        .is_some_and(|path| path == Path::new("--help") || path == Path::new("-h"))
    {
        println!("Pi Workbench\n\nUsage: pi-workbench [workspace-folder]\n");
        for command in commands::COMMANDS {
            if command.shortcut().is_some() {
                println!("{}: {}", command.shortcut_label(), command.title());
            }
        }
        return Ok(());
    }
    ensure!(
        args.next().is_none(),
        "Usage: pi-workbench [workspace-folder]"
    );
    // A folder given is shown; without one, the window comes back as it
    // was left, or opens where it was started.
    let folder = match folder {
        Some(folder) => Some(workspace_paths(&folder)?.0),
        None => None,
    };
    let fallback = workspace_paths(&std::env::current_dir()?)?.0;
    floem::Application::new()
        .window(
            move |id| ui::workbench(id, folder, fallback),
            Some(window_config()),
        )
        .run();
    Ok(())
}
