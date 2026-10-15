//! Drives the real `pi` against scripts/fake-model.mjs. Opt in with
//! `PI_CLIENT_LIVE=1`; needs `pi` and `node` on PATH.
use pi_client::{Event, Launch, PiProcess};
use std::{
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    sync::mpsc,
    time::Duration,
};

#[test]
fn pi_edits_a_file_and_reports_each_step() {
    if std::env::var("PI_CLIENT_LIVE").is_err() {
        return;
    }
    let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/fake-model.mjs");
    let mut model = Command::new("node")
        .arg(script)
        .stdout(Stdio::piped())
        .spawn()
        .expect("node");
    let mut ready = String::new();
    BufReader::new(model.stdout.take().unwrap())
        .read_line(&mut ready)
        .unwrap();
    let port = ready.trim().strip_prefix("listening ").unwrap().to_string();

    let agent_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        agent_dir.path().join("models.json"),
        format!(
            r#"{{"providers":{{"fake":{{"baseUrl":"http://127.0.0.1:{port}/v1","api":"openai-completions","apiKey":"fake","models":[{{"id":"fake"}}]}}}}}}"#
        ),
    )
    .unwrap();
    let work = tempfile::tempdir().unwrap();
    std::fs::write(work.path().join("a.rs"), "hello\n").unwrap();

    let launch = Launch {
        args: ["--provider", "fake", "--model", "fake", "--no-session"]
            .map(String::from)
            .into(),
        env: vec![(
            "PI_CODING_AGENT_DIR".into(),
            agent_dir.path().display().to_string(),
        )],
        ..Launch::default()
    };
    let (send, events) = mpsc::channel();
    let mut pi = PiProcess::spawn(&launch, work.path(), move |event| {
        let _ = send.send(event);
    })
    .unwrap();
    pi.prompt(
        r#"[{"tool":"edit","args":{"path":"a.rs","edits":[{"oldText":"hello","newText":"hello from pi"}]}},{"text":"Done."}]"#,
    )
    .unwrap();

    let mut seen = Vec::new();
    loop {
        match events
            .recv_timeout(Duration::from_secs(30))
            .expect("Pi went quiet")
        {
            Event::AgentSettled => break,
            Event::Exited(message) => panic!("Pi exited: {message}"),
            event => seen.push(event),
        }
    }
    model.kill().unwrap();
    model.wait().unwrap();
    assert!(
        seen.iter()
            .any(|e| matches!(e, Event::ToolCallDelta { .. }))
    );
    assert!(
        seen.iter()
            .any(|e| matches!(e, Event::ToolEnd { tool, is_error: false, .. } if tool == "edit"))
    );
    assert_eq!(
        std::fs::read_to_string(work.path().join("a.rs")).unwrap(),
        "hello from pi\n"
    );
}
