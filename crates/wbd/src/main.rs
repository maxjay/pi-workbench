//! wbd: the workbench's process inside WSL, next to the code and the agents.
//! It reads the page's calls from stdin, one JSON object per line, and
//! writes answers and events to stdout through one writer.

mod health;

use protocol::{Answer, Call, Event, Failure, FailureKind, Request};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if std::env::args().nth(1).as_deref() == Some("--version") {
        println!("{VERSION}");
        return;
    }

    // Everything wbd says goes through this one writer, a line at a time.
    let (out, mut lines) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(mut line) = lines.recv().await {
            line.push('\n');
            if stdout.write_all(line.as_bytes()).await.is_err() || stdout.flush().await.is_err() {
                break;
            }
        }
    });

    let _ = out.send(protocol::event(&Event::Ready {
        version: VERSION.into(),
    }));

    // Each call is answered on its own, so a slow one doesn't hold up the rest.
    let mut calls = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = calls.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let out = out.clone();
        tokio::spawn(async move {
            let _ = out.send(respond(&line).await);
        });
    }

    // The page hung up. Finish what was asked, then stop.
    drop(out);
    let _ = writer.await;
}

/// The line that answers one line from the page.
async fn respond(line: &str) -> String {
    match serde_json::from_str::<Request>(line) {
        Ok(Request { id, call }) => protocol::reply(id, &answer(call).await),
        Err(err) => protocol::failed(
            id_of(line),
            &Failure::new(FailureKind::BadRequest, err.to_string()),
        ),
    }
}

async fn answer(call: Call) -> Answer {
    match call {
        Call::Health => Answer::Health(health::check().await),
    }
}

/// The id of a line that couldn't be read as a call, if it has one.
fn id_of(line: &str) -> Option<u64> {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()?
        .get("id")?
        .as_u64()
}
