//! Running things inside WSL, and keeping wb-agent installed there. The app
//! carries a Linux build of wb-agent and copies it into `~/.local/bin` when
//! what's there is missing or a different version.

use std::io::Write;
use std::os::windows::process::CommandExt;
use std::process::{Command, Stdio};

/// The Linux wb-agent, put here by build.rs. Empty in a build without one.
const BUNDLED: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wb-agent"));

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

pub fn install() -> Result<(), String> {
    if BUNDLED.is_empty() || installed().as_deref() == Some(env!("CARGO_PKG_VERSION")) {
        return Ok(());
    }
    let script = r#"set -e
mkdir -p "$HOME/.local/bin"
cat > "$HOME/.local/bin/.wb-agent.new"
chmod +x "$HOME/.local/bin/.wb-agent.new"
mv "$HOME/.local/bin/.wb-agent.new" "$HOME/.local/bin/wb-agent""#;
    let failed = |err: std::io::Error| format!("couldn't install wb-agent in WSL: {err}");
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
            "couldn't install wb-agent in WSL: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// The version of the wb-agent in WSL, if there is one.
fn installed() -> Option<String> {
    let output = command(["sh", "-c", r#"exec "$HOME/.local/bin/wb-agent" --version"#])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}
