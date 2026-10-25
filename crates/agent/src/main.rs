//! wb-agent: the workbench's helper inside WSL. It reads one request per
//! line on stdin and writes replies and events, one per line, on stdout.
//! Everything it says goes through one writer, so lines never interleave.

mod config;
mod done;
mod error;
mod files;
mod health;
mod json;
mod mcp;
mod messages;
mod models;
mod pi;
mod question;
mod router;
mod session;
mod sessions;
mod store;
mod transcript;

use protocol::{Event, FailureCode, FromAgent, Outcome, Reply, Request};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    if std::env::args().any(|arg| arg == "--version") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return;
    }

    let (out, mut lines) = mpsc::unbounded_channel::<FromAgent>();
    let writer = tokio::spawn(async move {
        let mut stdout = tokio::io::stdout();
        while let Some(message) = lines.recv().await {
            let mut line = serde_json::to_string(&message).expect("protocol types serialize");
            line.push('\n');
            if stdout.write_all(line.as_bytes()).await.is_err() || stdout.flush().await.is_err() {
                break;
            }
        }
    });

    let ctx = router::Ctx::from_env(out.clone());
    ctx.sessions.stop_idle(config::idle_after());
    let _ = out.send(FromAgent::Event(Event::Ready {
        version: env!("CARGO_PKG_VERSION").into(),
    }));

    let mut stdin = BufReader::new(tokio::io::stdin()).lines();
    while let Ok(Some(line)) = stdin.next_line().await {
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Request>(&line) {
            Ok(request) => Reply {
                id: request.id,
                outcome: router::handle(&ctx, request.call).await,
            },
            Err(err) => Reply {
                id: id_of(&line),
                outcome: Outcome::Error(protocol::Failure {
                    code: FailureCode::BadRequest,
                    message: err.to_string(),
                }),
            },
        };
        let _ = out.send(FromAgent::Reply(reply));
    }

    // The workbench has gone: let each Pi finish writing its session, and
    // what's left to say go out, without waiting on either for long.
    ctx.sessions.shutdown().await;
    drop(ctx);
    drop(out);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), writer).await;
}

/// The id of a line that isn't a request we understand, if it has one, so
/// the workbench can still match the error to what it sent.
fn id_of(line: &str) -> u64 {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|value| value.get("id")?.as_u64())
        .unwrap_or(0)
}
