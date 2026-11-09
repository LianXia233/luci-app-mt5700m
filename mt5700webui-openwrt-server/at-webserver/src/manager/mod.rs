//! Connection manager — Rust port of `usr/sbin/mt5700m-manager`.
//!
//! The shell script owned four jobs: reconcile the netifd interfaces, keep the
//! data path healthy (probe, back off, recycle), publish a JSON status for
//! LuCI/rpcd, and account temperature. This module keeps the same CLI verbs and
//! the same on-disk state so the init script, hotplug scripts and rpcd plugin
//! keep working unchanged during and after the migration.
//!
//! External tools are still used on purpose (uci, ubus, ifup/ifdown, ping,
//! nslookup, date, logger). They are the OpenWrt system interface; replacing
//! them with hand-written netlink/UCI code would add risk without changing
//! behaviour. The *logic* — ordering, retries, backoff, budgets — is now Rust.

pub mod health;
pub mod net;
pub mod usb;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::core::sys;

pub const TAG: &str = "mt5700m-manager";

#[derive(Debug, Clone)]
pub struct Config {
    pub state_dir: PathBuf,
    pub lock_dir: PathBuf,
    pub log_file: PathBuf,
    pub at_helper: String,
    pub dns_script: String,
    pub iface4: String,
    pub iface6: String,
    pub log_max_lines: usize,
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            state_dir: PathBuf::from(env_or("MT5700M_MANAGER_STATE", "/var/run/mt5700m")),
            lock_dir: PathBuf::from(env_or(
                "MT5700M_MANAGER_LOCK",
                "/var/lock/mt5700m-manager.lock",
            )),
            log_file: PathBuf::from(env_or("MT5700M_MANAGER_LOG", "/tmp/mt5700m-manager.log")),
            at_helper: env_or("MT5700M_AT_HELPER", "/usr/sbin/mt5700m-at"),
            dns_script: env_or("MT5700M_DNS_SCRIPT", "/usr/share/mt5700m/dns.sh"),
            iface4: "MT5700M".to_string(),
            iface6: "MT5700Mv6".to_string(),
            log_max_lines: env_or("MT5700M_LOG_MAX_LINES", "2000")
                .parse()
                .unwrap_or(2000),
        }
    }

    pub fn state_path(&self, name: &str) -> PathBuf {
        self.state_dir.join(name)
    }
}

pub fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/* ------------------------------------------------------------------ logging */

impl Config {
    pub fn log(&self, message: &str) {
        let _ = fs::create_dir_all(&self.state_dir);
        let stamp = sys::run("date", &["+%Y-%m-%d %H:%M:%S"], Duration::from_secs(2));
        let stamp = stamp.stdout.trim().to_string();
        let line = format!("{} {}\n", stamp, message);
        if let Ok(mut file) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_file)
        {
            use std::io::Write;
            let _ = file.write_all(line.as_bytes());
        }
        self.rotate_log();
        let _ = sys::run(
            "logger",
            &["-t", TAG, "--", message],
            Duration::from_secs(2),
        );
    }

    fn rotate_log(&self) {
        if self.log_max_lines == 0 {
            return;
        }
        let Ok(text) = fs::read_to_string(&self.log_file) else {
            return;
        };
        let lines: Vec<&str> = text.lines().collect();
        if lines.len() <= self.log_max_lines {
            return;
        }
        let keep = &lines[lines.len() - self.log_max_lines..];
        let _ = fs::write(&self.log_file, format!("{}\n", keep.join("\n")));
    }
}

/* --------------------------------------------------------------------- lock */

impl Config {
    /// mkdir-based lock with the same stale-reclaim rules as the shell version.
    pub fn acquire_lock(&self) -> bool {
        if fs::create_dir(&self.lock_dir).is_ok() {
            let _ = fs::write(self.lock_dir.join("pid"), format!("{}\n", std::process::id()));
            return true;
        }

        let pid_path = self.lock_dir.join("pid");
        let owner: Option<u32> = fs::read_to_string(&pid_path)
            .ok()
            .and_then(|s| s.trim().parse().ok());

        match owner {
            None => {
                // A process died between mkdir and writing its pid. Reclaim
                // only after a grace period so a racing creator is not robbed.
                let old = fs::metadata(&self.lock_dir)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| SystemTime::now().duration_since(t).ok())
                    .map(|d| d.as_secs() > 60)
                    .unwrap_or(false);
                if old {
                    let _ = fs::remove_dir_all(&self.lock_dir);
                    if fs::create_dir(&self.lock_dir).is_ok() {
                        let _ = fs::write(&pid_path, format!("{}\n", std::process::id()));
                        return true;
                    }
                }
                false
            }
            Some(pid) => {
                if Path::new(&format!("/proc/{}", pid)).exists() {
                    return false;
                }
                let _ = fs::remove_dir_all(&self.lock_dir);
                if fs::create_dir(&self.lock_dir).is_ok() {
                    let _ = fs::write(&pid_path, format!("{}\n", std::process::id()));
                    true
                } else {
                    false
                }
            }
        }
    }

    pub fn acquire_lock_wait(&self) -> bool {
        for _ in 0..8 {
            if self.acquire_lock() {
                return true;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        false
    }

    pub fn release_lock(&self) {
        let pid_path = self.lock_dir.join("pid");
        let owner: Option<u32> = fs::read_to_string(&pid_path)
            .ok()
            .and_then(|s| s.trim().parse().ok());
        if owner.map(|p| p == std::process::id()).unwrap_or(true) {
            let _ = fs::remove_dir_all(&self.lock_dir);
        }
    }
}

/* ---------------------------------------------------------------------- uci */

pub fn uci_get(key: &str) -> Option<String> {
    let out = sys::run("uci", &["-q", "get", key], Duration::from_secs(5));
    if !out.ok {
        return None;
    }
    let value = out.stdout.trim().to_string();
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

pub fn uci_set(key: &str, value: &str) -> bool {
    sys::run(
        "uci",
        &["-q", "set", &format!("{}={}", key, value)],
        Duration::from_secs(5),
    )
    .ok
}

pub fn uci_delete(key: &str) -> bool {
    sys::run("uci", &["-q", "delete", key], Duration::from_secs(5)).ok
}

pub fn uci_add_list(key: &str, value: &str) -> bool {
    sys::run(
        "uci",
        &["-q", "add_list", &format!("{}={}", key, value)],
        Duration::from_secs(5),
    )
    .ok
}

pub fn uci_commit(config: &str) -> bool {
    sys::run("uci", &["-q", "commit", config], Duration::from_secs(10)).ok
}

/// Set only when different; `changed` records that a commit is required.
pub fn set_if_changed(key: &str, value: &str, changed: &mut bool) -> bool {
    if uci_get(key).as_deref() == Some(value) {
        return true;
    }
    if uci_set(key, value) {
        *changed = true;
        true
    } else {
        false
    }
}

pub fn delete_if_present(key: &str, changed: &mut bool) -> bool {
    if uci_get(key).is_none() {
        return true;
    }
    if uci_delete(key) {
        *changed = true;
        true
    } else {
        false
    }
}

/* -------------------------------------------------------------------- state */

pub fn state_read(path: &Path, default: u64) -> u64 {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.split_whitespace().collect::<String>())
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

pub fn state_write(path: &Path, value: u64) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, format!("{}\n", value));
}

pub fn state_clear(path: &Path) {
    let _ = fs::remove_file(path);
}

/* ------------------------------------------------------------------- policy */

#[derive(Debug, Clone)]
pub struct Policy {
    pub max_retries: u32,
    pub backoff_base: u64,
    pub backoff_max: u64,
    pub round_timeout: u64,
    pub probe_timeout: u64,
    pub ping_host: String,
    pub ping_host6: String,
    pub dns_host: String,
    pub v6_cooldown: u64,
    pub halt_notify: u64,
    pub radio_settle: u64,
}

fn numeric(value: Option<String>, default: u64) -> u64 {
    value
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

impl Policy {
    pub fn load() -> Self {
        let get = |name: &str| uci_get(&format!("mt5700m.recovery.{}", name));
        let mut policy = Self {
            max_retries: numeric(get("max_retries"), 3).max(1) as u32,
            backoff_base: numeric(get("backoff_base"), 60),
            backoff_max: numeric(get("backoff_max"), 900),
            round_timeout: numeric(get("round_timeout"), 90).max(30),
            probe_timeout: numeric(get("probe_timeout"), 5).max(2),
            ping_host: get("ping_host").unwrap_or_else(|| "223.5.5.5".to_string()),
            ping_host6: get("ping_host6").unwrap_or_else(|| "2400:3200::1".to_string()),
            dns_host: get("dns_host").unwrap_or_else(|| "www.baidu.com".to_string()),
            v6_cooldown: numeric(get("v6_cooldown"), 300),
            halt_notify: numeric(get("halt_notify"), 600),
            radio_settle: numeric(get("radio_settle"), 45).max(5),
        };
        if policy.backoff_max < policy.backoff_base {
            policy.backoff_max = policy.backoff_base;
        }
        policy
    }
}

/* ---------------------------------------------------------------- dispatch */

pub fn run(args: &[String]) -> i32 {
    let cfg = Config::from_env();
    match args.first().map(|s| s.as_str()) {
        Some("monitor") => health::monitor(&cfg),
        Some("status") | Some("status-json") | None => {
            println!("{}", health::status_json(&cfg));
            0
        }
        Some("log") => {
            if let Ok(text) = fs::read_to_string(&cfg.log_file) {
                let lines: Vec<&str> = text.lines().collect();
                let start = lines.len().saturating_sub(120);
                println!("{}", lines[start..].join("\n"));
            }
            0
        }
        Some("reset-recovery") => {
            health::reset_recovery(&cfg);
            println!("recovery state cleared (retry counter, backoff and halted flag)");
            0
        }
        Some("refresh-temperature") => {
            if health::refresh_temperature(&cfg) {
                0
            } else {
                1
            }
        }
        Some(action @ ("sync" | "connect" | "disconnect" | "redial" | "down")) => {
            if !cfg.acquire_lock_wait() {
                eprintln!("manager is busy");
                return 2;
            }
            let code = match action {
                "sync" => health::sync_manager(&cfg),
                "connect" => health::connect(&cfg),
                "disconnect" => health::disconnect(&cfg),
                "redial" => health::redial(&cfg),
                _ => {
                    net::bring_down(&cfg);
                    0
                }
            };
            cfg.release_lock();
            code
        }
        Some(other) => {
            eprintln!(
                "Usage: mt5700m-manager {{status-json|sync|connect|disconnect|redial|down|reset-recovery|refresh-temperature|log|monitor}} (got {})",
                other
            );
            64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_falls_back_on_garbage() {
        assert_eq!(numeric(Some("12".into()), 5), 12);
        assert_eq!(numeric(Some("x".into()), 5), 5);
        assert_eq!(numeric(None, 5), 5);
    }

    #[test]
    fn state_roundtrip() {
        let dir = std::env::temp_dir().join(format!("mt5700m-state-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("counter");
        state_write(&path, 7);
        assert_eq!(state_read(&path, 0), 7);
        state_clear(&path);
        assert_eq!(state_read(&path, 3), 3);
    }
}
