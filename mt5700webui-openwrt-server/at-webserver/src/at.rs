//! Shared AT channel: UBUS (via ubus-at-daemon) first, direct serial fallback,
//! then the network AT endpoint (host:20249). This module is the single point
//! where both frontends meet the modem, mirroring the shell `at_cmd()` cascade
//! and the Python `UbusTransport` exactly.

use crate::json;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const UBUS_OBJECT: &str = "at-daemon";
pub const UBUS_METHOD: &str = "sendat";
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
    /// ubus or json tooling absent / at-daemon not registered (shell: 127)
    UbusMissing,
    /// ubus call failed or reported non-success status
    UbusFailed(String),
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
            AtError::UbusMissing => 127,
            AtError::UbusFailed(_) => 1,
            AtError::SerialTimeout(_) => 124,
            AtError::ModemError(_) => 1,
        }
    }

    pub fn message(&self) -> String {
        match self {
            AtError::Disabled => "AT backend disabled (mt5700m.settings.enabled != 1)".into(),
            AtError::Empty => "empty AT command".into(),
            AtError::UbusMissing => "at-daemon ubus object not available".into(),
            AtError::UbusFailed(e) => format!("ubus sendat failed: {}", e),
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

/// Anchored final-result check, port of `at_response_ok()`: only a real
/// result line counts, never the word ERROR inside a payload.
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

// ---------------------------------------------------------------- UBUS

/// Port of `at_ubus_cmd()`. Ok(text) on success; UbusMissing maps to the
/// shell's 127 path (the caller then falls back to direct serial).
pub fn ubus_sendat(settings: &Settings, device: &str, command: &str) -> Result<String, AtError> {
    if Command::new("ubus")
        .arg("list")
        .arg(UBUS_OBJECT)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_err()
    {
        return Err(AtError::UbusMissing);
    }

    let mut payload: std::collections::BTreeMap<String, json::Value> =
        std::collections::BTreeMap::new();
    payload.insert("at_port".into(), json::str_val(device));
    payload.insert("timeout".into(), json::num_val(settings.timeout_s));
    payload.insert("at_cmd".into(), json::str_val(command));
    let payload = json::Value::Obj(payload).dump();

    let output = Command::new("ubus")
        .args(["call", UBUS_OBJECT, UBUS_METHOD])
        .arg(&payload)
        .output()
        .map_err(|_| AtError::UbusMissing)?;

    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(AtError::UbusFailed(detail.trim().to_string()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed =
        json::parse(stdout.trim()).ok_or_else(|| AtError::UbusFailed("bad ubus json".into()))?;

    let status = parsed.get("status").and_then(|v| v.as_str()).unwrap_or("");
    if status != "success" {
        return Err(AtError::UbusFailed(format!("status={}", status)));
    }
    let response = parsed
        .get("response")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if response.is_empty() {
        return Err(AtError::UbusFailed("empty response".into()));
    }
    if !response_ok(&response) {
        return Err(AtError::ModemError(response));
    }
    Ok(response)
}

// ---------------------------------------------------------------- Serial

/// Port of `at_serial_cmd()`. Opens the device, sends the command and reads
/// until a final result line appears or the timeout elapses.
pub fn serial_sendat(device: &str, timeout_s: u64, command: &str) -> Result<String, AtError> {
    use std::fs::OpenOptions;

    let mut port = OpenOptions::new()
        .read(true)
        .write(true)
        .open(device)
        .map_err(|_| AtError::NoSerialPort)?;

    // Best effort line discipline setup; the shell ignores stty failures too.
    let _ = Command::new("stty")
        .args([
            "-F", device, "115200", "raw", "-echo", "-echoe", "-echok", "-echoctl", "-echoke",
            "-ixon", "-ixoff", "min", "0", "time", "5",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let read_handle = spawn_reader(port.try_clone().expect("clone tty fd"), buffer.clone(), stop.clone());

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
            // Serial timeout is a transport failure, not a modem ERROR; the
            // auto path must try the network endpoint (shell returns 124).
            stop.store(true, std::sync::atomic::Ordering::Relaxed);
            return Err(AtError::SerialTimeout(snapshot));
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

/// Public wrapper used by the CLI text-mode SMS fallback.
pub fn spawn_public_reader(
    port: std::fs::File,
    buffer: Arc<Mutex<Vec<u8>>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
) -> std::thread::JoinHandle<()> {
    spawn_reader(port, buffer, stop)
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

/// Port of `at_network_cmd()` using a native TCP stream instead of spawning
/// nc. Sends `command\r` and reads until a final result or timeout.
pub fn network_sendat(
    host: &str,
    port: u16,
    timeout_s: u64,
    command: &str,
) -> Result<String, AtError> {
    let addr = format!("{}:{}", host, port);
    let mut stream = std::net::TcpStream::connect(&addr).map_err(|_| AtError::NetworkFailed)?;
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
            Ok(0) => break, // remote closed
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

/// One-shot cascade port of `at_cmd()`. Produces the same stdout text and the
/// same success/failure semantics as the shell script.
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

fn serial_cascade(settings: &Settings, command: &str) -> AtOutcome {
    let Some(device) = detect_mt5700m_at_port(settings) else {
        return AtOutcome {
            text: String::new(),
            error: Some(AtError::NoSerialPort),
        };
    };
    match ubus_sendat(settings, &device, command) {
        Ok(text) => AtOutcome::ok_text(text),
        Err(AtError::UbusMissing) => match serial_sendat(&device, settings.timeout_s, command) {
            Ok(text) => AtOutcome::ok_text(text),
            Err(e) => fail(e),
        },
        // A live at-daemon owns the descriptor; ubus failures other than
        // "object missing" are final, exactly like the shell `[ rc -ne 127 ]`.
        Err(e) => fail(e),
    }
}

fn network_cascade(settings: &Settings, command: &str) -> AtOutcome {
    let mut last: Option<AtError> = None;
    for target in network_hosts(settings) {
        match network_sendat(&target, settings.port, settings.timeout_s, command) {
            Ok(text) => return AtOutcome::ok_text(text),
            // The shell's nc path prints the reply text before judging it,
            // so a modem ERROR response must still reach stdout.
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
    if let Some(device) = detect_mt5700m_at_port(settings) {
        match ubus_sendat(settings, &device, command) {
            Ok(text) => return AtOutcome::ok_text(text),
            Err(AtError::UbusMissing) => {
                match serial_sendat(&device, settings.timeout_s, command) {
                    Ok(text) => return AtOutcome::ok_text(text),
                    // Any serial failure (timeout or modem ERROR) falls
                    // through to the network endpoints, matching the shell's
                    // rc != 0 path.
                    Err(_) => {}
                }
            }
            Err(e) => return fail(e),
        }
    }
    network_cascade(settings, command)
}

// Small helpers to keep the cascade functions tidy.
fn fail(e: AtError) -> AtOutcome {
    // The shell prints whatever response text it obtained before failing
    // (ubus response, serial partial read, network reply), so carry the text
    // out for the caller to print.
    let text = match &e {
        AtError::ModemError(t) | AtError::SerialTimeout(t) => t.clone(),
        _ => String::new(),
    };
    AtOutcome { text, error: Some(e) }
}

// ---------------------------------------------------------------- Probing

/// Port of usb.sh `mt5700m_usb_info()`: first normal-state device wins,
/// otherwise the first device of any state.
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
        if vendor != "3466" {
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

/// Walk up from a tty device node's sysfs entry until a USB device directory
/// with idVendor/idProduct is found. Port of `mt5700m_usb_device_dir_for_path`.
fn usb_device_dir_for_path(tty: &str) -> Option<std::path::PathBuf> {
    let mut path = std::fs::canonicalize(format!("/sys/class/tty/{}/device", tty)).ok()?;
    loop {
        if path.join("idVendor").is_file() && path.join("idProduct").is_file() {
            return Some(path);
        }
        if !path.pop() || path.parent().is_none() {
            return None;
        }
    }
}

fn interface_dir_for_tty(tty: &str) -> Option<std::path::PathBuf> {
    let mut path = std::fs::canonicalize(format!("/sys/class/tty/{}/device", tty)).ok()?;
    loop {
        if path.join("bInterfaceClass").is_file() || path.join("interface").is_file() {
            return Some(path);
        }
        if !path.pop() || path.parent().is_none() {
            return None;
        }
    }
}

fn port_belongs_to_normal(tty: &str) -> bool {
    let Some(dir) = usb_device_dir_for_path(tty) else {
        return false;
    };
    read_trim(dir.join("idVendor")).as_deref() == Some("3466")
        && read_trim(dir.join("idProduct")).as_deref() == Some("3301")
}

/// Port of `mt5700m_port_is_pcui()`.
pub fn port_is_pcui(tty: &str) -> bool {
    if !port_belongs_to_normal(tty) {
        return false;
    }
    let Some(iface) = interface_dir_for_tty(tty) else {
        return false;
    };
    let class = read_trim(iface.join("bInterfaceClass"))
        .map(|v| v.to_lowercase())
        .unwrap_or_default();
    let subclass = read_trim(iface.join("bInterfaceSubClass"))
        .map(|v| v.to_lowercase())
        .unwrap_or_default();
    let protocol = read_trim(iface.join("bInterfaceProtocol"))
        .map(|v| v.to_lowercase())
        .unwrap_or_default();
    if class == "ff" && subclass == "06" && protocol == "12" {
        return true;
    }
    // Interface description match: *PC*UI* or *PC*-[space]*UI* (case-insensitive).
    let desc = std::fs::read_to_string(iface.join("interface"))
        .unwrap_or_default()
        .to_lowercase();
    let pc = desc.find("pc");
    let ui = desc.find("ui");
    matches!((pc, ui), (Some(p), Some(u)) if u >= p)
}

/// Port of `mt5700m_pcui_port()`: lexical order of /dev/ttyUSB* like the
/// shell glob, first PCUI port wins.
pub fn mt5700m_pcui_port() -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir("/dev")
        .ok()?
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.starts_with("ttyUSB").then_some(n)
        })
        .collect();
    names.sort();
    for name in names {
        let tty = format!("/dev/{}", name);
        if port_is_pcui(&tty) {
            return Some(tty);
        }
    }
    None
}

/// Port of `detect_mt5700m_at_port()`.
pub fn detect_mt5700m_at_port(settings: &Settings) -> Option<String> {
    if !settings.at_port.is_empty() && port_is_pcui(&settings.at_port) {
        return Some(settings.at_port.clone());
    }
    mt5700m_pcui_port()
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

/// Matches ` dev (eth2|usb|wwan|wwan0|qmimux|rmnet|mhi|USB)` then grabs the
/// value after `via`.
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

/// Port of `network_hosts()`.
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
}
