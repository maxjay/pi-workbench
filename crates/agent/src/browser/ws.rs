//! Just enough WebSocket for the DevTools protocol: a client's handshake,
//! and frames in both directions, whole messages out of them. The browser
//! never compresses frames, so neither does this.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const TEXT: u8 = 1;
pub const CLOSE: u8 = 8;
pub const PING: u8 = 9;
pub const PONG: u8 = 10;

/// One frame, as written.
pub fn encode(opcode: u8, payload: &[u8], masked: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 14);
    out.push(0x80 | opcode);
    let mask_bit = if masked { 0x80 } else { 0 };
    match payload.len() {
        n if n < 126 => out.push(mask_bit | n as u8),
        n if n <= u16::MAX as usize => {
            out.push(mask_bit | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(mask_bit | 127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    if masked {
        let key = mask_key();
        out.extend_from_slice(&key);
        out.extend(payload.iter().enumerate().map(|(i, b)| b ^ key[i % 4]));
    } else {
        out.extend_from_slice(payload);
    }
    out
}

/// Clients mask what they send; the key needn't be secret, only varied.
fn mask_key() -> [u8; 4] {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(0x9e37_79b9);
    NEXT.fetch_add(0x6d2b_79f5, Ordering::Relaxed).to_le_bytes()
}

/// A whole message: a text or binary one put back together, or a control frame.
#[derive(Debug, PartialEq)]
pub struct Message {
    pub opcode: u8,
    pub payload: Vec<u8>,
}

/// Turns bytes, as they arrive, into messages.
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
    /// A message arriving in several frames: its opcode and what's come.
    partial: Option<(u8, Vec<u8>)>,
}

impl Decoder {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    pub fn next(&mut self) -> Option<Message> {
        loop {
            let (fin, opcode, payload) = self.frame()?;
            if opcode >= CLOSE {
                return Some(Message { opcode, payload });
            }
            let (kind, mut sofar) = match (opcode, self.partial.take()) {
                (0, Some(partial)) => partial,
                (0, None) => continue,
                (kind, _) => (kind, Vec::new()),
            };
            sofar.extend_from_slice(&payload);
            if fin {
                return Some(Message {
                    opcode: kind,
                    payload: sofar,
                });
            }
            self.partial = Some((kind, sofar));
        }
    }

    fn frame(&mut self) -> Option<(bool, u8, Vec<u8>)> {
        let b = &self.buf;
        if b.len() < 2 {
            return None;
        }
        let fin = b[0] & 0x80 != 0;
        let opcode = b[0] & 0x0f;
        let masked = b[1] & 0x80 != 0;
        let (len, mut at) = match b[1] & 0x7f {
            126 if b.len() >= 4 => (u16::from_be_bytes([b[2], b[3]]) as usize, 4),
            127 if b.len() >= 10 => (
                u64::from_be_bytes(b[2..10].try_into().expect("eight bytes")) as usize,
                10,
            ),
            126 | 127 => return None,
            n => (n as usize, 2),
        };
        let key = if masked {
            let key: [u8; 4] = b.get(at..at + 4)?.try_into().expect("four bytes");
            at += 4;
            Some(key)
        } else {
            None
        };
        if b.len() < at + len {
            return None;
        }
        let mut payload: Vec<u8> = self.buf.drain(..at + len).skip(at).collect();
        if let Some(key) = key {
            payload
                .iter_mut()
                .enumerate()
                .for_each(|(i, x)| *x ^= key[i % 4]);
        }
        Some((fin, opcode, payload))
    }
}

/// Asks to speak WebSocket at `path`. Returns whatever came after the
/// server's answer, which is already the start of the first frame.
pub async fn handshake<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    path: &str,
) -> std::io::Result<Vec<u8>> {
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: d2ItYWdlbnQtYnJvd3Nlcg==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await?;
    let (head, rest) = read_head(stream).await?;
    if !head.starts_with("HTTP/1.1 101") {
        let status = head.lines().next().unwrap_or_default().to_string();
        return Err(std::io::Error::other(format!(
            "the browser refused the connection: {status}"
        )));
    }
    Ok(rest)
}

/// An HTTP message's head, up to the blank line, and the bytes after it.
pub async fn read_head<S: AsyncRead + Unpin>(stream: &mut S) -> std::io::Result<(String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buf.split_off(end + 4);
            return Ok((String::from_utf8_lossy(&buf).into_owned(), rest));
        }
        if buf.len() > 64 * 1024 {
            return Err(std::io::Error::other("an HTTP head that never ends"));
        }
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_at_every_length() {
        for len in [0, 5, 125, 126, 300, 65_535, 65_536, 200_000] {
            let payload: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            for masked in [false, true] {
                let mut decoder = Decoder::default();
                let bytes = encode(TEXT, &payload, masked);
                // Arriving a few bytes at a time changes nothing.
                for chunk in bytes.chunks(7) {
                    decoder.push(chunk);
                }
                let message = decoder.next().expect("a whole message");
                assert_eq!(message.opcode, TEXT);
                assert_eq!(message.payload, payload);
                assert_eq!(decoder.next(), None);
            }
        }
    }

    #[test]
    fn fragments_make_one_message_around_a_ping() {
        let mut bytes = Vec::new();
        bytes.extend([0x01, 3]);
        bytes.extend(b"abc");
        bytes.extend(encode(PING, b"", false));
        bytes.extend([0x80, 2]);
        bytes.extend(b"de");
        let mut decoder = Decoder::default();
        decoder.push(&bytes);
        assert_eq!(decoder.next().unwrap().opcode, PING);
        assert_eq!(decoder.next().unwrap().payload, b"abcde");
        assert_eq!(decoder.next(), None);
    }
}
