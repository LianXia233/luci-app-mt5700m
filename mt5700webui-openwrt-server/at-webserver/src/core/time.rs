//! Unified clock/timezone interface.
//!
//! Every module that needs "now" or the device timezone reads it here. The
//! rules the NTP module and the pages rely on:
//!
//!   * Unix timestamps are raw (`SystemTime` since epoch) — no offset is
//!     ever added twice;
//!   * the timezone always comes from the device configuration
//!     (`system.system.timezone`, else `/etc/TZ`), never a hardcoded value;
//!   * local display strings are produced by the system `date`, which already
//!     honours the device timezone;
//!   * an unsynchronised clock is still returned (there is no alternative),
//!     but the NTP service marks it untrusted — this module never claims a
//!     sync happened.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Unix seconds.
pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Unix milliseconds (same value as `runtime::now_ms`, exposed here so new
/// time-aware modules do not depend on the task layer).
pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// True when the clock is plausibly past the build era. This is *not* proof
/// of synchronisation; it only rejects 1970/2000 fallback clocks so the UI
/// does not print "1970-01-01" as if it were real.
pub fn clock_plausible() -> bool {
    now_unix() >= 1_600_000_000 // 2020-09-13
}

/// Device timezone as configured (e.g. `CST-8`, `Asia/Shanghai`).
pub fn device_timezone() -> String {
    crate::core::sys::uci_get("system.system.timezone")
        .or_else(|| crate::core::sys::uci_get("system.ntp.timezone"))
        .or_else(|| {
            crate::core::sys::read_file("/etc/TZ")
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "UTC".to_string())
}

/// Local wall-clock string from the system `date` (honours TZ).
pub fn local_time_string() -> String {
    let out = crate::core::sys::run(
        "date",
        &["+%Y-%m-%d %H:%M:%S %z"],
        Duration::from_secs(2),
    );
    if out.ok {
        let s = out.stdout.trim().to_string();
        if !s.is_empty() {
            return s;
        }
    }
    format!("unix:{}", now_unix())
}

/// Format a Unix timestamp in local time using the system `date`.
pub fn format_unix_local(ts: u64) -> String {
    let arg = format!("@{}", ts);
    let out = crate::core::sys::run(
        "date",
        &["-d", &arg, "+%Y-%m-%d %H:%M:%S %z"],
        Duration::from_secs(2),
    );
    if out.ok {
        let s = out.stdout.trim().to_string();
        if !s.is_empty() {
            return s;
        }
    }
    format!("unix:{}", ts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_time_is_sane() {
        assert!(now_unix() > 1_500_000_000);
        assert!(now_unix_ms() > 1_500_000_000_000);
        // The two reads are separate syscalls; allow one second of skew so a
        // tick between them cannot make the suite flaky.
        let delta = now_unix_ms() as i64 / 1000 - now_unix() as i64;
        assert!(delta.abs() <= 1, "seconds and milliseconds disagree by {}", delta);
    }

    #[test]
    fn timezone_is_never_empty() {
        assert!(!device_timezone().trim().is_empty());
    }

    #[test]
    fn local_string_is_never_empty() {
        assert!(!local_time_string().trim().is_empty());
    }
}
