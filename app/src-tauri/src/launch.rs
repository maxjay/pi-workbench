//! Which wb-agent to run. `WB_AGENT` names one for development. Otherwise,
//! on Windows it's the one installed in WSL, and elsewhere the one next to
//! the app, or on PATH.

use std::process::Command;

#[cfg(windows)]
pub fn command() -> Result<Command, String> {
    use crate::wsl;
    // A path inside WSL; without one, the copy this app installs.
    match std::env::var("WB_AGENT") {
        Ok(path) => Ok(wsl::command(["sh", "-lc", r#"exec "$0""#, &path])),
        Err(_) => {
            wsl::install()?;
            Ok(wsl::command([
                "sh",
                "-lc",
                r#"exec "$HOME/.local/bin/wb-agent""#,
            ]))
        }
    }
}

#[cfg(not(windows))]
pub fn command() -> Result<Command, String> {
    if let Some(path) = std::env::var_os("WB_AGENT") {
        return Ok(Command::new(path));
    }
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("wb-agent")))
        .filter(|path| path.is_file());
    Ok(Command::new(beside.unwrap_or_else(|| "wb-agent".into())))
}
