//! The browser: the workbench's Chromium, reached over the DevTools
//! protocol. wb-agent keeps one connection to it to follow every page and
//! download, record history and drive your tabs, and lets Pi in through
//! [`gate`], where it sees only its own session's pages.
//!
//! A page you open is in your tab strip. A page Pi opens isn't: its
//! session's card shows it, and opening it from there puts it in the strip.

mod cdp;
mod dial;
mod gate;
mod history;
mod host;
mod server;
mod ws;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use protocol::{Download, DownloadState, Event, FromAgent, Image, Shot, Visit, WebTab};
use serde_json::{Value, json};
use tokio::sync::Notify;

use crate::error::Error;
use crate::session::Out;
use cdp::Cdp;
pub use dial::{Dialer, Tunnels};
use history::History;
pub use host::Host;

/// How long a page shows Pi driving after Pi last acted on it.
const DRIVING: Duration = Duration::from_secs(3);

pub struct Web {
    out: Out,
    dialer: Option<Dialer>,
    agent_dir: PathBuf,
    data_dir: PathBuf,
    cdp: Mutex<Option<Arc<Cdp>>>,
    state: Mutex<State>,
    history: History,
    changed: Notify,
    /// Where Pi's `browser` server connects, once listening.
    port: Mutex<Option<u16>>,
    /// wb-browser, when the app runs it.
    host: Option<Host>,
}

#[derive(Default)]
struct State {
    connected: bool,
    tabs: Vec<Tab>,
    downloads: Vec<Download>,
    /// Who a page belongs to, when that's known before the page is.
    claims: HashMap<String, Claim>,
    /// Pages attached to, or being.
    following: HashSet<String>,
    /// The page each session's Pi last used, and when.
    used: HashMap<String, (String, Instant)>,
    /// Pi's connections, by number, with their session.
    connections: HashMap<u64, Option<String>>,
}

#[derive(Clone)]
struct Claim {
    shown: bool,
    /// You put it in your strip, so it stays there whoever claims it.
    kept: bool,
    session: Option<String>,
    /// The Pi that opened it: a session's id, or `terminal`.
    pi: Option<String>,
}

#[derive(Clone)]
struct Tab {
    id: String,
    /// wb-agent's session with the page, for commands to it.
    attached: Option<String>,
    url: String,
    title: String,
    loading: bool,
    shown: bool,
    kept: bool,
    session: Option<String>,
    pi: Option<String>,
    pi_at: Option<Instant>,
    can_back: bool,
    can_forward: bool,
}

impl Tab {
    /// Whether the Pi known by `key` may see the page.
    fn seen_by(&self, key: &str) -> bool {
        self.pi.as_deref() == Some(key) || self.session.as_deref() == Some(key)
    }

    fn view(&self) -> WebTab {
        WebTab {
            id: self.id.clone(),
            url: self.url.clone(),
            title: self.title.clone(),
            loading: self.loading,
            shown: self.shown,
            session: self.session.clone(),
            pi_driving: self.pi_at.is_some_and(|at| at.elapsed() < DRIVING),
            can_back: self.can_back,
            can_forward: self.can_forward,
        }
    }
}

impl Web {
    /// Without a dialer there's no browser to reach, and the workbench says so.
    pub fn new(
        out: Out,
        agent_dir: PathBuf,
        data_dir: PathBuf,
        dialer: Option<Dialer>,
        host: Option<Host>,
    ) -> Arc<Web> {
        Arc::new(Web {
            history: History::load(data_dir.join("history.jsonl")),
            out,
            dialer,
            agent_dir,
            data_dir,
            cdp: Mutex::default(),
            state: Mutex::default(),
            changed: Notify::new(),
            port: Mutex::default(),
            host,
        })
    }

    pub fn start(self: &Arc<Self>) {
        if let Err(err) = server::ensure(&self.agent_dir, &self.data_dir) {
            eprintln!("wb-agent: couldn't add the browser server: {err}");
        }
        let Some(dialer) = self.dialer.clone() else {
            return;
        };
        let web = self.clone();
        tokio::spawn(async move { web.emitting().await });
        let web = self.clone();
        tokio::spawn(async move { web.connecting(dialer).await });
        let web = self.clone();
        tokio::spawn(async move {
            match gate::listen(web.clone()).await {
                Ok(port) => *web.port.lock().expect("not poisoned") = Some(port),
                Err(err) => eprintln!("wb-agent: Pi can't reach the browser: {err}"),
            }
        });
    }

    /// The port Pi's `browser` server connects to, once it's listening.
    pub fn port(&self) -> Option<u16> {
        *self.port.lock().expect("not poisoned")
    }

    pub fn view(&self) -> protocol::Browser {
        let state = self.state.lock().expect("not poisoned");
        protocol::Browser {
            connected: state.connected,
            tabs: state.tabs.iter().map(Tab::view).collect(),
            downloads: state.downloads.clone(),
            server: server::present(&self.agent_dir),
            connections: state.connections.values().filter(|s| s.is_some()).count() as u32,
        }
    }

    fn changed(&self) {
        self.changed.notify_one();
    }

    /// Tells the workbench about changes, gathered up for a moment so a
    /// page loading costs one message, not ten.
    async fn emitting(self: Arc<Self>) {
        loop {
            self.changed.notified().await;
            tokio::time::sleep(Duration::from_millis(60)).await;
            let browser = self.view();
            let _ = self.out.send(FromAgent::Event(Event::Browser { browser }));
        }
    }

    /// Stays connected to the browser, trying again while it's away.
    async fn connecting(self: Arc<Self>, dialer: Dialer) {
        let mut wait = Duration::from_secs(1);
        loop {
            if let Ok(cdp) = Cdp::connect(&dialer).await {
                wait = Duration::from_secs(1);
                self.follow(cdp).await;
                let mut state = self.state.lock().expect("not poisoned");
                state.connected = false;
                state.tabs.clear();
                state.following.clear();
                drop(state);
                *self.cdp.lock().expect("not poisoned") = None;
                self.changed();
            }
            tokio::time::sleep(wait).await;
            wait = (wait * 2).min(Duration::from_secs(8));
        }
    }

    /// Follows the browser until the connection ends.
    async fn follow(self: &Arc<Self>, cdp: Arc<Cdp>) {
        let mut events = cdp.events();
        *self.cdp.lock().expect("not poisoned") = Some(cdp.clone());
        let ready = async {
            cdp.call(
                "Target.setDiscoverTargets",
                json!({ "discover": true }),
                None,
            )
            .await?;
            // Downloads still go where the browser puts them; this only tells us.
            let _ = cdp
                .call(
                    "Browser.setDownloadBehavior",
                    json!({ "behavior": "default", "eventsEnabled": true }),
                    None,
                )
                .await;
            Ok::<_, String>(())
        };
        if ready.await.is_err() {
            return;
        }
        self.state.lock().expect("not poisoned").connected = true;
        self.changed();
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            };
            if event["method"] == "closed" {
                return;
            }
            self.event(&cdp, &event);
        }
    }

    fn event(self: &Arc<Self>, cdp: &Arc<Cdp>, event: &Value) {
        let params = &event["params"];
        let method = event["method"].as_str().unwrap_or_default();
        let mut state = self.state.lock().expect("not poisoned");
        match method {
            "Target.targetCreated" | "Target.targetInfoChanged" => {
                let info = &params["targetInfo"];
                if info["type"] != "page" {
                    return;
                }
                let id = text(info, "targetId");
                let (url, title) = (text(info, "url"), text(info, "title"));
                let index = match state.tabs.iter().position(|tab| tab.id == id) {
                    Some(index) => index,
                    None => {
                        let opener = info["openerId"].as_str().and_then(|opener| {
                            state.tabs.iter().find(|tab| tab.id == opener).cloned()
                        });
                        // Claimed, or like the page that opened it, or yours.
                        let claim = state.claims.remove(&id).unwrap_or(match opener {
                            Some(opener) => Claim {
                                shown: opener.shown,
                                kept: false,
                                session: opener.session,
                                pi: opener.pi,
                            },
                            None => Claim {
                                shown: true,
                                kept: false,
                                session: None,
                                pi: None,
                            },
                        });
                        state.tabs.push(Tab {
                            id: id.clone(),
                            attached: None,
                            url: String::new(),
                            title: String::new(),
                            loading: false,
                            shown: claim.shown,
                            kept: claim.kept,
                            session: claim.session,
                            pi: claim.pi,
                            pi_at: None,
                            can_back: false,
                            can_forward: false,
                        });
                        state.tabs.len() - 1
                    }
                };
                let tab = &mut state.tabs[index];
                tab.url = url;
                tab.title = title;
                if tab.shown {
                    self.history.record(&tab.url, &tab.title, now());
                }
                // Attached to once it's a page, not as it's made: in CEF, a
                // popup attached to as it's made never loads.
                if state.following.insert(id.clone()) {
                    let cdp = cdp.clone();
                    tokio::spawn(async move {
                        let _ = cdp
                            .call(
                                "Target.attachToTarget",
                                json!({ "targetId": id, "flatten": true }),
                                None,
                            )
                            .await;
                    });
                }
            }
            "Target.targetDestroyed" => {
                let id = text(params, "targetId");
                state.tabs.retain(|tab| tab.id != id);
                state.following.remove(&id);
            }
            "Target.attachedToTarget" => {
                let info = &params["targetInfo"];
                if info["type"] != "page" {
                    return;
                }
                let id = text(info, "targetId");
                let session = text(params, "sessionId");
                if let Some(tab) = state.tabs.iter_mut().find(|tab| tab.id == id) {
                    tab.attached = Some(session.clone());
                }
                // Whatever the page did before it was followed, read now.
                let (web, cdp) = (self.clone(), cdp.clone());
                tokio::spawn(async move {
                    let _ = cdp.call("Page.enable", json!({}), Some(&session)).await;
                    web.steps(&cdp, &id, &session).await;
                });
            }
            "Page.frameNavigated" => {
                let frame = &params["frame"];
                let id = text(frame, "id");
                if frame.get("parentId").is_some() {
                    return;
                }
                let Some(tab) = state.tabs.iter_mut().find(|tab| tab.id == id) else {
                    return;
                };
                tab.url = text(frame, "url");
            }
            "Page.frameStartedLoading"
            | "Page.frameStoppedLoading"
            | "Page.navigatedWithinDocument" => {
                let session = event["sessionId"].as_str().unwrap_or_default();
                let frame = text(params, "frameId");
                let Some(tab) = state
                    .tabs
                    .iter_mut()
                    .find(|tab| tab.attached.as_deref() == Some(session))
                else {
                    return;
                };
                // The main frame shares its page's id.
                if frame != tab.id {
                    return;
                }
                tab.loading = method == "Page.frameStartedLoading";
                if !tab.loading {
                    let (web, cdp, id, session) = (
                        self.clone(),
                        cdp.clone(),
                        tab.id.clone(),
                        session.to_string(),
                    );
                    tokio::spawn(async move { web.steps(&cdp, &id, &session).await });
                }
            }
            "Browser.downloadWillBegin" => {
                state.downloads.insert(
                    0,
                    Download {
                        id: text(params, "guid"),
                        name: text(params, "suggestedFilename"),
                        url: text(params, "url"),
                        received: 0,
                        total: 0,
                        state: DownloadState::Going,
                        started: now(),
                    },
                );
                state.downloads.truncate(50);
            }
            "Browser.downloadProgress" => {
                let id = text(params, "guid");
                if let Some(download) = state.downloads.iter_mut().find(|d| d.id == id) {
                    download.received = params["receivedBytes"].as_f64().unwrap_or_default() as u64;
                    download.total = params["totalBytes"].as_f64().unwrap_or_default() as u64;
                    download.state = match params["state"].as_str() {
                        Some("completed") => DownloadState::Done,
                        Some("canceled") => DownloadState::Cancelled,
                        _ => DownloadState::Going,
                    };
                }
            }
            _ => return,
        }
        drop(state);
        self.changed();
    }

    /// After a page moved: its title, which the browser doesn't announce
    /// for a page in the background, and whether it can go back or forward.
    /// The title is read again a moment later, for pages that set it late.
    async fn steps(&self, cdp: &Cdp, id: &str, session: &str) {
        for wait in [0, 1500] {
            tokio::time::sleep(Duration::from_millis(wait)).await;
            let Ok(info) = cdp
                .call("Target.getTargetInfo", json!({ "targetId": id }), None)
                .await
            else {
                return;
            };
            let info = &info["targetInfo"];
            {
                let mut state = self.state.lock().expect("not poisoned");
                if let Some(tab) = state.tabs.iter_mut().find(|tab| tab.id == id) {
                    tab.url = text(info, "url");
                    tab.title = text(info, "title");
                    if tab.shown {
                        self.history.record(&tab.url, &tab.title, now());
                    }
                }
            }
            self.changed();
            if wait == 0 {
                self.history_steps(cdp, id, session).await;
            }
        }
    }

    async fn history_steps(&self, cdp: &Cdp, id: &str, session: &str) {
        let Ok(history) = cdp
            .call("Page.getNavigationHistory", json!({}), Some(session))
            .await
        else {
            return;
        };
        let at = history["currentIndex"].as_i64().unwrap_or_default();
        let count = history["entries"].as_array().map_or(0, Vec::len) as i64;
        let mut state = self.state.lock().expect("not poisoned");
        if let Some(tab) = state.tabs.iter_mut().find(|tab| tab.id == id) {
            tab.can_back = at > 0;
            tab.can_forward = at + 1 < count;
        }
        drop(state);
        self.changed();
    }

    fn connection(&self) -> Result<Arc<Cdp>, Error> {
        self.cdp
            .lock()
            .expect("not poisoned")
            .clone()
            .ok_or_else(|| Error::NotFound("the browser isn't running".into()))
    }

    /// wb-agent's session with a page.
    fn attached(&self, tab: &str) -> Result<String, Error> {
        let state = self.state.lock().expect("not poisoned");
        let found = state.tabs.iter().find(|t| t.id == tab);
        found
            .ok_or_else(|| Error::NotFound(format!("no page {tab}")))?
            .attached
            .clone()
            .ok_or_else(|| Error::NotFound(format!("page {tab} isn't ready yet")))
    }

    /// Says who a page belongs to: now if it's known, or when it appears.
    fn claim(&self, tab: &str, claim: Claim) {
        let mut state = self.state.lock().expect("not poisoned");
        match state.tabs.iter_mut().find(|t| t.id == tab) {
            Some(found) => {
                found.kept |= claim.kept;
                found.shown = claim.shown || found.kept;
                found.session = claim.session;
                found.pi = claim.pi;
            }
            None => {
                state.claims.insert(tab.to_string(), claim);
            }
        }
        drop(state);
        self.changed();
    }

    /// A new page, made by wb-browser when it's there, else by DevTools.
    async fn create(&self, url: &str, claim: Claim) -> Result<String, Error> {
        let id = match &self.host {
            Some(host) if host.is_ready() => host.open(url).await.map_err(Error::NotFound)?,
            _ => {
                let cdp = self.connection()?;
                let created = cdp
                    .call("Target.createTarget", json!({ "url": url }), None)
                    .await
                    .map_err(Error::NotFound)?;
                text(&created, "targetId")
            }
        };
        self.claim(&id, claim);
        Ok(id)
    }

    /// Whether new pages are made by wb-browser, so a Pi's own way of
    /// making them has to come here.
    fn hosts_tabs(&self) -> bool {
        self.host.as_ref().is_some_and(Host::is_ready)
    }

    /// What wb-browser said, through the app.
    pub fn host_said(&self, said: protocol::HostSaid) {
        if let Some(host) = &self.host {
            host.heard(said);
        }
    }

    pub async fn open(&self, url: &str, session: Option<String>) -> Result<WebTab, Error> {
        let id = self
            .create(
                url,
                Claim {
                    shown: true,
                    kept: true,
                    session,
                    pi: None,
                },
            )
            .await?;
        // The browser may not have said it's there yet.
        for _ in 0..40 {
            if let Ok(tab) = self.tab(&id) {
                return Ok(tab);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        self.tab(&id)
    }

    fn tab(&self, id: &str) -> Result<WebTab, Error> {
        let state = self.state.lock().expect("not poisoned");
        let found = state.tabs.iter().find(|t| t.id == id).map(Tab::view);
        found.ok_or_else(|| Error::NotFound(format!("no page {id}")))
    }

    pub fn show(&self, tab: &str) -> Result<(), Error> {
        self.edit(tab, |t| {
            t.shown = true;
            t.kept = true;
        })
    }

    pub fn set_session(&self, tab: &str, session: Option<String>) -> Result<(), Error> {
        self.edit(tab, |t| t.session = session)
    }

    fn edit(&self, tab: &str, change: impl FnOnce(&mut Tab)) -> Result<(), Error> {
        let mut state = self.state.lock().expect("not poisoned");
        let found = state.tabs.iter_mut().find(|t| t.id == tab);
        change(found.ok_or_else(|| Error::NotFound(format!("no page {tab}")))?);
        drop(state);
        self.changed();
        Ok(())
    }

    pub async fn navigate(&self, tab: &str, url: &str) -> Result<(), Error> {
        self.page(tab, "Page.navigate", json!({ "url": url }))
            .await
            .map(drop)
    }

    pub async fn reload(&self, tab: &str) -> Result<(), Error> {
        self.page(tab, "Page.reload", json!({})).await.map(drop)
    }

    pub async fn go(&self, tab: &str, by: i32) -> Result<(), Error> {
        let history = self
            .page(tab, "Page.getNavigationHistory", json!({}))
            .await?;
        let at = history["currentIndex"].as_i64().unwrap_or_default() + i64::from(by);
        let Some(entry) = history["entries"]
            .get(at.max(0) as usize)
            .filter(|_| at >= 0)
        else {
            return Ok(());
        };
        self.page(
            tab,
            "Page.navigateToHistoryEntry",
            json!({ "entryId": entry["id"] }),
        )
        .await
        .map(drop)
    }

    pub async fn close(&self, tab: &str) -> Result<(), Error> {
        let cdp = self.connection()?;
        cdp.call("Target.closeTarget", json!({ "targetId": tab }), None)
            .await
            .map(drop)
            .map_err(Error::NotFound)
    }

    async fn page(&self, tab: &str, method: &str, params: Value) -> Result<Value, Error> {
        let session = self.attached(tab)?;
        self.connection()?
            .call(method, params, Some(&session))
            .await
            .map_err(Error::NotFound)
    }

    pub fn history(&self, query: &str) -> Vec<Visit> {
        self.history.search(query, 8)
    }

    /// The page as it looks now, at most 640 pixels wide.
    pub async fn shot(&self, tab: &str) -> Result<Shot, Error> {
        let metrics = self.page(tab, "Page.getLayoutMetrics", json!({})).await?;
        let viewport = &metrics["cssLayoutViewport"];
        let width = viewport["clientWidth"].as_f64().unwrap_or(1280.0).max(1.0);
        let height = viewport["clientHeight"].as_f64().unwrap_or(800.0).max(1.0);
        let picture = self
            .page(
                tab,
                "Page.captureScreenshot",
                json!({
                    "format": "jpeg",
                    "quality": 70,
                    "clip": { "x": 0, "y": 0, "width": width, "height": height, "scale": (640.0 / width).min(1.0) },
                }),
            )
            .await
            .ok();
        let view = self.tab(tab)?;
        Ok(Shot {
            tab: view.id,
            url: view.url,
            title: view.title,
            image: picture.and_then(|p| {
                let data = p["data"].as_str()?.to_string();
                base64::engine::general_purpose::STANDARD
                    .decode(&data)
                    .ok()?;
                Some(Image {
                    mime_type: "image/jpeg".into(),
                    data,
                })
            }),
        })
    }

    /// Whether the Pi known by `key` may see a page; None when the page
    /// isn't known yet, or is no one's yet.
    fn visible(&self, tab: &str, key: &str) -> Option<bool> {
        let state = self.state.lock().expect("not poisoned");
        if let Some(found) = state.tabs.iter().find(|t| t.id == tab) {
            if found.seen_by(key) {
                return Some(true);
            }
            // A page the browser has just opened, with no one's name on it
            // yet, may be the one this Pi is opening.
            let anyones = found.kept || found.pi.is_some() || found.session.is_some();
            return anyones.then_some(false);
        }
        let claim = state.claims.get(tab)?;
        Some(claim.pi.as_deref() == Some(key) || claim.session.as_deref() == Some(key))
    }

    fn has_pages(&self, key: &str) -> bool {
        let state = self.state.lock().expect("not poisoned");
        state.tabs.iter().any(|t| t.seen_by(key))
    }

    /// A Pi used a page: drove it, or read it.
    fn used(self: &Arc<Self>, key: &str, tab: &str, driving: bool) {
        let mut state = self.state.lock().expect("not poisoned");
        state
            .used
            .insert(key.to_string(), (tab.to_string(), Instant::now()));
        if !driving {
            return;
        }
        if let Some(found) = state.tabs.iter_mut().find(|t| t.id == tab) {
            let was = found.pi_at.is_some_and(|at| at.elapsed() < DRIVING);
            found.pi_at = Some(Instant::now());
            drop(state);
            if !was {
                self.changed();
            }
            // And again once it's been still for long enough to stop showing.
            let web = self.clone();
            tokio::spawn(async move {
                tokio::time::sleep(DRIVING + Duration::from_millis(100)).await;
                web.changed();
            });
        }
    }

    fn connected_pi(&self, session: Option<String>) -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        self.state
            .lock()
            .expect("not poisoned")
            .connections
            .insert(id, session);
        self.changed();
        id
    }

    fn disconnected_pi(&self, id: u64) {
        self.state
            .lock()
            .expect("not poisoned")
            .connections
            .remove(&id);
        self.changed();
    }

    /// Whether a session's Pi has used the browser since `since`.
    pub fn used_since(&self, session: &str, since: Instant) -> bool {
        let state = self.state.lock().expect("not poisoned");
        state.used.get(session).is_some_and(|(_, at)| *at >= since)
    }

    /// The page a session's Pi last used, as it looks now.
    pub async fn session_shot(&self, session: &str) -> Option<Shot> {
        let tab = {
            let state = self.state.lock().expect("not poisoned");
            state.used.get(session)?.0.clone()
        };
        self.shot(&tab).await.ok()
    }

    pub async fn clear_data(&self) -> Result<(), Error> {
        self.connection()?
            .call("Storage.clearCookies", json!({}), None)
            .await
            .map(drop)
            .map_err(Error::NotFound)
    }

    pub fn add_server(&self) -> Result<(), Error> {
        server::add(&self.agent_dir)?;
        self.changed();
        Ok(())
    }
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_string()
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}
