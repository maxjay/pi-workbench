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
                format!(
                    "{header}
"
                ),
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
