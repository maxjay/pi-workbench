//! Running things inside WSL, and keeping wbd installed there. The app
//! carries a Linux build of wbd and copies it into `~/.local/bin` when
//! what is there is missing or a different version.

use std::io::Write;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};

/// The Linux wbd, put here by build.rs. Empty in a build without one.
const BUNDLED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wbd"));

/// Keeps `wsl.exe` from opening a console window of its own.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A command run in the default WSL distro, without a shell around it.
pub fn command<'a>(args: impl IntoIterator<Item = &'a str>) -> Command {
    let mut command = Command::new("wsl.exe");
    command
        .arg("-e")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW);
    command
}

/// Puts the wbd this app carries into WSL, unless the one there is the same version.
pub fn install() -> Result<(), String> {
    if BUNDLED.is_empty() || installed().as_deref() == Some(env!("CARGO_PKG_VERSION")) {
        return Ok(());
    }
    // Written beside it and moved into place, so a wbd that is running is
    // replaced whole and never half.
    let script = r#"set -e
mkdir -p "$HOME/.local/bin"
cat > "$HOME/.local/bin/.wbd.new"
chmod +x "$HOME/.local/bin/.wbd.new"
mv "$HOME/.local/bin/.wbd.new" "$HOME/.local/bin/wbd""#;
    let failed = |err: std::io::Error| format!("couldn't install wbd in WSL: {err}");
    let mut child = command(["sh", "-c", script])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(failed)?;
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(BUNDLED)
        .map_err(failed)?;
    let output = child.wait_with_output().map_err(failed)?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "couldn't install wbd in WSL: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// The version of the wbd in WSL, if there is one.
fn installed() -> Option<String> {
    let output = command(["sh", "-c", r#"exec "$HOME/.local/bin/wbd" --version"#])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}
