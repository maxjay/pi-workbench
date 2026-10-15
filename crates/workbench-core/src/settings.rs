//! The user's settings file. Every field is optional; a missing or broken
//! file means the defaults, and the error is reported rather than fatal.
use anyhow::{Context, Result};
use serde::Deserialize;
use std::{collections::BTreeMap, fs, io::ErrorKind, path::PathBuf};

/// What a new settings file starts with: every setting, at its default.
pub const TEMPLATE: &str = r#"{
  // Editor and terminal font. Changes apply to new windows.
  "font_family": null,
  "font_size": 14,

  // The branch "Changes since" compares with, by its merge base. By default
  // the first of develop, main and master that exists.
  "base_branch": null,

  // How Pi is started. PI_WORKBENCH_PI and PI_WORKBENCH_PI_ARGS still win.
  "pi": { "program": "pi", "args": [] },

  // The terminal's shell; by default PowerShell on Windows and $SHELL elsewhere.
  "terminal": { "shell": null, "args": [] },

  // Shortcuts by command, such as "QuickOpen": "ctrl+o" or "NextChange": "alt+j".
  // Use "" to remove one. Names are listed under Shortcuts in the README.
  "keys": {}
}
"#;

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub font_family: Option<String>,
    pub font_size: Option<f32>,
    pub base_branch: Option<String>,
    pub pi: Program,
    pub terminal: Program,
    pub keys: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Program {
    pub program: Option<String>,
    pub shell: Option<String>,
    pub args: Vec<String>,
}

impl Program {
    /// The terminal names its program `shell`; Pi names it `program`.
    pub fn command(&self) -> Option<&str> {
        self.program.as_deref().or(self.shell.as_deref())
    }
}

/// Where the settings file lives.
pub fn path() -> Result<PathBuf> {
    let dirs = directories::ProjectDirs::from("dev", "pi-workbench", "Pi Workbench")
        .context("No configuration directory")?;
    Ok(dirs.config_dir().join("settings.json"))
}

/// Reads the settings file. A missing file is the defaults.
pub fn load() -> Result<Settings> {
    let path = path()?;
    match fs::read_to_string(&path) {
        Ok(text) => parse(&text).with_context(|| format!("In {}", path.display())),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(Settings::default()),
        Err(err) => Err(err).with_context(|| format!("Cannot read {}", path.display())),
    }
}

/// Creates the settings file from the template if it does not exist yet.
pub fn ensure_file() -> Result<PathBuf> {
    let path = path()?;
    if !path.exists() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, TEMPLATE)?;
    }
    Ok(path)
}

/// JSON with `//` line comments, which the template uses to explain itself.
pub fn parse(text: &str) -> Result<Settings> {
    Ok(serde_json::from_str(&strip_comments(text))?)
}

fn strip_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let mut in_string = false;
        let mut escaped = false;
        let mut cut = line.len();
        let bytes = line.as_bytes();
        for (i, &byte) in bytes.iter().enumerate() {
            match byte {
                _ if escaped => escaped = false,
                b'\\' if in_string => escaped = true,
                b'"' => in_string = !in_string,
                b'/' if !in_string && bytes.get(i + 1) == Some(&b'/') => {
                    cut = i;
                    break;
                }
                _ => {}
            }
        }
        out.push_str(&line[..cut]);
        out.push('\n');
    }
    out
}

/// A shortcut as written in the settings file, such as "ctrl+shift+o" or "alt+j".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keys {
    pub key: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

pub fn parse_keys(text: &str) -> Option<Keys> {
    let mut keys = Keys {
        key: String::new(),
        ctrl: false,
        shift: false,
        alt: false,
    };
    for part in text.split('+').map(str::trim) {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "cmd" | "control" => keys.ctrl = true,
            "shift" => keys.shift = true,
            "alt" | "option" => keys.alt = true,
            "" => return None,
            key if keys.key.is_empty() => keys.key = key.to_owned(),
            _ => return None,
        }
    }
    (!keys.key.is_empty()).then_some(keys)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_template_parses_to_the_defaults() {
        let settings = parse(TEMPLATE).unwrap();
        assert_eq!(settings.font_size, Some(14.0));
        assert_eq!(settings.pi.command(), Some("pi"));
        assert_eq!(settings.terminal.command(), None);
        assert!(settings.keys.is_empty());
    }

    #[test]
    fn comments_inside_strings_are_kept() {
        let settings = parse(
            r#"{ "base_branch": "release//2", // the release line
                 "keys": { "QuickOpen": "ctrl+o" } }"#,
        )
        .unwrap();
        assert_eq!(settings.base_branch.as_deref(), Some("release//2"));
        assert_eq!(settings.keys["QuickOpen"], "ctrl+o");
        assert!(parse(r#"{ "fontsize": 3 }"#).is_err());
    }

    #[test]
    fn shortcuts_parse_with_modifiers_in_any_order() {
        let keys = parse_keys("Shift+Ctrl+O").unwrap();
        assert_eq!(
            (keys.key.as_str(), keys.ctrl, keys.shift, keys.alt),
            ("o", true, true, false)
        );
        assert_eq!(parse_keys("alt+down").unwrap().key, "down");
        assert_eq!(parse_keys("ctrl+"), None);
        assert_eq!(parse_keys("ctrl+a+b"), None);
    }
}
