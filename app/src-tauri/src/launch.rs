//! Which wbd to run. On Windows it's `~/.local/bin/wbd` in the default WSL
//! distro, installed there by the app; elsewhere, the `wbd` on PATH.
//! `WBD` names another one: on Windows, a path inside WSL.

use std::process::Command;

#[cfg(windows)]
pub fn command() -> Result<Command, String> {
    use crate::wsl;

    // A login shell, so wbd gets the PATH your terminal has, with pi,
    // claude and dova on it.
    match std::env::var("WBD") {
        Ok(path) => Ok(wsl::command(["sh", "-lc", r#"exec "$0""#, &path])),
        Err(_) => {
            wsl::install()?;
            Ok(wsl::command([
                "sh",
                "-lc",
                r#"exec "$HOME/.local/bin/wbd""#,
            ]))
        }
    }
}

#[cfg(not(windows))]
pub fn command() -> Result<Command, String> {
    Ok(Command::new(
        std::env::var("WBD").unwrap_or_else(|_| "wbd".into()),
    ))
}
