//! NTP time synchronisation for the MT5700M plugin.
//!
//! The plugin deliberately does **not** implement NTP. It configures and
//! observes the time service the system already runs (`sysntpd` on stock
//! OpenWrt, `chrony` when installed), and offers a bounded manual sync for
//! diagnostics. On a system with no NTP daemon it never starts a competing
//! clock controller; manual sync uses a one-shot `ntpd -nq` query.
//!
//! The clock itself is always read through `core::time`; the timezone comes
//! from the device configuration and is never hardcoded here. NTP failure is
//! recorded and surfaced, never allowed to fail plugin startup, dialing or the
//! AT scheduler.

pub mod api;
pub mod config;
pub mod provider;
pub mod service;
pub mod state;

pub const STATE_FILE: &str = "/var/run/mt5700m/ntp.state";

pub const DEFAULT_SERVERS: [&str; 3] = [
    "ntp.tencent.com",
    "time.cloudflare.com",
    "ntp.aliyun.com",
];

pub const MAX_SERVERS: usize = 16;
