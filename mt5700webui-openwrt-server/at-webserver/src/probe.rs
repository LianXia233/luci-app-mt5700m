//! `at-webserver atprobe` — direct AT port diagnostic.
//!
//! Opens the MT5700M PCUI port with exactly the same termios setup the daemon
//! uses, writes one AT command and reports what came back. Nothing else runs:
//! no arbiter, no snapshot collectors, no WebSocket, no control socket. So a
//! timeout here is a property of the modem/port, not of channel contention.
//!
//! Usage: `at-webserver atprobe [device] [command...]`
//! Default device is resolved the same way the daemon resolves it, and the
//! default command list keeps every probe read-only — `AT+CGSN` in particular
//! only *reads* the IMEI and never writes it.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Read-only probes. Deliberately excludes every command that can change
/// stored state, and never includes an IMEI write of any kind.
const DEFAULT_COMMANDS: &[&str] = &["AT", "AT+CSQ", "AT+COPS?", "AT+CGSN", "AT^HCSQ?"];

#[cfg(target_os = "linux")]
fn raw_fd(f: &std::fs::File) -> Option<i32> {
    use std::os::unix::io::AsRawFd;
    Some(f.as_raw_fd())
}

#[cfg(not(target_os = "linux"))]
fn raw_fd(_f: &std::fs::File) -> Option<i32> {
    None
}

#[cfg(target_os = "linux")]
fn set_modem_lines(fd: i32, on: bool) {
    crate::serial::set_modem_lines(fd, on)
}

#[cfg(not(target_os = "linux"))]
fn set_modem_lines(_fd: i32, _on: bool) {}

pub fn run(args: &[String]) -> i32 {
    let (device, commands): (String, Vec<String>) = match args.first() {
        Some(d) if d.starts_with('/') => (
            d.clone(),
            args[1..].iter().map(|s| s.as_str().to_string()).collect(),
        ),
        _ => (
            crate::serial::auto_detect_serial()
                .or_else(|| Some("/dev/ttyUSB1".to_string()))
                .unwrap_or_default(),
            if args.is_empty() {
                DEFAULT_COMMANDS.iter().map(|s| s.to_string()).collect()
            } else {
                args.iter().map(|s| s.as_str().to_string()).collect()
            },
        ),
    };

    if device.is_empty() {
        eprintln!("atprobe: no serial device found");
        return 2;
    }
    println!("atprobe: device={}", device);

    let mut port = match crate::serial::open_serial_exclusive(&device) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("atprobe: open failed: {}", e);
            return 2;
        }
    };
    let Ok(rd) = port.try_clone() else {
        eprintln!("atprobe: could not clone descriptor");
        return 2;
    };

    // Drain the reader in the background so a URC flood cannot block us and
    // so bytes are already buffered when the response lands.
    let buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (b, s) = (buf.clone(), stop.clone());
    let reader = std::thread::spawn(move || {
        let mut rd = rd;
        let mut chunk = [0u8; 512];
        while !s.load(std::sync::atomic::Ordering::Relaxed) {
            match rd.read(&mut chunk) {
                Ok(0) => std::thread::sleep(Duration::from_millis(20)),
                Ok(n) => b.lock().unwrap().extend_from_slice(&chunk[..n]),
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                // O_NONBLOCK + VMIN=1 => idle reads report EAGAIN/WouldBlock.
                // That is the normal idle state, so keep draining instead of
                // dropping the reader.
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
    });

    let mut failures = 0;
    for cmd in &commands {
        // Settle between commands: the port may still be delivering URCs.
        std::thread::sleep(Duration::from_millis(300));
        buf.lock().unwrap().clear();

        // Many USB modems only accept AT commands once DTR is asserted; the
        // option driver leaves it low after open, so the first command would
        // be swallowed. Toggle it high and give the modem a moment.
        if let Some(fd) = raw_fd(&port) {
            set_modem_lines(fd, true);
            std::thread::sleep(Duration::from_millis(200));
        }

        if port.write_all(cmd.as_bytes()).is_err() || port.write_all(b"\r").is_err() {
            println!("{:<12} WRITE-FAIL", cmd);
            failures += 1;
            continue;
        }
        let _ = port.flush();

        let deadline = Instant::now() + Duration::from_secs(12);
        let mut text = String::new();
        loop {
            let snapshot = buf.lock().unwrap().clone();
            text = String::from_utf8_lossy(&snapshot).to_string();
            if text.contains("OK") || text.contains("ERROR") {
                break;
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        // Drop the URC lines so the verdict reflects the command's own answer.
        let own: Vec<&str> = text
            .lines()
            .filter(|l| !l.trim_start().starts_with('^') || l.contains("HCSQ") || l.contains("COPS"))
            .filter(|l| !l.trim().is_empty())
            .collect();
        let body = own.join(" | ");
        if body.is_empty() {
            let raw = text.len();
            println!("{:<12} TIMEOUT ({} bytes of URC only)", cmd, raw);
            failures += 1;
        } else {
            println!("{:<12} {}", cmd, body);
        }
    }

    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = reader.join();
    if failures == 0 {
        println!("atprobe: all {} probes answered", commands.len());
        0
    } else {
        println!("atprobe: {}/{} probes unanswered", failures, commands.len());
        1
    }
}
