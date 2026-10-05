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

use crate::core::json;

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

/// Read timeout for one control-socket round trip.
///
/// The daemon answers a `send` only after its own bounded budget:
/// `queued_timeout + timeout + 2s`, where `queued_timeout` defaults to
/// `timeout + 2`. A caller-visible `timeout` of 8 s therefore yields a
/// daemon-side budget of ~20 s. The previous hard-coded 20 s read timeout sat
/// exactly on that boundary: whenever the daemon actually used its full
/// budget, the CLI timed out at the same moment, classified the daemon as
/// `Unavailable`, and fell back to direct serial — which the daemon had
/// locked exclusively (TIOCEXCL), hanging the CLI forever.
///
/// The read timeout must be strictly greater than the daemon budget, and it
/// is derived from the requested timeout instead of being a fixed constant.
fn read_timeout(timeout: u64) -> std::time::Duration {
    // 2 * timeout + 10 s covers queued_timeout(timeout + 2) + timeout + 2
    // plus scheduling slack, and never lands exactly on the daemon boundary.
    std::time::Duration::from_secs(timeout.saturating_mul(2).saturating_add(10).max(20))
}

pub fn request(payload: &json::Value) -> Result<json::Value, ControlError> {
    let timeout = payload
        .get("timeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(5);
    let wire = format!("{}\n", payload.dump());
    let response_text = exchange(&wire, timeout).ok_or(ControlError::Unavailable)?;
    json::parse(response_text.trim())
        .ok_or_else(|| ControlError::BadResponse(response_text.to_string()))
}

#[cfg(unix)]
fn exchange(wire: &str, timeout: u64) -> Option<String> {
    use std::io::{BufRead, Write};
    use std::os::unix::net::UnixStream;
    let mut conn = UnixStream::connect(CONTROL_SOCKET).ok()?;
    let _ = conn.set_read_timeout(Some(read_timeout(timeout)));
    let _ = conn.set_write_timeout(Some(std::time::Duration::from_secs(5)));
    conn.write_all(wire.as_bytes()).ok()?;
    let mut rdr = std::io::BufReader::new(&conn);
    let mut line = String::new();
    if rdr.read_line(&mut line).ok()? == 0 {
        return None;
    }
    Some(line)
}

#[cfg(not(unix))]
fn exchange(_wire: &str, _timeout: u64) -> Option<String> {
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

/// Call a unified API route on the daemon and return its domain JSON.
///
/// Used by the CLI so the shell-facing verbs run the *same* module code as
/// LuCI and the WebUI instead of a second implementation.
pub fn daemon_api(method: &str, params: &json::Value, timeout: u64) -> Result<json::Value, ControlError> {
    let mut map = std::collections::BTreeMap::new();
    map.insert("cmd".to_string(), json::str_val("api"));
    map.insert("path".to_string(), json::str_val(method));
    map.insert("params".to_string(), params.clone());
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
    Ok(resp.get("result").cloned().unwrap_or(json::Value::Null))
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

/// Request the daemon's cached state snapshot (no AT traffic at all).
/// The snapshot is produced by the background collectors; the LuCI status
/// page can render it immediately instead of waiting for the modem.
pub fn daemon_cached() -> Result<json::Value, ControlError> {
    let mut map = std::collections::BTreeMap::new();
    map.insert("cmd".to_string(), json::str_val("cached"));
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
    resp.get("snapshot")
        .cloned()
        .ok_or_else(|| ControlError::BadResponse("missing snapshot field".into()))
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