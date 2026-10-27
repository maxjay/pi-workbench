//! Which wb-agent to run. `WB_AGENT` names one for development. Otherwise,
//! on Windows it's the one installed in WSL, and elsewhere the one next to
//! the app, or on PATH. It reaches the browser through the app.

use std::process::Command;

#[cfg(windows)]
pub fn command() -> Result<Command, String> {
    use crate::wsl;
    // A path inside WSL; without one, the copy this app installs.
    match std::env::var("WB_AGENT") {
        Ok(path) => Ok(wsl::command([
            "sh",
            "-lc",
            r#"exec "$0" --browser-tunnel"#,
            &path,
        ])),
        Err(_) => {
            wsl::install()?;
            Ok(wsl::command([
                "sh",
                "-lc",
                r#"exec "$HOME/.local/bin/wb-agent" --browser-tunnel"#,
            ]))
        }
    }
}

#[cfg(not(windows))]
pub fn command() -> Result<Command, String> {
    if let Some(path) = std::env::var_os("WB_AGENT") {
        let mut command = Command::new(path);
        command.arg("--browser-tunnel");
        return Ok(command);
    }
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("wb-agent")))
        .filter(|path| path.is_file());
    let mut command = Command::new(beside.unwrap_or_else(|| "wb-agent".into()));
    command.arg("--browser-tunnel");
    Ok(command)
}
