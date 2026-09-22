//! Shared AT channel for the single-binary MT5700M backend.
//!
//! `ubus-at-daemon` and `sms-tool_q` have been removed. The daemon owns the
//! AT serial port exclusively (TIOCEXCL + continuous descriptor). Every other
//! producer — the LuCI `mt5700m-at` CLI and the WebUI — reaches the modem
//! through that daemon, either by the control socket (CLI) or the WebSocket
//! (WebUI). This module is the CLI/transport layer:
//!
//!   * cascades a command: **control socket** → **direct serial (exclusive)**
//!     → **network AT endpoint**;
//!   * drives serial auto-scan discovery and manual port selection (`at_port`);
//!   * performs exclusive serial open (via `serial`), used when the daemon is
//!     not running or for the CLI's own one-shot transport.

use crate::serial;
use crate::sock;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

fn has_anchored_terminator(buffer: &str) -> bool {
    for raw in buffer.split('\n') {
        let line = raw.trim_end_matches('\r').trim_end();
        match line {
            "OK" | "ERROR" | "+CME ERROR" | "+CMS ERROR" => return true,
            l if l.starts_with("+CME ERROR:") || l.starts_with("+CMS ERROR:") => return true,
            _ => {}
        }
    }
    false
}

pub fn sanitize_command(cmd: &str) -> String {
    cmd.chars()
        .filter(|c| *c != '\0' && *c != '\r' && *c != '\n')
        .collect()
}

// ---------------------------------------------------------------- Serial

/// Port of `at_serial_cmd()`. Opens the device exclusively, sends the command
/// and reads until a final result line appears or the timeout elapses.
pub fn serial_sendat(device: &str, timeout_s: u64, command: &str) -> Result<String, AtError> {
    let mut port = serial::open_serial_exclusive(device).map_err(|_| AtError::NoSerialPort)?;

    let buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let rd = port.try_clone().map_err(|_| AtError::NoSerialPort)?;
    let read_handle = spawn_reader(rd, buffer.clone(), stop.clone());

    // Drain stale bytes for 1s like `run_with_timeout 1 cat`.
    std::thread::sleep(Duration::from_secs(1));
    buffer.lock().unwrap().clear();

    let wire = format!("{}\r", command);
    if port.write_all(wire.as_bytes()).is_err() {
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = read_handle.join();
        return Err(AtError::SerialTimeout(String::new()));
    }
    let _ = port.flush();

    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_s);
    loop {
        let snapshot = {
            let buf = buffer.lock().unwrap();
            String::from_utf8_lossy(&buf).replace('\r', "")
        };
        if has_anchored_terminator(&snapshot) {
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            let _ = read_handle.join();
            if !response_ok(&snapshot) {
                return Err(AtError::ModemError(snapshot));
            }
            return Ok(snapshot);
        }
        if std::time::Instant::now() >= deadline {
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            return Err(AtError::SerialTimeout(snapshot));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Send an already-encoded SMS-SUBMIT PDU over an exclusive direct serial
/// port: two-phase `AT+CMGS` (command, wait `>`, payload + 0x1A, wait result).
/// Used by the CLI when the daemon is unavailable.
pub fn serial_cmgs(
    device: &str,
    timeout_s: u64,
    pdu: &crate::sms::SmsPdu,
) -> Result<String, AtError> {
    let mut port = serial::open_serial_exclusive(device).map_err(|_| AtError::NoSerialPort)?;
    let buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let rd = port.try_clone().map_err(|_| AtError::NoSerialPort)?;
    let read_handle = spawn_reader(rd, buffer.clone(), stop.clone());

    std::thread::sleep(Duration::from_secs(1));
    buffer.lock().unwrap().clear();

    // 1) Switch to PDU mode; wait for OK.
    if port.write_all(b"AT+CMGF=0\r").is_err() {
        return Err(AtError::SerialTimeout(String::new()));
    }
    let _ = port.flush();
    wait_for(
        &port, &buffer, &stop, timeout_s,
        |t| t.lines().any(|l| l.trim() == "OK"),
    )?;
    buffer.lock().unwrap().clear();

    // 2) Start CMGS with the user-data length; modem replies with '>'.
    if port
        .write_all(format!("AT+CMGS={}\r", pdu.length).as_bytes())
        .is_err()
    {
        return Err(AtError::SerialTimeout(String::new()));
    }
    let _ = port.flush();
    wait_for(&port, &buffer, &stop, timeout_s, |t| {
        t.lines().any(|l| l.trim() == ">") || has_anchored_terminator(t)
    })?;

    // 3) Send the hex PDU and the SUB (CTRL-Z) terminator.
    let wire = format!("{}\u{1a}", pdu.hex);
    if port.write_all(wire.as_bytes()).is_err() {
        return Err(AtError::SerialTimeout(String::new()));
    }
    let _ = port.flush();

    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_s + 10);
    let snapshot = loop {
        let snap = {
            let buf = buffer.lock().unwrap();
            String::from_utf8_lossy(&buf).replace('\r', "")
        };
        let done = snap.lines().any(|l| {
            let t = l.trim();
            t == "OK" || t == "ERROR" || t.starts_with("+CME ERROR") || t.starts_with("+CMS ERROR")
        });
        if done || std::time::Instant::now() >= deadline {
            break snap;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = read_handle.join();
    if !response_ok(&snapshot) {
        return Err(AtError::ModemError(snapshot));
    }
    Ok(snapshot)
}

/// Poll `buffer` until `pred` holds, an anchored terminator appears, or the
/// timeout elapses. `writer` is unused except to keep the TTY write held open.
#[allow(clippy::type_complexity)]
fn wait_for<F>(
    _writer: &std::fs::File,
    buffer: &Arc<Mutex<Vec<u8>>>,
    stop: &Arc<std::sync::atomic::AtomicBool>,
    timeout_s: u64,
    pred: F,
) -> Result<(), AtError>
where
    F: Fn(&str) -> bool + Send + 'static,
{
    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_s);
    loop {
        let snap = {
            let buf = buffer.lock().unwrap();
            String::from_utf8_lossy(&buf).replace('\r', "")
        };
        if pred(&snap) || has_anchored_terminator(&snap) {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            return Err(AtError::SerialTimeout(snap));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn spawn_reader(
    mut port: std::fs::File,
    buffer: Arc<Mutex<Vec<u8>>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut chunk = [0u8; 512];
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            match port.read(&mut chunk) {
                Ok(0) => std::thread::sleep(Duration::from_millis(50)),
                Ok(n) => buffer.lock().unwrap().extend_from_slice(&chunk[..n]),
                Err(_) => break,
            }
        }
    })
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
/// reads until a final result or timeout. MANDATORY connect_timeout prevents
/// an unreachable endpoint from freezing the CLI cascade in SYN_SENT.
pub fn network_sendat(
    host: &str,
    port: u16,
    timeout_s: u64,
    command: &str,
) -> Result<String, AtError> {
    let addr = format!("{}:{}", host, port);
    let addrs: Vec<std::net::SocketAddr> = std::net::ToSocketAddrs::to_socket_addrs(&addr)
        .map_err(|_| AtError::NetworkFailed)?
        .collect();
    let mut stream: Option<std::net::TcpStream> = None;
    for a in &addrs {
        if let Ok(s) = std::net::TcpStream::connect_timeout(a, Duration::from_secs(timeout_s.max(1))) {
            stream = Some(s);
            break;
        }
    }
    let mut stream = stream.ok_or(AtError::NetworkFailed)?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    let _ = stream.set_nodelay(true);

    stream
        .write_all(format!("{}\r", command).as_bytes())
        .map_err(|_| AtError::NetworkFailed)?;
    let _ = stream.flush();

    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_s);
    let mut acc: Vec<u8> = Vec::new();
    loop {
        let mut chunk = [0u8; 1024];
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => acc.extend_from_slice(&chunk[..n]),
            Err(_) => {
                if std::time::Instant::now() >= deadline {
                    break;
                }
                continue;
            }
        }
        let text = String::from_utf8_lossy(&acc).replace('\r', "");
        if has_anchored_terminator(&text) || std::time::Instant::now() >= deadline {
            break;
        }
    }

    let text = String::from_utf8_lossy(&acc).replace('\r', "");
    if text.is_empty() {
        return Err(AtError::NetworkFailed);
    }
    if !response_ok(&text) {
        return Err(AtError::ModemError(text));
    }
    Ok(text)
}

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

    match settings.mode {
        Mode::Serial => serial_cascade(settings, &command),
        Mode::Network => network_cascade(settings, &command),
        Mode::Auto => auto_cascade(settings, &command),
    }
}

/// Try the daemon control socket first. `Ok(None)` means "try the next
/// transport", `Ok(Some(text))` means the daemon answered, `Err(e)` a thrown
/// modem/transport failure to surface.
fn daemon_transport(command: &str, timeout: u64) -> Result<Option<String>, AtError> {
    match sock::daemon_send(command, timeout) {
        Ok(text) => {
            if response_ok(&text) {
                Ok(Some(text))
            } else {
                Err(AtError::ModemError(text))
            }
        }
        Err(sock::ControlError::Unavailable) => Ok(None),
        Err(sock::ControlError::BadResponse(e)) => Err(AtError::DaemonFailed(e)),
    }
}

fn serial_cascade(settings: &Settings, command: &str) -> AtOutcome {
    match daemon_transport(command, settings.timeout_s) {
        Ok(Some(text)) => return AtOutcome::ok_text(text),
        Ok(None) => {}
        Err(e) => return fail(e),
    }
    let Some(device) = detect_mt5700m_at_port(settings) else {
        return AtOutcome {
            text: String::new(),
            error: Some(AtError::NoSerialPort),
        };
    };
    match serial_sendat(&device, settings.timeout_s, command) {
        Ok(text) => AtOutcome::ok_text(text),
        Err(e) => fail(e),
    }
}

fn network_cascade(settings: &Settings, command: &str) -> AtOutcome {
    let mut last: Option<AtError> = None;
    for target in network_hosts(settings) {
        match network_sendat(&target, settings.port, settings.timeout_s, command) {
            Ok(text) => return AtOutcome::ok_text(text),
            Err(e @ AtError::ModemError(_)) => {
                let text = match &e {
                    AtError::ModemError(t) => t.clone(),
                    _ => unreachable!(),
                };
                return AtOutcome { text, error: Some(e) };
            }
            Err(e) => last = Some(e),
        }
    }
    fail(last.unwrap_or(AtError::NetworkFailed))
}

fn auto_cascade(settings: &Settings, command: &str) -> AtOutcome {
    if let Ok(Some(text)) = daemon_transport(command, settings.timeout_s) {
        return AtOutcome::ok_text(text);
    }
    // The daemon is unavailable (or failed over); use direct serial, then the
    // network endpoints as a final fallback, matching the shell's rc path.
    if let Some(device) = detect_mt5700m_at_port(settings) {
        match serial_sendat(&device, settings.timeout_s, command) {
            Ok(text) => return AtOutcome::ok_text(text),
            Err(_) => {}
        }
    }
    network_cascade(settings, command)
}

fn fail(e: AtError) -> AtOutcome {
    let text = match &e {
        AtError::ModemError(t) | AtError::SerialTimeout(t) => t.clone(),
        _ => String::new(),
    };
    AtOutcome { text, error: Some(e) }
}

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
pub fn detect_mt5700m_at_port(settings: &Settings) -> Option<String> {
    if !settings.at_port.is_empty() && settings.at_port != "auto" {
        // Manual selection: honour an explicit non-auto path directly.
        return Some(settings.at_port.clone());
    }
    serial::auto_detect_serial()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchored_error_detection() {
        assert!(response_ok("OK\n"));
        assert!(response_ok("+COPS: 0,2,\"46000\",7\r\nOK"));
        assert!(!response_ok("+CME ERROR: 3\n"));
        assert!(!response_ok("ERROR"));
        assert!(!response_ok("+CMS ERROR: 322"));
        // payload containing the word ERROR must not fail
        assert!(response_ok("+CMGL: 1,\"REC READ\",\"ERROR\"\r\nOK"));
        assert!(response_ok("apn=ERROR\nOK"));
    }

    #[test]
    fn terminator_scan() {
        assert!(has_anchored_terminator("AT\nOK"));
        assert!(has_anchored_terminator("+CME ERROR: 100"));
        assert!(!has_anchored_terminator("+CUSD: 0,foo"));
        assert!(!has_anchored_terminator("SOMEERRORLINE"));
    }

    #[test]
    fn sanitize() {
        assert_eq!(sanitize_command("AT^XY=1\r\n"), "AT^XY=1");
        assert_eq!(sanitize_command("AT\0X"), "ATX");
    }

    #[test]
    fn manual_port_honoured() {
        let mut s = Settings::default();
        s.at_port = "/dev/ttyUSB7".into();
        assert_eq!(detect_mt5700m_at_port(&s).as_deref(), Some("/dev/ttyUSB7"));
        s.at_port = "auto".into();
        // auto falls through to filesystem scan (no device on host -> None).
        assert_eq!(detect_mt5700m_at_port(&s), serial::auto_detect_serial());
    }
}