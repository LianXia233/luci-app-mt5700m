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
    let mut merged = std::collections::BTreeMap::new();
    merged.insert("type".to_string(), json::str_val(msg_type));
    if let Value::Obj(m) = data {
        for (k, v) in m {
            merged.insert(k.clone(), v.clone());
        }
    }
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

    for incoming in listener.incoming() {
        let Ok(mut stream) = incoming else { continue };
        let _ = stream.set_nodelay(true);
        let client = client.clone();
        let scan = scan.clone();
        let peers = peers.clone();
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
                            let response = run_command(&client, &scan, &peers, &command);
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
    command: &str,
) -> Value {
    if command.trim() == "AT+CONNECT?" {
        let kind = if client.describe() == "SERIAL" { "1" } else { "0" };
        return ok_response(&format!("+CONNECT: {}\r\nOK", kind));
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
