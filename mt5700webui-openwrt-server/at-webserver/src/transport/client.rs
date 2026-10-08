//! AT client for the single-binary MT5700M backend.
//!
//! The daemon owns the AT serial port exclusively (`TIOCEXCL`) and is the only
//! process allowed to write to it. This module is the **client** view of that
//! contract:
//!
//!   * `at_cmd` forwards one command to the daemon over the control socket and
//!     returns its answer — there is no direct serial or network fallback, by
//!     design: a second writer on the same tty is not a degraded mode, it is a
//!     broken modem session (and a deadlock against the daemon's lock);
//!   * port discovery here is descriptor-based only (`detect_pcui_port`), so a
//!     client never probes the port the daemon owns;
//!   * USB / gateway facts used by the UI are read from sysfs and the routing
//!     table, not from the modem.

use crate::serial::manager as serial;
use crate::transport::control as sock;
use std::path::Path;
use std::process::{Command, Stdio};

pub const PREFERRED_AT_PORT: &str = "/dev/ttyUSB1";

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    Auto,
    Serial,
    Network,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub enabled: bool,
    pub mode: Mode,
    pub at_port: String,
    pub host: String,
    pub port: u16,
    pub timeout_s: u64,
    /// Allow `status` to spend a few short AT round-trips on values that have
    /// no background collector (active APN, QCI, MSISDN, subscribed rate).
    /// Off by default so callers that need a strictly cache-only, zero-AT read
    /// can opt out; `cmd_status` turns it on.
    pub query_extras: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            enabled: true,
            mode: Mode::Auto,
            at_port: String::new(),
            host: "192.168.8.1".into(),
            port: 20249,
            timeout_s: 8,
            query_extras: false,
        }
    }
}

/// Failure modes. Numeric exit codes match the shell contract (`exit 64`,
/// `return 127`, ...) so LuCI's `fs.exec` callers keep working unchanged.
#[derive(Debug)]
pub enum AtError {
    /// uci mt5700m.settings.enabled != 1 (shell: return 2)
    Disabled,
    /// command sanitized to nothing
    Empty,
    /// daemon answered but reported an internal/transport fault
    DaemonFailed(String),
    /// serial port not found / not writable
    NoSerialPort,
    /// serial read timed out; carries whatever partial output arrived
    SerialTimeout(String),
    /// every network target failed
    NetworkFailed,
    /// transport fine but the modem ended with an anchored ERROR result
    ModemError(String),
}

impl AtError {
    pub fn exit_code(&self) -> i32 {
        match self {
            AtError::Disabled => 2,
            AtError::Empty | AtError::NoSerialPort | AtError::NetworkFailed => 1,
            AtError::DaemonFailed(_) => 1,
            AtError::SerialTimeout(_) => 124,
            AtError::ModemError(_) => 1,
        }
    }

    pub fn message(&self) -> String {
        match self {
            AtError::Disabled => "AT backend disabled (mt5700m.settings.enabled != 1)".into(),
            AtError::Empty => "empty AT command".into(),
            AtError::DaemonFailed(e) => format!("AT daemon failed: {}", e),
            AtError::NoSerialPort => "AT serial port not found".into(),
            AtError::SerialTimeout(_) => "serial response timeout".into(),
            AtError::NetworkFailed => "network AT endpoint unreachable".into(),
            AtError::ModemError(_) => "modem returned ERROR".into(),
        }
    }
}

/// Text + error. The shell always prints whatever response text it obtained
/// before failing, so `text` is populated even on ModemError/SerialTimeout.
#[derive(Debug, Default)]
pub struct AtOutcome {
    pub text: String,
    pub error: Option<AtError>,
}

impl AtOutcome {
    pub fn ok(&self) -> bool {
        self.error.is_none()
    }

    pub fn ok_text(text: impl Into<String>) -> Self {
        AtOutcome {
            text: text.into(),
            error: None,
        }
    }
}

/// Anchored final-result check: only a real result line counts, never the
/// word ERROR inside a payload.
pub fn response_ok(response: &str) -> bool {
    for raw in response.split('\n') {
        let line = raw.trim_start_matches('\r').trim_start();
        let body = line
            .strip_prefix("+CME ")
            .or_else(|| line.strip_prefix("+CMS "))
            .unwrap_or(line);
        if let Some(rest) = body.strip_prefix("ERROR") {
            if rest.is_empty() || rest.starts_with(' ') || rest.starts_with(':') {
                return false;
            }
        }
    }
    true
}


pub fn sanitize_command(cmd: &str) -> String {
    cmd.chars()
        .filter(|c| *c != '\0' && *c != '\r' && *c != '\n')
        .collect()
}

// ---------------------------------------------------------------- Serial

/// `EAGAIN`'s raw errno on Linux. Kept as a helper so the constant lives in
/// one place and the loop above stays readable.
#[cfg(target_os = "linux")]
fn libc_eagain() -> i32 {
    11
}

#[cfg(not(target_os = "linux"))]
fn libc_eagain() -> i32 {
    35
}

/// True when the `uci` binary exists (OpenWrt). Non-OpenWrt hosts fall back
/// to defaults.
pub fn uci_available() -> bool {
    Command::new("uci")
        .arg("-q")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

// ---------------------------------------------------------------- Network

/// Port of `at_network_cmd()` using a native TCP stream. Sends `command\r` and
// ---------------------------------------------------------------- Dispatch

/// One-shot cascade : control socket → direct serial → network. Produces the
/// same stdout text and success/failure semantics as the shell script.
pub fn at_cmd(settings: &Settings, command: &str) -> AtOutcome {
    if !settings.enabled {
        return AtOutcome {
            text: String::new(),
            error: Some(AtError::Disabled),
        };
    }
    let command = sanitize_command(command);
    if command.is_empty() {
        return AtOutcome {
            text: String::new(),
            error: Some(AtError::Empty),
        };
    }
    // SINGLE AT OWNER: the daemon owns the port (exclusively, TIOCEXCL) and
    // this client is only a forwarder. Opening the tty here — even as a
    // "fallback" — would either deadlock on the daemon's lock or, worse, race
    // a second writer onto the modem. If the daemon is not running there is no
    // AT access, and the caller surfaces that as a normal unavailability.
    fail_daemon(&command, settings.timeout_s)
}

/// Forward one command to the daemon over the control socket.
fn fail_daemon(command: &str, timeout_s: u64) -> AtOutcome {
    match sock::daemon_send(command, timeout_s) {
        Ok(text) => {
            if response_ok(&text) {
                AtOutcome::ok_text(text)
            } else {
                AtOutcome {
                    text: text.clone(),
                    error: Some(AtError::ModemError(text)),
                }
            }
        }
        Err(sock::ControlError::Unavailable) => AtOutcome {
            text: String::new(),
            error: Some(AtError::DaemonFailed(
                "后端 at-webserver 未运行（AT 串口唯一所有者）：请先启动 /etc/init.d/at-webserver"
                    .to_string(),
            )),
        },
        Err(sock::ControlError::BadResponse(e)) => AtOutcome {
            text: String::new(),
            error: Some(AtError::DaemonFailed(e)),
        },
    }
}



/// Whether the AT daemon is currently holding the serial port.
///
/// The daemon opens the AT tty with `TIOCEXCL` (exclusive). If it is running,
/// any direct `open()` of the same tty from this process either fails with
/// EBUSY or — far worse — blocks indefinitely inside the tty layer, because
/// the exclusive lock is never released while the daemon lives. Falling back
/// to direct serial *while the daemon owns the port* is therefore not a
/// degraded path, it is a deadlock: `mt5700m-at` hangs forever, rpcd's
/// `fs.exec` has no timeout, and the LuCI page never finishes loading.
///
/// The control socket is the single source of truth: it exists and accepts
/// connections only while the daemon is up. So:
///   * daemon reachable but the request failed -> surface the error, NEVER
///     touch the serial port (it is locked by the daemon);
///   * daemon unreachable (socket gone) -> nobody owns the port, direct
///     serial is safe and keeps the CLI usable standalone.



// ---------------------------------------------------------------- Probing

/// Port of usb.sh `mt5700m_usb_info()`.
pub fn mt5700m_usb_info() -> Option<String> {
    let root = std::env::var("MT5700M_SYSFS_ROOT").unwrap_or_else(|_| "/sys".into());
    let base = format!("{}/bus/usb/devices", root);
    let mut first: Option<String> = None;
    for entry in std::fs::read_dir(&base).ok()?.flatten() {
        let path = entry.path();
        let vendor = read_trim(path.join("idVendor")).map(|v| v.to_lowercase());
        let product = read_trim(path.join("idProduct")).map(|v| v.to_lowercase());
        let (Some(vendor), Some(product)) = (vendor, product) else {
            continue;
        };
        if vendor != serial::QUECTEL_VID {
            continue;
        }
        let state = match product.as_str() {
            "3301" => "normal",
            "3302" => "upgrade",
            "3303" => "dump",
            _ => "unknown",
        };
        let slot = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        if state == "normal" {
            return Some(format!("{}|{}|{}", state, product, slot));
        }
        if first.is_none() {
            first = Some(format!("{}|{}|{}", state, product, slot));
        }
    }
    first
}

fn read_trim(path: std::path::PathBuf) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
}

/// Resolve the AT port: manual `at_port` (if it is a live PCUI or exists) else
/// auto-scan. Mirrors `detect_mt5700m_at_port()`.
pub fn detect_mt5700m_at_port(_settings: &Settings) -> Option<String> {
    // Descriptor-only detection: clients must never probe the port with an AT
    // command, because the daemon owns it exclusively. The daemon itself uses
    // `auto_detect_serial()` (which may probe) before it takes ownership.
    serial::detect_pcui_port().or_else(|| {
        if Path::new(PREFERRED_AT_PORT).exists() {
            Some(PREFERRED_AT_PORT.to_string())
        } else {
            None
        }
    })
}

/// Re-export for the daemon / frontends.
pub fn scan_serial_ports() -> Vec<serial::SerialPortInfo> {
    serial::scan_serial_ports()
}

pub fn auto_detect_serial() -> Option<String> {
    serial::auto_detect_serial()
}

/// Port of `detect_modem_gateway()`.
pub fn detect_modem_gateway() -> Option<String> {
    let default_routes = run_ip("route show default")?;
    if let Some(gw) = extract_via(&default_routes) {
        return Some(gw);
    }
    let routes_10 = run_ip("route show 10.0.0.0/8")?;
    if extract_via(&routes_10).is_some() {
        return Some("10.0.0.1".into());
    }
    None
}

fn run_ip(args: &str) -> Option<String> {
    let out = Command::new("ip")
        .arg("-4")
        .args(args.split_whitespace())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).to_string())
}

fn extract_via(routes: &str) -> Option<String> {
    const DEVS: [&str; 8] = ["eth2", "usb", "wwan", "wwan0", "qmimux", "rmnet", "mhi", "USB"];
    for line in routes.lines() {
        if !DEVS.iter().any(|d| line.contains(&format!(" dev {}", d))) {
            continue;
        }
        let mut words = line.split_whitespace();
        while let Some(w) = words.next() {
            if w == "via" {
                if let Some(gw) = words.next() {
                    return Some(gw.to_string());
                }
            }
        }
    }
    None
}

pub fn network_hosts(settings: &Settings) -> Vec<String> {
    let gateway = detect_modem_gateway().unwrap_or_default();
    let mut hosts: Vec<String> = Vec::new();
    if !gateway.is_empty() {
        hosts.push(gateway.clone());
    }
    if !settings.host.is_empty() && settings.host != gateway {
        hosts.push(settings.host.clone());
    }
    if settings.host != "192.168.8.1" && gateway != "192.168.8.1" {
        hosts.push("192.168.8.1".into());
    }
    if settings.host != "10.0.0.1" && gateway != "10.0.0.1" {
        hosts.push("10.0.0.1".into());
    }
    hosts
}
