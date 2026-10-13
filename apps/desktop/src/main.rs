mod commands;
mod state;
mod ui;

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

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);
    if path == Path::new("--help") || path == Path::new("-h") {
        println!(
            "Pi Workbench\n\nUsage: pi-workbench [workspace-folder]\n\nCtrl/Cmd+S: save · Ctrl/Cmd+W: close file · Ctrl/Cmd+B: explorer\nCtrl/Cmd+\\: split · Ctrl/Cmd+Shift+P: commands"
        );
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
