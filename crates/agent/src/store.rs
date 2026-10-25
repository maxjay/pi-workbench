//! Pi's own session store, read to list past sessions. wb-agent keeps no
//! copy of any conversation: each session is a JSONL file Pi writes, whose
//! first line names the folder it ran in (docs/session-format.md).

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use std::collections::HashMap;

use protocol::{SessionSummary, Usage};
use serde_json::Value;

use crate::config;
use crate::transcript::shorten;

/// Where Pi saves sessions: `PI_CODING_AGENT_SESSION_DIR`, then
/// `sessionDir` in settings.json, then `sessions` in the agent directory.
pub fn sessions_dir(agent_dir: &Path) -> PathBuf {
    let expand = |dir: &str| match dir.strip_prefix("~/") {
        Some(rest) => config::home().join(rest),
        None => PathBuf::from(dir),
    };
    if let Ok(dir) = std::env::var("PI_CODING_AGENT_SESSION_DIR")
        && !dir.is_empty()
    {
        return expand(&dir);
    }
    let configured = fs::read_to_string(agent_dir.join("settings.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .and_then(|settings| settings["sessionDir"].as_str().map(expand))
        .filter(|dir| dir.is_absolute());
    configured.unwrap_or_else(|| agent_dir.join("sessions"))
}

/// The saved sessions that ran in a folder under `root`, newest first.
pub fn under(agent_dir: &Path, root: &Path) -> Vec<SessionSummary> {
    let dir = sessions_dir(agent_dir);
    // One folder per working directory, or every session in one folder.
    let folders = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .chain([dir.clone()]);
    let mut sessions: Vec<SessionSummary> = folders
        .flat_map(|folder| jsonl_in(&folder))
        .filter_map(|file| summarize(&file, root))
        .collect();
    sessions.sort_by_key(|session| std::cmp::Reverse(session.updated));
    sessions
}

fn jsonl_in(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl"))
        .collect()
}

/// The folder a saved session ran in, from its header.
pub fn header_cwd(file: &Path) -> Option<PathBuf> {
    let handle = fs::File::open(file).ok()?;
    let line = BufReader::new(handle).lines().next()?.ok()?;
    let header: Value = serde_json::from_str(&line).ok()?;
    (header["type"] == "session")
        .then(|| header["cwd"].as_str().map(PathBuf::from))
        .flatten()
}

/// A session file's title and folder, if it ran under `root`. Only the
/// lines a title can come from are parsed.
fn summarize(file: &Path, root: &Path) -> Option<SessionSummary> {
    let handle = fs::File::open(file).ok()?;
    let updated = handle
        .metadata()
        .and_then(|meta| meta.modified())
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    let mut lines = BufReader::new(handle).lines().map_while(Result::ok);
    let header: Value = serde_json::from_str(&lines.next()?).ok()?;
    let cwd = header["cwd"].as_str()?.to_string();
    if header["type"] != "session" || !Path::new(&cwd).starts_with(root) {
        return None;
    }
    let mut name = None;
    let mut first = None;
    for line in lines {
        let wanted = line.contains("\"session_info\"")
            || (first.is_none() && line.contains("\"role\":\"user\""));
        if !wanted {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match entry["type"].as_str() {
            Some("session_info") => name = entry["name"].as_str().map(str::to_string),
            Some("message") if entry["message"]["role"] == "user" => {
                first = Some(crate::messages::text(&entry["message"]["content"]));
            }
            _ => {}
        }
    }
    let title = name
        .filter(|name| !name.trim().is_empty())
        .or(first)
        .map(|text| shorten(&text))
        .filter(|title| !title.is_empty())?;
    Some(SessionSummary {
        id: None,
        file: Some(file.display().to_string()),
        title,
        cwd,
        updated,
        busy: false,
        asking: false,
        stopped: false,
        done: false,
    })
}

/// A saved session as Pi would load it: the messages on its current
/// branch, the name it was given, the model and thinking level it last
/// used, and what it has cost.
pub struct Saved {
    pub cwd: PathBuf,
    pub name: Option<String>,
    pub model: Option<String>,
    pub thinking: Option<String>,
    pub messages: Vec<Value>,
    pub usage: Usage,
}

/// Reads a session file. Entries form a tree, branching where you went
/// back to an earlier point; the branch shown is the one ending in the
/// last entry written.
pub fn read(file: &Path) -> Option<Saved> {
    let text = fs::read_to_string(file).ok()?;
    let mut lines = text.lines();
    let header: Value = serde_json::from_str(lines.next()?).ok()?;
    if header["type"] != "session" {
        return None;
    }
    let entries: Vec<Value> = lines
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let by_id: HashMap<&str, &Value> = entries
        .iter()
        .filter_map(|entry| Some((entry["id"].as_str()?, entry)))
        .collect();
    let mut branch = Vec::new();
    let mut at = entries.iter().rev().find_map(|entry| entry["id"].as_str());
    while let Some(id) = at {
        let Some(&entry) = by_id.get(id) else { break };
        branch.push(entry);
        at = entry["parentId"].as_str();
        if branch.len() > entries.len() {
            break;
        }
    }
    branch.reverse();

    let mut saved = Saved {
        cwd: PathBuf::from(header["cwd"].as_str()?),
        name: None,
        model: None,
        thinking: None,
        messages: Vec::new(),
        usage: Usage::default(),
    };
    let mut add = |usage: &Value| {
        let count = |key: &str| usage[key].as_u64().unwrap_or(0);
        saved.usage.input += count("input") + count("cacheRead") + count("cacheWrite");
        saved.usage.output += count("output");
        saved.usage.cost += usage["cost"]["total"].as_f64().unwrap_or(0.0);
    };
    // Usage counts across every branch, as Pi's totals do.
    for entry in &entries {
        match entry["type"].as_str() {
            Some("message") if entry["message"]["role"] == "assistant" => {
                add(&entry["message"]["usage"]);
            }
            Some("usage" | "compaction" | "branch_summary") => add(&entry["usage"]),
            _ => {}
        }
    }
    for entry in branch {
        match entry["type"].as_str() {
            Some("message") => {
                let message = &entry["message"];
                if message["role"] == "assistant" {
                    if let (Some(provider), Some(model)) =
                        (message["provider"].as_str(), message["model"].as_str())
                    {
                        saved.model = Some(format!("{provider}/{model}"));
                    }
                    let usage = &message["usage"];
                    saved.usage.context = usage["totalTokens"].as_u64().filter(|&n| n > 0);
                }
                if message["role"] != "system" {
                    saved.messages.push(message.clone());
                }
            }
            Some("model_change") => {
                if let (Some(provider), Some(model)) =
                    (entry["provider"].as_str(), entry["modelId"].as_str())
                {
                    saved.model = Some(format!("{provider}/{model}"));
                }
            }
            Some("thinking_level_change") => {
                saved.thinking = entry["thinkingLevel"].as_str().map(str::to_string);
            }
            Some("session_info") => saved.name = entry["name"].as_str().map(str::to_string),
            _ => {}
        }
    }
    Some(saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(dir: &Path, name: &str, cwd: &str, lines: &[&str]) {
        let header = format!(r#"{{"type":"session","version":3,"id":"x","cwd":"{cwd}"}}"#);
        let text = std::iter::once(header.as_str())
            .chain(lines.iter().copied())
            .collect::<Vec<_>>()
            .join("\n");
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(name), text).unwrap();
    }

    #[test]
    fn lists_sessions_that_ran_under_a_folder() {
        let agent = tempfile::tempdir().unwrap();
        let sessions = agent.path().join("sessions");
        let prompt =
            r#"{"type":"message","message":{"role":"user","content":"draft the release notes"}}"#;
        session(
            &sessions.join("--scratch-1--"),
            "a.jsonl",
            "/scratch/1",
            &[prompt],
        );
        session(
            &sessions.join("--scratch-2--"),
            "b.jsonl",
            "/scratch/2",
            &[prompt, r#"{"type":"session_info","name":"Release notes"}"#],
        );
        session(&sessions.join("--code--"), "c.jsonl", "/code", &[prompt]);
        // Never asked anything: nothing to show.
        session(
            &sessions.join("--scratch-3--"),
            "d.jsonl",
            "/scratch/3",
            &[],
        );

        let mut found = under(agent.path(), Path::new("/scratch"));
        assert!(found.iter().all(|s| !s.done && !s.busy));
        found.sort_by(|a, b| a.cwd.cmp(&b.cwd));
        let titles: Vec<_> = found.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["draft the release notes", "Release notes"]);
        assert!(found[0].file.as_deref().unwrap().ends_with("a.jsonl"));
    }

    #[test]
    fn reads_the_branch_ending_last() {
        let dir = tempfile::tempdir().unwrap();
        session(
            dir.path(),
            "s.jsonl",
            "/scratch/1",
            &[
                r#"{"type":"message","id":"a","parentId":null,"message":{"role":"user","content":"first"}}"#,
                r#"{"type":"message","id":"b","parentId":"a","message":{"role":"assistant","provider":"local","model":"qwen","content":[{"type":"text","text":"dropped"}],"usage":{"input":10,"output":5,"totalTokens":15,"cost":{"total":0.5}}}}"#,
                r#"{"type":"thinking_level_change","id":"c","parentId":"a","thinkingLevel":"high"}"#,
                r#"{"type":"message","id":"d","parentId":"c","message":{"role":"assistant","provider":"local","model":"qwen","content":[{"type":"text","text":"kept"}],"usage":{"input":20,"cacheRead":4,"output":6,"totalTokens":30,"cost":{"total":0.25}}}}"#,
                r#"{"type":"session_info","id":"e","parentId":"d","name":"Tidy"}"#,
            ],
        );
        let saved = read(&dir.path().join("s.jsonl")).unwrap();
        let texts: Vec<_> = saved
            .messages
            .iter()
            .map(|m| crate::messages::text(&m["content"]))
            .collect();
        assert_eq!(texts, ["first", "kept"]);
        assert_eq!(saved.name.as_deref(), Some("Tidy"));
        assert_eq!(saved.thinking.as_deref(), Some("high"));
        assert_eq!(saved.model.as_deref(), Some("local/qwen"));
        assert_eq!((saved.usage.input, saved.usage.output), (34, 11));
        assert_eq!(saved.usage.context, Some(30));
        assert!((saved.usage.cost - 0.75).abs() < 1e-9);
    }
}
