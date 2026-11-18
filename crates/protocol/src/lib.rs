//! Every message the page and wbd send each other. JSON-RPC 2.0, one object
//! per line, in both directions.
//!
//! The page asks:
//!   {"jsonrpc":"2.0","id":1,"method":"health"}
//! wbd answers with the same id, and either a result or an error:
//!   {"jsonrpc":"2.0","id":1,"result":{"kind":"health",...}}
//!   {"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"...","data":{"kind":"notFound"}}}
//! and says things on its own as events:
//!   {"jsonrpc":"2.0","method":"event","params":{"event":"ready","version":"2.0.0"}}
//!
//! These Rust types are the source. `cargo test -p protocol` writes the
//! TypeScript ones into app/ui/src/protocol.

use serde::{Deserialize, Serialize};
use serde_json::json;
use ts_rs::TS;

/// What the page can ask.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(tag = "method", content = "params")]
#[ts(export)]
pub enum Call {
    /// What wbd is, and what it found beside it.
    #[serde(rename = "health")]
    Health,
}

/// A call, with the id its answer will carry.
#[derive(Debug, Deserialize)]
pub struct Request {
    pub id: u64,
    #[serde(flatten)]
    pub call: Call,
}

/// What wbd answers. `kind` says which, so the page can check it got the
/// answer it asked for.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum Answer {
    Health(Health),
}

/// wbd, and the programs it works with. A program that isn't found has no version.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct Health {
    /// wbd's own version.
    pub version: String,
    /// The WSL distro wbd runs in, if it runs in one.
    pub distro: Option<String>,
    pub pi: Option<String>,
    pub claude: Option<String>,
    pub dova: Option<String>,
}

/// What wbd says without being asked.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[serde(tag = "event", rename_all = "camelCase")]
#[ts(export)]
pub enum Event {
    /// Sent once, first: wbd is up and reading.
    Ready { version: String },
}

/// Why a call wasn't answered: a JSON-RPC error.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct Failure {
    pub code: i32,
    pub message: String,
    pub data: FailureData,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, TS)]
#[ts(export)]
pub struct FailureData {
    pub kind: FailureKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum FailureKind {
    /// The line wasn't a call wbd knows.
    BadRequest,
}

impl Failure {
    pub fn new(kind: FailureKind, message: impl Into<String>) -> Self {
        let code = match kind {
            FailureKind::BadRequest => -32600,
        };
        Self {
            code,
            message: message.into(),
            data: FailureData { kind },
        }
    }
}

/// The line that answers call `id`.
pub fn reply(id: u64, answer: &Answer) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": answer }).to_string()
}

/// The line that refuses call `id`, or a line that had no id wbd could read.
pub fn failed(id: Option<u64>, failure: &Failure) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": failure }).to_string()
}

/// The line that carries an event.
pub fn event(event: &Event) -> String {
    json!({ "jsonrpc": "2.0", "method": "event", "params": event }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_is_read_with_or_without_the_envelope() {
        let full: Request =
            serde_json::from_str(r#"{"jsonrpc":"2.0","id":7,"method":"health"}"#).unwrap();
        assert_eq!((full.id, full.call), (7, Call::Health));
        let bare: Request = serde_json::from_str(r#"{"id":8,"method":"health"}"#).unwrap();
        assert_eq!(bare.id, 8);
    }

    #[test]
    fn an_unknown_method_is_not_a_call() {
        assert!(serde_json::from_str::<Request>(r#"{"id":1,"method":"nope"}"#).is_err());
    }

    #[test]
    fn the_lines_wbd_writes() {
        let health = Health {
            version: "2.0.0".into(),
            distro: Some("Ubuntu".into()),
            pi: Some("1.5.2".into()),
            claude: None,
            dova: Some("0.4.2".into()),
        };
        assert_eq!(
            reply(3, &Answer::Health(health)),
            r#"{"id":3,"jsonrpc":"2.0","result":{"claude":null,"distro":"Ubuntu","dova":"0.4.2","kind":"health","pi":"1.5.2","version":"2.0.0"}}"#
        );
        assert_eq!(
            failed(None, &Failure::new(FailureKind::BadRequest, "not JSON")),
            r#"{"error":{"code":-32600,"data":{"kind":"badRequest"},"message":"not JSON"},"id":null,"jsonrpc":"2.0"}"#
        );
        assert_eq!(
            event(&Event::Ready {
                version: "2.0.0".into()
            }),
            r#"{"jsonrpc":"2.0","method":"event","params":{"event":"ready","version":"2.0.0"}}"#
        );
    }
}
