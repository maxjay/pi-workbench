//! Structural comparison with [difftastic](https://difftastic.wilfred.me.uk),
//! when it is installed, to tell changes in meaning from reformatting.
use crate::git::run_for;
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{
    collections::HashSet,
    fs,
    path::Path,
    process::{Command, Stdio},
    sync::OnceLock,
    time::Duration,
};

const TIMEOUT: Duration = Duration::from_secs(5);

/// Lines difftastic reports as changed, zero-based, on each side.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changed {
    pub base: HashSet<usize>,
    pub current: HashSet<usize>,
}

fn command() -> Command {
    let mut command = Command::new("difft");
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    // The user's difftastic settings, such as ignoring comments, would hide changes.
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("DFT_") {
            command.env_remove(name);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// Whether `difft` is on the path; checked once.
pub fn available() -> bool {
    static FOUND: OnceLock<bool> = OnceLock::new();
    *FOUND.get_or_init(|| {
        let mut probe = command();
        probe.arg("--version");
        run_for(probe, None, TIMEOUT, "difft").is_ok_and(|output| output.status.success())
    })
}

/// Compares two versions of a file named `name` structurally. `None` when
/// difftastic is missing, fails, or only understands the file as plain text.
pub fn compare(name: &str, base: &str, current: &str) -> Option<Changed> {
    if !available() {
        return None;
    }
    run(name, base, current).ok().flatten()
}

fn run(name: &str, base: &str, current: &str) -> Result<Option<Changed>> {
    // The file name picks the language, so both copies keep it.
    let name = Path::new(name).file_name().context("No file name")?;
    let dir = tempfile::tempdir()?;
    let (old, new) = (dir.path().join("base"), dir.path().join("current"));
    fs::create_dir(&old)?;
    fs::create_dir(&new)?;
    let (old, new) = (old.join(name), new.join(name));
    fs::write(&old, base)?;
    fs::write(&new, current)?;
    let mut diff = command();
    diff.env("DFT_UNSTABLE", "yes")
        .args(["--display", "json", "--color", "never"])
        .arg(&old)
        .arg(&new);
    let output = run_for(diff, None, TIMEOUT, "difft")?;
    ensure!(output.status.success(), "difft failed");
    parse(&output.stdout)
}

fn parse(json: &[u8]) -> Result<Option<Changed>> {
    let report: Value = serde_json::from_slice(json)?;
    // Falling back to a line diff says nothing about structure.
    if report["language"].as_str().is_none_or(|l| l == "Text") {
        return Ok(None);
    }
    let mut changed = Changed::default();
    for chunk in report["chunks"].as_array().into_iter().flatten() {
        for line in chunk.as_array().into_iter().flatten() {
            let number = |side: &str| line[side]["line_number"].as_u64().map(|n| n as usize);
            changed.base.extend(number("lhs"));
            changed.current.extend(number("rhs"));
        }
    }
    Ok(Some(changed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_changed_lines_from_the_json_report() {
        let json = br#"{"chunks":[[{"lhs":{"line_number":1,"changes":[]},"rhs":{"line_number":1,"changes":[]}},{"rhs":{"line_number":4,"changes":[]}}]],"language":"Rust","status":"changed"}"#;
        let changed = parse(json).unwrap().unwrap();
        assert_eq!(changed.base, HashSet::from([1]));
        assert_eq!(changed.current, HashSet::from([1, 4]));
        let text = br#"{"chunks":[],"language":"Text","status":"changed"}"#;
        assert_eq!(parse(text).unwrap(), None);
        let same = br#"{"language":"Rust","status":"unchanged"}"#;
        assert_eq!(parse(same).unwrap(), Some(Changed::default()));
    }

    #[test]
    fn reformatting_is_not_a_structural_change() {
        if !available() {
            return;
        }
        let base = "fn main() {\n    let x = 1;\n    foo(a,b);\n}\n";
        let current = "fn main() {\n    let x = 2;\n    foo(\n        a,\n        b,\n    );\n}\n";
        let changed = compare("main.rs", base, current).unwrap();
        assert_eq!(changed.current, HashSet::from([1]));
    }
}
