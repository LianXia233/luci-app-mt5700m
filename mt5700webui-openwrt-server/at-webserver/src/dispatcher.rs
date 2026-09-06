//! URC dispatcher, port of the Python `Dispatcher` / Go `urc.go`:
//! incoming calls (RING/+CLIP/^CEND), new SMS (+CMTI), memory full,
//! signal changes (^HCSQ) and passthrough lines (^REJINFO/+CUSD).
//! Produces `(type, data)` pairs ready for the WebSocket broadcast.

use crate::json::{self, Value};
use std::time::Instant;

const CALL_DEDUP_WINDOW_SECS: u64 = 30;
const SIGNAL_CHANGE_THRESHOLD: f64 = 1.0;

/// Field map of `^PDCPDATAINFO:`, port of the Python `PDCP_FIELDS` table.
/// The bool flag marks "report in tenths" (value / 10). Query responses
/// (`AT^PDCPDATAINFO?`) append two extra cumulative byte counters after
/// these 14 fields; they are ignored here (the frontend regex captures
/// them in an optional group for its own delta math).
const PDCP_FIELDS: [(&str, bool); 14] = [
    ("id", false),
    ("pduSessionId", false),
    ("discardTimerLen", false),
    ("avgDelay", true),
    ("minDelay", true),
    ("maxDelay", true),
    ("highPriQueMaxBuffTime", true),
    ("lowPriQueMaxBuffTime", true),
    ("highPriQueBuffPktNums", false),
    ("lowPriQueBuffPktNums", false),
    ("ulPdcpRate", false),
    ("dlPdcpRate", false),
    ("ulDiscardCnt", false),
    ("dlDiscardCnt", false),
];

/// Parse one `^PDCPDATAINFO:` line into the `pdcp_data` payload. Shared by
/// the URC stream path (SERIAL/NETWORK) and the UBUS-mode poll simulation.
pub fn handle_pdcp(line: &str) -> Option<Value> {
    let body = line.strip_prefix("^PDCPDATAINFO:")?.trim();
    let parts: Vec<&str> = body.split(',').map(|p| p.trim()).collect();
    if parts.len() < PDCP_FIELDS.len() {
        return None;
    }
    let mut m = std::collections::BTreeMap::new();
    for (i, (name, tenth)) in PDCP_FIELDS.iter().enumerate() {
        let v: f64 = parts[i].parse().ok()?;
        if *tenth {
            m.insert(name.to_string(), json::num_val(v / 10.0));
        } else {
            m.insert(name.to_string(), json::num_val(v as u64));
        }
    }
    Some(Value::Obj(m))
}

pub struct Dispatcher {
    last_call_number: String,
    last_call_at: Option<Instant>,
    call_state: &'static str,
    last_rsrp: Option<f64>,
    memory_full_notified: bool,
}

fn timestamp() -> String {
    // OpenWrt busybox date honors the system timezone.
    std::process::Command::new("date")
        .arg("+%Y-%m-%d %H:%M:%S")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn quoted_field(line: &str) -> Option<&str> {
    let q1 = line.find('"')?;
    let rest = &line[q1 + 1..];
    let q2 = rest.find('"')?;
    Some(&rest[..q2])
}

pub fn passthrough(line: &str) -> bool {
    let t = line.trim();
    if t.starts_with("^REJINFO") {
        return true;
    }
    // +CUSD with comma: async USSD reply from the network.
    t.starts_with("+CUSD:") && t.contains(',')
}

impl Dispatcher {
    pub fn new() -> Self {
        Dispatcher {
            last_call_number: String::new(),
            last_call_at: None,
            call_state: "idle",
            last_rsrp: None,
            memory_full_notified: false,
        }
    }

    /// Handle one modem line. Returns events to broadcast.
    pub fn handle_line(&mut self, line: &str) -> Vec<(&'static str, Value)> {
        let mut events = Vec::new();
        let t = line.trim();
        if t.is_empty() {
            return events;
        }

        // ---- calls ----
        if t == "RING" || t == "IRING" || t == "^IRING" {
            self.call_state = "ringing";
            return events;
        }
        if t.starts_with("+CLIP:") {
            let Some(number) = quoted_field(t).map(String::from) else {
                return events;
            };
            let same_call = self.last_call_number == number
                && self
                    .last_call_at
                    .map(|at| at.elapsed() < std::time::Duration::from_secs(CALL_DEDUP_WINDOW_SECS))
                    .unwrap_or(false)
                && self.call_state != "idle";
            if same_call {
                return events;
            }
            self.last_call_number = number.clone();
            self.last_call_at = Some(Instant::now());
            self.call_state = "ringing";
            let mut m = std::collections::BTreeMap::new();
            m.insert("time".to_string(), json::str_val(&timestamp()));
            m.insert("number".to_string(), json::str_val(&number));
            m.insert("state".to_string(), json::str_val("ringing"));
            events.push(("incoming_call", Value::Obj(m)));
            return events;
        }
        if t.contains("^CEND:") || t == "NO CARRIER" {
            if !self.last_call_number.is_empty() {
                let mut m = std::collections::BTreeMap::new();
                m.insert("time".to_string(), json::str_val(&timestamp()));
                m.insert(
                    "number".to_string(),
                    json::str_val(&self.last_call_number),
                );
                m.insert("state".to_string(), json::str_val("ended"));
                events.push(("incoming_call", Value::Obj(m)));
            }
            self.last_call_number.clear();
            self.last_call_at = None;
            self.call_state = "idle";
            return events;
        }

        // ---- new SMS ----
        if t.starts_with("+CMTI:") {
            // +CMTI: "ME",12 / +CMTI: "SM",4
            let storage = quoted_field(t).unwrap_or("SM").to_string();
            let index = t
                .rsplit(',')
                .next()
                .map(|s| s.trim().trim_matches(|c| c == '"' || c == ' '))
                .unwrap_or("");
            if !index.is_empty() {
                // Payload is a plain field map; the WS broadcast layer adds
                // the {"type","data"} envelope (Python parity).
                let mut data = std::collections::BTreeMap::new();
                data.insert("storage".to_string(), json::str_val(&storage));
                data.insert("index".to_string(), json::str_val(index));
                events.push(("new_sms", Value::Obj(data)));
            }
            return events;
        }

        // ---- memory full ----
        if t.starts_with("^SMMEMFULL") || t.contains("MEMORY FULL") || t.contains("CMS ERROR: 322") {
            if !self.memory_full_notified {
                self.memory_full_notified = true;
                let mut m = std::collections::BTreeMap::new();
                m.insert("state".to_string(), json::str_val("full"));
                events.push(("memory_full", Value::Obj(m)));
            }
            return events;
        }

        // ---- signal change ----
        if t.starts_with("^HCSQ:") {
            if let Some(ev) = self.handle_hcsq(t) {
                events.push(("signal", ev));
            }
            return events;
        }

        // ---- PDCP stats ----
        if t.starts_with("^PDCPDATAINFO:") {
            if let Some(ev) = handle_pdcp(t) {
                events.push(("pdcp_data", ev));
            }
            return events;
        }

        // ---- passthrough ----
        // raw_data payload must be a plain string (the frontend checks
        // `typeof data == "string"` before dispatching it).
        if passthrough(t) {
            events.push(("raw_data", json::str_val(t)));
        }
        events
    }

    fn handle_hcsq(&mut self, line: &str) -> Option<Value> {
        let body = line.strip_prefix("^HCSQ:")?.trim();
        let fields: Vec<String> = body
            .split(',')
            .map(|f| f.trim().trim_matches('"').to_string())
            .collect();
        if fields.is_empty() {
            return None;
        }
        let valid = |v: &str| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) && v != "255";
        let to_num = |v: &str| v.parse::<u64>().unwrap_or(0);

        let mut m = std::collections::BTreeMap::new();
        m.insert("sysmode".to_string(), json::str_val(&fields[0]));
        let get = |i: usize| fields.get(i).map(|s| s.as_str()).unwrap_or("");

        let mut rsrp_val: Option<f64> = None;
        match fields[0].as_str() {
            "NR" => {
                if valid(get(1)) {
                    let n = to_num(get(1)) as i64;
                    let v = if n >= 97 { -44.0 } else { (n - 141) as f64 };
                    m.insert("rsrp".to_string(), json::num_val(v as i64));
                    rsrp_val = Some(v);
                }
                if valid(get(2)) {
                    let n = to_num(get(2));
                    let sinr = if n >= 251 { 30.0 } else { -20.2 + n as f64 * 0.2 };
                    m.insert("sinr".to_string(), json::num_val(sinr));
                }
                if valid(get(3)) {
                    let n = to_num(get(3));
                    let rsrq = if n >= 34 { -3.0 } else { -20.0 + n as f64 * 0.5 };
                    m.insert("rsrq".to_string(), json::num_val(rsrq));
                }
            }
            "LTE" => {
                if valid(get(1)) {
                    m.insert("rssi".to_string(), json::num_val(to_num(get(1)) as i64 - 121));
                }
                if valid(get(2)) {
                    let n = to_num(get(2)) as i64;
                    let v = if n >= 97 { -44.0 } else { (n - 141) as f64 };
                    m.insert("rsrp".to_string(), json::num_val(v as i64));
                    rsrp_val = Some(v);
                }
                if valid(get(3)) {
                    let n = to_num(get(3));
                    let sinr = if n >= 251 { 30.0 } else { -20.2 + n as f64 * 0.2 };
                    m.insert("sinr".to_string(), json::num_val(sinr));
                }
                if valid(get(4)) {
                    let n = to_num(get(4));
                    let rsrq = if n >= 34 { -3.0 } else { -20.0 + n as f64 * 0.5 };
                    m.insert("rsrq".to_string(), json::num_val(rsrq));
                }
            }
            "WCDMA" | "GSM" => {
                if valid(get(1)) {
                    m.insert("rssi".to_string(), json::num_val(to_num(get(1)) as i64 - 121));
                }
            }
            _ => return None,
        }

        if let Some(rsrp) = rsrp_val {
            if let Some(prev) = self.last_rsrp {
                if (rsrp - prev).abs() < SIGNAL_CHANGE_THRESHOLD {
                    return None;
                }
            }
            self.last_rsrp = Some(rsrp);
            let level = if rsrp >= -85.0 {
                "优秀"
            } else if rsrp >= -95.0 {
                "良好"
            } else if rsrp >= -105.0 {
                "一般"
            } else {
                "较差"
            };
            m.insert("level".to_string(), json::str_val(level));
        } else {
            return None;
        }
        Some(Value::Obj(m))
    }
}

impl Default for Dispatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_call_dedup_and_end() {
        let mut d = Dispatcher::new();
        let ev = d.handle_line("+CLIP: \"13800138000\"");
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].0, "incoming_call");
        // second CLIP within window: suppressed
        assert!(d.handle_line("+CLIP: \"13800138000\"").is_empty());
        // end of call
        let ev = d.handle_line("^CEND: 1,255,16,2");
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].0, "incoming_call");
        assert!(d.handle_line("+CLIP: \"13800138000\"").len() == 1);
    }

    #[test]
    fn cmti_sms_event() {
        let mut d = Dispatcher::new();
        let ev = d.handle_line("+CMTI: \"SM\",12");
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].0, "new_sms");
        let dump = ev[0].1.dump();
        assert!(dump.contains("\"index\":\"12\""));
        assert!(dump.contains("\"storage\":\"SM\""));
    }

    #[test]
    fn memory_full_once() {
        let mut d = Dispatcher::new();
        assert_eq!(d.handle_line("^SMMEMFULL").len(), 1);
        assert!(d.handle_line("^SMMEMFULL").is_empty());
    }

    #[test]
    fn hcsq_signal_threshold() {
        let mut d = Dispatcher::new();
        assert!(d.handle_line("^HCSQ: \"LTE\",60,50,20,30").len() == 1);
        // same rsrp: suppressed
        assert!(d.handle_line("^HCSQ: \"LTE\",60,50,20,30").is_empty());
        // rsrp jump beyond threshold
        assert!(d.handle_line("^HCSQ: \"LTE\",60,70,20,30").len() == 1);
    }

    #[test]
    fn passthrough_rules() {
        assert!(passthrough("^REJINFO: 3"));
        assert!(passthrough("+CUSD: 0,\"...\",15"));
        assert!(!passthrough("+CUSD: 0"));
        assert!(!passthrough("OK"));
    }

    #[test]
    fn pdcp_line_parses_all_fields() {
        // Real modem URC/URC-query line (16 fields: 14 stats + 2 cumulative
        // byte counters that the frontend regex captures optionally).
        let ev = handle_pdcp("^PDCPDATAINFO: 1,5,65535,0,0,0,70,50,2380,168,512,1024,3,9,571749518,571748729")
            .expect("parses");
        let dump = ev.dump();
        assert!(dump.contains("\"id\":1"));
        assert!(dump.contains("\"pduSessionId\":5"));
        assert!(dump.contains("\"discardTimerLen\":65535"));
        assert!(dump.contains("\"avgDelay\":0"));
        assert!(dump.contains("\"highPriQueMaxBuffTime\":7")); // 70 tenths
        assert!(dump.contains("\"lowPriQueMaxBuffTime\":5")); // 50 tenths
        assert!(dump.contains("\"highPriQueBuffPktNums\":2380"));
        assert!(dump.contains("\"lowPriQueBuffPktNums\":168"));
        assert!(dump.contains("\"ulPdcpRate\":512"));
        assert!(dump.contains("\"dlPdcpRate\":1024"));
        assert!(dump.contains("\"ulDiscardCnt\":3"));
        assert!(dump.contains("\"dlDiscardCnt\":9"));
        // extra cumulative byte counters must not leak into the payload
        assert!(!dump.contains("571749518"));
    }

    #[test]
    fn pdcp_short_line_rejected() {
        assert!(handle_pdcp("^PDCPDATAINFO: 1,5").is_none());
        assert!(handle_pdcp("^HCSQ: \"LTE\",60,50,20,30").is_none());
    }

    #[test]
    fn pdcp_dispatch_event_type() {
        let mut d = Dispatcher::new();
        let ev = d.handle_line(
            "^PDCPDATAINFO: 1,5,65535,0,0,0,0,0,0,0,0,0,0,0,562533856,562533859",
        );
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].0, "pdcp_data");
    }
}
