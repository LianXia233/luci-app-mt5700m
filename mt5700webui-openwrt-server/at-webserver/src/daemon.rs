//! Daemon mode: WebSocket AT server on :8765, protocol-compatible with the
//! Go (v3.0.2) and Python backends. Text frames carry raw AT commands; JSON
//! frames carry auth and pushed URC events. Client reads match responses to
//! commands by order, so the read loop is deliberately serial.
//!
//! Transports: UBUS (default, via `ubus call at-daemon sendat`, no URC
//! stream), SERIAL and NETWORK (persistent streams with URC scanning and
//! the day/night band-lock scheduler).

use crate::at::{self, Settings as CliSettings};
use crate::dispatcher::Dispatcher;
use crate::json::{self, Value};
use crate::scheduler;
use crate::ws::{self, WsError};
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
    pub connection_type: String, // UBUS | NETWORK | SERIAL
    pub ubus_at_port: String,
    pub ubus_timeout: u64,
    pub network_host: String,
    pub network_port: u16,
    pub serial_port: String,
    pub websocket_port: u16,
    pub websocket_auth_key: String,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        DaemonConfig {
            enabled: true,
            connection_type: "UBUS".into(),
            ubus_at_port: String::new(),
            ubus_timeout: 10,
            network_host: "192.168.8.1".into(),
            network_port: 20249,
            serial_port: at::PREFERRED_AT_PORT.into(),
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
    if !at::uci_available() {
        return c;
    }
    let g = |k: &str| uci_get(&format!("at-webserver.config.{}", k));
    if let Some(v) = g("enabled") {
        c.enabled = as_bool(&v);
    }
    if let Some(v) = g("connection_type") {
        c.connection_type = v.to_uppercase();
    }
    if let Some(v) = g("ubus_at_port") {
        c.ubus_at_port = v;
    }
    if let Some(v) = g("ubus_timeout") {
        c.ubus_timeout = v.parse().unwrap_or(10);
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
}

impl AtClient {
    fn new(config: DaemonConfig) -> Arc<Self> {
        let kind = config.connection_type.clone();
        let stream = match kind.as_str() {
            "SERIAL" => std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&config.serial_port)
                .map(Stream::Serial)
                .unwrap_or(Stream::None),
            "NETWORK" => TcpStream::connect((config.network_host.as_str(), config.network_port))
                .map(Stream::Tcp)
                .unwrap_or(Stream::None),
            _ => Stream::None,
        };
        let client = Arc::new(AtClient {
            config,
            lock: Mutex::new(stream),
            rx: Arc::new(Mutex::new(VecDeque::new())),
            in_flight: Arc::new(AtomicBool::new(false)),
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
            "SERIAL" => "SERIAL",
            "NETWORK" => "NETWORK",
            _ => "UBUS",
        }
    }

    /// Send one AT command and wait for the final result. Serialized.
    pub fn send(&self, command: &str, timeout: u64) -> Result<String, String> {
        let _guard = self.lock.lock().map_err(|_| "client lock poisoned")?;
        match self.config.connection_type.as_str() {
            "UBUS" => {
                let settings = CliSettings {
                    enabled: true,
                    mode: at::Mode::Serial,
                    at_port: self.config.ubus_at_port.clone(),
                    host: self.config.network_host.clone(),
                    port: self.config.network_port,
                    timeout_s: timeout.max(self.config.ubus_timeout),
                };
                let device = if self.config.ubus_at_port.is_empty() {
                    at::mt5700m_pcui_port()
                        .unwrap_or_else(|| at::PREFERRED_AT_PORT.to_string())
                } else {
                    self.config.ubus_at_port.clone()
                };
                at::ubus_sendat(&settings, &device, command)
                    .map_err(|e| e.message())
            }
            "SERIAL" | "NETWORK" => self.stream_command(command, timeout),
            _ => Err("unknown connection type".into()),
        }
    }

    fn stream_command(&self, command: &str, timeout: u64) -> Result<String, String> {
        self.in_flight.store(true, Ordering::SeqCst);
        // Drain pending input first so stale bytes do not pollute the reply.
        self.rx.lock().unwrap().clear();
        {
            let mut guard = self.lock.lock().map_err(|_| "client lock poisoned")?;
            let wire = format!("{}\r", command);
            let result = match &mut *guard {
                Stream::Serial(f) => f.write_all(wire.as_bytes()).and_then(|_| f.flush()),
                Stream::Tcp(s) => s.write_all(wire.as_bytes()).and_then(|_| s.flush()),
                Stream::None => {
                    self.in_flight.store(false, Ordering::SeqCst);
                    return Err("transport not connected".into());
                }
            };
            if let Err(e) = result {
                self.in_flight.store(false, Ordering::SeqCst);
                return Err(e.to_string());
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

// ---------------------------------------------------------------- Broadcast

fn broadcast(peers: &Arc<Mutex<Vec<Arc<ClientConn>>>>, msg_type: &str, data: &Value) {
    // Protocol-compatible with the Python backend's `ws.broadcast(type,
    // data)`: the payload rides in a nested "data" field. The frontend
    // consumes `msg.data.xxx` (and expects `typeof data == "string"` for
    // raw_data), so a top-level merge would break every event consumer.
    let mut merged = std::collections::BTreeMap::new();
    merged.insert("type".to_string(), json::str_val(msg_type));
    merged.insert("data".to_string(), data.clone());
    let msg = Value::Obj(merged).dump();
    let mut list = peers.lock().unwrap();
    list.retain(|p| p.try_send(&msg));
}

// ---------------------------------------------------------------- Cellscan (async path)

struct ScanState {
    in_progress: AtomicBool,
    abort: AtomicBool,
}

fn handle_cellscan(
    client: &Arc<AtClient>,
    scan: &Arc<ScanState>,
    command: &str,
    peers: &Arc<Mutex<Vec<Arc<ClientConn>>>>,
) -> Option<Value> {
    let cmd = command.trim();
    if cmd == "AT^CELLSCAN=ABORT" || cmd == "AT^CELLSCAN=ABORTED" {
        if scan.in_progress.load(Ordering::SeqCst) {
            scan.abort.store(true, Ordering::SeqCst);
            // The documented interrupt token; harmless on transports that
            // cannot inject it.
            let _ = client.send("abcd", 2);
        }
        return Some(ok_response(""));
    }
    if cmd == "AT^CELLSCAN=STATE" {
        let state = if scan.in_progress.load(Ordering::SeqCst) {
            "scanning"
        } else {
            "idle"
        };
        return Some(ok_response(&format!("+CELLSCAN: {}", state)));
    }
    if cmd == "AT^CELLSCAN" {
        if scan.in_progress.load(Ordering::SeqCst) {
            return Some(err_response("scan already in progress"));
        }
        scan.in_progress.store(true, Ordering::SeqCst);
        scan.abort.store(false, Ordering::SeqCst);
        let client = client.clone();
        let scan = scan.clone();
        let peers = peers.clone();
        thread::spawn(move || {
            let result = client.send("AT^CELLSCAN", 180);
            scan.in_progress.store(false, Ordering::SeqCst);
            let mut obj = std::collections::BTreeMap::new();
            match result {
                Ok(text) => {
                    obj.insert("state".to_string(), json::str_val("done"));
                    obj.insert("result".to_string(), json::str_val(text.trim()));
                }
                Err(e) => {
                    obj.insert("state".to_string(), json::str_val("error"));
                    obj.insert("error".to_string(), json::str_val(&e));
                }
            }
            broadcast(&peers, "cellscan", &Value::Obj(obj));
        });
        return Some(ok_response(""));
    }
    None
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

// ---------------------------------------------------------------- PDCP rate simulation (UBUS mode)

/// Sampling interval bounds for the poll simulation. The frontend asks for
/// 750 ms by default (`AT^PDCPDATAINFO=1[,ms]`); clamped to protect the
/// shared ubus AT channel.
const PDCP_MIN_INTERVAL_MS: u64 = 250;
const PDCP_MAX_INTERVAL_MS: u64 = 5000;
const PDCP_DEFAULT_INTERVAL_MS: u64 = 750;

enum PdcpCmd {
    Start(u64),
    Stop,
}

/// Recognize the sampling switch the frontend sends over the WebSocket.
/// `AT^PDCPDATAINFO=1[,ms]` starts (defaults to 750 ms), `=0` stops.
/// Anything else falls through to the modem unchanged.
fn parse_pdcp_command(command: &str) -> Option<PdcpCmd> {
    const PREFIX: &str = "AT^PDCPDATAINFO=";
    let rest = command.trim().strip_prefix(PREFIX)?;
    if rest == "0" {
        return Some(PdcpCmd::Stop);
    }
    let mut it = rest.split(',');
    if it.next()? != "1" {
        return None;
    }
    let interval = match it.next() {
        Some(v) if !v.is_empty() => v.parse::<u64>().ok()?,
        _ => PDCP_DEFAULT_INTERVAL_MS,
    };
    Some(PdcpCmd::Start(interval.clamp(
        PDCP_MIN_INTERVAL_MS,
        PDCP_MAX_INTERVAL_MS,
    )))
}

/// Shared sampling switch between the WebSocket command handler and the
/// poller thread. Std-only condvar, same style as `ClientConn`.
struct PdcpState {
    inner: Mutex<(bool, u64)>, // (running, interval_ms)
    cond: Condvar,
}

impl PdcpState {
    fn new() -> Arc<Self> {
        Arc::new(PdcpState {
            inner: Mutex::new((false, PDCP_DEFAULT_INTERVAL_MS)),
            cond: Condvar::new(),
        })
    }

    fn start(&self, interval_ms: u64) {
        let mut g = self.inner.lock().unwrap();
        *g = (true, interval_ms);
        self.cond.notify_all();
    }

    fn stop(&self) {
        let mut g = self.inner.lock().unwrap();
        g.0 = false;
        self.cond.notify_all();
    }

    /// Snapshot the switch, sleeping one interval (or 1 s while stopped)
    /// unless it changes in the meantime.
    fn wait_tick(&self, running: bool, interval_ms: u64) -> (bool, u64) {
        let sleep = if running {
            Duration::from_millis(interval_ms)
        } else {
            Duration::from_secs(1)
        };
        let (guard, _) = self
            .cond
            .wait_timeout_while(
                self.inner.lock().unwrap(),
                sleep,
                |s| s.0 == running && s.1 == interval_ms,
            )
            .unwrap();
        *guard
    }
}

/// Poll `AT^PDCPDATAINFO?` while the frontend sampling switch is on and
/// broadcast each result as the same `pdcp_data` event the URC stream path
/// produces. UBUS is request/response only, so this stands in for the
/// modem's `AT^PDCPDATAINFO=1` push — the push itself is deliberately NOT
/// enabled on the modem, keeping URC noise out of the shared serial port.
fn spawn_pdcp_poller(
    client: Arc<AtClient>,
    peers: Arc<Mutex<Vec<Arc<ClientConn>>>>,
    state: Arc<PdcpState>,
) {
    thread::spawn(move || {
        let mut running = false;
        let mut interval_ms = PDCP_DEFAULT_INTERVAL_MS;
        loop {
            let (run, iv) = state.wait_tick(running, interval_ms);
            running = run;
            interval_ms = iv;
            if !running {
                continue;
            }
            // Nobody is listening: skip the modem round-trip until a
            // client reconnects and re-arms the switch.
            if peers.lock().unwrap().is_empty() {
                continue;
            }
            let text = match client.send("AT^PDCPDATAINFO?", 3) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let line = match text
                .lines()
                .map(str::trim)
                .find(|l| l.starts_with("^PDCPDATAINFO:"))
            {
                Some(l) => l,
                None => continue,
            };
            if let Some(data) = crate::dispatcher::handle_pdcp(line) {
                broadcast(&peers, "pdcp_data", &data);
            }
        }
    });
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

fn spawn_urc_monitor(client: Arc<AtClient>, peers: Arc<Mutex<Vec<Arc<ClientConn>>>>) {
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
                    broadcast(&peers, msg_type, &data);
                }
            }
        }
    });
}

// ---------------------------------------------------------------- Server

pub fn run(args: &[String]) -> i32 {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("at-webserver daemon: WebSocket AT bridge on :8765 (UBUS/SERIAL/NETWORK)");
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
    let scan = Arc::new(ScanState {
        in_progress: AtomicBool::new(false),
        abort: AtomicBool::new(false),
    });
    let peers: Arc<Mutex<Vec<Arc<ClientConn>>>> = Arc::new(Mutex::new(Vec::new()));

    eprintln!(
        "at-webserver-rs listening on {} via {}",
        addr,
        client.describe()
    );

    if client.describe() != "UBUS" {
        spawn_urc_monitor(client.clone(), peers.clone());
        scheduler::spawn(client.clone(), Arc::new(AtomicBool::new(false)));
    }

    // UBUS mode has no URC stream: emulate the PDCP rate push with a
    // poller driven by the frontend sampling switch.
    let pdcp = PdcpState::new();
    if client.describe() == "UBUS" {
        spawn_pdcp_poller(client.clone(), peers.clone(), pdcp.clone());
    }

    for incoming in listener.incoming() {
        let Ok(mut stream) = incoming else { continue };
        let _ = stream.set_nodelay(true);
        let client = client.clone();
        let scan = scan.clone();
        let peers = peers.clone();
        let pdcp = pdcp.clone();
        thread::spawn(move || {
            if ws::handshake(&mut stream).is_err() {
                return;
            }
            // AUTH: first frame must carry {"auth_key": ...} within 10s.
            let _ = stream.set_read_timeout(Some(Duration::from_secs(WS_AUTH_TIMEOUT)));
            let auth_ok = match ws::read_frame(&mut stream) {
                Ok(frame) if frame.opcode == ws::OP_TEXT => {
                    let text = ws::payload_to_string(&frame.payload);
                    json::parse(&text)
                        .and_then(|v| v.get("auth_key").and_then(|k| k.as_str()).map(String::from))
                        .map(|k| k == client.config.websocket_auth_key)
                        .unwrap_or_else(|| {
                            // No auth key configured on either side: allow.
                            client.config.websocket_auth_key.is_empty()
                        })
                }
                _ => false,
            };
            if !auth_ok {
                let msg = r#"{"error":"Authentication failed","message":"密钥验证失败"}"#;
                let _ = ws::write_frame(&mut stream, ws::OP_TEXT, msg.as_bytes());
                return;
            }
            let ok_msg = r#"{"success":true,"message":"认证成功"}"#;
            let _ = ws::write_frame(&mut stream, ws::OP_TEXT, ok_msg.as_bytes());

            let conn = Arc::new(ClientConn {
                out: Mutex::new(VecDeque::new()),
                cond: Condvar::new(),
                alive: AtomicBool::new(true),
            });
            peers.lock().unwrap().push(conn.clone());
            {
                let writer_stream = stream.try_clone().expect("clone ws stream");
                let conn_weak = conn.clone();
                thread::spawn(move || conn_weak.writer_loop(writer_stream));
            }

            let _ = stream.set_read_timeout(None);
            // Serial read loop: ordered matching, one command at a time.
            loop {
                match ws::read_frame(&mut stream) {
                    Ok(frame) => match frame.opcode {
                        ws::OP_TEXT => {
                            let command = ws::payload_to_string(&frame.payload);
                            if command == "ping" {
                                if !conn.try_send("pong") {
                                    break;
                                }
                                continue;
                            }
                            let response = run_command(&client, &scan, &peers, &pdcp, &command);
                            if !conn.try_send(&response.dump()) {
                                break;
                            }
                        }
                        ws::OP_PING => {
                            let _ = ws::write_frame(&mut stream, ws::OP_PONG, &frame.payload);
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
            peers.lock().unwrap().retain(|p| !Arc::ptr_eq(p, &conn));
        });
    }
    0
}

fn run_command(
    client: &Arc<AtClient>,
    scan: &Arc<ScanState>,
    peers: &Arc<Mutex<Vec<Arc<ClientConn>>>>,
    pdcp: &Arc<PdcpState>,
    command: &str,
) -> Value {
    if command.trim() == "AT+CONNECT?" {
        let kind = if client.describe() == "SERIAL" { "1" } else { "0" };
        return ok_response(&format!("+CONNECT: {}\r\nOK", kind));
    }
    // UBUS mode: the sampling switch drives the local poller instead of the
    // modem push (which the shared serial transport could not deliver).
    if client.describe() == "UBUS" {
        match parse_pdcp_command(command) {
            Some(PdcpCmd::Stop) => {
                pdcp.stop();
                return ok_response("");
            }
            Some(PdcpCmd::Start(ms)) => {
                pdcp.start(ms);
                return ok_response("");
            }
            None => {}
        }
    }
    if let Some(resp) = handle_schedule_command(command) {
        return resp;
    }
    if let Some(resp) = handle_cellscan(client, scan, command, peers) {
        return resp;
    }
    if scan.in_progress.load(Ordering::SeqCst) {
        return err_response("正在扫频，模组暂时无法响应其它命令，请先取消扫频");
    }
    let command = normalize_syscfgex(command);
    match client.send(&command, 2) {
        Ok(text) => {
            if at::response_ok(&text) {
                ok_response(text.trim())
            } else {
                err_response(text.trim())
            }
        }
        Err(e) => err_response(&e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdcp_switch_on_variants() {
        match parse_pdcp_command("AT^PDCPDATAINFO=1,750") {
            Some(PdcpCmd::Start(ms)) => assert_eq!(ms, 750),
            _ => panic!("start with interval"),
        }
        match parse_pdcp_command("AT^PDCPDATAINFO=1") {
            Some(PdcpCmd::Start(ms)) => assert_eq!(ms, PDCP_DEFAULT_INTERVAL_MS),
            _ => panic!("start default"),
        }
        // interval clamped into [250, 5000]
        match parse_pdcp_command("AT^PDCPDATAINFO=1,50") {
            Some(PdcpCmd::Start(ms)) => assert_eq!(ms, PDCP_MIN_INTERVAL_MS),
            _ => panic!("interval clamped low"),
        }
        match parse_pdcp_command("AT^PDCPDATAINFO=1,99999") {
            Some(PdcpCmd::Start(ms)) => assert_eq!(ms, PDCP_MAX_INTERVAL_MS),
            _ => panic!("interval clamped high"),
        }
    }

    #[test]
    fn pdcp_switch_off_and_fallthrough() {
        assert!(matches!(
            parse_pdcp_command("AT^PDCPDATAINFO=0"),
            Some(PdcpCmd::Stop)
        ));
        // unrelated commands pass through to the modem
        assert!(parse_pdcp_command("AT^PDCPDATAINFO?").is_none());
        assert!(parse_pdcp_command("AT^PDCPDATAINFO=2").is_none());
        assert!(parse_pdcp_command("AT+CGMR").is_none());
    }

    #[test]
    fn pdcp_poller_parses_ubus_response_text() {
        // Shape of what client.send returns in UBUS mode.
        let text = "\r\n^PDCPDATAINFO: 1,5,65535,0,0,0,30,0,792,0,512,1024,0,0,571749518,571748729\r\nOK\r\n";
        let line = text
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with("^PDCPDATAINFO:"))
            .expect("line found");
        let data = crate::dispatcher::handle_pdcp(line).expect("parsed");
        let dump = data.dump();
        assert!(dump.contains("\"ulPdcpRate\":512"));
        assert!(dump.contains("\"dlPdcpRate\":1024"));
        assert!(dump.contains("\"highPriQueMaxBuffTime\":3"));
    }

    #[test]
    fn broadcast_envelope_is_nested_data() {
        // The frontend consumes msg.data.xxx (Python ws.broadcast parity);
        // a top-level merge would break every event consumer.
        let data = crate::dispatcher::handle_pdcp(
            "^PDCPDATAINFO: 1,5,65535,0,0,0,0,0,0,0,0,512,0,0,1,2",
        )
        .unwrap();
        let mut merged = std::collections::BTreeMap::new();
        merged.insert("type".to_string(), json::str_val("pdcp_data"));
        merged.insert("data".to_string(), data);
        let msg = Value::Obj(merged).dump();
        let ev: Value = json::parse(&msg).unwrap();
        assert_eq!(ev.get("type").and_then(|v| v.as_str()), Some("pdcp_data"));
        let inner = ev.get("data").expect("nested data object");
        assert!(inner
            .get("ulPdcpRate")
            .and_then(|v| v.as_u64())
            .is_some());
    }
}
