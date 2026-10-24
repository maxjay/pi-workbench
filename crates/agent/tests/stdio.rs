//! wb-agent as the app runs it: a process spoken to in lines. These start
//! the real binary with a fake `pi` and a scratch agent directory.

#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

struct Agent {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    _dir: tempfile::TempDir,
}

impl Agent {
    fn start() -> Agent {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let pi = bin.join("pi");
        std::fs::write(&pi, "#!/bin/sh\necho 9.9.9\n").unwrap();
        std::fs::set_permissions(&pi, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut child = Command::new(env!("CARGO_BIN_EXE_wb-agent"))
            .env("PI_CODING_AGENT_DIR", dir.path().join("agent"))
            .env("PATH", &bin)
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
