//! Daemon mode: WebSocket AT server on :8765, protocol-compatible with the
//! Go (v3.0.2) and Python backends. Text frames carry raw AT commands; JSON
//! frames carry auth and pushed URC events. Client reads match responses to
//! commands by order, so the read loop is deliberately serial.
//!
//! Transports: SERIAL (default — the daemon exclusively owns the AT serial
//! port, discovered by auto-scan or chosen by hand) and NETWORK (the modem's
//! own TCP AT endpoint). The LuCI CLI (`mt5700m-at`) reaches the same
//! exclusive serial port through a local Unix control socket. `ubus-at-daemon`
//! and `sms-tool_q` are no longer used.

use crate::transport::client;
use crate::scheduler::arbiter::{AtArbiter, AtRequestSpec, AtResult, AtTransport};
use crate::serial::presence::DeviceMonitor;
use crate::transport::urc::Dispatcher;
use crate::core::error::BackendError;
use crate::state::bus::{Event, EventBus, Subscription, DEFAULT_TOPICS};
use crate::core::json::{self, Value};
use crate::scheduler::plan;
use crate::serial::manager;
use crate::state::cache::StateCache;
use crate::scheduler::jobs::TaskManager;
use crate::transport::ws::{self, WsError};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

const WS_AUTH_TIMEOUT: u64 = 10;
const WS_WRITE_TIMEOUT: u64 = 10;

#[derive(Clone)]
pub struct DaemonConfig {
    pub enabled: bool,
    pub connection_type: String, // SERIAL | NETWORK
    pub network_host: String,
    pub network_port: u16,
    pub serial_port: String,
    pub serial_timeout: u64,
    pub websocket_port: u16,
    pub websocket_auth_key: String,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        DaemonConfig {
            enabled: true,
            connection_type: "SERIAL".into(),
            network_host: "192.168.8.1".into(),
            network_port: 20249,
            serial_port: "auto".into(),
            serial_timeout: 10,
            websocket_port: 8765,
            websocket_auth_key: String::new(),
        }
    }
}

fn uci_get(key: &str) -> Option<String> {
    let out = std::process::Command::new("uci")
        .args(["-q", "get", key])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn as_bool(v: &str) -> bool {
    matches!(v.trim().to_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

pub fn load_config() -> DaemonConfig {
    let mut c = DaemonConfig::default();
    if !client::uci_available() {
        return c;
    }
    let g = |k: &str| uci_get(&format!("at-webserver.config.{}", k));
    if let Some(v) = g("enabled") {
        c.enabled = as_bool(&v);
    }
    if let Some(v) = g("connection_type") {
        c.connection_type = v.to_uppercase();
    }
    if let Some(v) = g("network_host") {
        c.network_host = v;
    }
    if let Some(v) = g("network_port") {
        c.network_port = v.parse().unwrap_or(20249);
    }
    if let Some(v) = g("serial_port") {
        c.serial_port = v;
    }
    // Honor the manual-specific path when the operator selected `custom`.
    if c.serial_port.trim() == "custom" {
        if let Some(custom) = g("serial_port_custom") {
            if !custom.trim().is_empty() {
                c.serial_port = custom;
            }
        }
    }
    if let Some(v) = g("serial_timeout") {
        c.serial_timeout = v.parse().unwrap_or(10);
    }
    if let Some(v) = g("websocket_port") {
        c.websocket_port = v.parse().unwrap_or(8765);
    }
    if let Some(v) = g("websocket_auth_key") {
        c.websocket_auth_key = v;
    }
    c
}

// ---------------------------------------------------------------- Shared AT client

enum Stream {
    None,
    Serial(std::fs::File),
    Tcp(TcpStream),
}

pub struct AtClient {
    config: DaemonConfig,
    lock: Mutex<Stream>,
    /// Byte stream from the SERIAL/NETWORK transports, fed by the reader
    /// thread; drained by command responses and the idle URC monitor.
    rx: Arc<Mutex<VecDeque<u8>>>,
    in_flight: Arc<AtomicBool>,
    /// Serial device this daemon actually owns, reported back to the CLI so
    /// `status` shows the port in use rather than whatever a fresh scan would
    /// pick. Those can differ: the option driver renumbers ttyUSB* on every
    /// USB re-enumeration, and a second port may also answer AT probes while
    /// this process holds TIOCEXCL on the first one.
    attached_port: Option<String>,
}

/// Resolve the serial port for the daemon: honour an explicit path, otherwise
/// fall back to auto-scan discovery.
fn resolve_serial_port(config: &DaemonConfig) -> String {
    let p = config.serial_port.trim();
    if !p.is_empty() && p != "auto" && p != "custom" {
        return p.to_string();
    }
    client::auto_detect_serial().unwrap_or_else(|| client::PREFERRED_AT_PORT.to_string())
}

#[cfg_attr(not(unix), allow(dead_code))]
impl AtClient {
    fn new(config: DaemonConfig) -> Arc<Self> {
        let kind = config.connection_type.clone();
        // The link kind is a system fact the pages display: record it once here
        // (the system module's `system.service_mode` route reads it) instead of
        // letting a frontend ask the modem with `AT+CONNECT?`.
        crate::core::modem::record(&kind);
        let mut attached_port: Option<String> = None;
        let stream = match kind.as_str() {
            "SERIAL" => {
                let path = resolve_serial_port(&config);
                match manager::open_serial_exclusive(&path) {
                    Ok(f) => {
                        eprintln!("at-webserver: attached to serial {}", path);
                        attached_port = Some(path);
                        Stream::Serial(f)
                    }
                    Err(e) => {
                        eprintln!("at-webserver: serial open failed: {}", e);
                        Stream::None
                    }
                }
            }
            "NETWORK" => TcpStream::connect((config.network_host.as_str(), config.network_port))
                .map(Stream::Tcp)
                .unwrap_or(Stream::None),
            _ => {
                eprintln!("at-webserver: unknown connection type '{}'", kind);
                Stream::None
            }
        };
        let client = Arc::new(AtClient {
            config,
            lock: Mutex::new(stream),
            rx: Arc::new(Mutex::new(VecDeque::new())),
            in_flight: Arc::new(AtomicBool::new(false)),
            attached_port,
        });
        client.spawn_reader();
        client
    }

    /// Continuous read thread for the persistent transports. All bytes go
    /// into `rx`; classification happens in `stream_command` (in flight) or
    /// the idle URC monitor (between commands).
    fn spawn_reader(&self) {
        let mut guard = match self.lock.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let read_end: Option<Box<dyn Read + Send>> = match &mut *guard {
            Stream::Serial(f) => f.try_clone().ok().map(|c| Box::new(c) as Box<dyn Read + Send>),
            Stream::Tcp(s) => s.try_clone().ok().map(|c| Box::new(c) as Box<dyn Read + Send>),
            Stream::None => None,
        };
        let mut read_end = match read_end {
            Some(r) => r,
            None => return,
        };
        drop(guard);
        let rx = self.rx.clone();
        thread::spawn(move || {
            let mut chunk = [0u8; 512];
            loop {
                match read_end.read(&mut chunk) {
                    Ok(0) => thread::sleep(Duration::from_millis(100)),
                    Ok(n) => rx.lock().unwrap().extend(chunk[..n].iter().copied()),
                    Err(_) => thread::sleep(Duration::from_millis(200)),
                }
            }
        });
    }

    fn describe(&self) -> &'static str {
        match self.config.connection_type.as_str() {
            "NETWORK" => "NETWORK",
            _ => "SERIAL",
        }
    }


    /// Send one SMS-SUBMIT PDU through the persistent stream (two-phase
    /// `AT+CMGS`). Used by the SMS module's send service through the arbiter, so
    /// the exclusive serial owner performs the whole transaction.
    fn send_pdu(&self, length: usize, hex: &str, timeout: u64) -> Result<String, String> {
        self.in_flight.store(true, Ordering::SeqCst);
        let mut guard = self.lock.lock().map_err(|_| "client lock poisoned")?;
        {
            // 1) PDU mode.
            if let Err(e) = self.write_locked(&mut guard, b"AT+CMGF=0\r") {
                self.in_flight.store(false, Ordering::SeqCst);
                return Err(e);
            }
            let step = self.wait_for(&mut guard, timeout, |t| {
                t.lines().any(|l| l.trim() == "OK")
            })?;
            if !self.step_ok(&step) {
                self.in_flight.store(false, Ordering::SeqCst);
                return Err(self.text(&step));
            }
        }
        {
            // 2) CMGS length; wait for the '>' prompt (or an error).
            let cmd = format!("AT+CMGS={}\r", length);
            if let Err(e) = self.write_locked(&mut guard, cmd.as_bytes()) {
                self.in_flight.store(false, Ordering::SeqCst);
                return Err(e);
            }
            let step = self.wait_for(&mut guard, timeout, |t| {
                t.lines().any(|l| l.trim() == ">")
                    || has_result(&t)
            })?;
            if !self.step_has_prompt(&step) && !self.step_ok(&step) {
                self.in_flight.store(false, Ordering::SeqCst);
                return Err(self.text(&step));
            }
        }
        {
            // 3) Payload + CTRL-Z; expect +CMGS/<mr> then OK.
            let payload = format!("{}\u{1a}", hex);
            if let Err(e) = self.write_locked(&mut guard, payload.as_bytes()) {
                self.in_flight.store(false, Ordering::SeqCst);
                return Err(e);
            }
            let step = self.wait_for(&mut guard, timeout + 10, has_result)?;
            self.in_flight.store(false, Ordering::SeqCst);
            if !self.step_ok(&step) {
                return Err(self.text(&step));
            }
            Ok(step)
        }
    }


    fn write_locked(&self, guard: &mut std::sync::MutexGuard<'_, Stream>, wire: &[u8]) -> Result<(), String> {
        match &mut **guard {
            Stream::Serial(f) => f.write_all(wire).map_err(|e| e.to_string()),
            Stream::Tcp(s) => s.write_all(wire).and_then(|_| s.flush()).map_err(|e| e.to_string()),
            Stream::None => Err("transport not connected".into()),
        }
    }

    fn text(&self, t: &str) -> String {
        if t.is_empty() {
            "transport error".to_string()
        } else {
            t.to_string()
        }
    }

    fn step_ok(&self, t: &str) -> bool {
        t.lines().any(|l| l.trim() == "OK")
    }

    fn step_has_prompt(&self, t: &str) -> bool {
        t.lines().any(|l| l.trim() == ">")
    }

    /// Poll the shared rx queue until `pred` holds / a result line arrives /
    /// timeout. Returns the accumulated text.
    fn wait_for(
        &self,
        _guard: &mut std::sync::MutexGuard<'_, Stream>,
        timeout: u64,
        pred: impl Fn(&str) -> bool,
    ) -> Result<String, String> {
        let deadline = std::time::Instant::now() + Duration::from_secs(timeout.max(2));
        let mut acc = String::new();
        loop {
            let drained: String = {
                let mut rx = self.rx.lock().unwrap();
                let out: Vec<u8> = rx.drain(..).collect();
                String::from_utf8_lossy(&out).replace('\r', "")
            };
            acc.push_str(&drained);
            if pred(&acc) || has_result(&acc) || std::time::Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        Ok(acc)
    }

    fn stream_command(&self, command: &str, timeout: u64) -> Result<String, String> {
        self.in_flight.store(true, Ordering::SeqCst);
        // Drain pending input first so stale bytes do not pollute the reply.
        self.rx.lock().unwrap().clear();
        {
            let mut guard = self.lock.lock().map_err(|_| "client lock poisoned")?;
            let wire = format!("{}\r", command);
            if let Err(e) = self.write_locked(&mut guard, wire.as_bytes()) {
                self.in_flight.store(false, Ordering::SeqCst);
                return Err(e);
            }
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(timeout.max(2));
        let mut acc = String::new();
        loop {
            let drained: String = {
                let mut rx = self.rx.lock().unwrap();
                let out: Vec<u8> = rx.drain(..).collect();
                String::from_utf8_lossy(&out).replace('\r', "")
            };
            acc.push_str(&drained);
            let done = acc.lines().any(|l| {
                let t = l.trim();
                t == "OK" || t == "ERROR" || t.starts_with("+CME ERROR:") || t.starts_with("+CMS ERROR:")
            });
            if done || std::time::Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        self.in_flight.store(false, Ordering::SeqCst);
        Ok(acc)
    }
}

// ---------------------------------------------------------------- AtTransport impl
//
// The daemon transport plugged into the single AT arbiter. Every command in
// the backend (WebSocket, control socket, snapshot collectors, band-lock
// scheduler) now flows through `AtArbiter`, which serialises the channel,
// deduplicates reads and applies timeouts/retries.

impl AtTransport for AtClient {
    fn send(&self, command: &str, timeout: Duration) -> AtResult {
        match self.config.connection_type.as_str() {
            "SERIAL" | "NETWORK" => {
                let text = self
                    .stream_command(command, timeout.as_secs().max(2))
                    .map_err(BackendError::TransportError)?;
                classify_response(&text)
            }
            _ => Err(BackendError::TransportError("unknown connection type".into())),
        }
    }

    fn send_interruptible(
        &self,
        command: &str,
        timeout: Duration,
        cancel: &AtomicBool,
        abort_wire: Option<&[u8]>,
    ) -> AtResult {
        self.in_flight.store(true, Ordering::SeqCst);
        // Drain pending input first so stale bytes do not pollute the reply.
        self.rx.lock().unwrap().clear();
        {
            let mut guard = self
                .lock
                .lock()
                .map_err(|_| BackendError::TransportError("client lock poisoned".into()))?;
            let wire = format!("{}\r", command);
            if let Err(e) = self.write_locked(&mut guard, wire.as_bytes()) {
                self.in_flight.store(false, Ordering::SeqCst);
                return Err(BackendError::TransportError(e));
            }
        }
        let deadline = std::time::Instant::now() + timeout.max(Duration::from_secs(2));
        let mut acc = String::new();
        let mut aborted = false;
        loop {
            // Cooperative cancellation: inject the abort token once, then keep
            // draining until the modem settles (or the deadline expires).
            if cancel.load(Ordering::SeqCst) && !aborted {
                aborted = true;
                if let Some(wire) = abort_wire {
                    if let Ok(mut guard) = self.lock.lock() {
                        let _ = self.write_locked(&mut guard, &[wire, b"\r"].concat());
                    }
                }
            }
            let drained: String = {
                let mut rx = self.rx.lock().unwrap();
                let out: Vec<u8> = rx.drain(..).collect();
                String::from_utf8_lossy(&out).replace('\r', "")
            };
            acc.push_str(&drained);
            let done = has_result(&acc);
            let timed_out = std::time::Instant::now() >= deadline;
            if done || timed_out {
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        self.in_flight.store(false, Ordering::SeqCst);
        if aborted {
            return Err(BackendError::TaskCancelled);
        }
        classify_response(&acc)
    }

    fn connected(&self) -> bool {
        match self.lock.lock() {
            Ok(g) => matches!(&*g, Stream::Serial(_) | Stream::Tcp(_)),
            Err(_) => false,
        }
    }

    fn send_sms_pdu(&self, parts: &[crate::core::channel::SmsPart]) -> Result<String, BackendError> {
        let total = parts.len();
        let mut last = String::new();
        for (i, part) in parts.iter().enumerate() {
            match self.send_pdu(part.length, &part.hex, self.config.serial_timeout) {
                Ok(text) => last = text,
                Err(e) => {
                    // Which part failed is transport knowledge (only this loop
                    // knows), so the index is added here and the message travels
                    // up unchanged to the user.
                    let msg = if total > 1 {
                        format!("第 {}/{} 条发送失败：{}", i + 1, total, e)
                    } else {
                        e
                    };
                    return Err(BackendError::TransportError(msg));
                }
            }
        }
        Ok(last)
    }
}

/// Classify a raw stream reply into Ok(text) / AtRejected / AtTimeout.
/// A reply without any OK/ERROR marker is treated as a timeout so a slow or
/// silent modem can never hang the arbiter.
fn classify_response(text: &str) -> AtResult {
    let t = text.trim();
    if t.is_empty() {
        return Err(BackendError::AtTimeout);
    }
    let rejected = t.lines().any(|l| {
        let l = l.trim();
        l == "ERROR" || l.starts_with("+CME ERROR:") || l.starts_with("+CMS ERROR:")
    });
    if rejected {
        return Err(BackendError::AtRejected(t.to_string()));
    }
    if t.lines().any(|l| l.trim() == "OK") {
        return Ok(text.to_string());
    }
    Err(BackendError::AtTimeout)
}

#[cfg_attr(not(unix), allow(dead_code))]
fn has_result(t: &str) -> bool {
    t.lines().any(|l| {
        let l = l.trim();
        l == "OK" || l == "ERROR" || l.starts_with("+CME ERROR:") || l.starts_with("+CMS ERROR:")
    })
}

// ---------------------------------------------------------------- Pseudo commands

fn ok_response(data: &str) -> Value {
    let mut m: std::collections::BTreeMap<String, Value> = Default::default();
    m.insert("success".to_string(), Value::Bool(true));
    if !data.is_empty() {
        m.insert("data".to_string(), json::str_val(data));
    }
    Value::Obj(m)
}

fn err_response(msg: &str) -> Value {
    let mut m: std::collections::BTreeMap<String, Value> = Default::default();
    m.insert("success".to_string(), Value::Bool(false));
    m.insert("error".to_string(), json::str_val(msg));
    Value::Obj(m)
}

pub fn normalize_syscfgex(command: &str) -> String {
    if !command.starts_with("AT^SYSCFGEX") {
        return command.to_string();
    }
    let cleaned: String = command
        .replace('\r', "")
        .replace('\n', "")
        .replace("OK", "");
    if cleaned.contains(",\"\",\"\"") {
        let parts: Vec<&str> = cleaned.split(',').collect();
        if parts.len() >= 5 {
            let bands = parts[4].trim_matches('"');
            let mut out = parts[..4].join(",");
            out.push_str(",\"");
            out.push_str(bands);
            out.push_str("\",\"\",\"\"");
            return out;
        }
    }
    cleaned
}

fn handle_schedule_command(command: &str) -> Option<Value> {
    let trimmed = command.trim();
    if trimmed == "AT+SCHED?" {
        return Some(ok_response(&sched_json()));
    }
    if let Some(rest) = trimmed.strip_prefix("AT+SCHED=") {
        let payload = rest.trim();
        if payload.is_empty() {
            return Some(err_response("empty schedule payload"));
        }
        let parsed = match json::parse(payload) {
            Some(v) => v,
            None => return Some(err_response("invalid schedule json")),
        };
        let Value::Obj(map) = parsed else {
            return Some(err_response("invalid schedule json"));
        };
        for (key, value) in &map {
            let uci_key = format!("at-webserver.config.{}", key);
            let text = match value {
                Value::Str(s) => s.clone(),
                Value::Bool(b) => b.to_string(),
                Value::Num(n) => n.clone(),
                _ => continue,
            };
            let _ = std::process::Command::new("uci")
                .args(["set", &uci_key, &text])
                .status();
        }
        let _ = std::process::Command::new("uci")
            .args(["commit", "at-webserver"])
            .status();
        return Some(ok_response(""));
    }
    None
}

fn sched_json() -> String {
    // Mirror the Go schedconfig field set.
    let keys = [
        "schedule_enabled",
        "schedule_check_interval",
        "schedule_timeout",
        "schedule_unlock_lte",
        "schedule_unlock_nr",
        "schedule_toggle_airplane",
        "schedule_night_enabled",
        "schedule_night_start",
        "schedule_night_end",
        "schedule_night_lte_type",
        "schedule_night_lte_bands",
        "schedule_night_lte_arfcns",
        "schedule_night_lte_scs_types",
        "schedule_night_lte_pcis",
        "schedule_night_nr_type",
        "schedule_night_nr_bands",
        "schedule_night_nr_arfcns",
        "schedule_night_nr_scs_types",
        "schedule_night_nr_pcis",
        "schedule_day_enabled",
        "schedule_day_lte_type",
        "schedule_day_lte_bands",
        "schedule_day_lte_arfcns",
        "schedule_day_lte_scs_types",
        "schedule_day_lte_pcis",
        "schedule_day_nr_type",
        "schedule_day_nr_bands",
        "schedule_day_nr_arfcns",
        "schedule_day_nr_scs_types",
        "schedule_day_nr_pcis",
    ];
    let mut map = std::collections::BTreeMap::new();
    for key in keys {
        let value = uci_get(&format!("at-webserver.config.{}", key)).unwrap_or_default();
        map.insert(key.to_string(), json::str_val(&value));
    }
    Value::Obj(map).dump()
}

// ---------------------------------------------------------------- Control socket
//
// The LuCI shell backend (`mt5700m-at`) reaches the modem through this local
// Unix socket instead of the removed `ubus-at-daemon`. Every command is
// serialised behind the daemon's exclusive serial lock, so the CLI never
// touches /dev/ttyUSB* itself.

/// Handle one control-socket request line and return the JSON response line.
/// Every AT exchange goes through the single arbiter — the LuCI CLI can
/// never contend with the WebSocket or the background collectors for the
/// serial port.
#[cfg_attr(not(unix), allow(dead_code))]
fn handle_control_request(
    arbiter: &Arc<AtArbiter>,
    cache: &Arc<StateCache>,
    tasks: &Arc<TaskManager>,
    line: &str,
) -> String {
    let parsed = json::parse(line);
    let cmd = parsed
        .as_ref()
        .and_then(|v| v.get("cmd"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let mut ok = std::collections::BTreeMap::new();
    match cmd {
        // Unified API over the control socket: the CLI (and therefore the LuCI
        // dial manager) consumes exactly the same registry as the frontends.
        "api" => {
            let path = parsed
                .as_ref()
                .and_then(|v| v.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if path.is_empty() {
                return err_response("缺少参数 path").dump();
            }
            let params = parsed
                .as_ref()
                .and_then(|v| v.get("params"))
                .cloned()
                .unwrap_or(Value::Null);
            let resp = crate::api::rpc::control_response(
                arbiter,
                cache,
                tasks.bus(),
                Some(tasks),
                path,
                &params,
            );
            return resp.dump();
        }
        "send" => {
            let command = parsed
                .as_ref()
                .and_then(|v| v.get("command"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let timeout = parsed
                .as_ref()
                .and_then(|v| v.get("timeout"))
                .and_then(|v| v.as_u64())
                .unwrap_or(5);
            if command.is_empty() {
                ok.insert("ok".to_string(), Value::Bool(false));
                ok.insert("error".to_string(), json::str_val("empty command"));
            } else {
                // Reads deduplicate with the WebSocket/collector traffic;
                // writes are serialised as interactive actions. Reads use
                // `ui_query` so a slow command is not retried against the
                // shared channel (see AtRequestSpec::ui_query).
                // 读命令闸门。**LuCI 的全部读命令都走这条 control
                // socket**（mt5700m-at -> client::at_cmd ->
                // 这里），不在 run_command 里；漏了这里闸门等于没装 ——
                // 实测表现为 cached 里始终没有 raw: topic。
                match crate::scheduler::gate::gate(&command, cache, tasks) {
                    crate::scheduler::gate::Gate::Cached(text) => {
                        ok.insert("ok".to_string(), Value::Bool(true));
                        ok.insert("response".to_string(), json::str_val(text.trim()));
                        return Value::Obj(ok).dump();
                    }
                    crate::scheduler::gate::Gate::Pending => {
                        ok.insert("ok".to_string(), Value::Bool(true));
                        ok.insert("pending".to_string(), Value::Bool(true));
                        ok.insert("response".to_string(), json::str_val(""));
                        return Value::Obj(ok).dump();
                    }
                    crate::scheduler::gate::Gate::Passthrough => {}
                }
                let mut spec = if command.ends_with('?') {
                    AtRequestSpec::ui_query(&command)
                } else {
                    AtRequestSpec::interactive(&command)
                };
                spec.timeout = Duration::from_secs(timeout.max(2));
                spec.queued_timeout = spec
                    .queued_timeout
                    .max(Duration::from_secs(timeout + 2));
                let is_write = !crate::scheduler::gate::is_read_command(&command);
                match await_request(arbiter, spec) {
                    Ok(text) => {
                        // 写操作改了模组状态，读缓存必须立刻作废
                        // （改 APN 后旧的 DHCP 地址不能继续显示）。
                        if is_write {
                            crate::scheduler::gate::invalidate_related(cache, &command);
                        }
                        ok.insert("ok".to_string(), Value::Bool(true));
                        ok.insert("response".to_string(), json::str_val(text.trim()));
                    }
                    Err(e) => {
                        ok.insert("ok".to_string(), Value::Bool(false));
                        ok.insert("error".to_string(), json::str_val(&e.message()));
                    }
                }
            }
        }
        "cached" => {
            // Fast-path state dump (SWR snapshot) for the LuCI CLI `cached`
            // subcommand — no AT traffic at all.
            ok.insert("ok".to_string(), Value::Bool(true));
            ok.insert("snapshot".to_string(), cache.snapshot());
        }
        "scan" => {
            let ports: Vec<Value> = client::scan_serial_ports()
                .iter()
                .map(|p| {
                    let mut m = std::collections::BTreeMap::new();
                    m.insert("path".to_string(), json::str_val(&p.path));
                    m.insert("state".to_string(), json::str_val(&p.state));
                    m.insert("is_pcui".to_string(), Value::Bool(p.is_pcui));
                    m.insert("answers_at".to_string(), Value::Bool(p.answers_at));
                    m.insert("vendor".to_string(), json::str_val(&p.vendor));
                    m.insert("product".to_string(), json::str_val(&p.product));
                    Value::Obj(m)
                })
                .collect();
            ok.insert("ok".to_string(), Value::Bool(true));
            ok.insert("ports".to_string(), Value::Arr(ports));
        }
        _ => {
            ok.insert("ok".to_string(), Value::Bool(false));
            ok.insert("error".to_string(), json::str_val("unknown control command"));
        }
    }
    let resp = Value::Obj(ok);
    format!("{}\n", resp.dump())
}

/// Submit a request to the arbiter and wait (bounded) for the result. Used
/// by the control socket which keeps request -> result semantics.
fn await_request(arbiter: &Arc<AtArbiter>, spec: AtRequestSpec) -> Result<String, BackendError> {
    let budget = spec.queued_timeout + spec.timeout + Duration::from_secs(2);
    let rx = arbiter.submit(spec);
    match rx.recv_timeout(budget) {
        Ok(r) => r,
        Err(_) => Err(BackendError::TaskTimeout),
    }
}

/// Bind the Unix control socket and service `mt5700m-at` requests.
fn spawn_control_socket(
    arbiter: Arc<AtArbiter>,
    cache: Arc<StateCache>,
    tasks: Arc<TaskManager>,
) {
    #[cfg(unix)]
    {
        use std::os::unix::net::UnixListener;
        let _ = std::fs::remove_file(crate::transport::control::CONTROL_SOCKET);
        if let Some(parent) = std::path::Path::new(crate::transport::control::CONTROL_SOCKET).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let listener = match UnixListener::bind(crate::transport::control::CONTROL_SOCKET) {
            Ok(l) => l,
            Err(e) => {
                eprintln!(
                    "at-webserver: could not bind control socket {}: {}",
                    crate::transport::control::CONTROL_SOCKET,
                    e
                );
                return;
            }
        };
        eprintln!(
            "at-webserver: control socket {} ready",
            crate::transport::control::CONTROL_SOCKET
        );
        thread::spawn(move || {
            for incoming in listener.incoming() {
                let Ok(stream) = incoming else { continue };
                let arbiter = arbiter.clone();
                let cache = cache.clone();
                let tasks = tasks.clone();
                thread::spawn(move || {
                    handle_control_conn(&arbiter, &cache, &tasks, stream);
                });
            }
        });
    }
    #[cfg(not(unix))]
    {
        let _ = (arbiter, cache, tasks);
    }
}

#[cfg(unix)]
fn handle_control_conn(
    arbiter: &Arc<AtArbiter>,
    cache: &Arc<StateCache>,
    tasks: &Arc<TaskManager>,
    mut stream: std::os::unix::net::UnixStream,
) {
    use std::io::BufRead;
    use std::io::Write;
    let mut line = String::new();
    if std::io::BufReader::new(&mut stream)
        .read_line(&mut line)
        .ok()
        .unwrap_or(0)
        == 0
    {
        return;
    }
    let resp = handle_control_request(arbiter, cache, tasks, &line);
    let _ = stream.write_all(resp.as_bytes());
    let _ = stream.flush();
}

// ---------------------------------------------------------------- LuCI RPC (8765 双协议)
//
// 8765 端口同时服务两类客户端，连接建立时按首字节嗅探分流：
//   - WebSocket（WebUI）：握手以 "GET " 开头，走 handle_ws_conn
//   - newline-JSON RPC（LuCI ucode 经 nc 转发）：以 '{' 开头，走 handle_rpc_conn
//
// RPC 协议（每行一个 JSON，请求-响应模型）：
//   请求: {"id":1,"method":"at","params":{"cmd":"AT+CSQ","auth_key":"..."}}
//   应答: {"id":1,"result":{"success":true,"data":"..."}}
//   错误: {"id":1,"error":{"code":-32601,"message":"..."}}
// 事件不主动推送：LuCI 通过 events(since) 轮询增量（EventBus 全局历史）。
// 所有 AT 交换仍走同一个 AtArbiter，LuCI 与 WebUI 永不争抢串口。

/// 构造 RPC 错误应答 {"id":...,"error":{"code":...,"message":...}}。
fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    let mut e = std::collections::BTreeMap::new();
    e.insert("code".to_string(), json::num_val(code));
    e.insert("message".to_string(), json::str_val(message));
    let mut m = std::collections::BTreeMap::new();
    m.insert("id".to_string(), id.clone());
    m.insert("error".to_string(), Value::Obj(e));
    Value::Obj(m)
}

/// 处理一条 RPC 请求行，返回 JSON 应答。复用与 WebSocket 相同的命令分发
/// （伪命令/扫频/调度/常规 AT），保证两侧行为一致、数据同源。
fn handle_rpc_request(
    client: &Arc<AtClient>,
    arbiter: &Arc<AtArbiter>,
    tasks: &Arc<TaskManager>,
    bus: &Arc<EventBus>,
    cache: &Arc<StateCache>,
    line: &str,
) -> Value {
    let parsed = json::parse(line).unwrap_or(Value::Null);
    let id = parsed.get("id").cloned().unwrap_or(Value::Null);
    let method = parsed.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let params = parsed.get("params").cloned().unwrap_or(Value::Null);

    // RPC 与 WebUI 共用 8765 端口：配置了 auth_key 时逐请求校验（空 key 免检，
    // 与 WebSocket 认证语义一致，由 rpcd 登录态 + 本地回环兜底）。
    let auth_key = params.get("auth_key").and_then(|v| v.as_str()).unwrap_or("");
    if !client.config.websocket_auth_key.is_empty() && auth_key != client.config.websocket_auth_key
    {
        return rpc_error(&id, -32001, "authentication failed");
    }

    let result = match method {
        "at" => {
            let cmd = params.get("cmd").and_then(|v| v.as_str()).unwrap_or("");
            if cmd.is_empty() {
                err_response("缺少参数 cmd")
            } else {
                // 前端可按参数边界透传 args（sms-send 等多词参数），
                // 缺省时后端对 cmd 做空格分词，兼容旧协议。
                let args = params.get("args").and_then(|v| v.as_arr()).cloned();
                handle_at_command(client, arbiter, tasks, bus, cache, cmd, args)
            }
        }
        // Unified API: {"method":"api","params":{"path":"signal.get"}}.
        // LuCI calls this through the ucode bridge, the WebUI through the
        // WebSocket envelope below; both reach the same module code.
        "api" => {
            let path = params.get("path").and_then(|v| v.as_str()).unwrap_or("");
            if path.is_empty() {
                err_response("缺少参数 path")
            } else {
                crate::api::rpc::control_response(arbiter, cache, bus, Some(tasks), path, &params)
            }
        }
        // 缓存快照：零 AT 流量，LuCI 首屏立即拿到后台采集器状态。
        "cached" => {
            let mut m = std::collections::BTreeMap::new();
            m.insert("ok".to_string(), Value::Bool(true));
            m.insert("snapshot".to_string(), cache.snapshot());
            // Report the device this process actually owns, so the CLI does not
            // have to re-probe and can disagree with reality.
            m.insert(
                "serial_port".to_string(),
                match &client.attached_port {
                    Some(p) => Value::Str(p.clone()),
                    None => Value::Null,
                },
            );
            Value::Obj(m)
        }
        // 增量事件拉取（与 WebSocket 推送同源：EventBus 全局历史）。
        "events" => {
            let since = params.get("since").and_then(|v| v.as_u64()).unwrap_or(0);
            let (latest, events) = bus.events_since(since);
            let mut m = std::collections::BTreeMap::new();
            m.insert("seq".to_string(), json::num_val(latest));
            m.insert("events".to_string(), Value::Arr(events));
            Value::Obj(m)
        }
        "scan" => {
            let ports: Vec<Value> = client::scan_serial_ports()
                .iter()
                .map(|p| {
                    let mut m = std::collections::BTreeMap::new();
                    m.insert("path".to_string(), json::str_val(&p.path));
                    m.insert("state".to_string(), json::str_val(&p.state));
                    m.insert("is_pcui".to_string(), Value::Bool(p.is_pcui));
                    m.insert("answers_at".to_string(), Value::Bool(p.answers_at));
                    m.insert("vendor".to_string(), json::str_val(&p.vendor));
                    m.insert("product".to_string(), json::str_val(&p.product));
                    Value::Obj(m)
                })
                .collect();
            let mut m = std::collections::BTreeMap::new();
            m.insert("ok".to_string(), Value::Bool(true));
            m.insert("ports".to_string(), Value::Arr(ports));
            Value::Obj(m)
        }
        "ping" => ok_response("pong"),
        _ => return rpc_error(&id, -32601, "method not found"),
    };

    let mut m = std::collections::BTreeMap::new();
    m.insert("id".to_string(), id);
    m.insert("result".to_string(), result);
    Value::Obj(m)
}

/// 统一的 AT / 伪命令分发（LuCI ucode `at` 方法）。
///
/// 规则：
///   * `AT…`（大小写不敏感）或 `command <AT…>` → 原生 run_command，直连
///     arbiter：与 WebSocket / 后台采集器读去重，相同 AT 命令不重复下发；
///   * 其余（status / network / system / advanced / sms-* / cellscan /
///     sim-pin / pdp-set …）→ 复用 CLI 模式的聚合逻辑（把自身以 `cli`
///     子进程方式重新执行）。子进程经控制 socket 回到本守护进程的同一
///     arbiter，因此同样不会与 WebUI / 采集器抢占串口；聚合输出与旧的
///     `fs.exec` 路径逐字节一致，LuCI 前端解析无需改动。
fn handle_at_command(
    client: &Arc<AtClient>,
    arbiter: &Arc<AtArbiter>,
    tasks: &Arc<TaskManager>,
    bus: &Arc<EventBus>,
    cache: &Arc<StateCache>,
    cmd: &str,
    args: Option<Vec<Value>>,
) -> Value {
    let trimmed = cmd.trim();
    if let Some(rest) = trimmed.strip_prefix("command ") {
        let at_cmd = rest.trim();
        if at_cmd.is_empty() {
            return err_response("缺少参数 cmd");
        }
        return run_command(client, arbiter, tasks, bus, cache, at_cmd);
    }
    if trimmed.to_ascii_uppercase().starts_with("AT") {
        return run_command(client, arbiter, tasks, bus, cache, trimmed);
    }
    let cli_args: Vec<String> = match args {
        Some(list) => list.iter().filter_map(|v| v.as_str().map(String::from)).collect(),
        None => trimmed.split_whitespace().map(String::from).collect(),
    };
    if cli_args.is_empty() {
        return err_response("缺少参数 cmd");
    }
    let (text, code) = cli_capture(&cli_args);
    if code == 0 {
        ok_response(text.trim())
    } else if !text.is_empty() {
        err_response(text.trim())
    } else {
        err_response("mt5700m-at failed")
    }
}

/// 以 CLI 模式重新执行自身并捕获输出（stdout 为空时回退 stderr）。
/// CLI 子进程经控制 socket 回到本守护进程，受 socket 读超时兜底；
/// 这里再套一层硬超时，避免子进程异常卡死时永久占住 RPC 线程
/// （否则并发 RPC 连接堆积，配合 ucode 侧超时仍可能拖慢整个后端）。
/// 跨平台：try_wait / kill / wait_with_output 均为 std API（Linux+Windows）。
fn cli_capture(args: &[String]) -> (String, i32) {
    let exe = std::env::current_exe()
        .unwrap_or_else(|_| std::path::PathBuf::from("at-webserver"));
    let mut child = match std::process::Command::new(&exe)
        .arg("cli")
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return (String::new(), 127),
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(25);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return (String::from("mt5700m-at timed out after 25s"), 124);
                }
                thread::sleep(Duration::from_millis(100));
            }
            Err(_) => return (String::new(), 127),
        }
    }
    let output = match child.wait_with_output() {
        Ok(o) => o,
        Err(_) => return (String::new(), 127),
    };
    let mut text = String::from_utf8_lossy(&output.stdout).to_string();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&output.stderr).to_string();
    }
    (text, output.status.code().unwrap_or(1))
}

/// Serve one newline-JSON RPC connection. 每个请求一行、每个应答一行，
/// ucode 插件（mt5700.uc）经 busybox nc 管道与后端交互。
fn handle_rpc_conn(
    stream: &mut TcpStream,
    client: &Arc<AtClient>,
    arbiter: &Arc<AtArbiter>,
    tasks: &Arc<TaskManager>,
    bus: &Arc<EventBus>,
    cache: &Arc<StateCache>,
) {
    use std::io::{BufRead, Write};
    let write_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut reader = std::io::BufReader::new(write_stream);
    loop {
        let mut line = String::new();
        let n = match reader.read_line(&mut line) {
            Ok(n) => n,
            Err(_) => break,
        };
        if n == 0 {
            break;
        }
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let resp = handle_rpc_request(client, arbiter, tasks, bus, cache, line);
        let mut out = resp.dump();
        out.push('\n');
        if stream.write_all(out.as_bytes()).is_err() {
            break;
        }
        if stream.flush().is_err() {
            break;
        }
    }
}

// ---------------------------------------------------------------- Client connections

pub struct ClientConn {
    out: Mutex<VecDeque<String>>,
    cond: Condvar,
    alive: AtomicBool,
}

impl ClientConn {
    fn try_send(&self, msg: &str) -> bool {
        let mut q = self.out.lock().unwrap();
        if q.len() >= 128 {
            return false;
        }
        q.push_back(msg.to_string());
        self.cond.notify_one();
        true
    }

    fn writer_loop(&self, mut stream: TcpStream) {
        loop {
            let msg = {
                let mut q = self.out.lock().unwrap();
                loop {
                    if !self.alive.load(Ordering::SeqCst) {
                        return;
                    }
                    match q.pop_front() {
                        Some(m) => break m,
                        None => {
                            let (guard, _) = self
                                .cond
                                .wait_timeout(q, Duration::from_secs(1))
                                .unwrap();
                            q = guard;
                        }
                    }
                }
            };
            ws::set_write_timeout(&stream, WS_WRITE_TIMEOUT);
            if ws::write_frame(&mut stream, ws::OP_TEXT, msg.as_bytes()).is_err() {
                return;
            }
        }
    }
}

// ---------------------------------------------------------------- Idle URC monitor
//
// Unsolicited modem lines (calls, SMS, signal, PDCP stats) are published on
// the event bus; every WebSocket subscriber receives them as `{type,data}`
// frames — no per-connection polling, no shared broadcast list.

fn spawn_urc_monitor(client: Arc<AtClient>, bus: Arc<EventBus>) {
    thread::spawn(move || {
        let mut dispatcher = Dispatcher::new();
        let mut leftover = String::new();
        loop {
            if client.in_flight.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_millis(200));
                continue;
            }
            let drained: String = {
                let mut rx = client.rx.lock().unwrap();
                let out: Vec<u8> = rx.drain(..).collect();
                String::from_utf8_lossy(&out).replace('\r', "")
            };
            if drained.is_empty() {
                thread::sleep(Duration::from_millis(300));
                continue;
            }
            leftover.push_str(&drained);
            let mut lines: Vec<String> = leftover.split('\n').map(|s| s.to_string()).collect();
            leftover = lines.pop().unwrap_or_default();
            for line in lines {
                if line.trim().is_empty() {
                    continue;
                }
                for (msg_type, data) in dispatcher.handle_line(&line) {
                    let topic = match msg_type {
                        "new_sms" | "memory_full" | "sms.ussd" => crate::state::bus::TOPIC_SMS,
                        "signal" => crate::state::bus::TOPIC_SIGNAL,
                        "pdcp_data" => crate::state::bus::TOPIC_TRAFFIC,
                        _ => crate::state::bus::TOPIC_MODEM,
                    };
                    bus.publish_now(topic, msg_type, data);
                }
            }
        }
    });
}

// ---------------------------------------------------------------- Server

pub fn run(args: &[String]) -> i32 {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("at-webserver daemon: WebSocket AT bridge on :8765 (SERIAL/NETWORK)");
        return 0;
    }
    let config = load_config();
    if !config.enabled {
        eprintln!("at-webserver disabled via UCI");
        return 0;
    }

    let addr = format!("0.0.0.0:{}", config.websocket_port);
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("bind {} failed: {}", addr, e);
            return 1;
        }
    };
    let client = AtClient::new(config.clone());

    // ------------------------------------------------------------------ Async core
    //
    // Wires: StateCache + EventBus + TaskManager + AtArbiter. Every AT exchange
    // in the backend (WebSocket command path, LuCI control socket, snapshot
    // collectors, band-lock scheduler, cell scans) flows through the single
    // arbiter, so nothing can contend for the serial channel and a slow modem
    // never blocks a UI path.
    let bus = EventBus::new();
    let cache = Arc::new(StateCache::new());
    let arbiter = AtArbiter::new(client.clone());
    let tasks = TaskManager::new(arbiter.clone(), cache.clone(), bus.clone());

    // Background collectors (signal/network/registration/temperature/traffic/
    // cell/sim/modem_info) + the day/night band-lock scheduler run as periodic
    // tasks, so the cache is warm before the first page load.
    crate::modules::spawn_all(&tasks);
    plan::register(&tasks);

    // USB hotplug: presence transitions invalidate the cache, cancel modem
    // tasks and push `usb.*` / `modem.*` events.
    let monitor = DeviceMonitor::new(cache.clone(), bus.clone(), tasks.clone());
    monitor.start();

    // URCs from the serial stream -> event bus (fanned out to subscribers).
    spawn_urc_monitor(client.clone(), bus.clone());

    // LuCI CLI control socket -> arbiter (request -> result semantics).
    spawn_control_socket(arbiter.clone(), cache.clone(), tasks.clone());

    eprintln!(
        "at-webserver-rs listening on {} via {}",
        addr,
        client.describe()
    );

    for incoming in listener.incoming() {
        let Ok(mut stream) = incoming else { continue };
        let _ = stream.set_nodelay(true);
        // 协议嗅探（无损 peek）：'{' -> newline-JSON RPC（LuCI ucode），
        // 其余（"GET "）-> WebSocket（WebUI）。避免握手消费掉首字节。
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut probe = [0u8; 1];
        let is_rpc = match stream.peek(&mut probe) {
            Ok(n) if n > 0 => probe[0] == b'{',
            _ => {
                let _ = stream.set_read_timeout(None);
                continue;
            }
        };
        let _ = stream.set_read_timeout(None);
        let client = client.clone();
        let arbiter = arbiter.clone();
        let tasks = tasks.clone();
        let bus = bus.clone();
        let cache = cache.clone();
        thread::spawn(move || {
            if is_rpc {
                handle_rpc_conn(&mut stream, &client, &arbiter, &tasks, &bus, &cache);
            } else {
                handle_ws_conn(&mut stream, &client, &arbiter, &tasks, &bus, &cache);
            }
        });
    }
    0
}

/// Serve one WebSocket connection: auth, then an **event-driven subscription
/// pump** (bus events -> `{type,data,timestamp}` frames) plus a serial command
/// loop for raw AT commands and `subscribe/unsubscribe/snapshot` control
/// frames. The command path goes through the arbiter; nothing here blocks on
/// the modem beyond the arbiter's own bounded wait.
fn handle_ws_conn(
    stream: &mut TcpStream,
    client: &Arc<AtClient>,
    arbiter: &Arc<AtArbiter>,
    tasks: &Arc<TaskManager>,
    bus: &Arc<EventBus>,
    cache: &Arc<StateCache>,
) {
    if ws::handshake(stream).is_err() {
        return;
    }
    // AUTH: first frame must carry {"auth_key": ...} within 10s.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(WS_AUTH_TIMEOUT)));
    let auth_ok = match ws::read_frame(stream) {
        Ok(frame) if frame.opcode == ws::OP_TEXT => {
            let text = ws::payload_to_string(&frame.payload);
            json::parse(&text)
                .and_then(|v| v.get("auth_key").and_then(|k| k.as_str()).map(String::from))
                .map(|k| k == client.config.websocket_auth_key)
                .unwrap_or_else(|| client.config.websocket_auth_key.is_empty())
        }
        _ => false,
    };
    if !auth_ok {
        let msg = r#"{"error":"Authentication failed","message":"密钥验证失败"}"#;
        let _ = ws::write_frame(stream, ws::OP_TEXT, msg.as_bytes());
        return;
    }
    let ok_msg = r#"{"success":true,"message":"认证成功"}"#;
    let _ = ws::write_frame(stream, ws::OP_TEXT, ok_msg.as_bytes());

    let conn = Arc::new(ClientConn {
        out: Mutex::new(VecDeque::new()),
        cond: Condvar::new(),
        alive: AtomicBool::new(true),
    });

    // Event-driven subscription pump: subscribe to the default topics and
    // stream bus events to this connection. `subscribe/unsubscribe` control
    // frames adjust the topic set at runtime; `snapshot` returns cached state.
    let topics: Vec<String> = DEFAULT_TOPICS.iter().map(|s| s.to_string()).collect();
    let (event_rx, sub) = bus.subscribe(&topics);
    {
        let conn_ev = conn.clone();
        thread::spawn(move || {
            while let Ok(ev) = event_rx.recv() {
                let msg = ev.to_json().dump();
                if !conn_ev.try_send(&msg) {
                    break;
                }
            }
        });
    }

    {
        let writer_stream = stream.try_clone().expect("clone ws stream");
        let conn_weak = conn.clone();
        thread::spawn(move || conn_weak.writer_loop(writer_stream));
    }

    let _ = stream.set_read_timeout(None);
    // Serial read loop: ordered matching, one command at a time.
    loop {
        match ws::read_frame(stream) {
            Ok(frame) => match frame.opcode {
                ws::OP_TEXT => {
                    let text = ws::payload_to_string(&frame.payload);
                    if text == "ping" {
                        if !conn.try_send("pong") {
                            break;
                        }
                        continue;
                    }
                    // Control frames: subscription / snapshot management.
                    if let Some(resp) = handle_ws_control(bus, cache, &sub, &text) {
                        if !conn.try_send(&resp.dump()) {
                            break;
                        }
                        continue;
                    }
                    let response = run_command(client, arbiter, tasks, bus, cache, &text);
                    if !conn.try_send(&response.dump()) {
                        break;
                    }
                }
                ws::OP_PING => {
                    let _ = ws::write_frame(stream, ws::OP_PONG, &frame.payload);
                }
                ws::OP_CLOSE => break,
                _ => {}
            },
            Err(WsError::Closed) => break,
            Err(WsError::Protocol(_)) | Err(WsError::Io(_)) => break,
        }
    }
    conn.alive.store(false, Ordering::SeqCst);
    conn.cond.notify_all();
    bus.unsubscribe(&sub);
}

/// Handle `subscribe` / `unsubscribe` / `snapshot` control frames. Returns
/// `Some(response)` when the frame was a control frame (not an AT command).
fn handle_ws_control(
    bus: &Arc<EventBus>,
    cache: &Arc<StateCache>,
    sub: &Subscription,
    text: &str,
) -> Option<Value> {
    let parsed = json::parse(text)?;
    let action = parsed.get("action").and_then(|v| v.as_str())?;
    let topic_list = |v: &Value| -> Vec<String> {
        match v {
            Value::Arr(items) => items
                .iter()
                .filter_map(|i| i.as_str().map(String::from))
                .collect(),
            _ => Vec::new(),
        }
    };
    match action {
        "subscribe" => {
            let topics = parsed.get("topics").map(&topic_list).unwrap_or_default();
            if topics.is_empty() {
                return Some(err_response("subscribe requires topics"));
            }
            bus.add_topics(sub, &topics);
            Some(ok_response("subscribed"))
        }
        "unsubscribe" => {
            let topics = parsed.get("topics").map(&topic_list).unwrap_or_default();
            bus.remove_topics(sub, &topics);
            Some(ok_response("unsubscribed"))
        }
        "snapshot" => {
            // SWR snapshot: return cached state immediately, no AT traffic.
            let mut m = std::collections::BTreeMap::new();
            m.insert("success".to_string(), Value::Bool(true));
            m.insert("snapshot".to_string(), cache.snapshot());
            Some(Value::Obj(m))
        }
        _ => None,
    }
}

fn run_command(
    client: &Arc<AtClient>,
    arbiter: &Arc<AtArbiter>,
    tasks: &Arc<TaskManager>,
    bus: &Arc<EventBus>,
    cache: &Arc<StateCache>,
    command: &str,
) -> Value {
    // Unified API over the WebSocket/LuCI command path: a command that reads
    // `api.<module>.<verb>` is dispatched to the module registry, so frontends
    // ask for domain data instead of building AT strings and parsing replies.
    if crate::api::rpc::is_api_method(command) {
        // `api.<route>` or `api.<route> {<json params>}` — see
        // `api::rpc::split_api_command`; the registry strips nothing here, it
        // just never sees an AT string.
        let (method, params) = crate::api::rpc::split_api_command(command);
        return crate::api::rpc::ws_response(arbiter, cache, bus, Some(tasks), &method, &params);
    }
    if command.trim() == "AT+CONNECT?" {
        let kind = if client.describe() == "SERIAL" { "1" } else { "0" };
        return ok_response(&format!("+CONNECT: {}\r\nOK", kind));
    }
    if let Some(resp) = handle_schedule_command(command) {
        return resp;
    }
    // `AT^CELLSCAN*` is the cell module's: it has the command builder, the
    // line parser and the exclusive task, so the raw-AT form funnels into the
    // same scan instead of a second implementation living in the daemon.
    if let Some(resp) = crate::modules::cell::scan::pseudo_command(tasks, command) {
        return resp;
    }
    let command = normalize_syscfgex(command);
    // 读命令闸门：渲染/轮询路径上的读一律走 raw 缓存，绝不同步压 AT
    // 通道。命中零阻塞；未命中提交单飞后台采集并立刻返回（SWR）。
    //
    // 写操作（拨号/设值/短信/清流量/升级/SIM PIN/终端）走Passthrough，
    // 行为与改造前完全一致。
    if let Some(cache) = Some(cache) {
        match crate::scheduler::gate::gate(&command, cache, tasks) {
            crate::scheduler::gate::Gate::Cached(text) => {
                if client::response_ok(&text) {
                    return ok_response(text.trim());
                }
                return err_response(text.trim());
            }
            crate::scheduler::gate::Gate::Pending => {
                // 无缓存可返：告知前端正在采集。前端据此显示「采集中」
                // 占位而不是空值，采集完成后由 EventBus 推真实值。
                return pending_response(&command);
            }
            crate::scheduler::gate::Gate::Passthrough => {}
        }
    }
    // Reads deduplicate with the snapshot collectors; writes are serialised
    // as interactive actions. Never blocks beyond the arbiter's bounded wait.
    //
    // Reads use `ui_query`, not `fast_query`: this path serves *both* frontends
    // (LuCI over the JSON-RPC port, the WebUI over WebSocket) and they share
    // this one channel. `fast_query`'s 3 s budget is shorter than several
    // commands' real latency, so a read would fail, retry while holding the
    // channel, and starve the other frontend — the two sides fighting over the
    // AT port rather than over-reading it.
    let is_write = !crate::scheduler::gate::is_read_command(&command);
    let spec = if command.ends_with('?') {
        AtRequestSpec::ui_query(&command)
    } else {
        AtRequestSpec::interactive(&command)
    };
    match await_request(arbiter, spec) {
        Ok(text) => {
            if client::response_ok(&text) {
                // 写操作改变了模组状态，之前采集的 raw 读缓存已经过期
                // （例如改 APN 后 AT^CGPADDR/AT^DHCP? 的旧地址必须丢掉）。
                // 保守作废全部 raw：条目数 < 100，且写是低频用户行为，
                // 代价（下次读重新采一次）远小于漏作废导致页面显示旧值。
                if is_write {
                    crate::scheduler::gate::invalidate_related(cache, &command);
                }
                ok_response(text.trim())
            } else {
                err_response(text.trim())
            }
        }
        Err(e) => err_response(&e.message()),
    }
}

/// 「采集中」占位应答。
///
/// 刻意返回 `success:true` + 空 data 而不是 error：前端拿它渲染
/// 「采集中」占位而不是错误弹窗。真实值随后由 EventBus 推过来。
fn pending_response(command: &str) -> Value {
    let mut m: std::collections::BTreeMap<String, Value> = Default::default();
    m.insert("success".to_string(), Value::Bool(true));
    m.insert("pending".to_string(), Value::Bool(true));
    m.insert("data".to_string(), json::str_val(""));
    m.insert(
        "message".to_string(),
        json::str_val(&format!("{} 采集中，稍后自动更新", command.trim())),
    );
    Value::Obj(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Arbiter backed by a disconnected transport (Stream::None): every
    /// request fails fast with a transport error instead of touching a modem.
    fn test_arbiter() -> Arc<AtArbiter> {
        AtArbiter::new(Arc::new(AtClient {
            config: DaemonConfig::default(),
            lock: Mutex::new(Stream::None),
            rx: Arc::new(Mutex::new(VecDeque::new())),
            in_flight: Arc::new(AtomicBool::new(false)),
            attached_port: None,
        }))
    }

    ///闸门需要 TaskManager 才能提交单飞采集，测试里造一个真实的
    /// （transport 是 Stream::None，采集会立刻失败但不影响断言）。
    fn test_tasks(arbiter: &Arc<AtArbiter>, cache: &Arc<StateCache>) -> Arc<TaskManager> {
        TaskManager::new(arbiter.clone(), cache.clone(), EventBus::new())
    }

    #[test]
    fn control_unknown_command_rejected() {
        let line = r#"{"cmd":"nope"}"#;
        let arbiter = test_arbiter();
        let cache = Arc::new(StateCache::new());
        let tasks = test_tasks(&arbiter, &cache);
        let resp = handle_control_request(&arbiter, &cache, &tasks, line);
        let v: Value = json::parse(&resp).expect("valid json");
        assert_eq!(v.get("ok").and_then(|x| x.as_bool()), Some(false));
    }

    #[test]
    fn control_send_empty_rejected() {
        let line = r#"{"cmd":"send","command":"   "}"#;
        let arbiter = test_arbiter();
        let cache = Arc::new(StateCache::new());
        let tasks = test_tasks(&arbiter, &cache);
        let resp = handle_control_request(&arbiter, &cache, &tasks, line);
        let v: Value = json::parse(&resp).expect("valid json");
        assert_eq!(v.get("ok").and_then(|x| x.as_bool()), Some(false));
    }

    #[test]
    fn control_send_with_no_transport_errors() {
        // Serial not connected => daemon reports ok=false with an error.
        //
        // 刻意用**写命令** `AT+CFUN=0`：读命令会被 read_gate 闸门拦下
        // （返回 pending 占位），那样就测不到「无传输时报错」这条路径了。
        let mut req = std::collections::BTreeMap::new();
        req.insert("cmd".to_string(), json::str_val("send"));
        req.insert("command".to_string(), json::str_val("AT+CFUN=0"));
        req.insert("timeout".to_string(), json::num_val(1));
        let line = format!("{}\n", json::Value::Obj(req).dump());
        let arbiter = test_arbiter();
        let cache = Arc::new(StateCache::new());
        let tasks = test_tasks(&arbiter, &cache);
        let v: Value = json::parse(&handle_control_request(&arbiter, &cache, &tasks, &line)).expect("json");
        assert_eq!(v.get("ok").and_then(|x| x.as_bool()), Some(false));
        assert!(v
            .get("error")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .contains("not connected"));
    }

    /// 读命令走闸门：无缓存时返回 pending 占位而**不是**阻塞等 AT。
    ///
    /// 这是「页面加载不被后端耗时任务阻塞」的机制保证 —— control socket
    /// 的send 分支同样适用（LuCI 的 `mt5700m-at command 'AT^DHCP?'`
    /// 走的就是这条路径）。
    #[test]
    fn control_send_read_command_is_gated() {
        let mut req = std::collections::BTreeMap::new();
        req.insert("cmd".to_string(), json::str_val("send"));
        req.insert("command".to_string(), json::str_val("AT^DHCP?"));
        let line = format!("{}\n", json::Value::Obj(req).dump());
        let arbiter = test_arbiter();
        let cache = Arc::new(StateCache::new());
        let tasks = test_tasks(&arbiter, &cache);
        let v: Value = json::parse(&handle_control_request(&arbiter, &cache, &tasks, &line)).expect("json");
        // 关键：ok=true（已受理），pending=true（采集中），而不是 ok=false。
        assert_eq!(v.get("ok").and_then(|x| x.as_bool()), Some(true));
        assert_eq!(v.get("pending").and_then(|x| x.as_bool()), Some(true));
        // pending 时刻意返回空串：前端据此渲染占位，不该拿到半截数据。
        assert_eq!(v.get("response").and_then(|x| x.as_str()), Some(""));
    }

    #[test]
    fn control_cached_returns_snapshot() {
        let arbiter = test_arbiter();
        let cache = Arc::new(StateCache::new());
        let tasks = test_tasks(&arbiter, &cache);
        cache.set("signal", json::str_val("warm"), "test");
        let line = r#"{"cmd":"cached"}"#;
        let resp = handle_control_request(&arbiter, &cache, &tasks, line);
        let v: Value = json::parse(&resp).expect("json");
        assert_eq!(v.get("ok").and_then(|x| x.as_bool()), Some(true));
        assert!(v.get("snapshot").is_some());
    }

    #[test]
    fn ws_control_subscribe_and_snapshot() {
        let bus = EventBus::new();
        let cache = Arc::new(StateCache::new());
        let (_, sub) = bus.subscribe(&["signal".to_string()]);
        let line = r#"{"action":"subscribe","topics":["network","cell"]}"#;
        let resp = handle_ws_control(&bus, &cache, &sub, line).expect("control handled");
        assert_eq!(resp.get("success").and_then(|x| x.as_bool()), Some(true));
        // snapshot returns cached state without any AT traffic
        let line2 = r#"{"action":"snapshot"}"#;
        let resp2 = handle_ws_control(&bus, &cache, &sub, line2).expect("control handled");
        assert!(resp2.get("snapshot").is_some());
        // non-control frames are treated as AT commands
        assert!(handle_ws_control(&bus, &cache, &sub, "AT+CSQ").is_none());
        bus.unsubscribe(&sub);
    }

    #[test]
    fn rpc_unknown_method_rejected() {
        let arbiter = test_arbiter();
        let cache = Arc::new(StateCache::new());
        let bus = EventBus::new();
        let tasks = TaskManager::new(arbiter.clone(), cache.clone(), bus.clone());
        let client = Arc::new(AtClient {
            config: DaemonConfig::default(),
            lock: Mutex::new(Stream::None),
            rx: Arc::new(Mutex::new(VecDeque::new())),
            in_flight: Arc::new(AtomicBool::new(false)),
            attached_port: None,
        });
        let resp = handle_rpc_request(
            &client,
            &arbiter,
            &tasks,
            &bus,
            &cache,
            r#"{"id":7,"method":"nope","params":{}}"#,
        );
        let err = resp.get("error").expect("error object");
        assert_eq!(
            err.get("code").and_then(|v| v.as_i64()),
            Some(-32601)
        );
        bus.stop();
    }

    #[test]
    fn rpc_at_empty_cmd_rejected() {
        let arbiter = test_arbiter();
        let cache = Arc::new(StateCache::new());
        let bus = EventBus::new();
        let tasks = TaskManager::new(arbiter.clone(), cache.clone(), bus.clone());
        let client = Arc::new(AtClient {
            config: DaemonConfig::default(),
            lock: Mutex::new(Stream::None),
            rx: Arc::new(Mutex::new(VecDeque::new())),
            in_flight: Arc::new(AtomicBool::new(false)),
            attached_port: None,
        });
        let resp = handle_rpc_request(
            &client,
            &arbiter,
            &tasks,
            &bus,
            &cache,
            r#"{"id":1,"method":"at","params":{"cmd":"  "}}"#,
        );
        let result = resp.get("result").expect("result object");
        assert_eq!(result.get("success").and_then(|v| v.as_bool()), Some(false));
        bus.stop();
    }

    #[test]
    fn rpc_cached_returns_snapshot() {
        let arbiter = test_arbiter();
        let cache = Arc::new(StateCache::new());
        let bus = EventBus::new();
        let tasks = TaskManager::new(arbiter.clone(), cache.clone(), bus.clone());
        let client = Arc::new(AtClient {
            config: DaemonConfig::default(),
            lock: Mutex::new(Stream::None),
            rx: Arc::new(Mutex::new(VecDeque::new())),
            in_flight: Arc::new(AtomicBool::new(false)),
            attached_port: None,
        });
        cache.set("signal", json::str_val("warm"), "test");
        let resp = handle_rpc_request(
            &client,
            &arbiter,
            &tasks,
            &bus,
            &cache,
            r#"{"id":2,"method":"cached","params":{}}"#,
        );
        let result = resp.get("result").expect("result object");
        assert_eq!(result.get("ok").and_then(|v| v.as_bool()), Some(true));
        assert!(result.get("snapshot").is_some());
        bus.stop();
    }

    #[test]
    fn rpc_events_returns_increments() {
        let arbiter = test_arbiter();
        let cache = Arc::new(StateCache::new());
        let bus = EventBus::new();
        let tasks = TaskManager::new(arbiter.clone(), cache.clone(), bus.clone());
        let client = Arc::new(AtClient {
            config: DaemonConfig::default(),
            lock: Mutex::new(Stream::None),
            rx: Arc::new(Mutex::new(VecDeque::new())),
            in_flight: Arc::new(AtomicBool::new(false)),
            attached_port: None,
        });
        bus.publish_now("signal", "signal.updated", json::num_val(-86));
        let resp = handle_rpc_request(
            &client,
            &arbiter,
            &tasks,
            &bus,
            &cache,
            r#"{"id":3,"method":"events","params":{"since":0}}"#,
        );
        let result = resp.get("result").expect("result object");
        let latest = result.get("seq").and_then(|v| v.as_u64()).unwrap_or(0);
        assert!(latest >= 1, "seq must advance, got {}", latest);
        let events = result.get("events").and_then(|v| v.as_arr());
        assert!(
            events.map(|e| !e.is_empty()).unwrap_or(false),
            "events must include the published one"
        );
        // 增量语义：since=latest 后为空
        let resp2 = handle_rpc_request(
            &client,
            &arbiter,
            &tasks,
            &bus,
            &cache,
            &format!(r#"{{"id":4,"method":"events","params":{{"since":{}}}}}"#, latest),
        );
        let result2 = resp2.get("result").expect("result object");
        let events2 = result2.get("events").and_then(|v| v.as_arr());
        assert!(
            events2.map(|e| e.is_empty()).unwrap_or(true),
            "no new events after latest seq"
        );
        bus.stop();
    }

    #[test]
    fn rpc_auth_key_required() {
        let arbiter = test_arbiter();
        let cache = Arc::new(StateCache::new());
        let bus = EventBus::new();
        let tasks = TaskManager::new(arbiter.clone(), cache.clone(), bus.clone());
        let mut cfg = DaemonConfig::default();
        cfg.websocket_auth_key = "sekret".into();
        let client = Arc::new(AtClient {
            config: cfg,
            lock: Mutex::new(Stream::None),
            rx: Arc::new(Mutex::new(VecDeque::new())),
            in_flight: Arc::new(AtomicBool::new(false)),
            attached_port: None,
        });
        // 无 auth_key -> 拒绝
        //
        // 用写命令 `AT+CFUN=0` 作载体：读命令会被 read_gate 闸门拦成
        // pending（success=true），那样就断不出「无传输 -> success=false」
        // 这条路径了。本测试关注的是鉴权，不是读命令的缓存行为。
        let resp = handle_rpc_request(
            &client,
            &arbiter,
            &tasks,
            &bus,
            &cache,
            r#"{"id":5,"method":"at","params":{"cmd":"AT+CFUN=0"}}"#,
        );
        assert!(resp.get("error").is_some());
        // 正确的 auth_key -> 放行（无传输 -> 命令报错而非鉴权错误）
        let resp = handle_rpc_request(
            &client,
            &arbiter,
            &tasks,
            &bus,
            &cache,
            r#"{"id":6,"method":"at","params":{"cmd":"AT+CFUN=0","auth_key":"sekret"}}"#,
        );
        assert!(resp.get("error").is_none(), "auth passes");
        let result = resp.get("result").expect("result object");
        assert_eq!(result.get("success").and_then(|v| v.as_bool()), Some(false));
        bus.stop();
    }

    #[test]
    fn resolve_serial_prefers_explicit_path() {
        let mut c = DaemonConfig::default();
        c.serial_port = "/dev/ttyUSB3".into();
        assert_eq!(resolve_serial_port(&c), "/dev/ttyUSB3");
        // "auto" falls back to system scan (host: none -> default path).
        c.serial_port = "auto".into();
        assert_eq!(resolve_serial_port(&c), client::PREFERRED_AT_PORT);
    }

    #[test]
    fn syscfgex_normalization() {
        let cmd = "AT^SYSCFGEX=\"0302\",3fffffff,1,2,7FFFFFFFFFFFFFFF,\"\",\"\"";
        let out = normalize_syscfgex(cmd);
        assert!(out.contains(",\"7FFFFFFFFFFFFFFF\",\"\",\"\""));
    }

    #[test]
    fn event_envelope_is_nested_data() {
        // The frontend consumes msg.data.xxx (Python ws.broadcast parity);
        // the event bridge adds a timestamp without breaking that shape.
        let data = crate::transport::urc::handle_pdcp(
            "^PDCPDATAINFO: 1,5,65535,0,0,0,0,0,0,0,0,512,0,0,1,2",
        )
        .unwrap();
        let ev = Event {
            topic: "traffic".into(),
            event: "pdcp_data".into(),
            data,
            timestamp: 1234567890,
        };
        let msg = ev.to_json().dump();
        let parsed: Value = json::parse(&msg).unwrap();
        assert_eq!(
            parsed.get("type").and_then(|v| v.as_str()),
            Some("pdcp_data")
        );
        let inner = parsed.get("data").expect("nested data object");
        assert!(inner
            .get("ulPdcpRate")
            .and_then(|v| v.as_u64())
            .is_some());
        assert_eq!(
            parsed.get("timestamp").and_then(|v| v.as_u64()),
            Some(1234567890)
        );
    }
}
