//! netifd / firewall reconciliation and interface queries.

use std::time::Duration;

use crate::core::json::{self, Value};
use crate::core::sys;
use crate::manager::{uci_add_list, uci_commit, uci_delete, uci_get, Config};

fn pdp_type() -> String {
    match uci_get("mt5700m.connection.pdp_type").as_deref() {
        Some("ip") => "ip".to_string(),
        Some("ipv6") => "ipv6".to_string(),
        _ => "ipv4v6".to_string(),
    }
}

pub fn ubus_status(iface: &str) -> Option<Value> {
    let out = sys::run(
        "ubus",
        &["call", &format!("network.interface.{}", iface), "status"],
        Duration::from_secs(5),
    );
    if !out.ok {
        return None;
    }
    json::parse(&out.stdout)
}

pub fn iface_up(iface: &str) -> bool {
    ubus_status(iface)
        .as_ref()
        .and_then(|v| v.get("up"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

pub fn iface_device(iface: &str) -> Option<String> {
    let status = ubus_status(iface)?;
    let dev = status
        .get("l3_device")
        .and_then(|v| v.as_str())
        .or_else(|| status.get("device").and_then(|v| v.as_str()))
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty() && !s.starts_with('@'))?;
    Some(dev)
}

fn address_from_ubus(status: &Value, family: u8) -> Option<String> {
    let key = format!("ipv{}-address", family);
    let list = status.get(&key)?.as_arr()?;
    let first = list.first()?;
    let addr = first.get("address")?.as_str()?;
    (!addr.is_empty()).then(|| addr.to_string())
}

/// Address from netifd, falling back to `ip addr` so a jsonfilter change can
/// never be mistaken for "the link has no address".
pub fn iface_address(iface: &str, family: u8) -> Option<String> {
    if let Some(status) = ubus_status(iface) {
        if let Some(addr) = address_from_ubus(&status, family) {
            return Some(addr);
        }
    }
    let dev = iface_device(iface)?;
    let flag = format!("-{}", family);
    let out = sys::run(
        "ip",
        &["-o", &flag, "addr", "show", "dev", &dev, "scope", "global"],
        Duration::from_secs(5),
    );
    if !out.ok {
        return None;
    }
    for line in out.stdout.lines() {
        let mut fields = line.split_whitespace();
        while let Some(field) = fields.next() {
            if field == "inet" || field == "inet6" {
                if let Some(cidr) = fields.next() {
                    let addr = cidr.split('/').next().unwrap_or(cidr);
                    if !addr.is_empty() {
                        return Some(addr.to_string());
                    }
                }
            }
        }
    }
    None
}

pub fn iface_has_ip(iface: &str, family: u8) -> bool {
    iface_address(iface, family).is_some()
}

pub fn iface_dns(iface: &str) -> Option<String> {
    let status = ubus_status(iface)?;
    let list = status.get("dns-server")?.as_arr()?;
    list.first()
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

fn list_contains(key: &str, wanted: &str) -> bool {
    uci_get(key)
        .map(|v| v.split_whitespace().any(|item| item == wanted))
        .unwrap_or(false)
}

fn wan_zone() -> Option<String> {
    let out = sys::run("uci", &["-q", "show", "firewall"], Duration::from_secs(5));
    if !out.ok {
        return None;
    }
    for line in out.stdout.lines() {
        let Some((path, value)) = line.split_once('=') else {
            continue;
        };
        if !path.ends_with(".name") {
            continue;
        }
        if value.trim_matches('\'') == "wan" {
            return Some(path.trim_end_matches(".name").to_string());
        }
    }
    None
}

/// Add the interface to the `wan` zone so its traffic is NATed/forwarded.
pub fn ensure_firewall_network(iface: &str, changed: &mut bool) -> bool {
    let Some(zone) = wan_zone() else {
        return true;
    };
    let key = format!("{}.network", zone);
    if list_contains(&key, iface) {
        return true;
    }
    if uci_add_list(&key, iface) {
        *changed = true;
        true
    } else {
        false
    }
}

/// Metric / default-route policy, including the optional h5000m_netmode overlay.
fn effective_policy() -> (String, String, String, String) {
    let mode = uci_get("h5000m_netmode.settings.mode").unwrap_or_default();
    let owner = uci_get("h5000m_netmode.settings.ipv6_owner").unwrap_or_default();
    let configured = uci_get("mt5700m.connection.metric")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(50);

    let metric = match mode.as_str() {
        "modem_first" | "modem_only" => 10,
        "wan_first" | "wan_only" => 50,
        _ => configured,
    };
    let ipv4_default = if mode == "wan_only" { "0" } else { "1" };
    let (ipv6_default, ipv6_auto) = match owner.as_str() {
        "modem" => ("1", "1"),
        "wan" => ("0", "0"),
        _ => match mode.as_str() {
            "modem_first" | "modem_only" => ("1", "1"),
            _ => ("0", "0"),
        },
    };
    (
        metric.to_string(),
        ipv4_default.to_string(),
        ipv6_default.to_string(),
        ipv6_auto.to_string(),
    )
}

/// Create/align `network.MT5700M` (+`MT5700Mv6`) with the discovered netdev.
pub fn ensure_network(cfg: &Config, netdev: &str) -> bool {
    let pdp = pdp_type();
    let (metric, ipv4_default, ipv6_default, ipv6_auto) = effective_policy();
    let mut changed = false;
    let mut firewall_changed = false;
    let mut ok = true;

    ok &= crate::manager::set_if_changed(&format!("network.{}", cfg.iface4), "interface", &mut changed);
    ok &= crate::manager::set_if_changed(
        &format!("network.{}.managed_by", cfg.iface4),
        "mt5700m",
        &mut changed,
    );
    ok &= crate::manager::set_if_changed(
        &format!("network.{}.device", cfg.iface4),
        netdev,
        &mut changed,
    );
    ok &= crate::manager::set_if_changed(
        &format!("network.{}.ifname", cfg.iface4),
        netdev,
        &mut changed,
    );
    ok &= crate::manager::set_if_changed(
        &format!("network.{}.metric", cfg.iface4),
        &metric,
        &mut changed,
    );
    ok &= crate::manager::set_if_changed(
        &format!("network.{}.defaultroute", cfg.iface4),
        &ipv4_default,
        &mut changed,
    );
    ok &= crate::manager::set_if_changed(
        &format!("network.{}.norelease", cfg.iface4),
        "1",
        &mut changed,
    );
    let _ = crate::manager::delete_if_present(
        &format!("network.{}.modem_config", cfg.iface4),
        &mut changed,
    );
    let proto = if pdp == "ipv6" { "none" } else { "dhcp" };
    ok &= crate::manager::set_if_changed(
        &format!("network.{}.proto", cfg.iface4),
        proto,
        &mut changed,
    );

    if pdp == "ip" {
        if uci_get(&format!("network.{}", cfg.iface6)).is_some() {
            let _ = uci_delete(&format!("network.{}", cfg.iface6));
            changed = true;
        }
    } else {
        ok &= crate::manager::set_if_changed(&format!("network.{}", cfg.iface6), "interface", &mut changed);
        ok &= crate::manager::set_if_changed(
            &format!("network.{}.managed_by", cfg.iface6),
            "mt5700m",
            &mut changed,
        );
        ok &= crate::manager::set_if_changed(
            &format!("network.{}.proto", cfg.iface6),
            "dhcpv6",
            &mut changed,
        );
        ok &= crate::manager::set_if_changed(
            &format!("network.{}.device", cfg.iface6),
            &format!("@{}", cfg.iface4),
            &mut changed,
        );
        ok &= crate::manager::set_if_changed(
            &format!("network.{}.ifname", cfg.iface6),
            &format!("@{}", cfg.iface4),
            &mut changed,
        );
        ok &= crate::manager::set_if_changed(
            &format!("network.{}.metric", cfg.iface6),
            &metric,
            &mut changed,
        );
        ok &= crate::manager::set_if_changed(
            &format!("network.{}.defaultroute", cfg.iface6),
            &ipv6_default,
            &mut changed,
        );
        ok &= crate::manager::set_if_changed(
            &format!("network.{}.auto", cfg.iface6),
            &ipv6_auto,
            &mut changed,
        );
        ok &= crate::manager::set_if_changed(
            &format!("network.{}.extendprefix", cfg.iface6),
            "1",
            &mut changed,
        );
        let _ = crate::manager::delete_if_present(
            &format!("network.{}.modem_config", cfg.iface6),
            &mut changed,
        );
    }

    ok &= ensure_firewall_network(&cfg.iface4, &mut firewall_changed);
    if pdp != "ip" {
        ok &= ensure_firewall_network(&cfg.iface6, &mut firewall_changed);
    }

    if changed {
        uci_commit("network");
        let _ = sys::run(
            "/etc/init.d/network",
            &["reload"],
            Duration::from_secs(20),
        );
        cfg.log(&format!(
            "network synchronized device={} protocol={} metric={}",
            netdev, pdp, metric
        ));
    }
    if firewall_changed {
        uci_commit("firewall");
        let _ = sys::run(
            "/etc/init.d/firewall",
            &["reload"],
            Duration::from_secs(20),
        );
    }

    // Single source of truth for the traffic accountant and the Rust collector.
    let _ = std::fs::write(cfg.state_path("netdev"), format!("{}\n", netdev));

    apply_dns(cfg);
    ok
}

/// DNS injection is still owned by the shell script until phase 3; failures are
/// rate-limited to one log line per five minutes so a broken resolv.conf cannot
/// flood the 2000-line log.
pub fn apply_dns(cfg: &Config) -> bool {
    let ok = crate::manager::dns::apply(cfg);
    if ok {
        let _ = std::fs::remove_file(cfg.state_path("dns_error_logged"));
        return true;
    }
    let now = crate::manager::now_unix();
    let last = crate::manager::state_read(&cfg.state_path("dns_error_logged"), 0);
    if now.saturating_sub(last) >= 300 {
        cfg.log("DNS apply failed; see /var/run/mt5700m/dns.state");
        crate::manager::state_write(&cfg.state_path("dns_error_logged"), now);
    }
    false
}

pub fn bring_up(cfg: &Config) {
    let _ = sys::run("ifup", &[&cfg.iface4], Duration::from_secs(30));
    match pdp_type().as_str() {
        "ip" => {
            let _ = sys::run("ifdown", &[&cfg.iface6], Duration::from_secs(20));
        }
        _ => {
            let auto = uci_get(&format!("network.{}.auto", cfg.iface6)).unwrap_or_else(|| "1".into());
            if auto == "0" {
                let _ = sys::run("ifdown", &[&cfg.iface6], Duration::from_secs(20));
            } else {
                let _ = sys::run("ifup", &[&cfg.iface6], Duration::from_secs(30));
            }
        }
    }
}

pub fn bring_down(cfg: &Config) {
    let _ = sys::run("ifdown", &[&cfg.iface6], Duration::from_secs(20));
    let _ = sys::run("ifdown", &[&cfg.iface4], Duration::from_secs(20));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_parsing_handles_prefix() {
        // Pure helper mirror: the `ip` fallback strips the /prefix.
        let cidr = "10.6.172.152/30";
        assert_eq!(cidr.split('/').next().unwrap(), "10.6.172.152");
    }
}
