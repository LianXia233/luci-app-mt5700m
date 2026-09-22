//! Local control socket between the LuCI shell backend (`mt5700m-at`) and the
//! AT daemon (`at-webserver`).
//!
//! With `ubus-at-daemon` removed, the daemon is the *sole* owner of the AT
//! serial port. The LuCI CLI (`mt5700m-at`) no longer opens /dev/ttyUSB*
//! itself; it talks to the daemon through this Unix socket, which serialises
//! access behind the daemon's exclusive lock. This is the functional
//! replacement for the old `ubus call at-daemon sendat` shared channel.
//!
//! Protocol (newline-delimited JSON, one request → one response):
//!
//! ```json
//! {"cmd":"send","command":"AT+CSQ","timeout":2}
//! {"cmd":"sms","number":"+86...","text":"hello"}
//! {"cmd":"scan"}
//! ```
//!
//! Response: `{"ok":true,"response":"..."}` (or `"ports":[...]` for scan) and
//! `{"ok":false,"error":"..."}` on failure.

use crate::json;

/// Default control-socket path (OpenWrt: /var/run → /tmp).
#[cfg_attr(not(unix), allow(dead_code))]
pub const CONTROL_SOCKET: &str = "/var/run/at-webserver.sock";

/// Send a request to the daemon control socket and read the JSON response.
///
/// `Err(ControlError::Unavailable)` when the daemon is not listening: callers
/// then fall back to direct serial (one-shot) so the CLI still works if the
/// daemon is down.
#[derive(Debug)]
pub enum ControlError {
    Unavailable,
    BadResponse(String),
}

impl ControlError {
    pub fn message(&self) -> String {
        match self {
            ControlError::Unavailable => "AT daemon control socket not available".into(),
            ControlError::BadResponse(e) => format!("bad daemon response: {}", e),
        }
    }
}

pub fn request(payload: &json::Value) -> Result<json::Value, ControlError> {
    let wire = format!("{}\n", payload.dump());
    let response_text = exchange(&wire).ok_or(ControlError::Unavailable)?;
    json::parse(response_text.trim())
        .ok_or_else(|| ControlError::BadResponse(response_text.to_string()))
}

#[cfg(unix)]
fn exchange(wire: &str) -> Option<String> {
    use std::io::{BufRead, Write};
    use std::os::unix::net::UnixStream;
    let mut conn = UnixStream::connect(CONTROL_SOCKET).ok()?;
    let _ = conn.set_read_timeout(Some(std::time::Duration::from_secs(20)));
    conn.write_all(wire.as_bytes()).ok()?;
    let mut rdr = std::io::BufReader::new(&conn);
    let mut line = String::new();
    if rdr.read_line(&mut line).ok()? == 0 {
        return None;
    }
    Some(line)
}

#[cfg(not(unix))]
fn exchange(_wire: &str) -> Option<String> {
    None
}

/// Helper to build a `send` request and interpret the response.
pub fn daemon_send(command: &str, timeout: u64) -> Result<String, ControlError> {
    let mut map = std::collections::BTreeMap::new();
    map.insert("cmd".to_string(), json::str_val("send"));
    map.insert("command".to_string(), json::str_val(command));
    map.insert("timeout".to_string(), json::num_val(timeout));
    let resp = request(&json::Value::Obj(map))?;
    let ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    if !ok {
        let err = resp
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error")
            .to_string();
        return Err(ControlError::BadResponse(err));
    }
    resp.get("response")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ControlError::BadResponse("missing response field".into()))
}

/// Request the daemon to enable PDU-mode SMS sending.
pub fn daemon_sms(number: &str, text: &str) -> Result<String, ControlError> {
    let mut map = std::collections::BTreeMap::new();
    map.insert("cmd".to_string(), json::str_val("sms"));
    map.insert("number".to_string(), json::str_val(number));
    map.insert("text".to_string(), json::str_val(text));
    let resp = request(&json::Value::Obj(map))?;
    let ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    if !ok {
        let err = resp
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown error")
            .to_string();
        return Err(ControlError::BadResponse(err));
    }
    resp.get("response")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| ControlError::BadResponse("missing response field".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_unavailable_without_daemon() {
        // No daemon is listening during tests, so this must produce
        // Unavailable rather than panicking.
        let mut map = std::collections::BTreeMap::new();
        map.insert("cmd".to_string(), json::str_val("send"));
        map.insert("command".to_string(), json::str_val("AT"));
        assert!(matches!(
            request(&json::Value::Obj(map)),
            Err(ControlError::Unavailable)
        ));
    }

    #[test]
    fn json_roundtrip_helpers() {
        let mut map = std::collections::BTreeMap::new();
        map.insert("hi".to_string(), json::str_val("世界"));
        let v = json::Value::Obj(map);
        let parsed = json::parse(&v.dump()).expect("round trip");
        assert_eq!(parsed.get("hi").and_then(|x| x.as_str()), Some("世界"));
    }
}