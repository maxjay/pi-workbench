//! Reads Pi's own session store, so past sessions can be listed and resumed
//! without the workbench keeping a second copy of any conversation.
use serde_json::Value;
use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Clone, Debug, PartialEq)]
pub struct SessionSummary {
    pub path: PathBuf,
    pub id: String,
    /// The name set with `/name`, or the first prompt, shortened.
    pub title: String,
    pub prompts: usize,
    pub modified: SystemTime,
    /// The working directory recorded in the session header.
    pub cwd: Option<String>,
}

/// Node's `os.homedir()`: `USERPROFILE` on Windows, `HOME` elsewhere.
fn home() -> Option<PathBuf> {
    let (first, second) = if cfg!(windows) {
        ("USERPROFILE", "HOME")
    } else {
        ("HOME", "USERPROFILE")
    };
    std::env::var_os(first)
        .or_else(|| std::env::var_os(second))
        .map(PathBuf::from)
}

fn expand_home(path: &str) -> PathBuf {
    match (path.strip_prefix('~'), home()) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
            home.join(rest.trim_start_matches(['/', '\\']))
        }
        _ => PathBuf::from(path),
    }
}

/// The plain form Pi sees: Rust hands child processes `\\server\share`
/// rather than `\\?\UNC\server\share`, and `C:\x` rather than `\\?\C:\x`.
pub fn plain_cwd(cwd: &Path) -> String {
    let cwd = cwd.to_string_lossy();
    if let Some(unc) = cwd.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else {
        cwd.strip_prefix(r"\\?\").unwrap_or(&cwd).to_string()
    }
}

/// `PI_CODING_AGENT_DIR`, or `~/.pi/agent`.
pub fn agent_dir() -> Option<PathBuf> {
    std::env::var_os("PI_CODING_AGENT_DIR")
        .map(PathBuf::from)
        .or_else(|| home().map(|home| home.join(".pi").join("agent")))
}

/// Where Pi keeps sessions for `cwd`. Precedence follows Pi:
/// `PI_CODING_AGENT_SESSION_DIR`, then `sessionDir` in the agent
/// `settings.json`, then the per-folder default, whose name is the path
/// without its leading separator, with `/`, `\` and `:` turned into `-`.
pub fn session_dir(cwd: &Path) -> Option<PathBuf> {
    if let Some(dir) = std::env::var("PI_CODING_AGENT_SESSION_DIR")
        .ok()
        .filter(|dir| !dir.is_empty())
    {
        return Some(expand_home(&dir));
    }
    let configured = agent_dir()
        .and_then(|dir| fs::read_to_string(dir.join("settings.json")).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|settings| settings["sessionDir"].as_str().map(expand_home));
    if configured.is_some() {
        return configured;
    }
    let cwd = plain_cwd(cwd);
    let trimmed = cwd.strip_prefix(['/', '\\']).unwrap_or(&cwd);
    let encoded: String = trimmed
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':') {
                '-'
            } else {
                c
            }
        })
        .collect();
    agent_dir().map(|dir| dir.join("sessions").join(format!("--{encoded}--")))
}

fn first_text(content: &Value) -> Option<String> {
    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(blocks) => blocks
            .iter()
            .find_map(|block| block["text"].as_str().map(String::from)),
        _ => None,
    }
}

fn shorten(text: &str) -> String {
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    match line.char_indices().nth(60) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_string(),
    }
}

/// Reads only what a list needs; malformed lines are skipped.
pub fn summarize(path: &Path) -> Option<SessionSummary> {
    let file = fs::File::open(path).ok()?;
    let modified = file.metadata().and_then(|m| m.modified()).ok()?;
    let mut id = None;
    let mut cwd = None;
    let mut name = None;
    let mut first_prompt = None;
    let mut prompts = 0;
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        let Ok(entry) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match entry["type"].as_str() {
            Some("session") => {
                id = entry["id"].as_str().map(String::from);
                cwd = entry["cwd"].as_str().map(String::from);
            }
            Some("session_info") => name = entry["name"].as_str().map(String::from),
            Some("message") if entry["message"]["role"] == "user" => {
                prompts += 1;
                if first_prompt.is_none() {
                    first_prompt = first_text(&entry["message"]["content"]);
                }
            }
            _ => {}
        }
    }
    let title = name
        .filter(|n| !n.trim().is_empty())
        .or(first_prompt)
        .map(|t| shorten(&t))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "Empty session".into());
    Some(SessionSummary {
        path: path.to_path_buf(),
        id: id?,
        title,
        prompts,
        modified,
        cwd,
    })
}

/// What Pi wrote with its edit and write tools in one session: the session's
/// working directory and, per successful tool call, the path as Pi gave it and
/// the lines it added. An edit's new text repeats context from its old text;
/// only lines the old text did not have count.
pub fn written(path: &Path) -> (Option<PathBuf>, Vec<(String, String)>) {
    let Ok(file) = fs::File::open(path) else {
        return (None, Vec::new());
    };
    let mut cwd = None;
    let mut calls: Vec<(String, String, String)> = Vec::new();
    let mut failed = std::collections::HashSet::new();
    for line in BufReader::new(file).lines().map_while(Result::ok) {
        // Most lines are not tool calls or results; skip parsing them.
        if !line.contains("\"toolCall\"")
            && !line.contains("\"isError\":true")
            && !line.contains("\"type\":\"session\"")
        {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if entry["type"] == "session" {
            cwd = entry["cwd"].as_str().map(PathBuf::from);
            continue;
        }
        let message = &entry["message"];
        if message["role"] == "toolResult" && message["isError"] == true {
            failed.extend(message["toolCallId"].as_str().map(String::from));
            continue;
        }
        let Some(content) = message["content"].as_array() else {
            continue;
        };
        for call in content.iter().filter(|c| c["type"] == "toolCall") {
            let arguments = &call["arguments"];
            let (Some(id), Some(target)) = (call["id"].as_str(), arguments["path"].as_str()) else {
                continue;
            };
            let mut texts = Vec::new();
            match call["name"].as_str() {
                Some("write") => texts.extend(arguments["content"].as_str().map(String::from)),
                Some("edit") => {
                    let edits = arguments["edits"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .chain(std::iter::once(arguments));
                    texts.extend(edits.filter_map(|edit| {
                        let new = edit["newText"].as_str()?;
                        Some(added_lines(
                            edit["oldText"].as_str().unwrap_or_default(),
                            new,
                        ))
                    }));
                }
                _ => {}
            }
            calls.extend(
                texts
                    .into_iter()
                    .map(|text| (id.to_owned(), target.to_owned(), text)),
            );
        }
    }
    let written = calls
        .into_iter()
        .filter(|(id, _, _)| !failed.contains(id))
        .map(|(_, target, text)| (target, text))
        .collect();
    (cwd, written)
}

/// The lines of `new` that `old` does not have, compared without indentation.
fn added_lines(old: &str, new: &str) -> String {
    let old: std::collections::HashSet<&str> = old.lines().map(str::trim).collect();
    new.lines()
        .filter(|line| !old.contains(line.trim()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Session files for `cwd` with when they last changed, without reading them.
/// Files from other projects in a shared folder are filtered out later, by
/// the working directory [`written`] reports.
pub fn files(cwd: &Path) -> Vec<(PathBuf, SystemTime)> {
    let Some(dir) = session_dir(cwd) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .filter_map(|path| {
            let modified = fs::metadata(&path).and_then(|m| m.modified()).ok()?;
            Some((path, modified))
        })
        .collect()
}

/// Whether a session started in `session_cwd` belongs to the project at `cwd`.
pub fn belongs(session_cwd: &Path, cwd: &Path) -> bool {
    same_dir(&session_cwd.to_string_lossy(), &plain_cwd(cwd))
}

fn same_dir(a: &str, b: &str) -> bool {
    let normal = |p: &str| {
        let p = p.replace('\\', "/");
        let p = p.trim_end_matches('/').to_string();
        if cfg!(windows) { p.to_lowercase() } else { p }
    };
    normal(a) == normal(b)
}

/// Sessions for `cwd`, newest first. A shared session folder can hold other
/// projects' sessions; resuming one would run Pi there, so those are left out.
pub fn list(cwd: &Path) -> Vec<SessionSummary> {
    let plain = plain_cwd(cwd);
    let Some(dir) = session_dir(cwd) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut sessions: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .filter_map(|path| summarize(&path))
        .filter(|s| s.cwd.as_deref().is_none_or(|c| same_dir(c, &plain)))
        .collect();
    sessions.sort_by_key(|s| std::cmp::Reverse(s.modified));
    sessions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_collects_edit_and_write_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let lines = [
            r#"{"type":"session","id":"1","cwd":"/work"}"#,
            r#"{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","id":"1","name":"edit","arguments":{"path":"src/a.rs","edits":[{"oldText":"keep\nx","newText":"keep\ny\nz"}]}},{"type":"toolCall","id":"2","name":"bash","arguments":{"command":"ls"}}]}}"#,
            r#"{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","id":"3","name":"write","arguments":{"path":"/work/b.md","content":"Bee"}}]}}"#,
            r#"{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","id":"4","name":"edit","arguments":{"path":"c.rs","oldText":"a","newText":"b"}}]}}"#,
            r#"{"type":"message","message":{"role":"assistant","content":[{"type":"toolCall","id":"5","name":"edit","arguments":{"path":"d.rs","oldText":"a","newText":"failed"}}]}}"#,
            r#"{"type":"message","message":{"role":"toolResult","toolCallId":"5","isError":true,"content":[]}}"#,
        ];
        fs::write(&path, lines.join("\n")).unwrap();
        let (cwd, written) = written(&path);
        assert_eq!(cwd, Some(PathBuf::from("/work")));
        assert_eq!(
            written,
            [
                ("src/a.rs".to_owned(), "y\nz".to_owned()),
                ("/work/b.md".to_owned(), "Bee".to_owned()),
                ("c.rs".to_owned(), "b".to_owned()),
            ]
        );
    }

    #[test]
    fn summaries_prefer_the_session_name_then_the_first_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let named = dir.path().join("a.jsonl");
        fs::write(
            &named,
            concat!(
                r#"{"type":"session","version":3,"id":"s1","cwd":"/p"}"#,
                "\n",
                r#"{"type":"message","id":"1","parentId":null,"message":{"role":"user","content":"Fix the login bug please"}}"#,
                "\n",
                "not json\n",
                r#"{"type":"session_info","id":"2","parentId":"1","name":"Login fix"}"#,
                "\n",
            ),
        )
        .unwrap();
        let summary = summarize(&named).unwrap();
        assert_eq!(
            (summary.id.as_str(), summary.title.as_str()),
            ("s1", "Login fix")
        );
        assert_eq!(summary.prompts, 1);

        let unnamed = dir.path().join("b.jsonl");
        let long = "x".repeat(80);
        fs::write(
            &unnamed,
            format!(
                "{}\n{}\n",
                r#"{"type":"session","id":"s2"}"#,
                serde_json::json!({"type":"message","message":{"role":"user","content":[{"type":"text","text":long}]}})
            ),
        )
        .unwrap();
        let summary = summarize(&unnamed).unwrap();
        assert_eq!(summary.title.chars().count(), 61);
        assert!(summary.title.ends_with('…'));
    }

    #[test]
    fn listing_skips_other_projects_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let session = |name: &str, cwd: &str| {
            let header = serde_json::json!({"type": "session", "id": name, "cwd": cwd});
            fs::write(
                dir.path().join(format!("{name}.jsonl")),
                format!("{header}\n"),
            )
            .unwrap();
        };
        session("mine", "/work/proj/");
        session("theirs", "/work/other");
        let summaries: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| summarize(&e.unwrap().path()))
            .filter(|s| s.cwd.as_deref().is_none_or(|c| same_dir(c, "/work/proj")))
            .collect();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].id, "mine");
    }

    #[test]
    fn session_dirs_match_pis_encoding() {
        let encoded = |cwd: &str| {
            session_dir(Path::new(cwd))
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        };
        if std::env::var_os("PI_CODING_AGENT_SESSION_DIR").is_none() {
            assert_eq!(encoded("/home/max/proj"), "--home-max-proj--");
            assert_eq!(encoded(r"\\?\C:\Users\Max\proj"), "--C--Users-Max-proj--");
            assert_eq!(encoded(r"\\?\UNC\srv\share\p"), "---srv-share-p--");
        }
    }
}
