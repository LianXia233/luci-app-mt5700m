//! DNS configuration and injection for the plugin-managed interfaces.
//!
//! Scope is deliberately narrow: only `network.MT5700M` (IPv4) and
//! `network.MT5700Mv6` (IPv6) are touched. Global DNS, WAN/LAN, DHCP and
//! routing are never modified. The apply implementation lives in
//! `root/usr/share/mt5700m/dns.sh` so the daemon and the dial manager call
//! the same function at every startup/up path.
//!
//! States are kept separate and visible:
//!   * **saved**   — the plugin UCI section (`mt5700m.dns`),
//!   * **applied** — `network.MT5700M*.dns`/`peerdns` written and committed,
//!   * **verified** — the values read back from UCI after the apply.

pub mod api;
pub mod config;
pub mod service;
pub mod state;


/// Path of the single apply helper (installed with the package).
pub const APPLY_SCRIPT: &str = "/usr/share/mt5700m/dns.sh";
/// Last apply result (tmpfs: a new boot has not applied anything yet).
pub const STATE_FILE: &str = "/var/run/mt5700m/dns.state";

pub const IFACE4: &str = "MT5700M";
pub const IFACE6: &str = "MT5700Mv6";
