//! The real wbd, spoken to in lines.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

/// Runs wbd, sends it these lines, and returns everything it said, in order.
fn say(lines: &[&str]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_wbd"))
        // No pi, claude or dova: what's installed on this machine must not change the test.
        .env("WB_PI", "/nowhere/pi")
        .env("WB_CLAUDE", "/nowhere/claude")
        .env("WB_DOVA", "/nowhere/dova")
        .env("WSL_DISTRO_NAME", "Ubuntu")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("wbd starts");
    let mut stdin = child.stdin.take().expect("stdin is piped");
    for line in lines {
        writeln!(stdin, "{line}").expect("wbd reads");
    }
    drop(stdin);
    let said = BufReader::new(child.stdout.take().expect("stdout is piped"))
        .lines()
        .map(|line| serde_json::from_str(&line.expect("a line")).expect("JSON"))
        .collect();
    assert!(child.wait().expect("wbd ends").success());
    said
}

#[test]
fn says_ready_first() {
    let said = say(&[]);
    assert_eq!(
        said,
        [json!({
            "jsonrpc": "2.0",
            "method": "event",
            "params": { "event": "ready", "version": env!("CARGO_PKG_VERSION") },
        })]
    );
}

#[test]
fn answers_health() {
    let said = say(&[r#"{"jsonrpc":"2.0","id":1,"method":"health"}"#]);
    assert_eq!(
        said[1],
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "kind": "health",
                "version": env!("CARGO_PKG_VERSION"),
                "distro": "Ubuntu",
                "pi": null,
                "claude": null,
                "dova": null,
            },
        })
    );
}

#[test]
fn refuses_what_it_cannot_read_and_keeps_going() {
    let said = say(&[
        "not json",
        r#"{"jsonrpc":"2.0","id":7,"method":"nope"}"#,
        r#"{"jsonrpc":"2.0","id":8,"method":"health"}"#,
    ]);
    // An event has no id at all, so it is never the answer to a line without one.
    let by_id = |id: Value| {
        said.iter()
            .find(|line| line.get("id") == Some(&id))
            .expect("an answer")
    };
    assert_eq!(by_id(Value::Null)["error"]["data"]["kind"], "badRequest");
    assert_eq!(by_id(json!(7))["error"]["code"], -32600);
    assert_eq!(by_id(json!(8))["result"]["kind"], "health");
}

#[test]
fn prints_its_version() {
    let output = Command::new(env!("CARGO_BIN_EXE_wbd"))
        .arg("--version")
        .output()
        .expect("wbd runs");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        env!("CARGO_PKG_VERSION")
    );
}
