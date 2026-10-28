//! Reaching the browser's DevTools port. On Windows the browser runs beside
//! the app and wb-agent runs in WSL, so each connection is a stream carried
//! in lines through the app ([`Tunnels`]). Elsewhere, in development, it's
//! an ordinary TCP connection.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use protocol::{FromAgent, Tunnel, TunnelOp};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, DuplexStream, WriteHalf};
use tokio::sync::mpsc;

use crate::session::Out;

pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

#[derive(Clone)]
pub enum Dialer {
    /// `host:port`.
    Direct(String),
    Tunnel(Arc<Tunnels>),
}

impl Dialer {
    pub async fn connect(&self) -> std::io::Result<Box<dyn Stream>> {
        match self {
            Dialer::Direct(address) => {
                let stream = tokio::net::TcpStream::connect(address).await?;
                stream.set_nodelay(true)?;
                Ok(Box::new(stream))
            }
            Dialer::Tunnel(tunnels) => Ok(Box::new(tunnels.open())),
        }
    }

    /// Why the app last couldn't reach the browser, if it said.
    pub fn refusal(&self) -> Option<String> {
        match self {
            Dialer::Direct(_) => None,
            Dialer::Tunnel(tunnels) => tunnels.refusal.lock().expect("not poisoned").clone(),
        }
    }
}

/// The streams carried through the app, by number.
pub struct Tunnels {
    out: Out,
    next: AtomicU32,
    /// Where bytes from the app go, for each open stream.
    open: Mutex<HashMap<u32, mpsc::UnboundedSender<Option<Vec<u8>>>>>,
    /// Why the app last couldn't connect a stream; cleared once one does.
    refusal: Mutex<Option<String>>,
}

impl Tunnels {
    pub fn new(out: Out) -> Arc<Self> {
        Arc::new(Tunnels {
            out,
            next: AtomicU32::new(1),
            open: Mutex::default(),
            refusal: Mutex::default(),
        })
    }

    /// A new stream to the browser. The app connects when it reads the
    /// request, and closes the stream if it can't.
    pub fn open(self: &Arc<Self>) -> DuplexStream {
        let stream = self.next.fetch_add(1, Ordering::Relaxed);
        let (mine, theirs) = tokio::io::duplex(256 * 1024);
        let (mut reading, writing) = tokio::io::split(theirs);
        let (tx, rx) = mpsc::unbounded_channel();
        self.open.lock().expect("not poisoned").insert(stream, tx);
        self.send(stream, TunnelOp::Open);
        tokio::spawn(deliver(rx, writing));
        let tunnels = self.clone();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match reading.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => tunnels.send(
                        stream,
                        TunnelOp::Data {
                            data: STANDARD.encode(&buf[..n]),
                        },
                    ),
                }
            }
            if tunnels.forget(stream) {
                tunnels.send(stream, TunnelOp::Close { reason: None });
            }
        });
        mine
    }

    /// A line from the app about one of the streams.
    pub fn incoming(&self, tunnel: Tunnel) {
        let open = self.open.lock().expect("not poisoned");
        let Some(tx) = open.get(&tunnel.stream) else {
            return;
        };
        match tunnel.op {
            TunnelOp::Data { data } => {
                *self.refusal.lock().expect("not poisoned") = None;
                if let Ok(bytes) = STANDARD.decode(data) {
                    let _ = tx.send(Some(bytes));
                }
            }
            TunnelOp::Close { reason } => {
                if reason.is_some() {
                    *self.refusal.lock().expect("not poisoned") = reason;
                }
                let _ = tx.send(None);
                drop(open);
                self.forget(tunnel.stream);
            }
            TunnelOp::Open => {}
        }
    }

    /// Takes a stream off the list; false if it was already gone.
    fn forget(&self, stream: u32) -> bool {
        self.open
            .lock()
            .expect("not poisoned")
            .remove(&stream)
            .is_some()
    }

    fn send(&self, stream: u32, op: TunnelOp) {
        let _ = self.out.send(FromAgent::Tunnel(Tunnel { stream, op }));
    }
}

/// Writes what the app sends into the stream, in order, until it closes.
async fn deliver(
    mut rx: mpsc::UnboundedReceiver<Option<Vec<u8>>>,
    mut writing: WriteHalf<DuplexStream>,
) {
    while let Some(Some(bytes)) = rx.recv().await {
        if writing.write_all(&bytes).await.is_err() {
            break;
        }
    }
    let _ = writing.shutdown().await;
}
