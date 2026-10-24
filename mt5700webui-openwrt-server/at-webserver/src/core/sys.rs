//! Minimal system integration helpers (std only).
//!
//! The backend runs on OpenWrt where UCI is the configuration layer and
//! `ubus`/init scripts are the service layer. These helpers wrap the CLI
//! tools with a hard timeout so a wedged `uci`/`ntpd` can never block a
//! request thread.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct CmdOutput {
    pub ok: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
}

impl CmdOutput {
    pub fn text(&self) -> String {
        let out = self.stdout.trim();
        if !out.is_empty() {
            out.to_string()
        } else {
            self.stderr.trim().to_string()
        }
    }

    pub fn error(&self) -> String {
        if self.timed_out {
            "command timed out".to_string()
        } else {
            let e = self.stderr.trim();
            if e.is_empty() {
                self.stdout.trim().to_string()
            } else {
                e.to_string()
            }
        }
    }
}

/// Run a command with a bounded wall-clock time, capturing stdout/stderr.
pub fn run(cmd: &str, args: &[&str], timeout: Duration) -> CmdOutput {
    let mut command = Command::new(cmd);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            return CmdOutput {
                ok: false,
                code: None,
                stdout: String::new(),
                stderr: e.to_string(),
                timed_out: false,
            }
        }
    };
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if Instant::now() >= deadline {
                    // Kill the direct child. Descendants that inherited the
                    // pipe are not waited for: the reader below is skipped on
                    // timeout, so a wrapper's `sleep` can never extend the
                    // caller's deadline. Commands run through here are
                    // expected to exit on their own; the helpers that matter
                    // (`uci`, `date`, `ntpd -nq`, `dns.sh`) are the direct
                    // process.
                    let _ = child.kill();
                    let _ = child.wait();
                    timed_out = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return CmdOutput {
                    ok: false,
                    code: None,
                    stdout: String::new(),
                    stderr: e.to_string(),
                    timed_out: false,
                };
            }
        }
    }
    let mut stdout = String::new();
    let mut stderr = String::new();
    // On timeout the pipes may still be held by a process that ignored the
    // group kill; reading them would defeat the bounded-time contract.
    if !timed_out {
        if let Some(mut s) = child.stdout.take() {
            let _ = s.read_to_string(&mut stdout);
        }
        if let Some(mut s) = child.stderr.take() {
            let _ = s.read_to_string(&mut stderr);
        }
    }
    let status = child.wait().ok();
    let code = status.and_then(|s| s.code());
    CmdOutput {
        ok: code == Some(0) && !timed_out,
        code,
        stdout,
        stderr,
        timed_out,
    }
}

/// True when the `uci` CLI exists on this system.
pub fn uci_available() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        Command::new("uci")
            .arg("-q")
            .arg("get")
            .arg("system.system.hostname")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
    })
}

pub fn uci_get(key: &str) -> Option<String> {
    if !uci_available() {
        return None;
    }
    let out = run("uci", &["-q", "get", key], Duration::from_secs(2));
    if !out.ok {
        return None;
    }
    let s = out.stdout.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Read a UCI list (order preserved). `uci get` prints list items
/// space-separated; our list values (hostnames, addresses, encoded server
/// records) never contain whitespace.
pub fn uci_get_list(key: &str) -> Vec<String> {
    uci_get(key)
        .map(|v| v.split_whitespace().map(String::from).collect())
        .unwrap_or_default()
}

/// Raw `uci show <section>` output (all options in one process instead of one
/// `uci get` per option). `None` when UCI is absent or the section is missing.
pub fn uci_show(section: &str) -> Option<String> {
    if !uci_available() {
        return None;
    }
    let out = run("uci", &["-q", "show", section], Duration::from_secs(3));
    if out.ok && !out.stdout.trim().is_empty() {
        Some(out.stdout)
    } else {
        None
    }
}

pub fn uci_set(key: &str, value: &str) -> Result<(), String> {
    let out = run("uci", &["-q", "set", &format!("{}={}", key, value)], Duration::from_secs(3));
    if out.ok {
        Ok(())
    } else {
        Err(out.error())
    }
}

pub fn uci_delete(key: &str) {
    let _ = run("uci", &["-q", "delete", key], Duration::from_secs(3));
}

pub fn uci_add_list(key: &str, value: &str) -> Result<(), String> {
    let out = run(
        "uci",
        &["-q", "add_list", &format!("{}={}", key, value)],
        Duration::from_secs(3),
    );
    if out.ok {
        Ok(())
    } else {
        Err(out.error())
    }
}

/// Replace a list in one logical operation (delete + add each value).
pub fn uci_set_list(key: &str, values: &[String]) -> Result<(), String> {
    uci_delete(key);
    for v in values {
        uci_add_list(key, v)?;
    }
    Ok(())
}

pub fn uci_commit(package: &str) -> Result<(), String> {
    let out = run("uci", &["-q", "commit", package], Duration::from_secs(5));
    if out.ok {
        Ok(())
    } else {
        Err(out.error())
    }
}

pub fn read_file(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Write a small file atomically (tmp + rename). Creates the parent dir.
pub fn write_file_atomic(path: &str, content: &str) -> Result<(), String> {
    let p = Path::new(path);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = format!("{}.tmp", path);
    std::fs::write(&tmp, content).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, p).map_err(|e| e.to_string())
}

/// Process name check without `/proc` parsing dependencies.
pub fn pidof(name: &str) -> bool {
    let out = run("pidof", &[name], Duration::from_secs(2));
    out.ok && !out.stdout.trim().is_empty()
}

pub fn service_running(name: &str) -> bool {
    // ubus is authoritative on OpenWrt (procd services); fall back to pidof.
    let out = run(
        "ubus",
        &["call", "service", "list", &format!("{{\"name\":\"{}\"}}", name)],
        Duration::from_secs(2),
    );
    if out.ok {
        let t = out.stdout.trim();
        if !t.is_empty() && t != "{}" {
            return true;
        }
    }
    pidof(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_timeout_is_enforced() {
        // `sh -c 'sleep 5'` must be killed well before its natural end.
        let started = Instant::now();
        let out = run("sh", &["-c", "sleep 5"], Duration::from_millis(200));
        assert!(out.timed_out);
        assert!(!out.ok);
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn output_and_exit_code_are_captured() {
        let out = run("sh", &["-c", "printf hello; exit 3"], Duration::from_secs(2));
        assert_eq!(out.stdout, "hello");
        assert_eq!(out.code, Some(3));
        assert!(!out.ok);
    }

    #[test]
    fn atomic_write_round_trip() {
        let path = "/tmp/mt5700m-sys-test/state";
        write_file_atomic(path, "a=1\n").unwrap();
        assert_eq!(read_file(path).as_deref(), Some("a=1\n"));
        let _ = std::fs::remove_dir_all("/tmp/mt5700m-sys-test");
    }
}
