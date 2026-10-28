//! Where Pi's `browser` server connects: a DevTools endpoint on 127.0.0.1,
//! in front of the browser. The path says whose Pi it is,
//! `/s/<session>/...`, or `/s/terminal/...` for a Pi wb-agent didn't
//! start. Through it each Pi sees only its own pages (those it opened, and
//! those its session sits beside), starts on a blank page of its own rather
//! than one of yours, and tells wb-agent which page it's using.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;

use super::{Claim, Dialer, Web, cdp, ws};

const PORT: u16 = 9339;

/// Ids for the gate's own commands, well above any Pi's (and still a
/// 32-bit integer, as the browser wants).
const OWN_IDS: u64 = 2_000_000_000;

/// What counts as Pi driving a page, rather than reading it.
const ACTIONS: [&str; 4] = [
    "Page.navigate",
    "Page.reload",
    "Page.navigateToHistoryEntry",
    "Input.",
];

pub async fn listen(web: Arc<Web>) -> std::io::Result<u16> {
    let first = std::env::var("WB_BROWSER_PORT")
        .ok()
        .and_then(|port| port.parse().ok())
        .unwrap_or(PORT);
    let mut tried = Err(std::io::Error::other("no port to listen on"));
    for port in first..first + 10 {
        tried = TcpListener::bind(("127.0.0.1", port))
            .await
            .map(|l| (l, port));
        if tried.is_ok() {
            break;
        }
    }
    let (listener, port) = tried?;
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let web = web.clone();
            tokio::spawn(async move {
                let _ = serve(web, stream, port).await;
            });
        }
    });
    Ok(port)
}

async fn serve(web: Arc<Web>, mut client: TcpStream, port: u16) -> std::io::Result<()> {
    client.set_nodelay(true)?;
    let (head, rest) = ws::read_head(&mut client).await?;
    let target = head.split_whitespace().nth(1).unwrap_or("/").to_string();
    let (key, path) = whose(&target);
    let Some(dialer) = web.dialer.clone() else {
        return respond(
            &mut client,
            "503 Service Unavailable",
            &json!({ "error": "no browser" }),
        )
        .await;
    };
    if !head.to_ascii_lowercase().contains("upgrade: websocket") {
        let base = format!("ws://127.0.0.1:{port}/s/{key}");
        return match path
            .split('?')
            .next()
            .unwrap_or_default()
            .trim_end_matches('/')
        {
            "/json/version" => {
                let mut version = match cdp::get_json(&dialer, "/json/version").await {
                    Ok(version) => version,
                    Err(err) => return unreachable(&mut client, &dialer, &err).await,
                };
                rebase(&mut version, &base);
                respond(&mut client, "200 OK", &version).await
            }
            "/json" | "/json/list" => {
                let list = match cdp::get_json(&dialer, "/json/list").await {
                    Ok(list) => list,
                    Err(err) => return unreachable(&mut client, &dialer, &err).await,
                };
                let mut mine: Vec<Value> = list
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|t| {
                        web.visible(t["id"].as_str().unwrap_or_default(), &key) == Some(true)
                    })
                    .cloned()
                    .collect();
                mine.iter_mut().for_each(|t| rebase(t, &base));
                respond(&mut client, "200 OK", &Value::Array(mine)).await
            }
            _ => {
                respond(
                    &mut client,
                    "404 Not Found",
                    &json!({ "error": "not here" }),
                )
                .await
            }
        };
    }

    // A Pi with no page of its own gets a blank one, so it never starts
    // on one of yours.
    if !web.has_pages(&key) {
        let claim = Claim {
            shown: key == TERMINAL,
            kept: false,
            session: session_of(&key),
            pi: Some(key.clone()),
        };
        let _ = web.create("about:blank", claim).await;
    }

    let mut upstream = match dialer.connect().await {
        Ok(upstream) => upstream,
        Err(err) => return unreachable(&mut client, &dialer, &err).await,
    };
    // Without compression, so the gate can read what passes.
    let forwarded: String = head
        .replacen(&target, &path, 1)
        .split_inclusive("\r\n")
        .filter(|line| {
            !line
                .to_ascii_lowercase()
                .starts_with("sec-websocket-extensions:")
        })
        .collect();
    upstream.write_all(forwarded.as_bytes()).await?;
    upstream.write_all(&rest).await?;
    let (answer, early) = match ws::read_head(&mut upstream).await {
        Ok(answer) => answer,
        Err(err) => return unreachable(&mut client, &dialer, &err).await,
    };
    client.write_all(answer.as_bytes()).await?;
    if !answer.starts_with("HTTP/1.1 101") {
        return Ok(());
    }
    let connection = web.connected_pi(session_of(&key));
    relay(web.clone(), key, client, upstream, early).await;
    web.disconnected_pi(connection);
    Ok(())
}

const TERMINAL: &str = "terminal";

/// Whose Pi is asking, and the browser's own path for what it asked.
fn whose(target: &str) -> (String, String) {
    let Some(rest) = target.strip_prefix("/s/") else {
        return (TERMINAL.into(), target.into());
    };
    let (key, path) = rest.split_once('/').unwrap_or((rest, ""));
    let key = if key.is_empty() { TERMINAL } else { key };
    (key.to_string(), format!("/{path}"))
}

fn session_of(key: &str) -> Option<String> {
    (key != TERMINAL).then(|| key.to_string())
}

/// Points the browser's own addresses at the gate.
fn rebase(target: &mut Value, base: &str) {
    if let Some(url) = target["webSocketDebuggerUrl"].as_str()
        && let Some((_, path)) = url.split_once("/devtools/")
    {
        target["webSocketDebuggerUrl"] = format!("{base}/devtools/{path}").into();
    }
}

/// Tells Pi the browser couldn't be reached, and why, rather than hanging
/// up on it.
async fn unreachable(
    client: &mut TcpStream,
    dialer: &Dialer,
    err: &std::io::Error,
) -> std::io::Result<()> {
    let why = dialer
        .refusal()
        .unwrap_or_else(|| format!("the browser didn't answer: {err}"));
    respond(
        client,
        "502 Bad Gateway",
        &json!({ "error": format!("Pi Workbench can't reach its browser: {why}") }),
    )
    .await
}

async fn respond(client: &mut TcpStream, status: &str, body: &Value) -> std::io::Result<()> {
    let body = serde_json::to_vec(body).expect("json serializes");
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json; charset=UTF-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    client.write_all(head.as_bytes()).await?;
    client.write_all(&body).await
}

/// What one Pi's connection has seen: which pages it may know about, its
/// sessions with them, and its commands still waiting on an answer.
struct Filter {
    web: Arc<Web>,
    key: String,
    /// Pages decided on, and whether this Pi sees them.
    decided: HashMap<String, bool>,
    /// This Pi's sessions with its pages: session id to page id.
    sessions: HashMap<String, String>,
    /// Sessions with pages this Pi doesn't see.
    hidden: HashSet<String>,
    creating: HashSet<u64>,
    listing: HashSet<u64>,
    own: AtomicU64,
    /// Pi asked to be attached to its pages as they come. The gate does
    /// it, once each is a page: in CEF, a popup attached to as it's made
    /// never loads.
    auto: bool,
    /// Pages the gate has attached Pi to.
    following: HashSet<String>,
    /// Answers to Pi's `Target.createTarget`, until it's attached to the page.
    held: HashMap<String, Value>,
    /// Messages for Pi after the one being passed on.
    after: Vec<Value>,
    starting: Option<Starting>,
}

/// Pi's `Target.setAutoAttach`, until it's attached to the pages there were.
struct Starting {
    /// The gate's own `Target.setDiscoverTargets`.
    discover: u64,
    answer: Value,
    /// Those pages, once the browser has told of them all.
    waiting: Option<HashSet<String>>,
}

impl Filter {
    fn sees(&mut self, page: &str, opener: Option<&str>) -> bool {
        if let Some(seen) = self.decided.get(page) {
            return *seen;
        }
        let seen = match self.web.visible(page, &self.key) {
            Some(seen) => seen,
            // Not known yet: a page this Pi is opening, or one opened from
            // a page it has.
            None => {
                !self.creating.is_empty()
                    || opener.is_some_and(|o| self.decided.get(o) == Some(&true))
            }
        };
        self.decided.insert(page.to_string(), seen);
        seen
    }

    /// Pi's browser-wide `Target.setAutoAttach`, done by the gate: the
    /// browser only tells it of pages, and it attaches Pi to each it sees.
    /// Pi hears back once it's attached to the pages already there, as it
    /// would from the browser.
    fn auto_attach(&mut self, asked: &Value) -> Value {
        self.auto = asked["params"]["autoAttach"] == true;
        let id = self.own.fetch_add(1, Ordering::Relaxed);
        self.starting = Some(Starting {
            discover: id,
            answer: json!({ "id": asked["id"], "result": {} }),
            waiting: None,
        });
        json!({ "id": id, "method": "Target.setDiscoverTargets", "params": { "discover": true } })
    }

    /// Answers Pi's `Target.setAutoAttach` once nothing it waits on is left.
    fn started(&mut self) {
        let done = self
            .starting
            .as_ref()
            .is_some_and(|s| s.waiting.as_ref().is_some_and(HashSet::is_empty));
        if done && let Some(starting) = self.starting.take() {
            self.after.push(starting.answer);
        }
    }

    /// Attaches Pi to a page it sees, if it asked to be.
    fn follow(&mut self, page: &str, own: &mut Vec<Value>) {
        if self.auto && self.following.insert(page.to_string()) {
            let id = self.own.fetch_add(1, Ordering::Relaxed);
            own.push(json!({
                "id": id,
                "method": "Target.attachToTarget",
                "params": { "targetId": page, "flatten": true },
            }));
        }
    }

    /// A page Pi's `Target.setAutoAttach` may wait on is attached, or gone.
    fn arrived(&mut self, page: &str) {
        if let Some(waiting) = self.starting.as_mut().and_then(|s| s.waiting.as_mut()) {
            waiting.remove(page);
            self.started();
        }
    }

    /// The answer to Pi's `Target.createTarget`. Pi takes the page to be
    /// attached to by then, as it would be if the browser attached it, so
    /// the answer waits until it is.
    fn created(&mut self, page: &str, answer: Value, own: &mut Vec<Value>) -> Option<Value> {
        if !self.auto || self.sessions.values().any(|p| p == page) {
            return Some(answer);
        }
        self.follow(page, own);
        self.held.insert(page.to_string(), answer);
        None
    }

    /// A message from Pi, on its way to the browser.
    fn sent(&mut self, message: &Value) {
        let id = message["id"].as_u64().unwrap_or_default();
        match message["method"].as_str().unwrap_or_default() {
            "Target.createTarget" => {
                self.creating.insert(id);
            }
            "Target.getTargets" => {
                self.listing.insert(id);
            }
            method => {
                if let Some(page) = message["sessionId"]
                    .as_str()
                    .and_then(|s| self.sessions.get(s))
                {
                    let driving = ACTIONS.iter().any(|a| method.starts_with(a));
                    self.web.used(&self.key, page, driving);
                }
            }
        }
    }

    /// A message from the browser, on its way to Pi: None to keep it from
    /// Pi. Commands the gate sends the browser itself go in `own`.
    fn received(&mut self, mut message: Value, own: &mut Vec<Value>) -> Option<Value> {
        if let Some(id) = message["id"].as_u64() {
            if id >= OWN_IDS {
                // The browser has told of every page there was by now.
                if let Some(starting) = self.starting.as_mut().filter(|s| s.discover == id) {
                    let attached: HashSet<&String> = self.sessions.values().collect();
                    let waiting = self.following.iter().filter(|p| !attached.contains(p));
                    starting.waiting = Some(waiting.cloned().collect());
                    self.started();
                }
                return None;
            }
            if self.creating.remove(&id)
                && let Some(page) = message["result"]["targetId"].as_str()
            {
                let page = page.to_string();
                self.decided.insert(page.clone(), true);
                self.web.claim(
                    &page,
                    Claim {
                        shown: self.key == TERMINAL,
                        kept: false,
                        session: session_of(&self.key),
                        pi: Some(self.key.clone()),
                    },
                );
                return self.created(&page, message, own);
            }
            if self.listing.remove(&id)
                && let Some(infos) = message["result"]["targetInfos"].as_array_mut()
            {
                let mut kept = Vec::new();
                for info in infos.drain(..) {
                    let page = info["targetId"].as_str().unwrap_or_default().to_string();
                    if info["type"] != "page" || self.sees(&page, info["openerId"].as_str()) {
                        kept.push(info);
                    }
                }
                *infos = kept;
            }
            return Some(message);
        }
        let params = &message["params"];
        let info = &params["targetInfo"];
        let session = message["sessionId"].as_str();
        if session.is_some_and(|s| self.hidden.contains(s)) {
            return None;
        }
        match message["method"].as_str().unwrap_or_default() {
            "Target.targetCreated" | "Target.targetInfoChanged" if info["type"] == "page" => {
                let page = info["targetId"].as_str().unwrap_or_default().to_string();
                if !self.sees(&page, info["openerId"].as_str()) {
                    return None;
                }
                self.follow(&page, own);
                Some(message)
            }
            "Target.attachedToTarget" if info["type"] == "page" => {
                let page = info["targetId"].as_str().unwrap_or_default().to_string();
                let attached = params["sessionId"].as_str().unwrap_or_default().to_string();
                if self.sees(&page, info["openerId"].as_str()) {
                    if let Some(answer) = self.held.remove(&page) {
                        self.after.push(answer);
                    }
                    self.arrived(&page);
                    self.sessions.insert(attached, page);
                    return Some(message);
                }
                // Attached all the same, and maybe waiting for Pi to let it
                // run: let it go, or it would never load.
                let mut next = |method: &str, params: Value, session: Option<&str>| {
                    let id = self.own.fetch_add(1, Ordering::Relaxed);
                    let mut command = json!({ "id": id, "method": method, "params": params });
                    if let Some(session) = session {
                        command["sessionId"] = session.into();
                    }
                    own.push(command);
                };
                if params["waitingForDebugger"] == true {
                    next(
                        "Runtime.runIfWaitingForDebugger",
                        json!({}),
                        Some(&attached),
                    );
                }
                next(
                    "Target.detachFromTarget",
                    json!({ "sessionId": attached }),
                    None,
                );
                self.hidden.insert(attached);
                None
            }
            "Target.detachedFromTarget" => {
                let detached = params["sessionId"].as_str().unwrap_or_default();
                if self.hidden.remove(detached) {
                    return None;
                }
                self.sessions.remove(detached);
                Some(message)
            }
            "Target.targetDestroyed" | "Target.targetCrashed" => {
                let page = params["targetId"].as_str().unwrap_or_default();
                self.following.remove(page);
                self.arrived(page);
                (self.decided.get(page) != Some(&false)).then_some(message)
            }
            _ => Some(message),
        }
    }
}

/// Carries frames both ways until either side ends, through the filter.
async fn relay<U>(web: Arc<Web>, key: String, client: TcpStream, upstream: U, early: Vec<u8>)
where
    U: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let filter = Arc::new(Mutex::new(Filter {
        web,
        key,
        decided: HashMap::new(),
        sessions: HashMap::new(),
        hidden: HashSet::new(),
        creating: HashSet::new(),
        listing: HashSet::new(),
        own: AtomicU64::new(OWN_IDS),
        auto: false,
        following: HashSet::new(),
        held: HashMap::new(),
        after: Vec::new(),
        starting: None,
    }));
    let (mut client_read, client_write) = client.into_split();
    let (mut up_read, up_write) = tokio::io::split(upstream);
    let to_browser = writer(up_write);
    let to_pi = writer(client_write);

    let pi_side = {
        let (filter, to_browser, to_pi) = (filter.clone(), to_browser.clone(), to_pi.clone());
        async move {
            let mut decoder = ws::Decoder::default();
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = match client_read.read(&mut buf).await {
                    Ok(0) | Err(_) => return,
                    Ok(n) => n,
                };
                decoder.push(&buf[..n]);
                while let Some(message) = decoder.next() {
                    if message.opcode == ws::TEXT
                        && let Ok(value) = serde_json::from_slice::<Value>(&message.payload)
                    {
                        let web = {
                            let mut filter = filter.lock().expect("not poisoned");
                            filter.sent(&value);
                            filter.web.clone()
                        };
                        // A new page, while wb-browser makes them: it makes this one.
                        if value["method"] == "Target.createTarget" && web.hosts_tabs() {
                            tokio::spawn(open_for_pi(
                                web,
                                filter.clone(),
                                to_browser.clone(),
                                to_pi.clone(),
                                value,
                            ));
                            continue;
                        }
                        if value["method"] == "Target.setAutoAttach"
                            && value.get("sessionId").is_none()
                        {
                            let discover = filter.lock().expect("not poisoned").auto_attach(&value);
                            let bytes = serde_json::to_vec(&discover).expect("json serializes");
                            let _ = to_browser.send(ws::encode(ws::TEXT, &bytes, true));
                            continue;
                        }
                    }
                    let closing = message.opcode == ws::CLOSE;
                    if to_browser
                        .send(ws::encode(message.opcode, &message.payload, true))
                        .is_err()
                        || closing
                    {
                        return;
                    }
                }
            }
        }
    };
    let browser_side = async move {
        let mut decoder = ws::Decoder::default();
        decoder.push(&early);
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            while let Some(message) = decoder.next() {
                let closing = message.opcode == ws::CLOSE;
                let forward = if message.opcode == ws::TEXT {
                    let Ok(value) = serde_json::from_slice::<Value>(&message.payload) else {
                        continue;
                    };
                    let mut own = Vec::new();
                    let mut filter = filter.lock().expect("not poisoned");
                    let kept = filter.received(value, &mut own);
                    let after = std::mem::take(&mut filter.after);
                    drop(filter);
                    for command in own {
                        let bytes = serde_json::to_vec(&command).expect("json serializes");
                        let _ = to_browser.send(ws::encode(ws::TEXT, &bytes, true));
                    }
                    kept.into_iter()
                        .chain(after)
                        .map(|value| serde_json::to_vec(&value).expect("json serializes"))
                        .collect()
                } else {
                    vec![message.payload]
                };
                for payload in forward {
                    if to_pi
                        .send(ws::encode(message.opcode, &payload, false))
                        .is_err()
                    {
                        return;
                    }
                }
                if closing {
                    return;
                }
            }
            match up_read.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(n) => decoder.push(&buf[..n]),
            }
        }
    };
    tokio::select! {
        () = pi_side => {}
        () = browser_side => {}
    }
}

/// Answers a Pi's `Target.createTarget` with a page wb-browser made, as the
/// browser's own answer would.
async fn open_for_pi(
    web: Arc<Web>,
    filter: Arc<Mutex<Filter>>,
    to_browser: mpsc::UnboundedSender<Vec<u8>>,
    to_pi: mpsc::UnboundedSender<Vec<u8>>,
    asked: Value,
) {
    let id = asked["id"].as_u64().unwrap_or_default();
    let url = asked["params"]["url"].as_str().unwrap_or("about:blank");
    let key = filter.lock().expect("not poisoned").key.clone();
    let claim = Claim {
        shown: key == TERMINAL,
        kept: false,
        session: session_of(&key),
        pi: Some(key),
    };
    let made = web.create(url, claim).await;
    let mut own = Vec::new();
    let answer = {
        let mut filter = filter.lock().expect("not poisoned");
        filter.creating.remove(&id);
        match made {
            Ok(tab) => {
                filter.decided.insert(tab.clone(), true);
                let answer = json!({ "id": id, "result": { "targetId": tab } });
                filter.created(&tab, answer, &mut own)
            }
            Err(err) => {
                Some(json!({ "id": id, "error": { "code": -32000, "message": err.to_string() } }))
            }
        }
    };
    for command in own {
        let bytes = serde_json::to_vec(&command).expect("json serializes");
        let _ = to_browser.send(ws::encode(ws::TEXT, &bytes, true));
    }
    if let Some(answer) = answer {
        let bytes = serde_json::to_vec(&answer).expect("json serializes");
        let _ = to_pi.send(ws::encode(ws::TEXT, &bytes, false));
    }
}

/// Writes what it's sent, in order, until the other end goes.
fn writer<W: AsyncWrite + Unpin + Send + 'static>(mut write: W) -> mpsc::UnboundedSender<Vec<u8>> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
    tokio::spawn(async move {
        while let Some(bytes) = rx.recv().await {
            if write.write_all(&bytes).await.is_err() {
                break;
            }
        }
        let _ = write.shutdown().await;
    });
    tx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_path_says_whose_pi_it_is() {
        assert_eq!(
            whose("/s/abc/json/version"),
            ("abc".to_string(), "/json/version".to_string())
        );
        assert_eq!(
            whose("/s/abc/devtools/browser/1-2"),
            ("abc".to_string(), "/devtools/browser/1-2".to_string())
        );
        assert_eq!(
            whose("/json/version"),
            (TERMINAL.to_string(), "/json/version".to_string())
        );
        assert_eq!(
            whose("/s//json"),
            (TERMINAL.to_string(), "/json".to_string())
        );
    }
}
