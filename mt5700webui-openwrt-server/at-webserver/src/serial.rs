//! Serial device ownership and discovery for the single-binary MT5700M
//! backend.
//!
//! The Rust backend (daemon mode) is the *sole* owner of the AT serial port.
//! It opens the TTY with exclusive semantics, holds the descriptor open
//! continuously, and serves both the WebUI (WebSocket :8765) and the LuCI
//! shell backend (`mt5700m-at`) over a local control socket — there is no
//! shared `ubus-at-daemon` transport anymore. `sms-tool_q` is likewise gone:
//! SMS is built and sent in-process (see `sms.rs`).
//!
//! The port itself is selected in two ways:
//!   * **Manual** — an explicit device path (`serial_port`/`at_port`), and
//!   * **Auto-scan** — enumerate `/dev/ttyUSB*`/`/dev/ttyACM*`, classify each
//!     node from its USB sysfs (VID/PID + interface type) and/or by probing
//!     `AT`, then pick the MT5700M PCUI port (or the first port that answers).

use std::fs::OpenOptions;
use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// VID of Quectel MT5700M USB devices.
pub const QUECTEL_VID: &str = "3466";
/// PID of the MT5700M in normal (functioning) mode.
pub const MT5700M_NORMAL_PID: &str = "3301";
/// The PCUI interface is USB class ff / subclass 06 / protocol 12.
pub const PCUI_SUBCLASS: &str = "06";
pub const PCUI_PROTOCOL: &str = "12";

/// Serial candidates we scan. `/dev/ttyUSB*` is the MT5700M norm; `/dev/ttyACM*`
/// covers modems whose CDC-ACM driver exposes the control channel as ACM nodes.
fn candidate_names() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir("/dev")
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    (n.starts_with("ttyUSB") || n.starts_with("ttyACM"))
                        .then_some(n)
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// Classification result for a single `/dev/...` node.
#[derive(Debug, Clone, Default)]
pub struct SerialPortInfo {
    pub path: String,
    pub vendor: String,
    pub product: String,
    pub interface_class: String,
    pub description: Option<String>,
    pub is_pcui: bool,
    pub answers_at: bool,
    pub state: String, // normal / upgrade / dump / unknown / other
}

fn read_trim(p: &Path) -> Option<String> {
    std::fs::read_to_string(p)
        .ok()
        .map(|s| s.trim().to_string())
}

/// Walk up from a tty sysfs class entry until USB vendor/product are found.
fn usb_device_dir(tty: &str) -> Option<std::path::PathBuf> {
    let name = Path::new(tty)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| tty.to_string());
    let mut path: std::path::PathBuf =
        std::fs::canonicalize(format!("/sys/class/tty/{}/device", name)).ok()?;
    loop {
        if path.join("idVendor").is_file() && path.join("idProduct").is_file() {
            return Some(path);
        }
        if !path.pop() {
            return None;
        }
    }
}

fn usb_interface_dir(tty: &str) -> Option<std::path::PathBuf> {
    let name = Path::new(tty)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| tty.to_string());
    let mut path: std::path::PathBuf =
        std::fs::canonicalize(format!("/sys/class/tty/{}/device", name)).ok()?;
    loop {
        if path.join("bInterfaceClass").is_file() || path.join("interface").is_file() {
            return Some(path);
        }
        if !path.pop() {
            return None;
        }
    }
}

fn is_pcui_interface(tty: &str) -> bool {
    let Some(iface) = usb_interface_dir(tty) else {
        return false;
    };
    let class = read_trim(&iface.join("bInterfaceClass"))
        .map(|v| v.to_lowercase())
        .unwrap_or_default();
    let subclass = read_trim(&iface.join("bInterfaceSubClass"))
        .map(|v| v.to_lowercase())
        .unwrap_or_default();
    let protocol = read_trim(&iface.join("bInterfaceProtocol"))
        .map(|v| v.to_lowercase())
        .unwrap_or_default();
    if class == "ff" && subclass == PCUI_SUBCLASS && protocol == PCUI_PROTOCOL {
        return true;
    }
    // Interface description fallback: "*PC*UI*" / "*PC*-[ ]UI*" (case-insensitive).
    let desc = std::fs::read_to_string(iface.join("interface"))
        .unwrap_or_default()
        .to_lowercase();
    let pc = desc.find("pc");
    let ui = desc.find("ui");
    matches!((pc, ui), (Some(p), Some(u)) if u >= p)
}

fn usb_state(product: &str) -> &'static str {
    match product {
        "3301" => "normal",
        "3302" => "upgrade",
        "3303" => "dump",
        _ => "unknown",
    }
}

/// Lightweight `AT` probe: does sending `AT` yield an `OK` anchored response?
///
/// Used by the auto-scan when sysfs classification is ambiguous (e.g. the
/// kernel has not matched the device so there is no `idVendor`). Probes the
/// port without holding TIOCEXCL so the scan may iterate candidates; the
/// selected port is later re-opened exclusively.
fn at_probe_ok(path: &str, timeout: Duration) -> bool {
    let Ok(mut port) = OpenOptions::new().read(true).write(true).open(path) else {
        return false;
    };
    // Best-effort raw line discipline, mirrors the shell `stty` invocation.
    let _ = std::process::Command::new("stty")
        .args([
            "-F", path, "115200", "raw", "-echo", "-echoe", "-echok", "-echoctl", "-echoke",
            "-ixon", "-ixoff", "min", "0", "time", "5",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    let Ok(rd) = port.try_clone() else {
        return false;
    };
    let buffer: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let buffer_reader = buffer.clone();
    let stop_reader = stop.clone();
    let handle = std::thread::spawn(move || {
        let mut chunk = [0u8; 256];
        let mut rd = rd;
        while !stop_reader.load(std::sync::atomic::Ordering::Relaxed) {
            match rd.read(&mut chunk) {
                Ok(0) => std::thread::sleep(Duration::from_millis(40)),
                Ok(n) => buffer_reader.lock().unwrap().extend_from_slice(&chunk[..n]),
                Err(_) => break,
            }
        }
    });
    use std::io::Write;
    let _ = port.write_all(b"AT\r");
    let _ = port.flush();
    let deadline = Instant::now() + timeout;
    let mut ok = false;
    loop {
        let text: String = {
            let buf = buffer.lock().unwrap();
            String::from_utf8_lossy(&buf).replace('\r', "")
        };
        if text.lines().any(|l| l.trim() == "OK") {
            ok = true;
            break;
        }
        if text.lines().any(|l| {
            let t = l.trim();
            t == "ERROR" || t.starts_with("+CME ERROR") || t.starts_with("+CMS ERROR")
        }) {
            break;
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = handle.join();
    ok
}

/// Enumerate and classify every serial candidate node.
pub fn scan_serial_ports() -> Vec<SerialPortInfo> {
    let mut out = Vec::new();
    for name in candidate_names() {
        let path = format!("/dev/{}", name);
        let mut info = SerialPortInfo {
            path: path.clone(),
            ..Default::default()
        };
        let dev_dir = usb_device_dir(&path);
        if let Some(dir) = &dev_dir {
            info.vendor = read_trim(&dir.join("idVendor"))
                .unwrap_or_default()
                .to_lowercase();
            info.product = read_trim(&dir.join("idProduct"))
                .unwrap_or_default()
                .to_lowercase();
            if info.vendor == QUECTEL_VID {
                info.state = usb_state(&info.product).to_string();
            } else {
                info.state = "other".into();
            }
        } else {
            info.state = "unclassified".into();
        }
        info.is_pcui = is_pcui_interface(&path);
        if let Some(iface) = usb_interface_dir(&path) {
            info.interface_class = read_trim(&iface.join("bInterfaceClass"))
                .unwrap_or_default()
                .to_lowercase();
            info.description =
                std::fs::read_to_string(iface.join("interface")).ok().map(|s| s.trim().to_string());
        }
        // Probe only when we cannot already tell this is our PCUI by sysfs.
        if info.vendor != QUECTEL_VID || info.product != MT5700M_NORMAL_PID {
            info.answers_at = at_probe_ok(&path, Duration::from_millis(1200));
        }
        out.push(info);
    }
    out
}

/// Auto-scan discovery: prefer the explicit MT5700M PCUI (sysfs ff:06:12 +
/// 3466:3301), then any node that answers `AT`.
pub fn auto_detect_serial() -> Option<String> {
    let scanned = scan_serial_ports();
    for p in &scanned {
        if p.is_pcui && p.state == "normal" {
            return Some(p.path.clone());
        }
    }
    for p in &scanned {
        if p.answers_at {
            return Some(p.path.clone());
        }
    }
    for p in &scanned {
        if p.state == "normal" {
            return Some(p.path.clone());
        }
    }
    // Lexical `/dev/ttyUSB1` historical default as a last resort.
    if Path::new("/dev/ttyUSB1").exists() {
        return Some("/dev/ttyUSB1".into());
    }
    None
}

// ---------------------------------------------------------------- Exclusive open
//
// The standard library cannot request `TIOCEXCL` itself, so we reach for the
// libc symbol directly. This adds **no crate dependency** — the Cargo.toml
// stays std-only. On non-Linux targets the ioctl is a no-op so the crate still
// builds (the daemon itself only runs on OpenWrt/Linux).

#[cfg(target_os = "linux")]
fn set_tiocexcl(fd: std::os::fd::RawFd) -> std::io::Result<()> {
    extern "C" {
        fn ioctl(fd: i32, request: u32, ...) -> i32;
    }
    // 0x540C = TIOCEXCL from <asm-generic/ioctls.h>.
    // SAFETY: `fd` is a valid open file descriptor and TIOCEXCL takes no
    // further argument.
    let rc = unsafe { ioctl(fd, 0x540Cu32) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(target_os = "linux"))]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn set_tiocexcl(_fd: i32) -> std::io::Result<()> {
    // The exclusive ioctl does not exist on this host build OS; the TTY is
    // never opened here in tests. On the deployed Linux target the real
    // ioctl runs via the cfg(linux) branch. This keeps `cargo build`/`cargo
    // test` available everywhere without adding external crates.
    Ok(())
}

/// Open a serial device for read/write and request exclusive ownership.
///
/// On Linux this asks the kernel to reject any further `open()` of the node
/// (`TIOCEXCL`), so the daemon truly owns the AT port. Line discipline is
/// applied via `stty` (best-effort, mirrors the previous shell transport).
pub fn open_serial_exclusive(path: &str) -> Result<std::fs::File, String> {
    let port = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|e| format!("open {}: {}", path, e))?;

    let _ = std::process::Command::new("stty")
        .args([
            "-F", path, "115200", "raw", "-echo", "-echoe", "-echok", "-echoctl", "-echoke",
            "-ixon", "-ixoff", "min", "0", "time", "5",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        let fd = port.as_raw_fd();
        if let Err(e) = set_tiocexcl(fd) {
            // TIOCEXCL authority relies on the device node; if the kernel
            // refuses (unsupported driver), opening may still proceed but we
            // surface it so operators can see exclusivity was not granted.
            eprintln!("warning: could not set TIOCEXCL on {}: {}", path, e);
        }
    }
    Ok(port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_returns_any_devices_without_panicking() {
        // On the host (no /dev/ttyUSB*) this must be empty and quiet.
        let scanned = scan_serial_ports();
        assert!(scanned.is_empty());
    }

    #[test]
    fn path_classification_smoke() {
        // Pure helpers on a bogus path must not panic and must not claim PCUI.
        assert!(!is_pcui_interface("/dev/nonexistent0"));
        assert_eq!(usb_state("3301"), "normal");
        assert_eq!(usb_state("3302"), "upgrade");
        assert_eq!(usb_state("3303"), "dump");
        assert_eq!(usb_state("1234"), "unknown");
    }
}