//! The real wbd, holding a conversation with each agent. The agents here
//! are stand-ins that print a run recorded from the real one
//! (tests/runs), since a test has no sign-in to either.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

/// Waits for what you wrote, prints the recorded run, then stays until wbd hangs up.
const STAND_IN: &str = "#!/bin/sh\nread line\n/bin/cat \"$WB_RUN\"\n/bin/cat > /dev/null\n";

struct Wbd {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    _dir: tempfile::TempDir,
    cwd: String,
}

impl Wbd {
    /// Runs wbd with both agents replaced by a stand-in that prints `run`.
    fn start(run: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let agent = dir.path().join("agent");
        std::fs::write(&agent, STAND_IN).unwrap();
        std::fs::set_permissions(&agent, std::fs::Permissions::from_mode(0o755)).unwrap();
        let cwd = dir.path().join("code");
        std::fs::create_dir(&cwd).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_wbd"))
            .env("WB_PI", &agent)
            .env("WB_CLAUDE", &agent)
            .env("WB_DOVA", "/nowhere/dova")
            .env("WB_DATA_DIR", dir.path().join("data"))
            .env(
                "WB_RUN",
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/runs")
                    .join(run),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("wbd starts");
        Wbd {
            stdin: child.stdin.take().unwrap(),
            stdout: BufReader::new(child.stdout.take().unwrap()),
            child,
            cwd: cwd.display().to_string(),
            _dir: dir,
        }
    }

    fn send(&mut self, line: Value) {
        writeln!(self.stdin, "{line}").expect("wbd reads");
    }

    /// The next line that `wanted` picks, skipping the others.
    fn until(&mut self, wanted: impl Fn(&Value) -> bool) -> Value {
        loop {
            let mut line = String::new();
            assert!(self.stdout.read_line(&mut line).unwrap() > 0, "wbd hung up");
            let line: Value = serde_json::from_str(&line).expect("JSON");
            if wanted(&line) {
                return line;
            }
        }
    }

    /// Starts a conversation and waits for its turn to end. Returns its id.
    fn talk(&mut self, agent: &str) -> String {
        let cwd = self.cwd.clone();
        self.send(json!({
            "jsonrpc": "2.0", "id": 1, "method": "conversation.start",
            "params": { "agent": agent, "cwd": cwd, "text": "Read note.txt and reply with only the one word in it." },
        }));
        let started = self.until(|line| line["id"] == 1);
        let id = started["result"]["state"]["id"]
            .as_str()
            .expect("an id")
            .to_string();
        self.until(|line| {
            line["params"]["event"] == "conversation"
                && line["params"]["changes"]
                    .as_array()
                    .is_some_and(|changes| changes.iter().any(|c| c["state"]["busy"] == false))
        });
        id
    }

    fn open(&mut self, id: &str) -> Value {
        self.send(json!({
            "jsonrpc": "2.0", "id": 2, "method": "conversation.open", "params": { "id": id },
        }));
        self.until(|line| line["id"] == 2)["result"].clone()
    }
}

impl Drop for Wbd {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// What a recorded run must come out as, whichever agent it was.
fn read_the_note_and_said_kiwi(view: &Value, tool: &str) {
    let entries = view["entries"].as_array().expect("entries");
    assert_eq!(entries[0]["kind"], "you");
    let read = entries
        .iter()
        .find(|entry| entry["kind"] == "tool")
        .expect("a tool call");
    assert_eq!(read["name"], tool);
    assert_eq!(read["status"], "done");
    assert!(read["title"].as_str().unwrap().ends_with("note.txt"));
    assert!(read["output"].as_str().unwrap().contains("kiwi"));
    let said = entries.last().unwrap();
    assert_eq!(
        (&said["kind"], &said["text"]),
        (&json!("text"), &json!("kiwi"))
    );
    assert_eq!(view["state"]["busy"], false);
    assert_eq!(view["state"]["ask"], Value::Null);
}

#[test]
fn a_run_of_pi() {
    let mut wbd = Wbd::start("pi.jsonl");
    let id = wbd.talk("pi");
    read_the_note_and_said_kiwi(&wbd.open(&id), "read");
}

#[test]
fn a_run_of_claude_code() {
    let mut wbd = Wbd::start("claude.jsonl");
    let id = wbd.talk("claude");
    let view = wbd.open(&id);
    read_the_note_and_said_kiwi(&view, "Read");
    assert_eq!(view["state"]["agent"], "claude");
}

#[test]
fn a_conversation_is_listed_and_opens_again_after_wbd_restarts() {
    let mut wbd = Wbd::start("pi.jsonl");
    let id = wbd.talk("pi");
    let cwd = wbd.cwd.clone();
    wbd.send(json!({
        "jsonrpc": "2.0", "id": 3, "method": "conversations.list", "params": { "cwd": cwd },
    }));
    let list = wbd.until(|line| line["id"] == 3)["result"]["list"].clone();
    assert_eq!(list[0]["id"], json!(id));
    assert_eq!(
        list[0]["title"],
        "Read note.txt and reply with only the one word in it."
    );
}
