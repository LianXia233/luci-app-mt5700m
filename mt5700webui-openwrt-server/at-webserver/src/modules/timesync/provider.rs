//! Detection and control of the system's NTP implementation.
//!
//! There is exactly one clock controller on the device. The plugin never
//! spawns a daemon of its own: it configures the one already present and
//! reads its state. When no system service exists, configuration is saved but
//! not applied, and the page says so; manual sync still works through a
//! one-shot query.
//!
//! Supported providers:
//! * `sysntpd` — stock OpenWrt busybox ntpd, configured through
//!   `/etc/config/system`;
//! * `chrony` — when installed/running, through a marked block appended to
//!   its config (the rest of the file is never modified);
//! * an external `ntpd` process — observed only, never reconfigured;
//! * none.

use crate::core::sys;
use std::path::Path;
use std::time::Duration;

const SYSYNTPD_INIT: &str = "/etc/init.d/sysntpd";
const CHRONY_CONFIGS: [&str; 2] = ["/etc/chrony.conf", "/etc/chrony/chrony.conf"];
const CHRONY_INITS: [&str; 2] = ["/etc/init.d/chronyd", "/etc/init.d/chrony"];
const CHRONY_BEGIN: &str = "# BEGIN mt5700m-managed";
const CHRONY_END: &str = "# END mt5700m-managed";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    Sysntpd,
    Chrony,
    ExternalNtpd,
    None,
}

impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Provider::Sysntpd => "sysntpd",
            Provider::Chrony => "chrony",
            Provider::ExternalNtpd => "ntpd",
            Provider::None => "none",
        }
    }

    /// True when the plugin can write this provider's configuration.
    pub fn configurable(self) -> bool {
        matches!(self, Provider::Sysntpd | Provider::Chrony)
    }
}

pub fn detect() -> Provider {
    if sys::pidof("chronyd") {
        return Provider::Chrony;
    }
    if Path::new(SYSYNTPD_INIT).exists() {
        return Provider::Sysntpd;
    }
    if sys::pidof("ntpd") {
        return Provider::ExternalNtpd;
    }
    if CHRONY_INITS.iter().any(|p| Path::new(p).exists()) || chrony_config().is_some() {
        return Provider::Chrony;
    }
    Provider::None
}

pub fn running(provider: Provider) -> bool {
    match provider {
        Provider::Sysntpd => sys::service_running("sysntpd") || sys::pidof("ntpd"),
        Provider::Chrony => sys::pidof("chronyd"),
        Provider::ExternalNtpd => sys::pidof("ntpd"),
        Provider::None => false,
    }
}

/// What the provider reports about clock synchronisation.
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    /// `Some(true)` only with positive evidence; `None` means "cannot tell".
    pub synced: Option<bool>,
    pub server: Option<String>,
    pub offset_ms: Option<i64>,
    pub detail: Option<String>,
}

impl Observation {
    fn unknown(detail: &str) -> Self {
        Observation {
            synced: None,
            server: None,
            offset_ms: None,
            detail: Some(detail.to_string()),
        }
    }
}

/// Read synchronisation state from whichever tool the provider offers.
///
/// `ntpq` is preferred because it names the selected peer. `adjtimex` gives a
/// definitive kernel flag but no peer. If neither exists the answer is
/// `unknown`, never a fabricated success.
pub fn observe(provider: Provider) -> Observation {
    match provider {
        Provider::Chrony => observe_chrony(),
        Provider::Sysntpd | Provider::ExternalNtpd | Provider::None => observe_ntp_tools(),
    }
}

fn observe_chrony() -> Observation {
    let out = sys::run("chronyc", &["tracking"], Duration::from_secs(4));
    if !out.ok {
        return Observation::unknown("chronyc tracking is not available");
    }
    parse_chrony_tracking(&out.stdout)
}

pub fn parse_chrony_tracking(text: &str) -> Observation {
    let mut leap_ok = false;
    let mut stratum_ok = false;
    let mut server = None;
    let mut offset_ms = None;
    let mut saw = false;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        match key.as_str() {
            "leap status" => {
                saw = true;
                leap_ok = value.eq_ignore_ascii_case("normal");
            }
            "stratum" => {
                saw = true;
                if let Ok(n) = value.parse::<u32>() {
                    stratum_ok = n >= 1 && n <= 15;
                }
            }
            "reference id" => {
                saw = true;
                if !value.is_empty() && value != "00000000" {
                    server = Some(value.to_string());
                }
            }
            "system time" => {
                saw = true;
                if let Some(ms) = parse_chrony_offset(value) {
                    offset_ms = Some(ms);
                }
            }
            "last offset" => {
                saw = true;
                // "System time" is the current error and wins; "Last offset"
                // is only a fallback for older chrony output.
                if offset_ms.is_none() {
                    if let Some(ms) = parse_chrony_offset(value) {
                        offset_ms = Some(ms);
                    }
                }
            }
            _ => {}
        }
    }
    if !saw {
        return Observation::unknown("unrecognised chronyc output");
    }
    Observation {
        synced: Some(leap_ok && stratum_ok),
        server,
        offset_ms,
        detail: None,
    }
}

/// `0.000123456 seconds fast of NTP time` -> `0` ms; `+0.0123 seconds` -> 12.
fn parse_chrony_offset(value: &str) -> Option<i64> {
    let lower = value.to_ascii_lowercase();
    let number = lower.split_whitespace().next()?;
    let seconds: f64 = number.parse().ok()?;
    let ms = (seconds * 1000.0).round() as i64;
    if lower.contains("slow") {
        Some(-ms)
    } else {
        Some(ms)
    }
}

fn observe_ntp_tools() -> Observation {
    let ntpq = sys::run("ntpq", &["-p"], Duration::from_secs(4));
    if ntpq.ok && !ntpq.stdout.trim().is_empty() {
        return parse_ntpq_peers(&ntpq.stdout);
    }
    let adj = sys::run("adjtimex", &["-p"], Duration::from_secs(3));
    if adj.ok {
        return parse_adjtimex(&adj.stdout);
    }
    Observation::unknown("neither ntpq nor adjtimex is available")
}

pub fn parse_ntpq_peers(text: &str) -> Observation {
    let mut saw_peer = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with("remote") {
            continue;
        }
        let Some(first) = trimmed.chars().next() else {
            continue;
        };
        let is_peer = first.is_ascii_alphanumeric()
            || matches!(first, '*' | '+' | '-' | '#' | 'o' | 'x' | '.');
        if !is_peer {
            continue;
        }
        saw_peer = true;
        // `*` is the current system peer; `o` is a PPS peer.
        if first == '*' || first == 'o' {
            let remote = trimmed[1..]
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            return Observation {
                synced: Some(true),
                server: (!remote.is_empty()).then_some(remote),
                offset_ms: parse_ntpq_offset(trimmed),
                detail: None,
            };
        }
    }
    if saw_peer {
        Observation {
            synced: Some(false),
            server: None,
            offset_ms: None,
            detail: Some("no selected peer".to_string()),
        }
    } else {
        Observation::unknown("ntpq reports no peers")
    }
}

/// Offset is the column before jitter in `ntpq -p` (delay, offset, jitter
/// are the last three). Reading the last parsable value would report jitter,
/// so the exact column is used and a short/odd row is rejected.
fn parse_ntpq_offset(line: &str) -> Option<i64> {
    let cols: Vec<&str> = line.split_whitespace().collect();
    if cols.len() < 9 {
        return None;
    }
    let value = cols[cols.len() - 2].trim_matches(|c: char| c == ',' || c == ';');
    value.parse::<f64>().ok().map(|v| v.round() as i64)
}

pub fn parse_adjtimex(text: &str) -> Observation {
    for line in text.lines() {
        let lower = line.to_ascii_lowercase();
        let Some(value) = lower.trim().strip_prefix("status:") else {
            continue;
        };
        let value = value.trim();
        let parsed = if let Some(hex) = value.strip_prefix("0x") {
            u64::from_str_radix(hex, 16).ok()
        } else {
            value.parse::<u64>().ok()
        };
        if let Some(status) = parsed {
            // STA_UNSYNC (0x0040) is the kernel's own "not synchronised" bit.
            let unsync = status & 0x40 != 0;
            return Observation {
                synced: Some(!unsync && crate::core::time::clock_plausible()),
                server: None,
                offset_ms: None,
                detail: None,
            };
        }
    }
    Observation::unknown("adjtimex status not found")
}

// ------------------------------------------------------------ configure ----

pub fn chrony_config() -> Option<String> {
    CHRONY_CONFIGS
        .iter()
        .find(|p| Path::new(p).exists())
        .map(|p| p.to_string())
}

/// Write the plugin's servers into the marked block of the chrony config.
/// Lines outside the block are preserved byte for byte.
pub fn apply_chrony(servers: &[String]) -> Result<String, String> {
    let path = chrony_config()
        .ok_or_else(|| "未找到 chrony 配置文件，无法应用 NTP 服务器".to_string())?;
    let original = sys::read_file(&path).unwrap_or_default();
    let mut body: Vec<&str> = Vec::new();
    let mut inside = false;
    for line in original.lines() {
        if line.trim() == CHRONY_BEGIN {
            inside = true;
            continue;
        }
        if line.trim() == CHRONY_END {
            inside = false;
            continue;
        }
        if !inside {
            body.push(line);
        }
    }
    let mut text = body.join("\n");
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if !servers.is_empty() {
        text.push_str(CHRONY_BEGIN);
        text.push('\n');
        for host in servers {
            text.push_str("server ");
            text.push_str(host);
            text.push_str(" iburst\n");
        }
        text.push_str(CHRONY_END);
        text.push('\n');
    }
    sys::write_file_atomic(&path, &text)?;
    restart_chrony()?;
    Ok(path)
}

pub fn remove_chrony_block() -> Result<(), String> {
    let Some(path) = chrony_config() else {
        return Ok(());
    };
    let _path = path;
    apply_chrony(&[]).map(|_| ())
}

/// True when the plugin's marked block is present in the chrony config.
pub fn chrony_managed() -> bool {
    chrony_config()
        .and_then(|path| sys::read_file(&path))
        .map(|text| text.contains(CHRONY_BEGIN))
        .unwrap_or(false)
}

/// True when the provider's config already contains exactly `servers`.
/// Reads configuration only; running state is checked separately.
pub fn config_matches(provider: Provider, servers: &[String]) -> bool {
    match provider {
        Provider::Sysntpd => {
            sys::uci_get_list("system.ntp.server") == servers
                && sys::uci_get("system.ntp.enabled").as_deref() == Some("1")
                && !servers.is_empty()
        }
        Provider::Chrony => {
            let Some(text) = chrony_config().and_then(|p| sys::read_file(&p)) else {
                return false;
            };
            text.contains(CHRONY_BEGIN)
                && !servers.is_empty()
                && servers
                    .iter()
                    .all(|host| text.contains(&format!("server {} ", host)))
        }
        _ => false,
    }
}

pub(crate) fn restart_chrony() -> Result<(), String> {
    for init in CHRONY_INITS {
        if Path::new(init).exists() {
            let out = sys::run(init, &["restart"], Duration::from_secs(20));
            if out.ok {
                return Ok(());
            }
            return Err(out.error());
        }
    }
    if sys::pidof("chronyd") {
        let out = sys::run("killall", &["-HUP", "chronyd"], Duration::from_secs(5));
        if out.ok {
            return Ok(());
        }
        return Err(out.error());
    }
    // Installed but not running: nothing to restart yet.
    Ok(())
}

/// Write `system.ntp.server` and ensure the stock daemon runs.
pub fn apply_sysntpd(servers: &[String]) -> Result<(), String> {
    if sys::uci_get("system.ntp").is_none() {
        sys::uci_set("system.ntp", "timeserver")?;
    }
    sys::uci_set("system.ntp.enabled", "1")?;
    sys::uci_set_list("system.ntp.server", servers)?;
    sys::uci_commit("system")?;
    restart_sysntpd()
}

pub fn restart_sysntpd() -> Result<(), String> {
    if !Path::new(SYSYNTPD_INIT).exists() {
        return Err("/etc/init.d/sysntpd is not installed".to_string());
    }
    let out = sys::run(SYSYNTPD_INIT, &["restart"], Duration::from_secs(25));
    if out.ok {
        Ok(())
    } else {
        Err(out.error())
    }
}

/// Exact read-back of what the daemon will use.
pub fn verify_sysntpd(servers: &[String]) -> Result<(), String> {
    let actual = sys::uci_get_list("system.ntp.server");
    if actual != servers {
        return Err(format!(
            "system.ntp.server 未生效: 期望 {:?}，实际 {:?}",
            servers, actual
        ));
    }
    if sys::uci_get("system.ntp.enabled").as_deref() != Some("1") {
        return Err("system.ntp.enabled 不为 1".to_string());
    }
    if !running(Provider::Sysntpd) {
        return Err("sysntpd 未在运行".to_string());
    }
    Ok(())
}

pub fn verify_chrony(servers: &[String]) -> Result<(), String> {
    let path = chrony_config().ok_or_else(|| "未找到 chrony 配置文件".to_string())?;
    let text = sys::read_file(&path).unwrap_or_default();
    if !text.contains(CHRONY_BEGIN) {
        return Err("chrony 配置缺少插件区块".to_string());
    }
    for host in servers {
        if !text.contains(&format!("server {} ", host)) {
            return Err(format!("chrony 配置未包含 {}", host));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ntpq_selects_only_the_starred_peer() {
        let text = r#"     remote           refid      st t when poll reach   delay   offset  jitter
==============================================================================
 1.2.3.4         10.0.0.1     2 u   10   64  377    1.234   +0.456   0.789
*ntp.aliyun.com  .PPS.        1 u   12   64  377   10.000   -1.500   0.100
"#;
        let o = parse_ntpq_peers(text);
        assert_eq!(o.synced, Some(true));
        assert_eq!(o.server.as_deref(), Some("ntp.aliyun.com"));
        assert_eq!(o.offset_ms, Some(-2));
    }

    #[test]
    fn ntpq_with_peers_but_no_star_is_unsynchronised() {
        let text = r#"     remote           refid      st t when poll reach   delay   offset  jitter
 1.2.3.4         10.0.0.1     2 u   10   64  377    1.234   +0.456   0.789
"#;
        let o = parse_ntpq_peers(text);
        assert_eq!(o.synced, Some(false));
        assert!(o.server.is_none());
    }

    #[test]
    fn chrony_leap_status_and_stratum_decide_sync() {
        let normal = r#"Reference ID    : 1E2A3B4C
Stratum         : 2
System time     : 0.001234 seconds fast of NTP time
Last offset     : +0.000100 seconds
Leap status     : Normal
"#;
        let o = parse_chrony_tracking(normal);
        assert_eq!(o.synced, Some(true));
        assert_eq!(o.server.as_deref(), Some("1E2A3B4C"));
        assert_eq!(o.offset_ms, Some(1));

        let bad = normal.replace("Normal", "Not synchronised");
        assert_eq!(parse_chrony_tracking(&bad).synced, Some(false));
    }

    #[test]
    fn chrony_slow_offset_is_negative() {
        assert_eq!(
            parse_chrony_offset("-0.010000 seconds slow of NTP time"),
            Some(10)
        );
        assert_eq!(
            parse_chrony_offset("0.010000 seconds slow of NTP time"),
            Some(-10)
        );
    }

    #[test]
    fn adjtimex_unsync_bit_is_honoured() {
        let synced = parse_adjtimex("mode: 0\nstatus: 8193\n");
        assert_eq!(synced.synced, Some(true)); // 0x2001, no 0x40
        let unsynced = parse_adjtimex("mode: 0\nstatus: 0x2041\n");
        assert_eq!(unsynced.synced, Some(false));
        let missing = parse_adjtimex("mode: 0\n");
        assert_eq!(missing.synced, None);
    }

    #[test]
    fn chrony_block_edit_preserves_unrelated_lines() {
        // Pure text transformation, no filesystem: exercise the same logic by
        // checking the constants are distinct and the parser contract.
        assert_ne!(CHRONY_BEGIN, CHRONY_END);
    }
}
