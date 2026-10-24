//! Runtime NTP state (`/var/run/mt5700m/ntp.state`).
//!
//! The saved configuration lives in UCI; this file records what actually
//! happened at runtime: the last apply, the last observation of the system
//! daemon, the last manual sync attempt and its outcome. It is runtime-only on
//! purpose — after a reboot the clock has not been observed yet and the page
//! must say "unknown", not repeat the previous boot's success.
//!
//! Three facts stay separate, exactly like DNS:
//!
//! * **saved** — UCI (`NtpConfig`);
//! * **applied** — the provider config was written and restarted (`apply`);
//! * **trusted** — the system daemon positively reports a synchronised clock
//!   and that observation is fresh (`synced == Some(true)` + `observed_at`).
//!
//! `synced` is never invented: only positive evidence sets `Some(true)`.

use crate::core::json::{self, Value};
use crate::core::sys;
use crate::modules::timesync::STATE_FILE;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// An observation older than this is no longer presented as current.
pub const STALE_AFTER_SECS: u64 = 180;

static STATE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq)]
pub struct ApplyRecord {
    /// `never` | `ok` | `error`
    pub result: String,
    pub at: Option<u64>,
    pub error: Option<String>,
    pub provider: Option<String>,
    pub servers: Vec<String>,
    /// Consecutive automatic-apply failures (drives the retry backoff).
    pub failures: u32,
    pub retry_at: Option<u64>,
}

impl Default for ApplyRecord {
    fn default() -> Self {
        ApplyRecord {
            result: "never".to_string(),
            at: None,
            error: None,
            provider: None,
            servers: Vec::new(),
            failures: 0,
            retry_at: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct NtpState {
    pub apply: ApplyRecord,
    /// `Some(true)` only with positive evidence from the system daemon.
    pub synced: Option<bool>,
    /// When `synced` was observed (Unix seconds).
    pub observed_at: Option<u64>,
    /// `system` | `manual`
    pub source: Option<String>,
    pub server: Option<String>,
    pub offset_ms: Option<i64>,
    pub last_attempt_at: Option<u64>,
    pub last_success_at: Option<u64>,
    pub last_error: Option<String>,
    /// `dns` | `network` | `timeout` | `server` | `clock` | `config` | `internal`
    pub last_error_kind: Option<String>,
    /// `synced` | `unverified` | `failed`
    pub last_sync_result: Option<String>,
    pub next_retry_at: Option<u64>,
    pub failures: u32,
    pub detail: Option<String>,
}

impl Default for NtpState {
    fn default() -> Self {
        NtpState {
            apply: ApplyRecord::default(),
            synced: None,
            observed_at: None,
            source: None,
            server: None,
            offset_ms: None,
            last_attempt_at: None,
            last_success_at: None,
            last_error: None,
            last_error_kind: None,
            last_sync_result: None,
            next_retry_at: None,
            failures: 0,
            detail: None,
        }
    }
}

impl NtpState {
    pub fn load() -> Self {
        match sys::read_file(STATE_FILE) {
            Some(text) => parse(&text),
            None => NtpState::default(),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        sys::write_file_atomic(STATE_FILE, &self.render())
    }

    /// Load, mutate and persist under one lock. The state file is a handful
    /// of fields; several task threads may finish observations at once, and a
    /// lost update would make the page report a stale error forever.
    pub fn update<F: FnOnce(&mut NtpState)>(f: F) -> Result<(), String> {
        let _guard = STATE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut st = NtpState::load();
        f(&mut st);
        st.save()
    }

    fn render(&self) -> String {
        let mut out = String::new();
        let mut push = |key: &str, value: &str| {
            // One key per line; values must never smuggle in a newline.
            let value = value.replace(['\n', '\r'], " ");
            out.push_str(key);
            out.push('=');
            out.push_str(&value);
            out.push('\n');
        };
        push("apply_result", &self.apply.result);
        push("apply_at", &self.apply.at.map(|v| v.to_string()).unwrap_or_default());
        push("apply_error", self.apply.error.as_deref().unwrap_or(""));
        push("apply_provider", self.apply.provider.as_deref().unwrap_or(""));
        push(
            "apply_servers",
            &self.apply.servers.join(" "),
        );
        push("apply_failures", &self.apply.failures.to_string());
        push(
            "apply_retry_at",
            &self.apply.retry_at.map(|v| v.to_string()).unwrap_or_default(),
        );
        push(
            "synced",
            match self.synced {
                Some(true) => "1",
                Some(false) => "0",
                None => "unknown",
            },
        );
        push(
            "observed_at",
            &self.observed_at.map(|v| v.to_string()).unwrap_or_default(),
        );
        push("source", self.source.as_deref().unwrap_or(""));
        push("server", self.server.as_deref().unwrap_or(""));
        push(
            "offset_ms",
            &self.offset_ms.map(|v| v.to_string()).unwrap_or_default(),
        );
        push(
            "last_attempt_at",
            &self
                .last_attempt_at
                .map(|v| v.to_string())
                .unwrap_or_default(),
        );
        push(
            "last_success_at",
            &self
                .last_success_at
                .map(|v| v.to_string())
                .unwrap_or_default(),
        );
        push("last_error", self.last_error.as_deref().unwrap_or(""));
        push(
            "last_error_kind",
            self.last_error_kind.as_deref().unwrap_or(""),
        );
        push(
            "last_sync_result",
            self.last_sync_result.as_deref().unwrap_or(""),
        );
        push(
            "next_retry_at",
            &self.next_retry_at.map(|v| v.to_string()).unwrap_or_default(),
        );
        push("failures", &self.failures.to_string());
        push("detail", self.detail.as_deref().unwrap_or(""));
        out
    }

    /// Fresh + positively synchronised. Only this state may label time
    /// "trusted"; `None`/stale always stays untrusted.
    pub fn trusted_at(&self, now: u64) -> bool {
        self.synced == Some(true)
            && self
                .observed_at
                .map(|at| now.saturating_sub(at) <= STALE_AFTER_SECS)
                .unwrap_or(false)
            && crate::core::time::clock_plausible()
    }

    pub fn to_json_at(&self, now: u64) -> Value {
        let stale = self
            .observed_at
            .map(|at| now.saturating_sub(at) > STALE_AFTER_SECS)
            .unwrap_or(true);
        let mut apply = BTreeMap::new();
        apply.insert("result".to_string(), json::str_val(&self.apply.result));
        apply.insert(
            "at".to_string(),
            self.apply.at.map(json::num_val).unwrap_or(Value::Null),
        );
        if let Some(at) = self.apply.at {
            apply.insert(
                "atText".to_string(),
                json::str_val(&crate::core::time::format_unix_local(at)),
            );
        }
        apply.insert(
            "error".to_string(),
            self.apply.error.as_deref().map(json::str_val).unwrap_or(Value::Null),
        );
        apply.insert(
            "provider".to_string(),
            self.apply
                .provider
                .as_deref()
                .map(json::str_val)
                .unwrap_or(Value::Null),
        );
        apply.insert(
            "servers".to_string(),
            Value::Arr(self.apply.servers.iter().map(|s| json::str_val(s)).collect()),
        );
        apply.insert(
            "failures".to_string(),
            json::num_val(self.apply.failures as i64),
        );
        apply.insert(
            "nextRetryAt".to_string(),
            self.apply.retry_at.map(json::num_val).unwrap_or(Value::Null),
        );

        let mut m = BTreeMap::new();
        m.insert("apply".to_string(), Value::Obj(apply));
        m.insert(
            "synced".to_string(),
            self.synced.map(Value::Bool).unwrap_or(Value::Null),
        );
        m.insert("trusted".to_string(), Value::Bool(self.trusted_at(now)));
        m.insert("stale".to_string(), Value::Bool(stale));
        m.insert(
            "observedAt".to_string(),
            self.observed_at.map(json::num_val).unwrap_or(Value::Null),
        );
        if let Some(at) = self.observed_at {
            m.insert(
                "observedAtText".to_string(),
                json::str_val(&crate::core::time::format_unix_local(at)),
            );
        }
        m.insert(
            "source".to_string(),
            self.source.as_deref().map(json::str_val).unwrap_or(Value::Null),
        );
        m.insert(
            "server".to_string(),
            self.server.as_deref().map(json::str_val).unwrap_or(Value::Null),
        );
        m.insert(
            "offsetMs".to_string(),
            self.offset_ms.map(json::num_val).unwrap_or(Value::Null),
        );
        m.insert(
            "lastAttemptAt".to_string(),
            self.last_attempt_at.map(json::num_val).unwrap_or(Value::Null),
        );
        if let Some(at) = self.last_attempt_at {
            m.insert(
                "lastAttemptAtText".to_string(),
                json::str_val(&crate::core::time::format_unix_local(at)),
            );
        }
        m.insert(
            "lastSuccessAt".to_string(),
            self.last_success_at.map(json::num_val).unwrap_or(Value::Null),
        );
        if let Some(at) = self.last_success_at {
            m.insert(
                "lastSuccessAtText".to_string(),
                json::str_val(&crate::core::time::format_unix_local(at)),
            );
        }
        m.insert(
            "lastError".to_string(),
            self.last_error.as_deref().map(json::str_val).unwrap_or(Value::Null),
        );
        m.insert(
            "lastErrorKind".to_string(),
            self.last_error_kind
                .as_deref()
                .map(json::str_val)
                .unwrap_or(Value::Null),
        );
        m.insert(
            "lastSyncResult".to_string(),
            self.last_sync_result
                .as_deref()
                .map(json::str_val)
                .unwrap_or(Value::Null),
        );
        m.insert(
            "nextRetryAt".to_string(),
            self.next_retry_at.map(json::num_val).unwrap_or(Value::Null),
        );
        m.insert("failures".to_string(), json::num_val(self.failures as i64));
        m.insert(
            "detail".to_string(),
            self.detail.as_deref().map(json::str_val).unwrap_or(Value::Null),
        );
        Value::Obj(m)
    }
}

/// Parse the key=value state file. Unknown keys are ignored so the daemon can
/// add diagnostics later without breaking an older parser.
pub fn parse(text: &str) -> NtpState {
    let mut st = NtpState::default();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "apply_result" => {
                if matches!(value, "never" | "ok" | "error" | "partial") {
                    st.apply.result = value.to_string();
                }
            }
            "apply_at" => st.apply.at = non_zero(value),
            "apply_error" => st.apply.error = non_empty(value),
            "apply_provider" => st.apply.provider = non_empty(value),
            "apply_servers" => st.apply.servers = split_list(value),
            "apply_failures" => st.apply.failures = value.parse::<u32>().unwrap_or(0),
            "apply_retry_at" => st.apply.retry_at = non_zero(value),
            "synced" => {
                st.synced = match value {
                    "1" | "true" | "yes" => Some(true),
                    "0" | "false" | "no" => Some(false),
                    _ => None,
                }
            }
            "observed_at" => st.observed_at = non_zero(value),
            "source" => {
                st.source = non_empty(value).filter(|v| matches!(v.as_str(), "system" | "manual"))
            }
            "server" => st.server = non_empty(value),
            "offset_ms" => st.offset_ms = value.parse::<i64>().ok(),
            "last_attempt_at" => st.last_attempt_at = non_zero(value),
            "last_success_at" => st.last_success_at = non_zero(value),
            "last_error" => st.last_error = non_empty(value),
            "last_error_kind" => {
                st.last_error_kind = non_empty(value).filter(|v| {
                    matches!(
                        v.as_str(),
                        "dns" | "network" | "timeout" | "server" | "clock" | "config" | "internal"
                    )
                })
            }
            "last_sync_result" => {
                st.last_sync_result = non_empty(value)
                    .filter(|v| matches!(v.as_str(), "synced" | "unverified" | "failed"))
            }
            "next_retry_at" => st.next_retry_at = non_zero(value),
            "failures" => st.failures = value.parse::<u32>().unwrap_or(0),
            "detail" => st.detail = non_empty(value),
            _ => {}
        }
    }
    st
}

fn non_zero(value: &str) -> Option<u64> {
    value.parse::<u64>().ok().filter(|v| *v > 0)
}

fn non_empty(value: &str) -> Option<String> {
    let v = value.trim();
    (!v.is_empty()).then(|| v.to_string())
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|v| !v.is_empty())
        .map(String::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_never_applied_and_untrusted() {
        let st = NtpState::default();
        assert_eq!(st.apply.result, "never");
        assert!(!st.trusted_at(1_700_000_000));
    }

    #[test]
    fn state_round_trip_preserves_every_field() {
        let text = r#"apply_result=ok
apply_at=1730000000
apply_error=
apply_provider=sysntpd
apply_servers=ntp.tencent.com time.cloudflare.com
synced=1
observed_at=1730000100
source=manual
server=ntp.tencent.com
offset_ms=-12
last_attempt_at=1730000100
last_success_at=1730000100
last_error=
last_error_kind=
last_sync_result=synced
next_retry_at=
failures=0
detail=
"#;
        let st = parse(text);
        assert_eq!(st.apply.result, "ok");
        assert_eq!(st.apply.at, Some(1730000000));
        assert_eq!(st.apply.provider.as_deref(), Some("sysntpd"));
        assert_eq!(st.apply.servers.len(), 2);
        assert_eq!(st.synced, Some(true));
        assert_eq!(st.offset_ms, Some(-12));
        assert_eq!(st.last_sync_result.as_deref(), Some("synced"));
        assert!(st.trusted_at(1730000150));
        assert!(!st.trusted_at(1730000100 + STALE_AFTER_SECS + 1));
    }

    #[test]
    fn unknown_is_not_false_and_not_trusted() {
        let st = parse("synced=unknown\nobserved_at=1730000100\n");
        assert_eq!(st.synced, None);
        assert!(!st.trusted_at(1730000100));
        let json = st.to_json_at(1730000100);
        assert_eq!(json.get("synced").unwrap(), &Value::Null);
        assert_eq!(json.get("trusted").unwrap(), &Value::Bool(false));
    }

    #[test]
    fn false_sync_is_reported_as_false_but_untrusted() {
        let st = parse("synced=0\nobserved_at=1730000100\n");
        assert_eq!(st.synced, Some(false));
        assert_eq!(st.to_json_at(1730000100).get("synced").unwrap(), &Value::Bool(false));
        assert!(!st.trusted_at(1730000100));
    }

    #[test]
    fn failed_attempt_keeps_kind_and_backoff() {
        let st = parse(
            "last_error_kind=dns\nlast_error=cannot resolve ntp.aliyun.com\nfailures=2\nnext_retry_at=1730000200\nlast_sync_result=failed\n",
        );
        assert_eq!(st.last_error_kind.as_deref(), Some("dns"));
        assert_eq!(st.failures, 2);
        assert_eq!(st.next_retry_at, Some(1730000200));
        assert_eq!(st.last_sync_result.as_deref(), Some("failed"));
    }

    #[test]
    fn garbage_values_are_ignored_not_trusted() {
        let st = parse(
            "apply_result=bogus\nsynced=maybe\nobservations_at=1\nlast_error_kind=weird\nfailures=NaN\n",
        );
        assert_eq!(st.apply.result, "never");
        assert_eq!(st.synced, None);
        assert_eq!(st.last_error_kind, None);
        assert_eq!(st.failures, 0);
    }

    #[test]
    fn render_sanitises_newlines_in_error_text() {
        let mut st = NtpState::default();
        st.last_error = Some("line1\nline2".to_string());
        st.apply.result = "error".to_string();
        let text = st.render();
        assert!(!text.contains("line1\nline2"));
        assert!(text.contains("line1 line2"));
        // And the parser reads it back as one value.
        let parsed = parse(&text);
        assert_eq!(parsed.last_error.as_deref(), Some("line1 line2"));
    }
}
