//! NTP server configuration (`mt5700m.ntp`).
//!
//! Order is priority: the system daemon gets the enabled hosts in exactly the
//! order the user arranged. Disabled hosts stay in the list so re-enabling
//! them restores their position.
//!
//! On the first successful apply the previous system NTP servers are backed
//! up in UCI. Turning the plugin's NTP switch off restores that backup instead
//! of leaving the device without a time source.

use crate::core::json::{self, Value};
use crate::core::sys;
use crate::modules::timesync::{DEFAULT_SERVERS, MAX_SERVERS};
use std::collections::BTreeMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

pub const PACKAGE: &str = "mt5700m";
pub const SECTION: &str = "ntp";

#[derive(Debug, Clone, PartialEq)]
pub struct NtpServer {
    pub host: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NtpConfig {
    pub enabled: bool,
    pub servers: Vec<NtpServer>,
}

impl Default for NtpConfig {
    fn default() -> Self {
        NtpConfig {
            enabled: true,
            servers: DEFAULT_SERVERS
                .iter()
                .map(|h| NtpServer {
                    host: h.to_string(),
                    enabled: true,
                })
                .collect(),
        }
    }
}

fn key(option: &str) -> String {
    format!("{}.{}.{}", PACKAGE, SECTION, option)
}

fn section_values() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(text) = sys::uci_show(&format!("{}.{}", PACKAGE, SECTION)) else {
        return out;
    };
    let prefix = format!("{}.{}.", PACKAGE, SECTION);
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let Some(name) = k.trim().strip_prefix(&prefix) else {
            continue;
        };
        let mut value = v.trim().to_string();
        if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
            value = value[1..value.len() - 1].replace("'\\''", "'");
        }
        out.insert(name.to_string(), value);
    }
    out
}

impl NtpConfig {
    pub fn load() -> Self {
        let values = section_values();
        let enabled = match values.get("enabled").map(|v| v.as_str()) {
            Some("0") | Some("false") | Some("off") | Some("no") => false,
            Some(_) => true,
            None => true,
        };
        let hosts = sys::uci_get_list(&key("server"));
        let disabled: Vec<String> = sys::uci_get_list(&key("disabled"))
            .into_iter()
            .map(|h| h.to_ascii_lowercase())
            .collect();
        // A missing section (fresh install) shows the documented defaults. An
        // existing section that was deliberately saved with an empty list must
        // stay empty; UCI cannot distinguish that from "no list option", so
        // `servers_set` records that the user saved an explicit list.
        let configured = values.get("servers_set").map(|v| v == "1").unwrap_or(false);
        let servers = if !configured && hosts.is_empty() {
            NtpConfig::default().servers
        } else {
            hosts
                .into_iter()
                .map(|host| NtpServer {
                    enabled: !disabled.iter().any(|d| d.eq_ignore_ascii_case(&host)),
                    host,
                })
                .collect()
        };
        NtpConfig { enabled, servers }
    }

    /// Replace the server list/order from the page's JSON.
    ///
    /// Accepted shape:
    /// `{enabled, servers:[{host,enabled}...]}`. Omitted `servers` keeps the
    /// current list; an empty list is allowed and validated below.
    pub fn merge_json(&mut self, params: &Value) -> Result<(), String> {
        if let Some(v) = params.get("enabled") {
            self.enabled = v
                .as_bool()
                .ok_or_else(|| "NTP 启用状态必须是布尔值".to_string())?;
        }
        if let Some(v) = params.get("servers") {
            let arr = v
                .as_arr()
                .ok_or_else(|| "NTP 服务器列表必须是数组".to_string())?;
            let mut servers = Vec::with_capacity(arr.len());
            for item in arr {
                let host = item
                    .get("host")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "NTP 服务器缺少 host".to_string())?
                    .trim()
                    .to_string();
                let enabled = match item.get("enabled") {
                    Some(v) => v
                        .as_bool()
                        .ok_or_else(|| format!("{} 的启用状态必须是布尔值", host))?,
                    None => true,
                };
                servers.push(NtpServer { host, enabled });
            }
            self.servers = servers;
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.servers.len() > MAX_SERVERS {
            return Err(format!("NTP 服务器最多 {} 个", MAX_SERVERS));
        }
        let mut seen: Vec<String> = Vec::new();
        for server in &self.servers {
            validate_host(&server.host)?;
            let lower = server.host.to_ascii_lowercase();
            if seen.contains(&lower) {
                return Err(format!("NTP 服务器重复: {}", server.host));
            }
            seen.push(lower);
        }
        if self.enabled && self.effective_servers().is_empty() {
            return Err("NTP 已启用，但至少需要一个启用的服务器".to_string());
        }
        Ok(())
    }

    /// Enabled hosts in priority order.
    pub fn effective_servers(&self) -> Vec<String> {
        self.servers
            .iter()
            .filter(|s| s.enabled)
            .map(|s| s.host.trim().to_string())
            .collect()
    }

    pub fn save(&self) -> Result<(), String> {
        if !sys::uci_available() {
            return Err("UCI 不可用，无法保存 NTP 配置".to_string());
        }
        self.validate()?;
        sys::uci_set(&format!("{}.{}", PACKAGE, SECTION), SECTION)?;
        sys::uci_set(
            &key("enabled"),
            if self.enabled { "1" } else { "0" },
        )?;
        let hosts: Vec<String> = self.servers.iter().map(|s| s.host.clone()).collect();
        let disabled: Vec<String> = self
            .servers
            .iter()
            .filter(|s| !s.enabled)
            .map(|s| s.host.clone())
            .collect();
        sys::uci_set_list(&key("server"), &hosts)?;
        sys::uci_set_list(&key("disabled"), &disabled)?;
        sys::uci_set(&key("servers_set"), "1")?;
        sys::uci_commit(PACKAGE)
    }

    pub fn to_json(&self) -> Value {
        let mut m = BTreeMap::new();
        m.insert("enabled".to_string(), Value::Bool(self.enabled));
        m.insert(
            "servers".to_string(),
            Value::Arr(self.servers.iter().map(server_json).collect()),
        );
        m.insert(
            "effective".to_string(),
            Value::Arr(
                self.effective_servers()
                    .iter()
                    .map(|h| json::str_val(h))
                    .collect(),
            ),
        );
        Value::Obj(m)
    }

    pub fn defaults_json() -> Value {
        NtpConfig::default().to_json()
    }
}

fn server_json(s: &NtpServer) -> Value {
    let mut m = BTreeMap::new();
    m.insert("host".to_string(), json::str_val(s.host.trim()));
    m.insert("enabled".to_string(), Value::Bool(s.enabled));
    Value::Obj(m)
}

/// Hostname or literal address. Shell metacharacters and whitespace are
/// rejected, so a value can never become an extra argument to `ntpd`.
pub fn validate_host(host: &str) -> Result<(), String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("NTP 服务器不能为空".to_string());
    }
    if host.len() > 253 {
        return Err(format!("NTP 服务器名称过长: {}", host));
    }
    if host.starts_with('-') || host.contains(char::is_whitespace) {
        return Err(format!("NTP 服务器名称无效: {}", host));
    }
    if Ipv4Addr::from_str(host).is_ok() || Ipv6Addr::from_str(host).is_ok() {
        return Ok(());
    }
    for label in host.split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(format!("NTP 服务器名称无效: {}", host));
        }
        if label.starts_with('-') || label.ends_with('-') {
            return Err(format!("NTP 服务器名称无效: {}", host));
        }
        if !label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(format!("NTP 服务器名称无效: {}", host));
        }
    }
    Ok(())
}

// --------------------------------------------------------------- backup ----

/// Save the system's current NTP servers once, before the plugin first
/// overwrites them. Idempotent.
pub fn backup_system_once() -> Result<(), String> {
    if sys::uci_get(&key("backed_up")).as_deref() == Some("1") {
        return Ok(());
    }
    let servers = sys::uci_get_list("system.ntp.server");
    let enabled = sys::uci_get("system.ntp.enabled").unwrap_or_else(|| "1".to_string());
    sys::uci_set(&key("backed_up"), "1")?;
    sys::uci_set(&key("previous_enabled"), if enabled == "0" { "0" } else { "1" })?;
    sys::uci_set_list(&key("previous_server"), &servers)?;
    sys::uci_commit(PACKAGE)
}

/// Restore the pre-plugin system NTP configuration. Does nothing when no
/// backup exists (the plugin never managed this system).
pub fn restore_system() -> Result<(), String> {
    if sys::uci_get(&key("backed_up")).as_deref() != Some("1") {
        return Ok(());
    }
    let servers = sys::uci_get_list(&key("previous_server"));
    let enabled = sys::uci_get(&key("previous_enabled")).unwrap_or_else(|| "1".to_string());
    sys::uci_set_list("system.ntp.server", &servers)?;
    sys::uci_set(
        "system.ntp.enabled",
        if enabled == "0" { "0" } else { "1" },
    )?;
    sys::uci_commit("system")?;
    sys::uci_delete(&key("previous_server"));
    sys::uci_delete(&key("previous_enabled"));
    sys::uci_set(&key("backed_up"), "0")?;
    sys::uci_commit(PACKAGE)
}

pub fn is_backed_up() -> bool {
    sys::uci_get(&key("backed_up")).as_deref() == Some("1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_documented_servers_in_order() {
        let c = NtpConfig::default();
        assert_eq!(
            c.effective_servers(),
            vec!["ntp.tencent.com", "time.cloudflare.com", "ntp.aliyun.com"]
        );
        assert!(c.validate().is_ok());
    }

    #[test]
    fn hostname_validation_rejects_argument_injection() {
        for bad in [
            "", " ", "-p", "ntp.aliyun.com;reboot", "ntp aliyun", "a..b", "-a.com",
            "a-.com", "a/b", "$(id)", "a\nb",
        ] {
            assert!(validate_host(bad).is_err(), "accepted {:?}", bad);
        }
        for good in [
            "ntp.tencent.com",
            "time.cloudflare.com",
            "ntp.aliyun.com",
            "192.168.1.1",
            "2402:4e00::",
            "lan-ntp",
            "_ntp._udp.example.com",
        ] {
            assert!(validate_host(good).is_ok(), "rejected {:?}", good);
        }
    }

    #[test]
    fn enabled_config_needs_one_enabled_server() {
        let mut c = NtpConfig::default();
        for s in &mut c.servers {
            s.enabled = false;
        }
        assert!(c.validate().is_err());
        c.enabled = false;
        assert!(c.validate().is_ok());
    }

    #[test]
    fn duplicate_hosts_are_rejected_case_insensitively() {
        let mut c = NtpConfig::default();
        c.servers[1].host = "NTP.TENCENT.COM".into();
        assert!(c.validate().unwrap_err().contains("重复"));
    }

    #[test]
    fn merge_json_keeps_order_and_disable_flags() {
        let mut c = NtpConfig::default();
        let p = json::parse(
            r#"{"enabled":true,"servers":[
                {"host":"b.example.com","enabled":false},
                {"host":"a.example.com","enabled":true}
            ]}"#,
        )
        .unwrap();
        c.merge_json(&p).unwrap();
        assert_eq!(c.effective_servers(), vec!["a.example.com"]);
        assert_eq!(c.servers[0].host, "b.example.com");
        assert!(!c.servers[0].enabled);
    }

    #[test]
    fn too_many_servers_are_rejected() {
        let mut c = NtpConfig::default();
        c.servers = (0..=MAX_SERVERS)
            .map(|i| NtpServer {
                host: format!("ntp{}.example.com", i),
                enabled: true,
            })
            .collect();
        assert!(c.validate().is_err());
    }
}
