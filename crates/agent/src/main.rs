//! wb-agent: the workbench's helper inside WSL. It reads one request per
//! line on stdin and writes replies and events, one per line, on stdout.
//! Everything it says goes through one writer, so lines never interleave.

mod browser;
mod codespaces;
mod config;
mod done;
mod error;
mod files;
mod git;
mod health;
mod json;
mod mcp;
mod messages;
mod models;
mod pi;
mod question;
mod router;
mod schedule;
mod session;
mod sessions;
mod store;
mod tasks;
mod terminals;
mod transcript;

use std::sync::Arc;

use protocol::{Call, Event, FailureCode, FromAgent, FromApp, Outcome, Reply, Request};
use tokio::io::AsyncWriteExt;
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

    // The browser: through the app with `--browser-tunnel`, at `WB_BROWSER`
    // (`host:port`) in development, or not at all.
    let tunnels = std::env::args()
        .any(|arg| arg == "--browser-tunnel")
        .then(|| browser::Tunnels::new(out.clone()));
    let dialer = match (&tunnels, std::env::var("WB_BROWSER")) {
        (Some(tunnels), _) => Some(browser::Dialer::Tunnel(tunnels.clone())),
        (None, Ok(address)) => Some(browser::Dialer::Direct(address)),
        (None, Err(_)) => None,
    };
    let host = tunnels.as_ref().map(|_| browser::Host::new(out.clone()));
    let web = browser::Web::new(
        out.clone(),
        config::agent_dir(),
        config::data_dir(),
        dialer,
        host,
    );
    let ctx = Arc::new(router::Ctx::from_env(out.clone(), web.clone()));
    web.start();
    ctx.sessions.stop_idle(config::idle_after());
    ctx.tasks.start();
    ctx.terminals.start();
    let _ = out.send(FromAgent::Event(Event::Ready {
        version: env!("CARGO_PKG_VERSION").into(),
    }));

    let mut stdin = requests();
    while let Some(line) = stdin.recv().await {
        if line.trim().is_empty() {
            continue;
        }
        // The app's own lines: streams to the browser, and what wb-browser said.
        if line.contains(r#""type":"tunnel""#) || line.contains(r#""type":"host""#) {
            match serde_json::from_str(&line) {
                Ok(FromApp::Tunnel(tunnel)) => {
                    if let Some(tunnels) = &tunnels {
                        tunnels.incoming(tunnel);
                    }
                    continue;
                }
                Ok(FromApp::Host(news)) => {
                    web.host_said(news.news);
                    continue;
                }
                Err(_) => {}
            }
        }
        let request = match serde_json::from_str::<Request>(&line) {
            Ok(request) => request,
            Err(err) => {
                let _ = out.send(FromAgent::Reply(Reply {
                    id: id_of(&line),
                    outcome: Outcome::Error(protocol::Failure {
                        code: FailureCode::BadRequest,
                        message: err.to_string(),
                    }),
                }));
                continue;
            }
        };
        // Keys go to a terminal in the order they were typed, so they're
        // handled here, in turn. Everything else is answered as it can be,
        // so a slow call doesn't hold up the rest.
        let typed = matches!(
            request.call,
            Call::TerminalInput { .. } | Call::TerminalResize { .. }
        );
        let (ctx, out) = (ctx.clone(), out.clone());
        let answer = async move {
            let outcome = router::handle(&ctx, request.call).await;
            let _ = out.send(FromAgent::Reply(Reply {
                id: request.id,
                outcome,
            }));
        };
        if typed {
            answer.await;
        } else {
            tokio::spawn(answer);
        }
    }

    // The workbench has gone: let each Pi finish writing its session, and
    // what's left to say go out, without waiting on either for long.
    ctx.terminals.shutdown();
    ctx.sessions.shutdown().await;
    drop(ctx);
    drop(out);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), writer).await;
}

/// The workbench's lines, read on a thread of their own until stdin ends.
/// A signal, or stdin left non-blocking by whoever made it, isn't the end:
/// the read waits and goes on, keeping what it had of the line.
fn requests() -> mpsc::UnboundedReceiver<String> {
    use std::io::{BufRead, ErrorKind};
    let (tx, rx) = mpsc::unbounded_channel();
    std::thread::spawn(move || {
        blocking_stdin();
        let stdin = std::io::stdin();
        let mut stdin = stdin.lock();
        let mut line = String::new();
        loop {
            match stdin.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if tx.send(std::mem::take(&mut line)).is_err() {
                        break;
                    }
                }
                Err(err) if err.kind() == ErrorKind::Interrupted => {}
                Err(err) if err.kind() == ErrorKind::WouldBlock => blocking_stdin(),
                Err(err) => {
                    eprintln!("wb-agent: couldn't read a request: {err}");
                    break;
                }
            }
        }
    });
    rx
}

/// Makes stdin wait for lines again, rather than fail when none has come.
fn blocking_stdin() {
    unsafe {
        let flags = libc::fcntl(0, libc::F_GETFL);
        if flags >= 0 {
            libc::fcntl(0, libc::F_SETFL, flags & !libc::O_NONBLOCK);
        }
    }
}

/// The id of a line that isn't a request we understand, if it has one, so
/// the workbench can still match the error to what it sent.
fn id_of(line: &str) -> u64 {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|value| value.get("id")?.as_u64())
        .unwrap_or(0)
}
