//! DNS apply state and live interface verification.
//!
//! Three different things must never be conflated:
//!
//! * **saved** — what is in UCI (`DnsConfig::load`);
//! * **applied** — what the last `dns.sh apply` wrote to `network.*`
//!   (`/var/run/mt5700m/dns.state`, written atomically by the script);
//! * **verified** — what the running network actually uses, read back from
//!   `ubus call network.interface.<iface> status`.
//!
//! The state file is runtime-only on purpose: after a reboot nothing has been
//! applied yet, and reporting the previous boot's success as current would be
//! a lie. The saved config is the only thing that persists.

use crate::core::json::{self, Value};
use crate::core::sys;
use crate::modules::dns::STATE_FILE;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct ApplyState {
    /// `never` | `ok` | `partial` | `error`
    pub result: String,
    pub applied_at: Option<u64>,
    pub error: Option<String>,
    pub v4_enabled: bool,
    pub v4_servers: Vec<String>,
    pub v6_enabled: bool,
    pub v6_servers: Vec<String>,
    /// IPv6 interface was absent at apply time; IPv4 still succeeded.
    pub v6_skipped: bool,
}

impl Default for ApplyState {
    fn default() -> Self {
        ApplyState {
            result: "never".to_string(),
            applied_at: None,
            error: None,
            v4_enabled: false,
            v4_servers: Vec::new(),
            v6_enabled: false,
            v6_servers: Vec::new(),
            v6_skipped: false,
        }
    }
}

impl ApplyState {
    pub fn load() -> Self {
        match sys::read_file(STATE_FILE) {
            Some(text) => parse(&text),
            None => ApplyState::default(),
        }
    }

    pub fn to_json(&self) -> Value {
        let mut m = BTreeMap::new();
        m.insert("result".to_string(), json::str_val(&self.result));
        m.insert(
            "appliedAt".to_string(),
            match self.applied_at {
                Some(ts) => json::num_val(ts),
                None => Value::Null,
            },
        );
        if let Some(ts) = self.applied_at {
            m.insert(
                "appliedAtText".to_string(),
                json::str_val(&crate::core::time::format_unix_local(ts)),
            );
        }
        m.insert(
            "error".to_string(),
            match &self.error {
                Some(e) => json::str_val(e),
                None => Value::Null,
            },
        );
        m.insert("v6Skipped".to_string(), Value::Bool(self.v6_skipped));
        m.insert(
            "applied".to_string(),
            servers_obj(&self.v4_servers, &self.v6_servers, self.v4_enabled, self.v6_enabled),
        );
        Value::Obj(m)
    }
}

/// Parse the shell-written state file. Unknown keys are ignored so the script
/// can add diagnostics without breaking older daemons.
pub fn parse(text: &str) -> ApplyState {
    let mut st = ApplyState::default();
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
            "result" => {
                if matches!(value, "ok" | "partial" | "error" | "never") {
                    st.result = value.to_string();
                }
            }
            "applied_at" => st.applied_at = value.parse::<u64>().ok().filter(|v| *v > 0),
            "error" => {
                let v = value.trim();
                st.error = (!v.is_empty()).then(|| v.to_string());
            }
            "v4_enabled" => st.v4_enabled = value == "1",
            "v4_servers" => st.v4_servers = split_list(value),
            "v6_enabled" => st.v6_enabled = value == "1",
            "v6_servers" => st.v6_servers = split_list(value),
            "v6_skipped" => st.v6_skipped = value == "1",
            _ => {}
        }
    }
    st
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|v| !v.is_empty())
        .map(String::from)
        .collect()
}

fn servers_obj(v4: &[String], v6: &[String], v4e: bool, v6e: bool) -> Value {
    let mut m = BTreeMap::new();
    m.insert(
        "ipv4".to_string(),
        Value::Arr(v4.iter().map(|s| json::str_val(s)).collect()),
    );
    m.insert(
        "ipv6".to_string(),
        Value::Arr(v6.iter().map(|s| json::str_val(s)).collect()),
    );
    m.insert("ipv4Enabled".to_string(), Value::Bool(v4e));
    m.insert("ipv6Enabled".to_string(), Value::Bool(v6e));
    Value::Obj(m)
}

/// Live state of one managed interface, from netifd.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InterfaceRuntime {
    pub name: String,
    /// The UCI section exists (the interface is configured).
    pub configured: bool,
    /// netifd reports the interface up.
    pub up: bool,
    /// Resolvers netifd actually handed to the resolver for this interface.
    pub dns: Vec<String>,
    pub l3_device: Option<String>,
}

/// Read one interface's runtime DNS list.
///
/// `ubus call network.interface.X status` is authoritative while the
/// interface is up. When it is not, the saved UCI list is reported separately
/// by the caller — this function deliberately returns an empty runtime list
/// instead of pretending the saved config is live.
pub fn interface_runtime(iface: &str) -> InterfaceRuntime {
    let configured = sys::uci_get(&format!("network.{}.proto", iface)).is_some()
        || sys::uci_get(&format!("network.{}", iface)).is_some();
    let mut rt = InterfaceRuntime {
        name: iface.to_string(),
        configured,
        ..Default::default()
    };
    if !configured {
        return rt;
    }
    let out = sys::run(
        "ubus",
        &["call", &format!("network.interface.{}", iface), "status"],
        std::time::Duration::from_secs(3),
    );
    if out.ok {
        if let Some(Value::Obj(m)) = json::parse(&out.stdout) {
            rt.up = m.get("up").and_then(|v| v.as_bool()).unwrap_or(false);
            rt.l3_device = m
                .get("l3_device")
                .or_else(|| m.get("device"))
                .and_then(|v| v.as_str())
                .map(str::to_string);
            if let Some(Value::Arr(items)) = m.get("dns-server") {
                rt.dns = items
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(str::to_string)
                    .collect();
            }
        }
    }
    rt
}

/// True when every expected server is present in the runtime list (order is
/// not significant, extra resolvers are a mismatch only when strict).
pub fn runtime_matches(expected: &[String], actual: &[String], strict: bool) -> bool {
    if expected.is_empty() {
        return actual.is_empty();
    }
    let all_present = expected.iter().all(|e| actual.iter().any(|a| a == e));
    if strict {
        all_present && actual.len() == expected.len()
    } else {
        all_present
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_state_is_never_applied() {
        let st = ApplyState::default();
        assert_eq!(st.result, "never");
        assert!(st.applied_at.is_none());
    }

    #[test]
    fn state_file_round_trip() {
        let text = r#"result=ok
applied_at=1730000000
v4_enabled=1
v4_servers=119.29.29.29 223.5.5.5
v6_enabled=1
v6_servers=2402:4e00:: 2400:3200::1
v6_skipped=0
error=
"#;
        let st = parse(text);
        assert_eq!(st.result, "ok");
        assert_eq!(st.applied_at, Some(1730000000));
        assert_eq!(st.v4_servers, vec!["119.29.29.29", "223.5.5.5"]);
        assert_eq!(st.v6_servers.len(), 2);
        assert!(st.error.is_none());
    }

    #[test]
    fn partial_and_error_keep_the_reason() {
        let st = parse("result=partial\nv4_enabled=1\nv4_servers=1.1.1.1\nerror=ipv6 iface missing\n");
        assert_eq!(st.result, "partial");
        assert_eq!(st.error.as_deref(), Some("ipv6 iface missing"));
        assert!(st.v6_servers.is_empty());
    }

    #[test]
    fn unknown_keys_and_garbage_are_ignored() {
        let st = parse("garbage\nresult=bogus\nx=1\napplied_at=notanumber\n");
        assert_eq!(st.result, "never");
        assert!(st.applied_at.is_none());
    }

    #[test]
    fn runtime_match_handles_order_and_extra_servers() {
        let expected = vec!["1.1.1.1".to_string(), "8.8.8.8".to_string()];
        let reversed = vec!["8.8.8.8".to_string(), "1.1.1.1".to_string()];
        assert!(runtime_matches(&expected, &reversed, false));
        assert!(runtime_matches(&expected, &reversed, true));
        let with_extra = vec!["1.1.1.1".to_string(), "8.8.8.8".to_string(), "9.9.9.9".to_string()];
        assert!(runtime_matches(&expected, &with_extra, false));
        assert!(!runtime_matches(&expected, &with_extra, true));
        assert!(!runtime_matches(&expected, &["1.1.1.1".to_string()], false));
        assert!(runtime_matches(&[], &[], true));
        assert!(!runtime_matches(&[], &["1.1.1.1".to_string()], true));
    }
}
