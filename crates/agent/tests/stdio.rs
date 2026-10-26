//! wb-agent as the app runs it: a process spoken to in lines. These start
//! the real binary with a fake `pi` and a scratch agent directory.
//!
//! The fake `pi` answers `--version`, and in RPC mode plays one scripted
//! run for any prompt, the way Pi reports one.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

const FAKE_PI: &str = r#"#!/bin/sh
[ "$1" = "--version" ] && { echo 9.9.9; exit 0; }
while IFS= read -r line; do
  case "$line" in
    *'"type":"get_state"'*)
      echo '{"type":"response","command":"get_state","success":true,"data":{"sessionFile":"/s.jsonl","model":{"provider":"local","id":"one"}}}' ;;
    *'"type":"prompt"'*)
      echo '{"type":"response","command":"prompt","success":true,"data":{"disposition":"started"}}'
      echo '{"type":"agent_start"}'
      echo '{"type":"message_start","message":{"role":"user","content":"hello"}}'
      echo '{"type":"message_start","message":{"role":"assistant","content":[]}}'
      echo '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"Hi "}}'
      echo '{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"there"}}'
      echo '{"type":"message_end","message":{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"Hi there"}]}}'
      echo '{"type":"agent_settled","aborted":false}' ;;
  esac
done
"#;

struct Agent {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    _dir: tempfile::TempDir,
}

impl Agent {
    fn start() -> Agent {
        Agent::start_with(&[])
    }

    fn start_with(env: &[(&str, &str)]) -> Agent {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let pi = bin.join("pi");
        std::fs::write(&pi, FAKE_PI).unwrap();
        std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut child = Command::new(env!("CARGO_BIN_EXE_wb-agent"))
            .env("PI_CODING_AGENT_DIR", dir.path().join("agent"))
            .env("PATH", &bin)
            .env("WB_SCRATCH_DIR", dir.path().join("scratch"))
            .env("WB_DATA_DIR", dir.path().join("data"))
            .envs(env.iter().copied())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Agent {
            child,
            stdin,
            stdout,
            _dir: dir,
        }
    }

    fn send(&mut self, line: &str) -> Value {
        writeln!(self.stdin, "{line}").unwrap();
        self.receive()
    }

    /// Reads lines until one matches, returning it.
    fn until(&mut self, wanted: impl Fn(&Value) -> bool) -> Value {
        loop {
            let line = self.receive();
            if wanted(&line) {
                return line;
            }
        }
    }

    fn receive(&mut self) -> Value {
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn says_it_is_ready_then_answers_health() {
    let mut agent = Agent::start();
    let ready = agent.receive();
    assert_eq!(ready["event"], "ready");

    let reply = agent.send(r#"{"id":1,"method":"health"}"#);
    assert_eq!(reply["id"], 1);
    assert_eq!(reply["ok"]["kind"], "health");
    assert_eq!(reply["ok"]["pi"], "9.9.9");
}

#[test]
fn writes_then_reads_a_file_and_toggles_a_server() {
    let mut agent = Agent::start();
    agent.receive();

    let mcp = json!({ "mcpServers": { "grafana": { "command": "mcp-grafana" } } }).to_string();
    let write =
        json!({ "id": 2, "method": "config.write", "params": { "file": "mcp", "text": mcp } });
    assert_eq!(agent.send(&write.to_string())["ok"]["kind"], "done");

    let off = r#"{"id":3,"method":"mcp.setEnabled","params":{"name":"grafana","enabled":false}}"#;
    assert_eq!(agent.send(off)["ok"]["kind"], "done");

    let list = agent.send(r#"{"id":4,"method":"mcp.list"}"#);
    assert_eq!(list["ok"]["servers"][0]["name"], "grafana");
    assert_eq!(list["ok"]["servers"][0]["enabled"], false);
}

#[test]
fn a_bad_line_gets_an_error_with_its_id() {
    let mut agent = Agent::start();
    agent.receive();

    let reply = agent.send(r#"{"id":5,"method":"nope"}"#);
    assert_eq!(reply["id"], 5);
    assert_eq!(reply["error"]["code"], "badRequest");

    let reply = agent.send("not json");
    assert_eq!(reply["id"], 0);
    assert_eq!(reply["error"]["code"], "badRequest");

    let reply =
        agent.send(r#"{"id":6,"method":"mcp.setEnabled","params":{"name":"x","enabled":true}}"#);
    assert_eq!(reply["error"]["code"], "notFound");
}

#[test]
fn runs_a_scratchpad() {
    let mut agent = Agent::start();
    agent.receive();

    // Events for the new session can come before the reply.
    let start = r#"{"id":1,"method":"session.start","params":{"cwd":null,"text":"hello"}}"#;
    writeln!(agent.stdin, "{start}").unwrap();
    let reply = agent.until(|line| line["type"] == "reply" && line["id"] == 1);
    let id = reply["ok"]["id"].as_str().unwrap().to_string();
    assert!(
        reply["ok"]["state"]["cwd"]
            .as_str()
            .unwrap()
            .contains("scratch")
    );

    // The reply streams in, and the run settles.
    let reply = agent.until(|line| {
        line["change"]["change"] == "entry" && line["change"]["entry"]["text"] == "Hi there"
    });
    assert_eq!(reply["id"], id.as_str());
    let settled = agent.until(|line| {
        line["change"]["change"] == "state"
            && line["change"]["state"]["busy"] == false
            && line["change"]["state"]["title"] == "hello"
    });
    assert_eq!(settled["change"]["state"]["model"], "local/one");

    let get = format!(r#"{{"id":2,"method":"session.get","params":{{"id":"{id}"}}}}"#);
    writeln!(agent.stdin, "{get}").unwrap();
    let session = agent.until(|line| line["type"] == "reply" && line["id"] == 2);
    let kinds: Vec<_> = session["ok"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].clone())
        .collect();
    assert_eq!(kinds, [json!("user"), json!("text")]);

    let close = format!(r#"{{"id":3,"method":"session.close","params":{{"id":"{id}"}}}}"#);
    writeln!(agent.stdin, "{close}").unwrap();
    agent.until(|line| line["change"]["change"] == "closed");
}

#[test]
fn a_session_that_isnt_live_is_not_found() {
    let mut agent = Agent::start();
    agent.receive();
    let reply = agent
        .send(r#"{"id":1,"method":"session.send","params":{"id":"s9","text":"hi","later":false}}"#);
    assert_eq!(reply["error"]["code"], "notFound");
}

#[test]
fn asks_the_question_tools_questions() {
    use std::os::unix::net::UnixStream;

    let mut agent = Agent::start();
    agent.receive();
    let start = r#"{"id":1,"method":"session.start","params":{"cwd":null,"text":null}}"#;
    writeln!(agent.stdin, "{start}").unwrap();
    let reply = agent.until(|line| line["type"] == "reply" && line["id"] == 1);
    let id = reply["ok"]["id"].as_str().unwrap().to_string();

    // What the `ask_user` extension does inside Pi, here by hand.
    let socket = std::env::temp_dir()
        .join(format!("wb-agent-{}", agent.child.id()))
        .join(format!("{id}.sock"));
    let mut tool = UnixStream::connect(&socket).unwrap();
    let asked = json!({ "questions": [
        { "label": "When", "question": "When can it happen?", "choices": [{ "label": "Tonight" }], "other": true },
    ]});
    writeln!(tool, "{asked}").unwrap();

    let state = agent.until(|line| {
        line["change"]["change"] == "state" && line["change"]["state"]["ask"].is_object()
    });
    let ask = &state["change"]["state"]["ask"];
    assert_eq!(ask["kind"]["type"], "questions");
    assert_eq!(
        ask["kind"]["questions"][0]["choices"][0]["label"],
        "Tonight"
    );

    let answer = json!({ "id": 2, "method": "session.answer", "params": {
        "id": id, "ask": ask["id"], "reply": { "type": "answers", "answers": ["Tonight"] },
    }});
    writeln!(agent.stdin, "{answer}").unwrap();
    agent.until(|line| line["type"] == "reply" && line["id"] == 2);
    let mut back = String::new();
    BufReader::new(&tool).read_line(&mut back).unwrap();
    assert_eq!(back.trim(), r#"{"answers":["Tonight"]}"#);
}

#[test]
fn stops_pi_once_its_been_idle() {
    let mut agent = Agent::start_with(&[("WB_IDLE_SECS", "1")]);
    agent.receive();
    let start = r#"{"id":1,"method":"session.start","params":{"cwd":null,"text":"hello"}}"#;
    writeln!(agent.stdin, "{start}").unwrap();
    let reply = agent.until(|line| line["type"] == "reply" && line["id"] == 1);
    let id = reply["ok"]["id"].as_str().unwrap().to_string();
    // The run settles, a second goes by with nothing to do, and Pi stops.
    let closed = agent.until(|line| line["change"]["change"] == "closed");
    assert_eq!(closed["id"], id.as_str());
}

#[test]
fn opens_a_codespace_with_a_terminal_and_saves_its_files() {
    let mut agent = Agent::start();
    agent.until(|line| line["event"] == "ready");
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().canonicalize().unwrap().display().to_string();
    let reply = |agent: &mut Agent, id: u64| agent.until(move |line| line["id"] == id);

    writeln!(agent.stdin, r#"{{"id":1,"method":"codespaces.open","params":{{"path":"{path}"}}}}"#).unwrap();
    let opened = reply(&mut agent, 1);
    assert_eq!(opened["ok"]["space"]["path"], path.as_str());
    assert_eq!(opened["ok"]["exists"], true);

    writeln!(agent.stdin, r#"{{"id":2,"method":"terminal.open","params":{{"cwd":"{path}","cols":80,"rows":24}}}}"#).unwrap();
    let term = reply(&mut agent, 2)["ok"]["id"].as_str().unwrap().to_string();
    writeln!(agent.stdin, r#"{{"id":3,"method":"terminal.input","params":{{"id":"{term}","data":"echo made-$((6*7))\r"}}}}"#).unwrap();
    let printed = agent.until(|line| {
        line["event"] == "terminal" && line["data"].as_str().is_some_and(|d| d.contains("made-42"))
    });
    assert_eq!(printed["id"], term.as_str());

    let file = format!("{path}/notes.md");
    writeln!(agent.stdin, r#"{{"id":4,"method":"file.write","params":{{"path":"{file}","text":"one","expected":null}}}}"#).unwrap();
    assert!(reply(&mut agent, 4)["ok"]["modified"].as_u64().unwrap() > 0);
    writeln!(agent.stdin, r#"{{"id":5,"method":"file.write","params":{{"path":"{file}","text":"two","expected":"zero"}}}}"#).unwrap();
    assert_eq!(reply(&mut agent, 5)["error"]["code"], "conflict");

    writeln!(agent.stdin, r#"{{"id":6,"method":"terminal.close","params":{{"id":"{term}"}}}}"#).unwrap();
    assert_eq!(reply(&mut agent, 6)["ok"]["kind"], "done");
    writeln!(agent.stdin, r#"{{"id":7,"method":"codespaces.list"}}"#).unwrap();
    assert_eq!(reply(&mut agent, 7)["ok"]["list"].as_array().unwrap().len(), 1);
}
