//! DNS injection — Rust port of `usr/share/mt5700m/dns.sh`.
//!
//! Scope is deliberately narrow: only the two interfaces this plugin owns
//! (`MT5700M`, `MT5700Mv6`) get `dns`/`peerdns` written. The global resolver,
//! LAN/WAN, dnsmasq and other interfaces are never touched.
//!
//! The state file (`/var/run/mt5700m/dns.state`) keeps the exact shape the
//! daemon's `modules::dns::state` parser already reads, so both frontends see
//! the same "applied" view during and after the migration.

use std::fs;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;
use std::time::Duration;

use crate::core::sys;
use crate::manager::{now_unix, uci_add_list, uci_commit, uci_delete, uci_get, uci_set, Config};

const SECTION: &str = "mt5700m.dns";
const DEFAULT_V4_PRIMARY: &str = "119.29.29.29";
const DEFAULT_V4_SECONDARY: &str = "223.5.5.5";
const DEFAULT_V6_PRIMARY: &str = "2402:4e00::";
const DEFAULT_V6_SECONDARY: &str = "2400:3200::1";

#[derive(Debug, Clone, Copy, PartialEq)]
enum FamilyResult {
    Applied,
    Skipped,
    Failed,
}

fn state_file() -> PathBuf {
    let dir = std::env::var("MT5700M_DNS_STATE_DIR")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "/var/run/mt5700m".to_string());
    PathBuf::from(dir).join("dns.state")
}

/* --------------------------------------------------------------- validation */

/// Same rules as the shell: dotted quad, no leading zeroes, first octet in
/// 1..=223 (so `0.0.0.0`, multicast and broadcast are rejected).
pub fn valid_ipv4(value: &str) -> bool {
    let Ok(addr) = value.parse::<Ipv4Addr>() else {
        return false;
    };
    let octets = addr.octets();
    if octets[0] == 0 || octets[0] >= 224 {
        return false;
    }
    // Reject forms the shell rejected but std accepts (e.g. "1.2.3.4 " is
    // already trimmed by the caller; embedded whitespace never parses).
    !value.trim().is_empty() && value.trim() == value
}

/// Unspecified, multicast and scoped addresses are not resolvers.
pub fn valid_ipv6(value: &str) -> bool {
    if value.is_empty() || value.contains('%') {
        return false;
    }
    let Ok(addr) = value.parse::<Ipv6Addr>() else {
        return false;
    };
    !addr.is_unspecified() && !addr.is_multicast()
}

fn cfg_bool(option: &str, default: bool) -> bool {
    match uci_get(&format!("{}.{}", SECTION, option))
        .map(|v| v.to_ascii_lowercase())
        .as_deref()
    {
        None | Some("") => default,
        Some("0") | Some("false") | Some("off") | Some("no") | Some("disabled") => false,
        _ => true,
    }
}

fn cfg_text(option: &str, default: &str) -> String {
    uci_get(&format!("{}.{}", SECTION, option)).unwrap_or_else(|| default.to_string())
}

fn ensure_section() -> bool {
    if uci_get(SECTION).is_some() {
        return true;
    }
    if !uci_set(SECTION, "dns") {
        return false;
    }
    let defaults = [
        ("ipv4_enabled", "1"),
        ("ipv4_primary", DEFAULT_V4_PRIMARY),
        ("ipv4_secondary", DEFAULT_V4_SECONDARY),
        ("ipv6_enabled", "1"),
        ("ipv6_primary", DEFAULT_V6_PRIMARY),
        ("ipv6_secondary", DEFAULT_V6_SECONDARY),
    ];
    for (key, value) in defaults {
        let _ = uci_set(&format!("{}.{}", SECTION, key), value);
    }
    uci_commit("mt5700m")
}

fn normalise(values: &[String]) -> String {
    values
        .iter()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn current_list(iface: &str) -> Vec<String> {
    uci_get(&format!("network.{}.dns", iface))
        .map(|v| {
            v.split_whitespace()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// Apply one family. `None` = interface absent (only legal for IPv6).
fn apply_family(iface: &str, enabled: bool, servers: &[String], changed: &mut bool) -> Option<FamilyResult> {
    if uci_get(&format!("network.{}", iface)).is_none() {
        if !enabled {
            return Some(FamilyResult::Applied);
        }
        return None;
    }

    let current = current_list(iface);
    let peer = uci_get(&format!("network.{}.peerdns", iface)).unwrap_or_default();

    if enabled {
        let wanted = normalise(servers);
        if normalise(&current) != wanted || peer != "0" {
            let _ = uci_delete(&format!("network.{}.dns", iface));
            for server in servers.iter().filter(|s| !s.trim().is_empty()) {
                if !uci_add_list(&format!("network.{}.dns", iface), server.trim()) {
                    return Some(FamilyResult::Failed);
                }
            }
            if !uci_set(&format!("network.{}.peerdns", iface), "0") {
                return Some(FamilyResult::Failed);
            }
            *changed = true;
        }
    } else if !current.is_empty() || !peer.is_empty() {
        let _ = uci_delete(&format!("network.{}.dns", iface));
        let _ = uci_delete(&format!("network.{}.peerdns", iface));
        *changed = true;
    }
    Some(FamilyResult::Applied)
}

fn write_state(
    result: &str,
    error: &str,
    v4_enabled: bool,
    v4_servers: &str,
    v6_enabled: bool,
    v6_servers: &str,
    v6_skipped: bool,
) {
    let path = state_file();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let body = format!(
        "result={}\napplied_at={}\nv4_enabled={}\nv4_servers={}\nv6_enabled={}\nv6_servers={}\nv6_skipped={}\nerror={}\n",
        result,
        now_unix(),
        v4_enabled as u8,
        v4_servers,
        v6_enabled as u8,
        v6_servers,
        v6_skipped as u8,
        error.replace('\n', " ")
    );
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    if fs::write(&tmp, body).is_ok() {
        let _ = fs::rename(&tmp, &path);
    }
}

pub fn run(args: &[String]) -> i32 {
    let cfg = Config::from_env();
    match args.first().map(|s| s.as_str()) {
        Some("apply") | None => {
            if apply(&cfg) {
                0
            } else {
                1
            }
        }
        Some("status") => {
            print!("{}", status());
            0
        }
        _ => {
            eprintln!("Usage: at-webserver dns {{apply|status}}");
            64
        }
    }
}

pub fn status() -> String {
    match fs::read_to_string(state_file()) {
        Ok(text) => text,
        Err(_) => "result=never\n".to_string(),
    }
}

/// Apply the saved configuration. Returns true unless the IPv4 family failed
/// (matching the shell: `partial` still returns 0).
pub fn apply(cfg: &Config) -> bool {
    if !ensure_section() {
        write_state("error", "cannot create mt5700m.dns section", false, "", false, "", false);
        return false;
    }

    let v4_enabled = cfg_bool("ipv4_enabled", true);
    let mut v4_primary = cfg_text("ipv4_primary", DEFAULT_V4_PRIMARY);
    let mut v4_secondary = cfg_text("ipv4_secondary", DEFAULT_V4_SECONDARY);
    let v6_enabled = cfg_bool("ipv6_enabled", true);
    let mut v6_primary = cfg_text("ipv6_primary", DEFAULT_V6_PRIMARY);
    let mut v6_secondary = cfg_text("ipv6_secondary", DEFAULT_V6_SECONDARY);

    let mut errors: Vec<String> = Vec::new();
    if v4_enabled {
        if v4_primary.trim().is_empty() {
            v4_primary = DEFAULT_V4_PRIMARY.to_string();
            errors.push("IPv4 主 DNS 为空，已使用默认值".to_string());
        } else if !valid_ipv4(&v4_primary) {
            v4_primary = DEFAULT_V4_PRIMARY.to_string();
            errors.push("IPv4 主 DNS 无效，已使用默认值".to_string());
        }
        if !v4_secondary.trim().is_empty() && !valid_ipv4(&v4_secondary) {
            v4_secondary.clear();
            errors.push("IPv4 备用 DNS 无效，已忽略".to_string());
        }
    }
    if v6_enabled {
        if v6_primary.trim().is_empty() {
            v6_primary = DEFAULT_V6_PRIMARY.to_string();
            errors.push("IPv6 主 DNS 为空，已使用默认值".to_string());
        } else if !valid_ipv6(&v6_primary) {
            v6_primary = DEFAULT_V6_PRIMARY.to_string();
            errors.push("IPv6 主 DNS 无效，已使用默认值".to_string());
        }
        if !v6_secondary.trim().is_empty() && !valid_ipv6(&v6_secondary) {
            v6_secondary.clear();
            errors.push("IPv6 备用 DNS 无效，已忽略".to_string());
        }
    }

    let v4_servers: Vec<String> = if v4_enabled {
        vec![v4_primary.trim().to_string(), v4_secondary.trim().to_string()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        Vec::new()
    };
    let v6_servers: Vec<String> = if v6_enabled {
        vec![v6_primary.trim().to_string(), v6_secondary.trim().to_string()]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect()
    } else {
        Vec::new()
    };

    let mut changed = false;
    let v4 = apply_family(&cfg.iface4, v4_enabled, &v4_servers, &mut changed);
    let v4_result = match v4 {
        Some(FamilyResult::Applied) => FamilyResult::Applied,
        Some(FamilyResult::Failed) | None => {
            errors.push("IPv4 接口不存在".to_string());
            FamilyResult::Failed
        }
        Some(FamilyResult::Skipped) => FamilyResult::Failed,
    };

    let v6 = apply_family(&cfg.iface6, v6_enabled, &v6_servers, &mut changed);
    let v6_result = match v6 {
        Some(result) => result,
        None => FamilyResult::Skipped,
    };
    if v6_result == FamilyResult::Failed {
        errors.push("IPv6 应用失败".to_string());
    }

    if changed {
        if !uci_commit("network") {
            write_state("error", "cannot commit network config", false, "", false, "", false);
            return false;
        }
        let reloaded = sys::run("ubus", &["call", "network", "reload"], Duration::from_secs(20));
        if !reloaded.ok {
            let _ = sys::run("/etc/init.d/network", &["reload"], Duration::from_secs(20));
        }
    }

    let result = if v4_result != FamilyResult::Applied {
        "error"
    } else if v6_result == FamilyResult::Failed || !errors.is_empty() {
        "partial"
    } else {
        "ok"
    };

    let state_v4_enabled = v4_enabled && v4_result == FamilyResult::Applied;
    let state_v6_enabled = v6_enabled && v6_result != FamilyResult::Failed;
    let error_text = errors.join("; ");
    write_state(
        result,
        &error_text,
        state_v4_enabled,
        &v4_servers.join(" "),
        state_v6_enabled,
        &v6_servers.join(" "),
        v6_result == FamilyResult::Skipped,
    );

    cfg.log(&format!(
        "DNS apply result={} v4={} v6={} skipped={} error={} changed={}",
        result,
        if v4_enabled { v4_servers.join(",") } else { "disabled".to_string() },
        if v6_enabled { v6_servers.join(",") } else { "disabled".to_string() },
        (v6_result == FamilyResult::Skipped) as u8,
        if error_text.is_empty() { "none".to_string() } else { error_text },
        changed as u8
    ));

    result != "error"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv4_rules_match_the_shell() {
        assert!(valid_ipv4("119.29.29.29"));
        assert!(valid_ipv4("223.5.5.5"));
        assert!(!valid_ipv4("0.1.2.3"));
        assert!(!valid_ipv4("224.0.0.1"));
        assert!(!valid_ipv4("1.2.3"));
        assert!(!valid_ipv4("01.2.3.4"));
        assert!(!valid_ipv4("1.2.3.4 "));
        assert!(!valid_ipv4(""));
    }

    #[test]
    fn ipv6_rules_match_the_shell() {
        assert!(valid_ipv6("2402:4e00::"));
        assert!(valid_ipv6("2400:3200::1"));
        assert!(!valid_ipv6("::"));
        assert!(!valid_ipv6("ff02::1"));
        assert!(!valid_ipv6("fe80::1%eth0"));
        assert!(!valid_ipv6("2400:3200"));
    }

    #[test]
    fn normalise_collapses_whitespace() {
        assert_eq!(
            normalise(&["1.1.1.1".to_string(), " 2.2.2.2 ".to_string(), "".to_string()]),
            "1.1.1.1 2.2.2.2"
        );
    }
}
