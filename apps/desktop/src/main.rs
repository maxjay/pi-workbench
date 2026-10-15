mod agent;
mod changes;
mod commands;
mod documents;
mod explorer;
mod ui;
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
    floem::window::WindowConfig::default()
        .size(floem::kurbo::Size::new(1280.0, 800.0))
        .title("Pi Workbench")
}

fn open_folder() {
    floem::open_file(
        floem::FileDialogOptions::new()
            .select_directories()
            .title("Open workspace"),
        |selected| {
            if let Some(info) = selected
                && let Some(path) = info.path.first()
            {
                match workspace_paths(path) {
                    Ok((root, layout)) => floem::new_window(
                        move |id| ui::workbench(id, root, layout),
                        Some(window_config()),
                    ),
                    Err(err) => eprintln!("Cannot open workspace: {err}"),
                }
            }
        },
    );
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
}

fn main() -> Result<()> {
    #[cfg(target_os = "linux")]
    prefer_software_gl_on_wsl();
    let mut args = std::env::args_os().skip(1);
    let path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    if path == Path::new("--help") || path == Path::new("-h") {
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
    let (root, layout) = workspace_paths(&path)?;
    floem::Application::new()
        .window(
            move |id| ui::workbench(id, root, layout),
            Some(window_config()),
        )
        .run();
    Ok(())
}
