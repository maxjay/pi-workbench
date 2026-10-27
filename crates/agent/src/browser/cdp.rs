//! A DevTools protocol client: commands with their results, and events.
//! One connection to the browser, with each page reached through the
//! session the browser attached for it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{broadcast, mpsc, oneshot};

use super::dial::{Dialer, Stream};
use super::ws;

pub struct Cdp {
    writer: mpsc::UnboundedSender<Vec<u8>>,
    next: AtomicU64,
    waiting: Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>,
    events: broadcast::Sender<Arc<Value>>,
    closed: AtomicBool,
}

impl Cdp {
    /// Connects to the browser itself, as its `/json/version` names it.
    pub async fn connect(dialer: &Dialer) -> std::io::Result<Arc<Cdp>> {
        let version = get_json(dialer, "/json/version").await?;
        let url = version["webSocketDebuggerUrl"]
            .as_str()
            .ok_or_else(|| std::io::Error::other("the browser didn't say where to connect"))?;
        let path = url
            .split_once("/devtools/")
            .map(|(_, rest)| format!("/devtools/{rest}"))
            .ok_or_else(|| std::io::Error::other(format!("an odd browser address: {url}")))?;
        let mut stream = dialer.connect().await?;
        let rest = ws::handshake(&mut stream, &path).await?;
        Ok(Cdp::run(stream, rest))
    }

    fn run(stream: Box<dyn Stream>, rest: Vec<u8>) -> Arc<Cdp> {
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (events, _) = broadcast::channel(1024);
        let cdp = Arc::new(Cdp {
            writer: tx,
            next: AtomicU64::new(1),
            waiting: Mutex::default(),
            events,
            closed: AtomicBool::new(false),
        });
        let (mut reading, mut writing) = tokio::io::split(stream);
        tokio::spawn(async move {
            while let Some(bytes) = rx.recv().await {
                if writing.write_all(&bytes).await.is_err() {
                    break;
                }
            }
        });
        let reader = cdp.clone();
        tokio::spawn(async move {
            let mut decoder = ws::Decoder::default();
            decoder.push(&rest);
            let mut buf = vec![0u8; 64 * 1024];
            'read: loop {
                while let Some(message) = decoder.next() {
                    match message.opcode {
                        ws::TEXT => reader.message(&message.payload),
                        ws::PING => {
                            let _ =
                                reader
                                    .writer
                                    .send(ws::encode(ws::PONG, &message.payload, true));
                        }
                        ws::CLOSE => break 'read,
                        _ => {}
                    }
                }
                match reading.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => decoder.push(&buf[..n]),
                }
            }
            reader.close();
        });
        cdp
    }

    fn message(&self, payload: &[u8]) {
        let Ok(value) = serde_json::from_slice::<Value>(payload) else {
            return;
        };
        if let Some(id) = value.get("id").and_then(Value::as_u64) {
            let waiting = self.waiting.lock().expect("not poisoned").remove(&id);
            if let Some(tx) = waiting {
                let result = match value.get("error") {
                    Some(error) => Err(error["message"].as_str().unwrap_or("failed").to_string()),
                    None => Ok(value.get("result").cloned().unwrap_or(Value::Null)),
                };
                let _ = tx.send(result);
            }
        } else if value.get("method").is_some() {
            let _ = self.events.send(Arc::new(value));
        }
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        // Waiting callers hear it's gone when their senders drop.
        self.waiting.lock().expect("not poisoned").clear();
        let _ = self.events.send(Arc::new(json!({ "method": "closed" })));
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Every event from now on, and a last `closed` one when the
    /// connection ends.
    pub fn events(&self) -> broadcast::Receiver<Arc<Value>> {
        self.events.subscribe()
    }

    /// A command to the browser, or with `session` to one page.
    pub async fn call(
        &self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<Value, String> {
        if self.is_closed() {
            return Err("the browser has gone".into());
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = session.into();
        }
        let (tx, rx) = oneshot::channel();
        self.waiting.lock().expect("not poisoned").insert(id, tx);
        let bytes = serde_json::to_vec(&message).expect("json serializes");
        let _ = self.writer.send(ws::encode(ws::TEXT, &bytes, true));
        match tokio::time::timeout(Duration::from_secs(30), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("the browser has gone".into()),
            Err(_) => {
                self.waiting.lock().expect("not poisoned").remove(&id);
                Err(format!("the browser didn't answer {method}"))
            }
        }
    }
}

/// One of the browser's HTTP endpoints, such as `/json/version`.
pub async fn get_json(dialer: &Dialer, path: &str) -> std::io::Result<Value> {
    let mut stream = dialer.connect().await?;
    let request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await?;
    let (head, mut body) = ws::read_head(&mut stream).await?;
    if !head.starts_with("HTTP/1.1 200") {
        return Err(std::io::Error::other(format!(
            "the browser answered {path} with {}",
            head.lines().next().unwrap_or_default()
        )));
    }
    let length = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("content-length")
            .then(|| value.trim().parse::<usize>().ok())?
    });
    let mut chunk = [0u8; 8192];
    while length.is_none_or(|length| body.len() < length) {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    serde_json::from_slice(&body).map_err(std::io::Error::other)
}
