//! dova, asked through `dova rpc`: one long-lived process, JSON-RPC 2.0, one
//! object per line. It starts with the first call and starts again if it
//! has gone. Each call names the folder it is about, so one process serves
//! every codespace, and dova reads the org, project and repo from that
//! folder's git remote.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use protocol::LogKind;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::oneshot;

use crate::error::Error;
use crate::log;

/// A call that takes longer than this is given up on. dova makes several
/// requests to Azure DevOps for some of them.
const PATIENCE: Duration = Duration::from_secs(45);

type Waiting = Mutex<HashMap<u64, oneshot::Sender<Result<Value, Error>>>>;

#[derive(Default)]
pub struct Dova {
    /// Where calls are written, while dova runs.
    stdin: Arc<tokio::sync::Mutex<Option<ChildStdin>>>,
    /// The calls waiting for their answer, by id.
    waiting: Arc<Waiting>,
    next: AtomicU64,
}

impl Dova {
    /// Asks dova one thing about the folder `cwd`.
    pub async fn call(&self, method: &str, cwd: &str, mut params: Value) -> Result<Value, Error> {
        params["cwd"] = json!(cwd);
        let what = format!("{method} {}", log::given(&params, "cwd"));
        let running = log::begin(LogKind::Dova, what.trim_end(), Some(cwd));
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let line = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let result = self.asked(id, method, format!("{line}\n")).await;
        running.end(&result);
        result
    }

    /// Writes a call to dova and waits for its answer.
    async fn asked(&self, id: u64, method: &str, line: String) -> Result<Value, Error> {
        let (answer, answered) = oneshot::channel();
        self.waiting
            .lock()
            .expect("not poisoned")
            .insert(id, answer);

        if let Err(err) = self.write(line).await {
            self.waiting.lock().expect("not poisoned").remove(&id);
            return Err(err);
        }
        match tokio::time::timeout(PATIENCE, answered).await {
            Ok(Ok(result)) => result,
            // The sender went without answering: dova stopped.
            Ok(Err(_)) => Err(Error::Dova("dova stopped before it answered".into())),
            Err(_) => {
                self.waiting.lock().expect("not poisoned").remove(&id);
                Err(Error::Dova(format!(
                    "dova took too long to answer {method}: more than {} s",
                    PATIENCE.as_secs()
                )))
            }
        }
    }

    /// Writes one line to dova, starting it first if it isn't running.
    async fn write(&self, line: String) -> Result<(), Error> {
        let mut stdin = self.stdin.lock().await;
        if stdin.is_none() {
            *stdin = Some(self.start()?);
        }
        let pipe = stdin.as_mut().expect("just started");
        let sent = async {
            pipe.write_all(line.as_bytes()).await?;
            pipe.flush().await
        };
        if let Err(err) = sent.await {
            // It went away. The next call starts it again.
            *stdin = None;
            return Err(Error::Dova(format!("couldn't reach dova: {err}")));
        }
        Ok(())
    }

    /// Starts `dova rpc`, and hears its answers until it ends.
    fn start(&self) -> Result<ChildStdin, Error> {
        let program = std::env::var("WB_DOVA").unwrap_or_else(|_| "dova".into());
        let mut child = Command::new(program)
            .arg("rpc")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|err| Error::Dova(format!("couldn't start dova: {err}")))?;
        log::note(LogKind::Dova, "dova rpc started");
        let stdin = child.stdin.take().expect("stdin is piped");
        let stdout = child.stdout.take().expect("stdout is piped");
        // What it prints beside its answers is for the log.
        let stderr = child.stderr.take().expect("stderr is piped");
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    log::note(LogKind::Dova, &format!("dova printed: {line}"));
                }
            }
        });
        let (waiting, gone) = (self.waiting.clone(), self.stdin.clone());
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(reply) = serde_json::from_str::<Value>(&line) else {
                    continue;
                };
                let Some(id) = reply["id"].as_u64() else {
                    continue;
                };
                if let Some(answer) = waiting.lock().expect("not poisoned").remove(&id) {
                    let _ = answer.send(read(reply));
                }
            }
            let ended = match child.wait().await {
                Ok(status) => format!("dova rpc ended: {status}"),
                Err(err) => format!("dova rpc ended: {err}"),
            };
            log::note(LogKind::Dova, &ended);
            // Nobody waiting will be answered now.
            waiting.lock().expect("not poisoned").clear();
            *gone.lock().await = None;
        });
        Ok(stdin)
    }
}

/// A reply from dova: its result, or why it refused.
fn read(mut reply: Value) -> Result<Value, Error> {
    let error = &reply["error"];
    if error.is_null() {
        return Ok(reply["result"].take());
    }
    let mut message = error["message"]
        .as_str()
        .unwrap_or("dova failed")
        .to_string();
    // dova says what to do about it, where it knows.
    for hint in error["data"]["hints"].as_array().into_iter().flatten() {
        if let Some(hint) = hint.as_str() {
            message.push(' ');
            message.push_str(hint);
        }
    }
    Err(match error["data"]["kind"].as_str() {
        Some("not_signed_in") => Error::NotSignedIn(message),
        _ => Error::Dova(message),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_carries_its_kind_and_what_to_do() {
        let refused = json!({
            "jsonrpc": "2.0", "id": 1,
            "error": { "code": -32000, "message": "Not signed in.",
                       "data": { "kind": "not_signed_in", "exitCode": 3, "hints": ["Run: az login"] } },
        });
        assert!(
            matches!(read(refused), Err(Error::NotSignedIn(m)) if m == "Not signed in. Run: az login")
        );
        let other = json!({ "id": 2, "error": { "message": "No such run.", "data": { "kind": "not_found" } } });
        assert!(matches!(read(other), Err(Error::Dova(m)) if m == "No such run."));
        assert_eq!(
            read(json!({ "id": 3, "result": [1, 2] })).unwrap(),
            json!([1, 2])
        );
    }
}
