//! Hand-rolled RFC 6455 WebSocket server codec. Text frames only from
//! clients in practice, but ping/pong and close are handled per spec. The
//! MT5700M WebUI protocol caps payloads well below 64 KiB; anything larger is
//! rejected to bound memory.

use crate::sha1::{base64, sha1};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub const OP_TEXT: u8 = 0x1;
pub const OP_PING: u8 = 0x9;
pub const OP_PONG: u8 = 0xA;
pub const OP_CLOSE: u8 = 0x8;
const MAX_PAYLOAD: usize = 64 * 1024;
const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

pub enum WsError {
    Io(std::io::Error),
    Protocol(&'static str),
    Closed,
}

impl From<std::io::Error> for WsError {
    fn from(e: std::io::Error) -> Self {
        WsError::Io(e)
    }
}

/// Read HTTP headers up to the blank line; returns (path, headers).
pub fn read_request(stream: &mut TcpStream) -> Result<(String, Vec<(String, String)>), WsError> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > 8 * 1024 {
            return Err(WsError::Protocol("request too large"));
        }
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Err(WsError::Closed);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let text = String::from_utf8_lossy(&buf);
    let mut lines = text.lines();
    let path = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .ok_or(WsError::Protocol("bad request line"))?
        .to_string();
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some(idx) = line.find(':') {
            headers.push((
                line[..idx].trim().to_lowercase(),
                line[idx + 1..].trim().to_string(),
            ));
        }
    }
    Ok((path, headers))
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

/// Perform the opening handshake; on success the stream is a WebSocket.
pub fn handshake(stream: &mut TcpStream) -> Result<(), WsError> {
    let (path, headers) = read_request(stream)?;
    let key = header(&headers, "sec-websocket-key")
        .ok_or(WsError::Protocol("missing Sec-WebSocket-Key"))?;
    if !header(&headers, "upgrade")
        .map(|v| v.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
    {
        return Err(WsError::Protocol("not a websocket upgrade"));
    }
    let mut accept = key.as_bytes().to_vec();
    accept.extend_from_slice(WS_GUID.as_bytes());
    let accept = base64(&sha1(&accept));
    let _ = path;
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
        accept
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()?;
    Ok(())
}

pub struct Frame {
    pub opcode: u8,
    pub payload: Vec<u8>,
}

fn read_exact_n(stream: &mut TcpStream, n: usize) -> Result<Vec<u8>, WsError> {
    let mut buf = vec![0u8; n];
    stream.read_exact(&mut buf)?;
    Ok(buf)
}

/// Read one frame. Unmasks client frames. Returns Err(Closed) on EOF,
/// Err(Protocol) on frames that exceed the size cap.
pub fn read_frame(stream: &mut TcpStream) -> Result<Frame, WsError> {
    let head = read_exact_n(stream, 2)?;
    let opcode = head[0] & 0x0F;
    let masked = head[1] & 0x80 != 0;
    let mut len = (head[1] & 0x7F) as usize;
    if len == 126 {
        let ext = read_exact_n(stream, 2)?;
        len = u16::from_be_bytes([ext[0], ext[1]]) as usize;
    } else if len == 127 {
        let ext = read_exact_n(stream, 8)?;
        let big = u64::from_be_bytes(ext.try_into().unwrap());
        if big > MAX_PAYLOAD as u64 {
            return Err(WsError::Protocol("frame too large"));
        }
        len = big as usize;
    }
    if len > MAX_PAYLOAD {
        return Err(WsError::Protocol("frame too large"));
    }
    let mask = if masked {
        Some(read_exact_n(stream, 4)?)
    } else {
        None
    };
    let mut payload = read_exact_n(stream, len)?;
    if let Some(mask) = mask {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mask[i % 4];
        }
    }
    Ok(Frame { opcode, payload })
}

/// Write an unmasked server frame.
pub fn write_frame(
    stream: &mut TcpStream,
    opcode: u8,
    payload: &[u8],
) -> Result<(), WsError> {
    let mut head = Vec::with_capacity(10);
    head.push(0x80 | opcode);
    if payload.len() < 126 {
        head.push(payload.len() as u8);
    } else if payload.len() <= 0xFFFF {
        head.push(126);
        head.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        head.push(127);
        head.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    let mut out = head;
    out.extend_from_slice(payload);
    stream.write_all(&out)?;
    stream.flush()?;
    Ok(())
}

pub fn set_write_timeout(stream: &TcpStream, secs: u64) {
    let _ = stream.set_write_timeout(Some(Duration::from_secs(secs)));
}

/// Strip any UTF-8 BOM and decode a text frame payload.
pub fn payload_to_string(payload: &[u8]) -> String {
    let s = String::from_utf8_lossy(payload);
    s.trim_start_matches('\u{FEFF}').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bom_strip() {
        let raw = [0xEF, 0xBB, 0xBF, b'A', b'T'];
        assert_eq!(payload_to_string(&raw), "AT");
    }
}
