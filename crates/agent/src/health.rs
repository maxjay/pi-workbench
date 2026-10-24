use std::path::Path;
use std::time::Duration;

use protocol::Health;
use tokio::process::Command;

pub async fn check(agent_dir: &Path) -> Health {
    Health {
        version: env!("CARGO_PKG_VERSION").into(),
        agent_dir: agent_dir.display().to_string(),
        distro: std::env::var("WSL_DISTRO_NAME").ok(),
        pi: pi_version().await,
    }
}

/// `pi --version`, or None when Pi isn't installed or doesn't answer.
async fn pi_version() -> Option<String> {
    let run = Command::new("pi")
        .arg("--version")
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(Duration::from_secs(5), run)
        .await
        .ok()?
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!version.is_empty()).then_some(version)
}
