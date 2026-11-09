//! Health evaluation, recovery policy and status publication.
//!
//! The shell manager encoded a lot of field knowledge in comments; the rules
//! preserved here are the ones that matter on a real link:
//!
//! * "up" is not enough — a DHCP client can run without a lease;
//! * an address on either stack means the data path is usable, no probe needed;
//! * without an address, ask the module (`AT^NDISSTATQRY`) before tearing
//!   anything down, and only then fall back to ICMP/DNS probes;
//! * carrier down always means "rebuild the host-side link", whatever the
//!   module believes;
//! * recovery is rate-limited (retry ceiling, exponential backoff, per-round
//!   budget) so a permanently broken link cannot cycle the radio forever.

use std::fs;
use std::time::{Duration, Instant};

use crate::core::json::{self, Value};
use crate::core::sys;
use crate::manager::{net, state_clear, state_read, state_write, usb, Config, Policy};
use std::collections::BTreeMap;

const HEALTHY: &str = "healthy";
const TRANSIENT: &str = "transient";
const UNHEALTHY: &str = "unhealthy";

#[derive(Debug, Clone, Default)]
struct DialStatus {
    stat4: Option<String>,
    stat6: Option<String>,
    err4: Option<String>,
    err6: Option<String>,
    available: bool,
}

impl DialStatus {
    fn detail(&self) -> String {
        if !self.available {
            return "ndisstatqry=unavailable".to_string();
        }
        let mut text = format!(
            "ndisstatqry=v4={}/v6={}",
            self.stat4.as_deref().unwrap_or("?"),
            self.stat6.as_deref().unwrap_or("?")
        );
        if let Some(err) = self.err4.as_deref().filter(|s| !s.is_empty()) {
            text.push_str(&format!(" err4={}", err));
        }
        if let Some(err) = self.err6.as_deref().filter(|s| !s.is_empty()) {
            text.push_str(&format!(" err6={}", err));
        }
        text
    }

    fn up(&self) -> bool {
        self.stat4.as_deref() == Some("1") || self.stat6.as_deref() == Some("1")
    }
}

#[derive(Debug, Clone)]
struct Health {
    state: &'static str,
    detail: String,
    addr4: Option<String>,
    addr6: Option<String>,
    device: Option<String>,
    carrier: bool,
    dial: DialStatus,
    connectivity: String,
}

impl Health {
    fn empty() -> Self {
        Self {
            state: UNHEALTHY,
            detail: String::new(),
            addr4: None,
            addr6: None,
            device: None,
            carrier: false,
            dial: DialStatus::default(),
            connectivity: "notrun".to_string(),
        }
    }

    fn address_line(&self) -> String {
        format!(
            "iface={} device={} ipv4={} ipv6={} carrier={}",
            "MT5700M",
            self.device.as_deref().unwrap_or("none"),
            self.addr4.as_deref().unwrap_or("none"),
            self.addr6.as_deref().unwrap_or("none"),
            self.carrier as u8
        )
    }
}

/* --------------------------------------------------------------- AT helpers */

fn at_command(cfg: &Config, command: &str, timeout_s: u64) -> String {
    let out = sys::run(
        &cfg.at_helper,
        &["command", command],
        Duration::from_secs(timeout_s),
    );
    if !out.ok {
        return String::new();
    }
    out.stdout
}

fn modem_dial_status(cfg: &Config) -> DialStatus {
    let raw = at_command(cfg, "AT^NDISSTATQRY=1", 12);
    let line = raw
        .lines()
        .find(|l| l.trim_start().starts_with("^NDISSTATQRY:"))
        .map(|l| l.trim_start().trim_start_matches("^NDISSTATQRY:").trim());
    let Some(line) = line else {
        return DialStatus::default();
    };
    let fields: Vec<&str> = line.split(',').collect();
    DialStatus {
        stat4: fields.first().map(|s| s.trim().to_string()),
        err4: fields.get(1).map(|s| s.trim().to_string()),
        stat6: fields.get(4).map(|s| s.trim().to_string()),
        err6: fields.get(5).map(|s| s.trim().to_string()),
        available: true,
    }
}

fn modem_connected(cfg: &Config) -> bool {
    let raw = at_command(cfg, "AT^SYSINFOEX", 12);
    raw.lines()
        .find(|l| l.trim_start().starts_with("^SYSINFOEX:"))
        .map(|l| l.trim_start().trim_start_matches("^SYSINFOEX:").trim())
        .map(|v| v.starts_with("2,") || v.starts_with("2 "))
        .unwrap_or(false)
}

fn modem_usb_mode(cfg: &Config) -> Option<u32> {
    let raw = at_command(cfg, "AT^SETMODE?", 12);
    raw.lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_digit()))
        .and_then(|l| l.parse().ok())
}

fn cached_usb_mode(cfg: &Config) -> String {
    let now = crate::manager::now_unix();
    let until = state_read(&cfg.state_path("usb_mode_at"), 0);
    if now < until {
        if let Ok(v) = fs::read_to_string(cfg.state_path("usb_mode")) {
            return v.trim().to_string();
        }
    }
    let value = modem_usb_mode(cfg).map(|v| v.to_string()).unwrap_or_default();
    state_write_text(&cfg.state_path("usb_mode"), &value);
    state_write(&cfg.state_path("usb_mode_at"), now + 60);
    value
}

fn state_write_text(path: &std::path::Path, text: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, format!("{}\n", text));
}

fn ensure_usb_mode(cfg: &Config) -> bool {
    let Some(mode) = modem_usb_mode(cfg) else {
        return false;
    };
    if mode == 4 {
        let _ = fs::remove_file(cfg.state_path("usb_mode_fix_time"));
        return true;
    }
    cfg.log(&format!(
        "usb mode={} not NCM(4); applying AT^SETMODE=4 (takes effect after module reboot)",
        mode
    ));
    let fixed_at = state_read(&cfg.state_path("usb_mode_fix_time"), 0);
    let now = crate::manager::now_unix();
    if now < fixed_at.saturating_add(600) {
        return false;
    }
    let _ = sys::run(
        &cfg.at_helper,
        &["command", "AT^SETMODE=4"],
        Duration::from_secs(12),
    );
    state_write(&cfg.state_path("usb_mode_fix_time"), now);
    false
}

/* -------------------------------------------------------------------- probes */

fn probe_ping(dev: Option<&str>, family: u8, target: &str, timeout: u64) -> bool {
    let limit = timeout * 2 + 2;
    let wait = timeout.to_string();
    let mut args: Vec<String> = Vec::new();
    args.push(format!("-{}", family));
    args.push("-c".to_string());
    args.push("2".to_string());
    args.push("-W".to_string());
    args.push(wait);
    if let Some(dev) = dev {
        args.push("-I".to_string());
        args.push(dev.to_string());
    }
    args.push(target.to_string());
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    sys::run("ping", &refs, Duration::from_secs(limit)).ok
}

fn probe_dns(server: &str, host: &str, timeout: u64) -> bool {
    sys::run(
        "nslookup",
        &[host, server],
        Duration::from_secs(timeout * 2 + 2),
    )
    .ok
}

fn connectivity_probe(policy: &Policy, health: &Health) -> (bool, String) {
    let dns = net::iface_dns("MT5700M")
        .or_else(|| net::iface_dns("MT5700Mv6"))
        .unwrap_or_default();
    let started = Instant::now();
    let budget = Duration::from_secs(policy.probe_timeout * 4);
    let device = health.device.as_deref();

    if probe_ping(device, 4, &policy.ping_host, policy.probe_timeout) {
        return (true, "ping4=ok ping6=skip dns=skip".to_string());
    }
    if started.elapsed() + Duration::from_secs(policy.probe_timeout * 2 + 2) > budget {
        return (false, "ping4=fail ping6=skip dns=skip(probe-budget)".to_string());
    }
    if probe_ping(device, 6, &policy.ping_host6, policy.probe_timeout) {
        return (true, "ping4=fail ping6=ok dns=skip".to_string());
    }
    if dns.is_empty() {
        return (false, "ping4=fail ping6=fail dns=skip(no-modem-dns)".to_string());
    }
    if started.elapsed() + Duration::from_secs(policy.probe_timeout * 2 + 2) > budget {
        return (false, "ping4=fail ping6=fail dns=skip(probe-budget)".to_string());
    }
    if probe_dns(&dns, &policy.dns_host, policy.probe_timeout) {
        return (true, format!("ping4=fail ping6=fail dns=ok(server={})", dns));
    }
    (false, format!("ping4=fail ping6=fail dns=fail(server={})", dns))
}

/* ---------------------------------------------------------------- evaluation */

fn evaluate(cfg: &Config, policy: &Policy) -> Health {
    let mut health = Health::empty();
    health.addr4 = net::iface_address(&cfg.iface4, 4);
    health.addr6 = net::iface_address(&cfg.iface6, 6);
    health.device = net::iface_device(&cfg.iface4).or_else(|| net::iface_device(&cfg.iface6));
    health.carrier = health
        .device
        .as_deref()
        .map(usb::carrier)
        .unwrap_or(false);

    let known = net::ubus_status(&cfg.iface4).is_some() || net::ubus_status(&cfg.iface6).is_some();
    if !known {
        health.state = UNHEALTHY;
        health.detail = format!("{} iface={}/{} absent in netifd", health.address_line(), cfg.iface4, cfg.iface6);
        return health;
    }

    if health.addr4.is_some() || health.addr6.is_some() {
        health.state = HEALTHY;
        health.detail = health.address_line();
        return health;
    }

    health.detail = health.address_line();
    health.dial = modem_dial_status(cfg);
    if health.carrier && health.dial.up() {
        health.state = TRANSIENT;
        health.detail = format!("{} dial={}", health.detail, health.dial.detail());
        return health;
    }
    if !health.dial.available {
        let (ok, detail) = connectivity_probe(policy, &health);
        health.connectivity = detail.clone();
        if ok {
            health.state = TRANSIENT;
            health.detail = format!("{} probe={}", health.detail, detail);
            return health;
        }
    }
    health.state = UNHEALTHY;
    health.detail = format!(
        "{} dial={} probe={}",
        health.detail,
        health.dial.detail(),
        health.connectivity
    );
    health
}

fn health_poll(cfg: &Config) -> Health {
    let mut health = Health::empty();
    health.addr4 = net::iface_address(&cfg.iface4, 4);
    health.addr6 = net::iface_address(&cfg.iface6, 6);
    health.device = net::iface_device(&cfg.iface4).or_else(|| net::iface_device(&cfg.iface6));
    health.carrier = health.device.as_deref().map(usb::carrier).unwrap_or(false);
    if health.addr4.is_some() || health.addr6.is_some() {
        health.state = HEALTHY;
        health.detail = health.address_line();
        return health;
    }
    health.detail = health.address_line();
    health.dial = modem_dial_status(cfg);
    if health.carrier && health.dial.up() {
        health.state = TRANSIENT;
        health.detail = format!("{} dial={}", health.detail, health.dial.detail());
        return health;
    }
    health.state = UNHEALTHY;
    health.detail = format!("{} dial={}", health.detail, health.dial.detail());
    health
}

/* ------------------------------------------------------------------ recovery */

fn failures(cfg: &Config) -> u32 {
    state_read(&cfg.state_path("recovery_failures"), 0) as u32
}

fn clear_failures(cfg: &Config) {
    if failures(cfg) == 0 && !cfg.state_path("recovery_halted").exists() {
        return;
    }
    state_write(&cfg.state_path("recovery_failures"), 0);
    state_clear(&cfg.state_path("recovery_next_attempt"));
    state_clear(&cfg.state_path("recovery_halted"));
    state_clear(&cfg.state_path("recovery_halt_notice"));
}

pub fn reset_recovery(cfg: &Config) {
    clear_failures(cfg);
    state_clear(&cfg.state_path("transient_notified"));
}

fn halt_notice(cfg: &Config, policy: &Policy, count: u32, detail: &str) {
    let now = crate::manager::now_unix();
    let last = state_read(&cfg.state_path("recovery_halt_notice"), 0);
    if now < last {
        return;
    }
    cfg.log(&format!(
        "ERROR: recovery halted after {}/{} consecutive failed rounds; {}",
        count, policy.max_retries, detail
    ));
    cfg.log("ERROR: automatic link rebuild is suspended until manual intervention. Check module registration/APN/signal, then run 'mt5700m-manager redial' or 'mt5700m-manager reset-recovery'");
    state_write(&cfg.state_path("recovery_halt_notice"), now + policy.halt_notify);
}

fn record_failure(cfg: &Config, policy: &Policy, health: &Health) {
    let count = failures(cfg) + 1;
    let now = crate::manager::now_unix();
    state_write(&cfg.state_path("recovery_failures"), count as u64);
    if count >= policy.max_retries {
        state_write(&cfg.state_path("recovery_halted"), now);
        state_clear(&cfg.state_path("recovery_next_attempt"));
        halt_notice(cfg, policy, count, &health.detail);
        return;
    }
    let mut backoff = policy.backoff_base;
    for _ in 1..count {
        backoff = backoff.saturating_mul(2);
    }
    backoff = backoff.min(policy.backoff_max);
    state_write(&cfg.state_path("recovery_next_attempt"), now + backoff);
    cfg.log(&format!(
        "recovery round {}/{} failed; backing off {}s before the next attempt",
        count, policy.max_retries, backoff
    ));
}

fn gate(cfg: &Config, policy: &Policy) -> Result<(), String> {
    if cfg.state_path("recovery_halted").exists() {
        let count = failures(cfg);
        halt_notice(cfg, policy, count, "");
        return Err(format!(
            "halted after {}/{} failed rounds, manual intervention required",
            count, policy.max_retries
        ));
    }
    let next = state_read(&cfg.state_path("recovery_next_attempt"), 0);
    let now = crate::manager::now_unix();
    if now < next {
        return Err(format!(
            "in backoff, next attempt in {}s (retries={}/{})",
            next - now,
            failures(cfg),
            policy.max_retries
        ));
    }
    Ok(())
}

fn note_transient(cfg: &Config, policy: &Policy, detail: &str) {
    let now = crate::manager::now_unix();
    let last = state_read(&cfg.state_path("transient_notified"), 0);
    if now < last {
        return;
    }
    cfg.log(&format!(
        "transient state anomaly: {} - connectivity still works, keeping the link up (retry counter reset)",
        detail
    ));
    state_write(&cfg.state_path("transient_notified"), now + policy.halt_notify);
}

fn recover_interface6(cfg: &Config, policy: &Policy, budget: u64) {
    let pdp = crate::manager::uci_get("mt5700m.connection.pdp_type").unwrap_or_default();
    if pdp == "ip" {
        return;
    }
    let auto = crate::manager::uci_get(&format!("network.{}.auto", cfg.iface6))
        .unwrap_or_else(|| "1".to_string());
    if auto == "0" || net::iface_has_ip(&cfg.iface6, 6) {
        return;
    }
    let now = crate::manager::now_unix();
    let last = state_read(&cfg.state_path("v6_recover_at"), 0);
    if now < last {
        return;
    }
    state_write(&cfg.state_path("v6_recover_at"), now + policy.v6_cooldown);
    cfg.log(&format!(
        "interface {} has no IPv6 address, recycling (next attempt not before {}s)",
        cfg.iface6, policy.v6_cooldown
    ));
    let limit = if budget == 0 { 30 } else { budget };
    let started = Instant::now();
    for attempt in 0..5 {
        if started.elapsed().as_secs() >= limit {
            cfg.log(&format!(
                "interface {} recycling stopped after {}s budget",
                cfg.iface6, limit
            ));
            return;
        }
        let _ = sys::run("ifdown", &[&cfg.iface6], Duration::from_secs(20));
        std::thread::sleep(Duration::from_secs(2));
        let _ = sys::run("ifup", &[&cfg.iface6], Duration::from_secs(30));
        if net::iface_has_ip(&cfg.iface6, 6) {
            cfg.log(&format!("interface {} got IPv6 after {} recycle", cfg.iface6, attempt));
            return;
        }
    }
    cfg.log(&format!("interface {} still has no IPv6 address", cfg.iface6));
}

fn modem_cycle_link(cfg: &Config, policy: &Policy, netdev: &str) {
    let _ = sys::run(
        &cfg.at_helper,
        &["command", "AT^NDISDUP=1,0"],
        Duration::from_secs(10),
    );
    std::thread::sleep(Duration::from_secs(2));
    let _ = sys::run(
        &cfg.at_helper,
        &["command", "AT^NDISDUP=1,1"],
        Duration::from_secs(10),
    );
    for _ in 0..5 {
        if usb::carrier(netdev) {
            return;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    cfg.log("NDIS rebuild did not restore carrier, cycling radio");
    let _ = sys::run(
        &cfg.at_helper,
        &["command", "AT+CFUN=0"],
        Duration::from_secs(10),
    );
    std::thread::sleep(Duration::from_secs(2));
    let _ = sys::run(
        &cfg.at_helper,
        &["command", "AT+CFUN=1"],
        Duration::from_secs(10),
    );
    modem_wait_service(cfg, policy.radio_settle);
}

fn modem_wait_service(cfg: &Config, limit: u64) {
    std::thread::sleep(Duration::from_secs(3));
    let mut waited = 0;
    while waited < limit {
        if modem_connected(cfg) {
            return;
        }
        std::thread::sleep(Duration::from_secs(2));
        waited += 2;
    }
    cfg.log(&format!(
        "module reports no service {}s after the radio cycle",
        limit
    ));
}

fn wait_for_health(cfg: &Config, policy: &Policy, budget: u64, deadline: Instant) -> bool {
    let started = Instant::now();
    let mut health = evaluate(cfg, policy);
    if health.state == HEALTHY || health.state == TRANSIENT {
        return true;
    }
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now()).as_secs();
        if remaining == 0 {
            return false;
        }
        let limit = budget.min(remaining);
        if started.elapsed().as_secs() >= limit {
            return false;
        }
        std::thread::sleep(Duration::from_secs(2));
        health = health_poll(cfg);
        if health.state == HEALTHY || health.state == TRANSIENT {
            return true;
        }
    }
}

fn bring_up_with_recovery(cfg: &Config, policy: &Policy, netdev: &str) -> bool {
    if let Err(reason) = gate(cfg, policy) {
        cfg.log(&format!("recovery skipped: {}", reason));
        return false;
    }
    let round = failures(cfg) + 1;
    let deadline = Instant::now() + Duration::from_secs(policy.round_timeout);
    let start = evaluate(cfg, policy);
    cfg.log(&format!(
        "recovery[start] state={} {} retries={}/{} next=round={}/{} budget={}s action=bring-up",
        start.state, start.detail, failures(cfg), policy.max_retries, round, policy.max_retries, policy.round_timeout
    ));

    net::bring_up(cfg);
    if wait_for_health(cfg, policy, 20, deadline) {
        return true;
    }

    let remaining = deadline.saturating_duration_since(Instant::now()).as_secs();
    if !usb::carrier(netdev) && remaining > 20 {
        cfg.log("recovery[carrier-missing] action=rebuild-modem-link");
        modem_cycle_link(cfg, policy, netdev);
        net::bring_up(cfg);
        if wait_for_health(cfg, policy, 16, deadline) {
            return true;
        }
    }

    let remaining = deadline.saturating_duration_since(Instant::now()).as_secs();
    if remaining > 20 {
        cfg.log("recovery[no-address] action=ifdown-ifup-recycle");
        let _ = sys::run("ifdown", &[&cfg.iface4], Duration::from_secs(30));
        std::thread::sleep(Duration::from_secs(2));
        net::bring_up(cfg);
        if wait_for_health(cfg, policy, 16, deadline) {
            return true;
        }
    }

    let remaining = deadline.saturating_duration_since(Instant::now()).as_secs();
    if !netdev.is_empty() && remaining > 20 {
        cfg.log("recovery[still-no-address] action=rebuild-modem-link(last-resort)");
        modem_cycle_link(cfg, policy, netdev);
        net::bring_up(cfg);
        if wait_for_health(cfg, policy, 16, deadline) {
            return true;
        }
    }

    let final_health = evaluate(cfg, policy);
    record_failure(cfg, policy, &final_health);
    if cfg.state_path("recovery_halted").exists() {
        cfg.log("recovery[halted] action=wait-for-manual-intervention");
    } else {
        cfg.log(&format!(
            "recovery[failed] backoff-until-{}",
            state_read(&cfg.state_path("recovery_next_attempt"), 0)
        ));
    }
    false
}

/* --------------------------------------------------------------- main flows */

fn connection_enabled() -> bool {
    crate::manager::uci_get("mt5700m.connection.enabled")
        .map(|v| v == "1")
        .unwrap_or(true)
}

fn set_connection_enabled(enabled: bool) {
    let mut changed = false;
    crate::manager::set_if_changed(
        "mt5700m.connection.enabled",
        if enabled { "1" } else { "0" },
        &mut changed,
    );
    if changed {
        crate::manager::uci_commit("mt5700m");
    }
}

fn apply_profile(cfg: &Config) -> bool {
    let pdp = match crate::manager::uci_get("mt5700m.connection.pdp_type").as_deref() {
        Some("ip") => "IP",
        Some("ipv6") => "IPV6",
        _ => "IPV4V6",
    };
    let dial_mode = match crate::manager::uci_get("mt5700m.connection.dial_mode").as_deref() {
        Some("0") | Some("1") | Some("2") => {
            crate::manager::uci_get("mt5700m.connection.dial_mode").unwrap()
        }
        _ => "1".to_string(),
    };
    let apn = crate::manager::uci_get("mt5700m.connection.apn").unwrap_or_default();
    let username = crate::manager::uci_get("mt5700m.connection.username").unwrap_or_default();
    let password = crate::manager::uci_get("mt5700m.connection.password").unwrap_or_default();
    let auth = match crate::manager::uci_get("mt5700m.connection.auth").as_deref() {
        Some("pap") => "1",
        Some("chap") => "2",
        _ => "0",
    };
    let out = sys::run(
        &cfg.at_helper,
        &[
            "advanced-set",
            "autodial",
            "1",
            &dial_mode,
            pdp,
            &apn,
            &username,
            &password,
            auth,
        ],
        Duration::from_secs(25),
    );
    out.ok
}

pub fn sync_manager(cfg: &Config) -> i32 {
    let info = usb::usb_info();
    let state = info
        .as_ref()
        .map(|(state, _, _)| state.as_str())
        .unwrap_or("absent");
    if state != "normal" {
        net::bring_down(cfg);
        if info.is_some() {
            cfg.log(&format!("module unavailable state={}", state));
        }
        return 0;
    }

    let policy = Policy::load();
    if connection_enabled() {
        let _ = ensure_usb_mode(cfg);
    }
    if !usb::bind_network_driver() {
        cfg.log("unable to bind MT5700M NCM interface");
    }
    if usb::pcui_port().is_none() {
        let _ = usb::bind_serial_driver();
        std::thread::sleep(Duration::from_secs(1));
    }
    let mut netdev = usb::wait_netdev(5).unwrap_or_default();
    if netdev.is_empty() {
        cfg.log("NCM interface missing, forcing rebind");
        let _ = usb::force_ncm_rebind();
        std::thread::sleep(Duration::from_secs(2));
        netdev = usb::wait_netdev(8).unwrap_or_default();
    }
    if netdev.is_empty() {
        cfg.log("normal-mode module has no network interface");
        return 1;
    }

    net::ensure_network(cfg, &netdev);

    if connection_enabled() {
        let health = evaluate(cfg, &policy);
        if health.state == HEALTHY || health.state == TRANSIENT {
            if health.state == TRANSIENT {
                note_transient(cfg, &policy, &health.detail);
            }
            clear_failures(cfg);
            recover_interface6(cfg, &policy, 0);
        } else if modem_connected(cfg) {
            cfg.log(&format!(
                "modem online but interface not healthy: {}",
                health.detail
            ));
            let _ = bring_up_with_recovery(cfg, &policy, &netdev);
        } else {
            net::bring_up(cfg);
        }
    } else {
        net::bring_down(cfg);
    }

    if cfg.state_path("recovery_halted").exists() {
        return 2;
    }
    0
}

pub fn connect(cfg: &Config) -> i32 {
    set_connection_enabled(true);
    let policy = Policy::load();
    reset_recovery(cfg);
    let rc = sync_manager(cfg);
    if rc != 0 && rc != 2 {
        return 1;
    }
    if !apply_profile(cfg) {
        cfg.log("module profile was not changed; continuing with retained profile");
    }
    if let Some(netdev) = usb::netdev() {
        let _ = bring_up_with_recovery(cfg, &policy, &netdev);
    } else {
        net::bring_up(cfg);
    }
    cfg.log("connection enabled");
    0
}

pub fn disconnect(cfg: &Config) -> i32 {
    set_connection_enabled(false);
    net::bring_down(cfg);
    cfg.log("connection disabled");
    0
}

pub fn redial(cfg: &Config) -> i32 {
    net::bring_down(cfg);
    std::thread::sleep(Duration::from_secs(1));
    set_connection_enabled(true);
    let policy = Policy::load();
    reset_recovery(cfg);
    let rc = sync_manager(cfg);
    if rc != 0 && rc != 2 {
        return 1;
    }
    if !apply_profile(cfg) {
        cfg.log("module profile was not changed during redial");
    }
    if let Some(netdev) = usb::netdev() {
        let _ = bring_up_with_recovery(cfg, &policy, &netdev);
    } else {
        net::bring_up(cfg);
    }
    cfg.log("connection recycled");
    0
}

pub fn monitor(cfg: &Config) -> i32 {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("manager: cannot start tokio runtime: {}", err);
            return 1;
        }
    };
    rt.block_on(async {
        loop {
            let mut interval = 15u64;
            if cfg.acquire_lock() {
                let rc = sync_manager(cfg);
                interval = match rc {
                    0 => 15,
                    2 => 60,
                    _ => 30,
                };
                let _ = refresh_temperature(cfg);
                cfg.release_lock();
            } else {
                // Rate-limit: the monitor ticks every 15 s; logging every miss
                // would fill the 2000-line tmpfs log within hours.
                let now = crate::manager::now_unix();
                let last = state_read(&cfg.state_path("lock_wait_logged"), 0);
                if now.saturating_sub(last) >= 300 {
                    cfg.log("lock held by another instance; skipping this cycle");
                    state_write(&cfg.state_path("lock_wait_logged"), now);
                }
            }
            tokio::time::sleep(Duration::from_secs(interval)).await;
        }
    })
}

/* ------------------------------------------------------------------- status */

pub fn status_json(cfg: &Config) -> String {
    let policy = Policy::load();
    let info = usb::usb_info();
    let state = info
        .as_ref()
        .map(|(s, _, _)| s.as_str())
        .unwrap_or("absent");
    let product = info.as_ref().map(|(_, p, _)| p.clone()).unwrap_or_default();
    let slot = info.as_ref().map(|(_, _, s)| s.clone()).unwrap_or_default();
    let netdev = usb::netdev().unwrap_or_default();
    let at_port = usb::pcui_port().unwrap_or_default();
    let up4 = net::iface_up(&cfg.iface4);
    let up6 = net::iface_up(&cfg.iface6);
    let carrier = !netdev.is_empty() && usb::carrier(&netdev);
    let addr4 = net::iface_address(&cfg.iface4, 4).unwrap_or_default();
    let addr6 = net::iface_address(&cfg.iface6, 6).unwrap_or_default();
    let healthy = !addr4.is_empty() || !addr6.is_empty();
    let metric = crate::manager::uci_get(&format!("network.{}.metric", cfg.iface4))
        .or_else(|| crate::manager::uci_get("mt5700m.connection.metric"))
        .unwrap_or_else(|| "50".to_string());

    let mut map: BTreeMap<String, Value> = BTreeMap::new();
    map.insert(
        "running".into(),
        Value::Bool(connection_enabled() && state == "normal"),
    );
    map.insert(
        "connected".into(),
        Value::Bool((up4 || up6) && carrier),
    );
    map.insert("ipv4_up".into(), Value::Bool(up4));
    map.insert("ipv6_up".into(), Value::Bool(up6));
    map.insert("carrier".into(), Value::Bool(carrier));
    map.insert("usb_state".into(), Value::Str(state.to_string()));
    map.insert("usb_pid".into(), Value::Str(product));
    map.insert("usb_slot".into(), Value::Str(slot));
    map.insert("usb_mode".into(), Value::Str(cached_usb_mode(cfg)));
    map.insert("at_port".into(), Value::Str(at_port));
    map.insert("network".into(), Value::Str(netdev));
    map.insert("interface".into(), Value::Str(cfg.iface4.clone()));
    map.insert("interface6".into(), Value::Str(cfg.iface6.clone()));
    map.insert("mode".into(), Value::Str("NCM / ECM".into()));
    map.insert("metric".into(), Value::Str(metric));
    map.insert(
        "health".into(),
        Value::Str(if healthy { "healthy" } else { "no-address" }.into()),
    );
    map.insert("ipv4_address".into(), Value::Str(addr4));
    map.insert("ipv6_address".into(), Value::Str(addr6));
    map.insert(
        "recovery".into(),
        Value::Str(format!("{}/{}", failures(cfg), policy.max_retries)),
    );
    map.insert(
        "recovery_halted".into(),
        Value::Bool(cfg.state_path("recovery_halted").exists()),
    );
    Value::Obj(map).dump()
}

/* --------------------------------------------------------------------- temp */

pub fn refresh_temperature(cfg: &Config) -> bool {
    let helper = std::path::Path::new(&cfg.at_helper);
    if !helper.exists() {
        return false;
    }
    let out = sys::run(&cfg.at_helper, &["temperature"], Duration::from_secs(5));
    if !out.ok {
        return false;
    }
    let value = out
        .stdout
        .lines()
        .find_map(|l| l.trim().strip_prefix("temperature="))
        .map(|v| v.trim().to_string());
    let Some(value) = value else {
        return false;
    };
    let Ok(number) = value.parse::<f64>() else {
        return false;
    };
    if !(-40.0..=150.0).contains(&number) {
        return false;
    }
    let mut body = String::new();
    for line in out.stdout.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("temp_") || trimmed.starts_with("temperature=") || trimmed.starts_with("temperature_sensor=") {
            body.push_str(trimmed);
            body.push('\n');
        }
    }
    body.push_str(&format!("updated={}\n", crate::manager::now_unix()));
    state_write_text(&cfg.state_path("temperature"), &body);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dial_detail_reports_unavailable() {
        assert_eq!(DialStatus::default().detail(), "ndisstatqry=unavailable");
    }

    #[test]
    fn dial_up_is_either_stack() {
        let d = DialStatus {
            stat4: Some("0".into()),
            stat6: Some("1".into()),
            available: true,
            ..Default::default()
        };
        assert!(d.up());
        assert!(d.detail().contains("v4=0/v6=1"));
    }
}
