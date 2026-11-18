//! What wbd is, and which of the programs it works with are here.

use std::process::Stdio;
use std::time::Duration;

use protocol::Health;
use tokio::process::Command;

pub async fn check() -> Health {
    let (pi, claude, dova) = tokio::join!(
        version("WB_PI", "pi"),
        version("WB_CLAUDE", "claude"),
        version("WB_DOVA", "dova"),
    );
    Health {
        version: env!("CARGO_PKG_VERSION").into(),
        distro: std::env::var("WSL_DISTRO_NAME").ok(),
        pi,
        claude,
        dova,
    }
}

/// The version `program --version` prints, or nothing if it isn't found or
/// doesn't answer. `var` names another binary to run in its place.
async fn version(var: &str, program: &str) -> Option<String> {
    let program = std::env::var(var).unwrap_or_else(|_| program.into());
    // Its own stdin: wbd's stdin is the page's, and a child must not read it.
    let run = Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .ok()?
        .ok()?;
    output
        .status
        .success()
        .then(|| number(&String::from_utf8_lossy(&output.stdout)))?
}

/// The version number in what a program printed: its first word that
/// starts with a digit.
fn number(printed: &str) -> Option<String> {
    printed
        .split_whitespace()
        .find(|word| word.starts_with(|c: char| c.is_ascii_digit()))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::number;

    #[test]
    fn finds_the_number_however_it_is_printed() {
        assert_eq!(number("1.5.2\n").as_deref(), Some("1.5.2"));
        assert_eq!(
            number("2.1.327 (Claude Code)\n").as_deref(),
            Some("2.1.327")
        );
        assert_eq!(number("dova 0.4.2\n").as_deref(), Some("0.4.2"));
        assert_eq!(number("no version here"), None);
    }
}
