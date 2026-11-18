//! Which wbd to run. On Windows it's `~/.local/bin/wbd` in the default WSL
//! distro; elsewhere, the `wbd` on PATH. `WBD` names another one: on
//! Windows, a path inside WSL.

use std::process::Command;

#[cfg(windows)]
pub fn command() -> Command {
    use std::os::windows::process::CommandExt;

    /// Keeps `wsl.exe` from opening a console window of its own.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    // A login shell, so wbd gets the PATH your terminal has, with pi,
    // claude and dova on it.
    let mut command = Command::new("wsl.exe");
    command.args(["-e", "sh", "-lc"]);
    match std::env::var("WBD") {
        Ok(path) => command.args([r#"exec "$0""#, &path]),
        Err(_) => command.arg(r#"exec "$HOME/.local/bin/wbd""#),
    };
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(not(windows))]
pub fn command() -> Command {
    Command::new(std::env::var("WBD").unwrap_or_else(|_| "wbd".into()))
}
