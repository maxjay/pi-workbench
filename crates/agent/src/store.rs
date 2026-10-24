//! Pi's own session store, read to list past sessions. wb-agent keeps no
//! copy of any conversation: each session is a JSONL file Pi writes, whose
//! first line names the folder it ran in (docs/session-format.md).

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use protocol::SessionSummary;
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
    })
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
        found.sort_by(|a, b| a.cwd.cmp(&b.cwd));
        let titles: Vec<_> = found.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["draft the release notes", "Release notes"]);
        assert!(found[0].file.as_deref().unwrap().ends_with("a.jsonl"));
    }
}
