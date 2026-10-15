//! The user's settings file. Every field is optional; a missing or broken
//! file means the defaults, and the error is reported rather than fatal.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{collections::BTreeMap, fs, io::ErrorKind, path::PathBuf};

/// What a new settings file starts with: every setting, at its default.
pub const TEMPLATE: &str = r#"{
  // Colour theme: "Pi Workbench", a bundled theme such as "Catppuccin Mocha",
  // "Dracula", "Nord" or "Solarized (light)", or the path of a .tmTheme file.
  // Ctrl+T picks one with a live preview. More .tmTheme files can go in a
  // "themes" folder next to this file.
  "theme": "Pi Workbench",

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
    pub theme: Option<String>,
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

/// Sets a top-level string setting in the file, keeping its comments and
/// layout: the existing line is rewritten, or a new one added at the top.
pub fn set_string(key: &str, value: &str) -> Result<()> {
    let path = ensure_file()?;
    let text = fs::read_to_string(&path)?;
    fs::write(&path, with_string(&text, key, value)?)?;
    Ok(())
}

fn with_string(text: &str, key: &str, value: &str) -> Result<String> {
    let encoded = serde_json::to_string(value)?;
    let before = parse(text).context("The settings file does not parse")?;
    let (open, members) = members(text).context("The settings file is not a JSON object")?;
    let changed = match members.iter().find(|(name, _)| name == key) {
        // Only the value changes: comments, other settings on the line and
        // line endings stay as they are.
        Some((_, range)) => format!("{}{encoded}{}", &text[..range.start], &text[range.end..]),
        None => {
            let newline = if text.contains("\r\n") { "\r\n" } else { "\n" };
            let comma = if members.is_empty() { "" } else { "," };
            let entry = format!("{newline}  \"{key}\": {encoded}{comma}");
            format!("{}{entry}{}", &text[..open], &text[open..])
        }
    };
    // Nothing else may change meaning.
    let after = parse(&changed).context("The settings file would not parse after the change")?;
    if key == "theme" {
        let mut expected = before;
        expected.theme = Some(value.to_owned());
        ensure!(
            after == expected,
            "Changing {key} would change other settings"
        );
    }
    Ok(changed)
}

/// A setting's name and where its value is in the file.
type Member = (String, std::ops::Range<usize>);

/// The top-level members of a JSON object with `//` comments: where to
/// insert after its opening brace, and each member's name and value span.
fn members(text: &str) -> Option<(usize, Vec<Member>)> {
    let bytes = text.as_bytes();
    let skip = |mut i: usize| {
        loop {
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if bytes[i..].starts_with(b"//") {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            } else {
                return i;
            }
        }
    };
    // The end of the string starting at `i`, just past its closing quote.
    let string_end = |mut i: usize| {
        i += 1;
        while i < bytes.len() {
            match bytes[i] {
                b'\\' => i += 2,
                b'"' => return Some(i + 1),
                _ => i += 1,
            }
        }
        None
    };
    let value_end = |mut i: usize| -> Option<usize> {
        match bytes.get(i)? {
            b'"' => string_end(i),
            b'{' | b'[' => {
                let mut depth = 0usize;
                while i < bytes.len() {
                    match bytes[i] {
                        b'"' => {
                            i = string_end(i)?;
                            continue;
                        }
                        b'{' | b'[' => depth += 1,
                        b'}' | b']' => {
                            depth -= 1;
                            if depth == 0 {
                                return Some(i + 1);
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
                None
            }
            _ => {
                while i < bytes.len()
                    && !matches!(bytes[i], b',' | b'}' | b']' | b'/')
                    && !bytes[i].is_ascii_whitespace()
                {
                    i += 1;
                }
                Some(i)
            }
        }
    };
    let mut i = skip(if text.starts_with('\u{feff}') { 3 } else { 0 });
    if bytes.get(i) != Some(&b'{') {
        return None;
    }
    let open = i + 1;
    i = open;
    let mut found = Vec::new();
    loop {
        i = skip(i);
        match bytes.get(i)? {
            b'}' => return Some((open, found)),
            b'"' => {}
            _ => return None,
        }
        let end = string_end(i)?;
        let name: String = serde_json::from_str(&text[i..end]).ok()?;
        i = skip(end);
        if bytes.get(i) != Some(&b':') {
            return None;
        }
        let start = skip(i + 1);
        let end = value_end(start)?;
        found.push((name, start..end));
        i = skip(end);
        if bytes.get(i) == Some(&b',') {
            i += 1;
        }
    }
}

/// JSON with `//` line comments, which the template uses to explain itself.
pub fn parse(text: &str) -> Result<Settings> {
    // Editors on Windows often start UTF-8 files with a byte order mark.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
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
    fn setting_a_string_keeps_comments_and_layout() {
        let changed = with_string(TEMPLATE, "theme", "Nord").unwrap();
        assert_eq!(parse(&changed).unwrap().theme.as_deref(), Some("Nord"));
        assert!(changed.contains("// Colour theme"));
        assert_eq!(changed.lines().count(), TEMPLATE.lines().count());
        let added = with_string("{\n  \"font_size\": 12\n}\n", "theme", "A \"b\"").unwrap();
        assert_eq!(parse(&added).unwrap().theme.as_deref(), Some("A \"b\""));
        assert_eq!(parse(&added).unwrap().font_size, Some(12.0));
        let empty = with_string("{}\n", "theme", "x").unwrap();
        assert_eq!(parse(&empty).unwrap().theme.as_deref(), Some("x"));
        // Several settings on one line, a trailing comment and CRLF all stay.
        let crowded = "{ \"theme\": \"a\", \"font_size\": 12 } // mine\r\n";
        assert_eq!(
            with_string(crowded, "theme", "b").unwrap(),
            "{ \"theme\": \"b\", \"font_size\": 12 } // mine\r\n"
        );
        let null = "{\r\n  \"theme\": null, // the default\r\n  \"font_size\": 13\r\n}\r\n";
        assert_eq!(
            with_string(null, "theme", "Nord").unwrap(),
            "{\r\n  \"theme\": \"Nord\", // the default\r\n  \"font_size\": 13\r\n}\r\n"
        );
        // "theme" inside another setting or a comment is not the setting.
        let decoy = "{\n  // \"theme\": \"old\"\n  \"keys\": { \"theme\": \"x\" }\n}\n";
        let set = with_string(decoy, "theme", "y").unwrap();
        assert_eq!(parse(&set).unwrap().theme.as_deref(), Some("y"));
        assert!(set.contains("// \"theme\": \"old\""));
        assert!(with_string("not json", "theme", "x").is_err());
        let marked = with_string("\u{feff}{\"theme\": \"a\"}", "theme", "b").unwrap();
        assert_eq!(marked, "\u{feff}{\"theme\": \"b\"}");
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
