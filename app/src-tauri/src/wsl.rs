//! Running things inside WSL, and keeping wbd installed there. The app
//! carries a Linux build of wbd and copies it into `~/.local/bin` when
//! what is there is missing or is not that very file. The version alone
//! does not say: two builds of one version can differ, and a page with a
//! wbd from another build asks for things it does not know.

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

/// Puts the wbd this app carries into WSL, unless the one there is the same file.
pub fn install() -> Result<(), String> {
    if BUNDLED.is_empty() || installed() == Some(cksum(BUNDLED)) {
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

/// What `cksum` says of the wbd in WSL, if there is one: its checksum and its length.
fn installed() -> Option<(u32, usize)> {
    let output = command(["sh", "-c", r#"cksum < "$HOME/.local/bin/wbd""#])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let said = String::from_utf8_lossy(&output.stdout);
    let mut parts = said.split_whitespace();
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

/// What the `cksum` every Linux has says of some bytes: the POSIX checksum, and how many there are.
fn cksum(bytes: &[u8]) -> (u32, usize) {
    let mut crc: u32 = 0;
    let mut take = |byte: u8| {
        crc ^= u32::from(byte) << 24;
        for _ in 0..8 {
            crc = match crc & 0x8000_0000 {
                0 => crc << 1,
                _ => (crc << 1) ^ 0x04C1_1DB7,
            };
        }
    };
    for &byte in bytes {
        take(byte);
    }
    // The length is counted in too, its low byte first.
    let mut length = bytes.len();
    while length != 0 {
        take((length & 0xFF) as u8);
        length >>= 8;
    }
    (!crc, bytes.len())
}

#[cfg(test)]
mod tests {
    use super::cksum;

    #[test]
    fn counts_as_cksum_does() {
        // What `printf 123456789 | cksum` and `printf "" | cksum` print.
        assert_eq!(cksum(b"123456789"), (930_766_865, 9));
        assert_eq!(cksum(b""), (4_294_967_295, 0));
    }
}
