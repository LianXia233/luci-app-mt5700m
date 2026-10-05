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
    let Ok(mut port) = open_tty_raw(path) else {
        return false;
    };
    // Raw line discipline straight from the kernel: `stty` is a busybox applet
    // and images frequently ship a busybox built without it, which would leave
    // the probe talking to a canonically-buffered TTY and never match `OK`.
    apply_raw_discipline(&port, path);

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
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                // O_NONBLOCK + VMIN=1 => EAGAIN is the idle state, not EOF.
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20));
                }
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
/// Descriptor-only port detection for CLI clients.
///
/// Picks the PCUI interface from the USB descriptors without sending a single
/// AT byte: while the daemon is running it holds the port with `TIOCEXCL`, so
/// a probe from another process would either fail or block. The daemon itself
/// uses `auto_detect_serial()` (which may probe) *before* taking ownership.
pub fn detect_pcui_port() -> Option<String> {
    for name in candidate_names() {
        if is_pcui_interface(&name) {
            return Some(name);
        }
    }
    None
}

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
fn set_tiocexcl(fd: i32) -> std::io::Result<()> {
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

// ------------------------------------------------------------------ Line discipline
//
// The AT port must be a raw 8N1 channel: no canonical line buffering, no echo,
// no CR/NL rewriting, no software flow control. The read discipline is
// VMIN=1 / VTIME=0 on a **non-blocking** descriptor, matching the verified
// `luci-app-mt5700` reference implementation (`serial_linux.rs`).
//
// Do NOT use VMIN=0. On Linux, a read on an idle tty with VMIN=0/VTIME=0
// returns 0 bytes, which every read loop here treats as EOF — so the reader
// thread exits immediately after open and every subsequent AT command times
// out. The reference project carries an explicit warning about exactly this.
//
// The previous implementation shelled out to `stty -F <dev> ...`. That is a
// silent trap on OpenWrt: `stty` is a *busybox applet*, and plenty of images
// ship a busybox built **without** it (`stty: applet not found`). When the
// helper is missing the command fails, the code ignored the status, and the
// TTY kept the kernel default line discipline. The modem then never saw a
// well-formed command — it only emitted its own URCs (`^PDCPDATAINFO:` …) and
// every `AT` read timed out, on both frontends at once.
//
// So we configure the TTY ourselves through `tcsetattr`, keeping the crate
// std-only (same trick as `TIOCEXCL` above): no new dependency, and the
// configuration now actually happens on every image.

#[cfg(target_os = "linux")]
mod termios_raw {
    // `struct termios` layout for the kernel's generic (asm-generic) ABI, which
    // is what OpenWrt's musl/glibc headers describe on arm64 and x86_64.
    // Field order and types follow <bits/termios-c_linux.h> / <asm/termbits.h>.
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct Termios {
        pub c_iflag: u32,
        pub c_oflag: u32,
        pub c_cflag: u32,
        pub c_lflag: u32,
        pub c_line: u8,
        pub c_cc: [u8; 32],
        pub c_ispeed: u32,
        pub c_ospeed: u32,
    }

    // <asm-generic/ioctls.h> / <asm/termbits.h>
    pub const TCGETS: u32 = 0x5401;
    pub const TCSETS: u32 = 0x5402;
    pub const TCFLSH: u32 = 0x540B;
    pub const B115200: u32 = 0o0010001;
    /// TCIOFLUSH — discard both input and output queues.
    pub const TCIOFLUSH: i32 = 2;

    // c_iflag
    pub const IGNBRK: u32 = 0o0000001;
    pub const BRKINT: u32 = 0o0000002;
    pub const PARMRK: u32 = 0o0000010;
    pub const ISTRIP: u32 = 0o0000040;
    pub const INLCR: u32 = 0o0000100;
    pub const IGNCR: u32 = 0o0000200;
    pub const ICRNL: u32 = 0o0000400;
    pub const IXON: u32 = 0o0002000;
    pub const IXOFF: u32 = 0o0010000;
    pub const IXANY: u32 = 0o0040000;
    pub const IMAXBEL: u32 = 0o0200000;
    pub const IUTF8: u32 = 0o0400000;
    // c_oflag
    pub const OPOST: u32 = 0o0000001;
    // c_cflag — the baud selector IS encoded in CBAUD (the low 5 bits). The
    // kernel's divisor table on every ABI we target (including arm64's generic
    // one) contains B115200, so `(c_cflag & !CBAUD) | B115200` is the portable
    // way to ask for 115200. Do NOT route the rate through `c_ispeed` with
    // CBAUD = BOTHER unless the target actually needs a nonstandard divisor:
    // the modem then keeps whatever rate the port last had, and a fresh open
    // frequently lands on B0, which transmits nothing at all.
    pub const CSIZE: u32 = 0o0000060;
    pub const PARENB: u32 = 0o0000400;
    /// Two stop bits — must be cleared for 8N1.
    pub const CSTOPB: u32 = 0o0000100;
    pub const CS8: u32 = 0o0000060;
    pub const CREAD: u32 = 0o0000200;
    pub const CLOCAL: u32 = 0o0004000;
    /// CBAUD mask (the low 5 bits select the divisor table entry).
    pub const CBAUD: u32 = 0o0000017;
    /// "Other" speed: rate carried out-of-band in `c_ispeed`.
    pub const BOTHER: u32 = 16;
    // CRTSCTS is bit 31 (0x80000000) on the generic ABI. Left OFF: the USB
    // serial adapter exposes no RTS/CTS wiring, and asserting RTS blocks the
    // driver's writes, so every AT command would silently never leave the host.
    pub const CRTSCTS: u32 = 0x8000_0000;
    // c_lflag
    pub const IEXTEN: u32 = 0o0100000;
    // c_lflag
    pub const ISIG: u32 = 0o0000001;
    pub const ICANON: u32 = 0o0000002;
    pub const ECHO: u32 = 0o0000010;
    pub const ECHOE: u32 = 0o0000020;
    pub const ECHOK: u32 = 0o0000040;
    pub const ECHONL: u32 = 0o0000100;
    pub const NOFLSH: u32 = 0o0001000;
    pub const ECHOCTL: u32 = 0o0002000;
    pub const ECHOKE: u32 = 0o0004000;
    // c_cc indices
    pub const VMIN: usize = 6;
    pub const VTIME: usize = 5;

    extern "C" {
        fn ioctl(fd: i32, request: u32, ...) -> i32;
    }

    /// Apply a raw 8N1 115200 configuration to `fd`.
    ///
    /// Returns the kernel's error when the ioctl fails so the caller can log
    /// it — a silent failure here is exactly the bug this replaces.
    pub fn configure(fd: i32) -> std::io::Result<()> {
        // Start from what the driver programmed, then *clear* the cooked-mode
        // bits. Setting them (as an earlier revision here did) is exactly
        // backwards: ICRNL/OPOST/ICANON/ECHO are what make a tty swallow and
        // rewrite AT commands, so leaving them on is what silently breaks the
        // port.
        // SAFETY: `fd` is a valid open descriptor and `t` is a properly aligned
        // `Termios` matching the kernel's generic-ABI layout.
        unsafe {
            let mut t: Termios = std::mem::zeroed();
            if ioctl(fd, TCGETS, &mut t as *mut Termios) != 0 {
                return Err(std::io::Error::last_os_error());
            }

            // input: no CR/NL translation, no parity/break mangling, no
            // software flow control (the modem drives the line, not us).
            t.c_iflag &= !(IGNBRK
                | BRKINT
                | PARMRK
                | ISTRIP
                | INLCR
                | IGNCR
                | ICRNL
                | IXON
                | IXOFF
                | IXANY
                | IMAXBEL
                | IUTF8);
            // output: no post-processing (OPOST would turn \n into \r\n).
            t.c_oflag &= !OPOST;
            // local: no echo, no canonical line buffering, no signals, no
            // extended input processing.
            t.c_lflag &= !(ECHO | ECHONL | ICANON | ISIG | IEXTEN | ECHOE | ECHOK | ECHOKE
                | NOFLSH | ECHOCTL);
            // control: 8N1, receiver on, ignore modem control lines, no
            // hardware flow control.
            t.c_cflag &= !(CSIZE | PARENB | CSTOPB | CRTSCTS);
            t.c_cflag |= CS8 | CREAD | CLOCAL;
            // Baud lives in CBAUD. 115200 is in the kernel divisor table on
            // every ABI we build for.
            t.c_cflag = (t.c_cflag & !CBAUD) | B115200;

            // VMIN=1/VTIME=0 together with a NONBLOCK descriptor is the
            // combination that actually works: with no data the read returns
            // -1/EAGAIN, which the reader loop retries. VMIN=0/VTIME=0 instead
            // makes a read on an idle tty return 0 bytes, and every read loop
            // treats 0 as EOF and exits -- which is why a freshly opened MT5700M
            // went silent and every AT command timed out.
            t.c_cc[VMIN] = 1;
            t.c_cc[VTIME] = 0;

            if ioctl(fd, TCSETS, &mut t as *mut Termios) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // Drop whatever the driver buffered while we were configuring: a
            // half-reply from an earlier probe would otherwise be matched
            // against the next command and select the wrong port.
            let _ = ioctl(fd, TCFLSH, TCIOFLUSH as i32);

            if std::env::var_os("AT_DEBUG_TERMIOS").is_some() {
                let mut v: Termios = std::mem::zeroed();
                if ioctl(fd, TCGETS, &mut v as *mut Termios) == 0 {
                    eprintln!(
                        "at-webserver: termios fd {} -> iflag={:#x} oflag={:#x} cflag={:#x} \
                         lflag={:#x} CBAUD={} VMIN={} VTIME={}",
                        fd,
                        v.c_iflag,
                        v.c_oflag,
                        v.c_cflag,
                        v.c_lflag,
                        v.c_cflag & CBAUD,
                        v.c_cc[VMIN],
                        v.c_cc[VTIME],
                    );
                }
            }
        }
        Ok(())
    }
}

/// Put `fd` into raw 8N1 @115200. A no-op on non-Linux hosts (tests).
#[cfg(target_os = "linux")]
fn configure_raw(fd: i32) -> std::io::Result<()> {
    termios_raw::configure(fd)
}

#[cfg(not(target_os = "linux"))]
fn configure_raw(_fd: i32) -> std::io::Result<()> {
    Ok(())
}

/// Put an open TTY into raw 8N1 @115200, falling back to the `stty` applet
/// only when the ioctl is unavailable.
///
/// The fallback exists purely for non-Linux hosts (where the whole path is a
/// no-op anyway); on the OpenWrt target the ioctl always runs, so a missing
/// `stty` applet can no longer leave the line discipline at the kernel
/// default. `path` is only used to build the fallback command line.
fn apply_raw_discipline(port: &std::fs::File, path: &str) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::io::AsRawFd;
        match configure_raw(port.as_raw_fd()) {
            Ok(()) => return,
            Err(e) => eprintln!("warning: tcsetattr on {} failed ({}), trying stty", path, e),
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = port;
    let _ = std::process::Command::new("stty")
        .args([
            "-F", path, "115200", "raw", "-echo", "-echoe", "-echok", "-echoctl", "-echoke",
            "-ixon", "-ixoff", "min", "1", "time", "0",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Assert DTR and RTS on an open TTY.
///
/// The `option` driver leaves both lines low after `open()`, and a good number
/// of USB modems ignore AT commands until DTR is asserted. Doing it here means
/// the wake-up handshake happens on every open, with no external helper and no
/// dependency on the shell environment.
#[cfg(target_os = "linux")]
pub fn set_modem_lines(fd: i32, on: bool) {
    extern "C" {
        fn ioctl(fd: i32, request: u32, ...) -> i32;
    }
    // TIOCMGET = 0x5415, TIOCMSET = 0x5416 (<asm-generic/ioctls.h>).
    const TIOCMGET: u32 = 0x5415;
    const TIOCMSET: u32 = 0x5416;
    const TIOCM_DTR: i32 = 0x002;
    const TIOCM_RTS: i32 = 0x004;
    // SAFETY: `fd` is a valid open descriptor; both ioctls take an int by
    // value, and `bits` is a properly aligned local.
    unsafe {
        let mut bits: i32 = 0;
        if ioctl(fd, TIOCMGET, &mut bits as *mut i32) != 0 {
            return;
        }
        if on {
            bits |= TIOCM_DTR | TIOCM_RTS;
        } else {
            bits &= !(TIOCM_DTR | TIOCM_RTS);
        }
        ioctl(fd, TIOCMSET, bits);
    }
}

/// Open a TTY the way the verified `luci-app-mt5700` reference does:
/// `O_RDWR | O_NOCTTY | O_NONBLOCK` via libc `open(2)`.
///
/// The flags matter:
/// * `O_NOCTTY` — never let the port become the controlling terminal, which
///   would deliver SIGHUP/SIGINT to the daemon on every modem reset.
/// * `O_NONBLOCK` — an idle modem must never park the reader thread inside
///   `read(2)`. Paired with `VMIN=1`/`VTIME=0` the kernel instead returns
///   `-1/EAGAIN`, which the daemon reader loop retries (`daemon::AtClient::spawn_reader`).
///
/// `std::fs::OpenOptions` cannot express `O_NONBLOCK`/`O_NOCTTY` in a portable
/// way, and this crate is intentionally std-only, so the three-line FFI is the
/// cheapest faithful equivalent. Non-Linux hosts fall back to `OpenOptions`.
#[cfg(target_os = "linux")]
fn open_tty_raw(path: &str) -> std::io::Result<std::fs::File> {
    use std::ffi::CString;
    use std::os::unix::io::FromRawFd;

    // <asm-generic/fcntl.h>: these values are identical on arm64/x86_64.
    const O_RDWR: i32 = 0o2;
    const O_NOCTTY: i32 = 0o400;
    const O_NONBLOCK: i32 = 0o4000;
    const O_CLOEXEC: i32 = 0o2000000;

    extern "C" {
        fn open(path: *const u8, flags: i32, ...) -> i32;
    }

    let cpath =
        CString::new(path).map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "path contains NUL"))?;
    // SAFETY: `cpath` is a valid NUL-terminated string that outlives the call;
    // the variadic `mode` argument is not needed because O_CREAT is absent.
    let fd = unsafe { open(cpath.as_ptr() as *const u8, O_RDWR | O_NOCTTY | O_NONBLOCK | O_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` is a fresh, owned descriptor that nothing else will close;
    // ownership transfers into the `File` so it is closed exactly once.
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

#[cfg(not(target_os = "linux"))]
fn open_tty_raw(path: &str) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new().read(true).write(true).open(path)
}

/// Open a serial device for read/write and request exclusive ownership.
///
/// On Linux this asks the kernel to reject any further `open()` of the node
/// (`TIOCEXCL`), so the daemon truly owns the AT port. Line discipline is
/// applied in-process via `tcsetattr` — no `stty` applet required, because
/// OpenWrt images frequently ship a busybox built without it.
pub fn open_serial_exclusive(path: &str) -> Result<std::fs::File, String> {
    let port = open_tty_raw(path).map_err(|e| format!("open {}: {}", path, e))?;

    // Configure line discipline through the kernel, and fall back to `stty`
    // only for exotic setups where the ioctl path is unavailable. The ioctl is
    // authoritative: it is the same call the old `stty raw` ended up making.
    apply_raw_discipline(&port, path);
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::io::AsRawFd;
        let fd = port.as_raw_fd();
        set_modem_lines(fd, true);
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