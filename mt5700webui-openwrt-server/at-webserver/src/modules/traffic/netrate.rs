//! Interface counters and the shared accounting report.
//!
//! Rate figures come from `/sys/class/net/<dev>/statistics` (zero AT traffic),
//! while the cumulative/day/month totals come from `mt5700m-traffic json` — the
//! single accounting writer on the box. Reading its JSON keeps both frontends
//! and this backend on one code path instead of each parsing the history file.

use crate::modules::traffic::state::NetRateState;
use crate::state::refresh::{now_secs_f64, read_counter};

/// Candidate modem-facing interfaces, most specific first.
///
/// Mirrors `mt5700m-traffic`'s default (`eth2`) with fallbacks so the numbers
/// survive a USB re-enumeration renaming the netdev.
const NETRATE_IFACES: [&str; 4] = ["eth2", "usb0", "wwan0", "eth1"];

/// Detect which interface carries the modem's traffic.
pub fn detect_iface() -> Option<&'static str> {
    NETRATE_IFACES
        .iter()
        .copied()
        .find(|d| std::path::Path::new(&format!("/sys/class/net/{}/statistics", d)).is_dir())
}

/// Shell out to `mt5700m-traffic json` and parse its output.
pub fn run_traffic_json() -> Option<crate::core::json::Value> {
    use std::process::{Command, Stdio};

    let out = Command::new("/bin/sh")
        .arg("-c")
        .arg("/usr/sbin/mt5700m-traffic json")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    crate::core::json::parse(text)
}

/// Read the interface counters and attach the accounting report.
pub fn collect() -> NetRateState {
    let Some(dev) = detect_iface() else {
        return NetRateState {
            available: false,
            reason: Some("/sys/class/net 下未找到可用接口（eth2/usb0/wwan0/eth1）".to_string()),
            timestamp: now_secs_f64(),
            ..Default::default()
        };
    };
    let base = format!("/sys/class/net/{}/statistics", dev);
    let rx = read_counter(&format!("{}/rx_bytes", base));
    let tx = read_counter(&format!("{}/tx_bytes", base));
    if rx.is_none() && tx.is_none() {
        return NetRateState {
            available: false,
            device: Some(dev.to_string()),
            reason: Some("接口计数器不可读".to_string()),
            timestamp: now_secs_f64(),
            ..Default::default()
        };
    }
    let (source, traffic) = match run_traffic_json() {
        Some(report) => ("mt5700m-traffic", Some(report)),
        None => ("unavailable", None),
    };
    NetRateState {
        available: true,
        device: Some(dev.to_string()),
        reason: None,
        rx_bytes: rx,
        tx_bytes: tx,
        timestamp: now_secs_f64(),
        source,
        traffic,
    }
}
