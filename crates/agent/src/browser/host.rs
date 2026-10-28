//! wb-browser, which hosts the browser's tabs in the workbench's window, as
//! wb-agent reaches it: through the app, in lines. While it's there, tabs
//! are made by asking it, as DevTools' own way would open a Chrome window
//! of its own. Without it, in development, DevTools makes them.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use protocol::{FromAgent, HostAsk, HostCall, HostSaid};
use tokio::sync::oneshot;

use crate::session::Out;

pub struct Host {
    out: Out,
    ready: AtomicBool,
    next: AtomicU64,
    waiting: Mutex<HashMap<u64, oneshot::Sender<Option<String>>>>,
}

impl Host {
    pub fn new(out: Out) -> Self {
        Host {
            out,
            ready: AtomicBool::new(false),
            next: AtomicU64::new(1),
            waiting: Mutex::default(),
        }
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
    }

    /// A new tab at `url`, by its DevTools target id.
    pub async fn open(&self, url: &str) -> Result<String, String> {
        let ask = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.waiting.lock().expect("not poisoned").insert(ask, tx);
        let _ = self.out.send(FromAgent::Host(HostCall {
            call: HostAsk::Open {
                ask,
                url: url.to_string(),
            },
        }));
        let answer = tokio::time::timeout(Duration::from_secs(15), rx).await;
        self.waiting.lock().expect("not poisoned").remove(&ask);
        match answer {
            Ok(Ok(Some(tab))) => Ok(tab),
            Ok(Ok(None)) => Err("the browser couldn't open a tab".into()),
            _ => Err("the browser didn't open the tab in time".into()),
        }
    }

    /// What wb-browser said.
    pub fn heard(&self, said: HostSaid) {
        let answer = match said {
            HostSaid::Ready(ready) => {
                self.ready.store(ready, Ordering::SeqCst);
                return;
            }
            HostSaid::Opened { ask, tab } => (ask, Some(tab)),
            HostSaid::Failed { ask } => (ask, None),
            // Pages, popups included, are followed through DevTools.
            HostSaid::Popup { .. } | HostSaid::Created { .. } | HostSaid::Closed { .. } => return,
        };
        if let Some(tx) = self.waiting.lock().expect("not poisoned").remove(&answer.0) {
            let _ = tx.send(answer.1);
        }
    }
}
