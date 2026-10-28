//! Web tabs: pages in the workbench's Chromium. You browse in them, and Pi
//! uses the same browser through its `browser` MCP server. A page Pi opens
//! stays out of your tab strip until you open it from Pi's card.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::Image;

/// The browser as wb-agent last saw it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Browser {
    /// Whether wb-agent can reach the browser at all.
    pub connected: bool,
    /// Every page, yours and Pi's, the newest last.
    pub tabs: Vec<WebTab>,
    /// Downloads, the newest first.
    pub downloads: Vec<Download>,
    /// Whether Pi's `browser` server is in `mcp.json`.
    pub server: bool,
    /// Pi sessions connected to the browser right now.
    pub connections: u32,
}

/// A page in the browser.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WebTab {
    /// The browser's id for the page.
    pub id: String,
    pub url: String,
    pub title: String,
    pub loading: bool,
    /// In your tab strip. Pages Pi opens aren't, until you open them.
    pub shown: bool,
    /// The session beside the page: the one that opened it, or one you
    /// picked. None for a page Pi opened from a terminal, or one of yours
    /// with no session yet.
    pub session: Option<String>,
    /// Pi has acted on the page in the last few seconds.
    pub pi_driving: bool,
    pub can_back: bool,
    pub can_forward: bool,
}

/// A page you've been to, for suggestions in the address bar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Visit {
    pub url: String,
    pub title: String,
    /// When, in milliseconds since 1970.
    #[ts(type = "number")]
    pub at: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Download {
    pub id: String,
    /// The file's name.
    pub name: String,
    pub url: String,
    #[ts(type = "number")]
    pub received: u64,
    /// 0 when the size isn't known.
    #[ts(type = "number")]
    pub total: u64,
    pub state: DownloadState,
    /// When it started, in milliseconds since 1970.
    #[ts(type = "number")]
    pub started: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DownloadState {
    Going,
    Done,
    Cancelled,
}

/// The page a tool call left Pi on: what a session's browser card shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Shot {
    pub tab: String,
    pub url: String,
    pub title: String,
    /// The page as it looked, small.
    pub image: Option<Image>,
}

/// What wb-agent asks of wb-browser, which hosts the browser's tabs in the
/// workbench's window. A tab is made there rather than by DevTools, which in
/// CEF would open a Chrome window of its own.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HostCall {
    #[serde(flatten)]
    pub call: HostAsk,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum HostAsk {
    /// A new tab, hidden until the page places it.
    Open {
        #[ts(type = "number")]
        ask: u64,
        url: String,
    },
}

/// What wb-browser says back, through the app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct HostNews {
    #[serde(flatten)]
    pub news: HostSaid,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum HostSaid {
    /// It's running and makes the tabs.
    Ready(bool),
    /// The tab asked for, by its DevTools target id.
    Opened {
        #[ts(type = "number")]
        ask: u64,
        tab: String,
    },
    Failed {
        #[ts(type = "number")]
        ask: u64,
    },
    /// A page opened a popup, this size if it said.
    Popup {
        tab: String,
        width: i32,
        height: i32,
    },
    /// A tab neither asked for nor a popup.
    Created {
        tab: String,
    },
    Closed {
        tab: String,
    },
}
