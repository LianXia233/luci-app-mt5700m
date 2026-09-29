//! Background state collectors (modem snapshot) for the async backend.
//!
//! Every state topic (signal / network / registration / temperature /
//! traffic / cell / sim / modem_info) is refreshed by a **periodic task**
//! registered on the shared `TaskManager`. The tasks run at background/low
//! priority through the single AT arbiter, so they can never stall a page
//! load, a WebSocket command or a user action — and a slow modem only delays
//! the *next* refresh, never the UI.
//!
//! Collectors write into the `StateCache` (fresh value + TTL) and publish a
//! `*.updated` event on the `EventBus`; the event bridge fans them out to the
//! WebSocket subscribers. Readers never block on the modem:
//!
//!   * cache fresh        -> return immediately
//!   * cache stale        -> return the stale value + background refresh
//!   * modem unavailable  -> collector skips, cache ages out fast
//!
//! No sensitive data (IMEI/IMSI/SMS content) is ever logged; `modem_info`
//! and `sim` values are stored in the cache (read by authorized frontends)
//! but only non-secret summary fields are echoed in events.

use crate::at_queue::AtResult;
use crate::event_bus::{
    EventBus, TOPIC_CELL, TOPIC_MODEM, TOPIC_NETWORK, TOPIC_REGISTRATION, TOPIC_SIGNAL, TOPIC_SIM,
    TOPIC_TEMPERATURE, TOPIC_TRAFFIC,
};
use crate::json::{self, Value};
use crate::state_cache::StateCache;
use crate::task::Priority;
use crate::task_manager::{TaskCtx, TaskManager};
use std::time::Duration;

/// Register every background collector as a periodic task on the manager.
/// Cadence mirrors the cache TTLs so values stay fresh between refreshes.
pub fn spawn_all(tasks: &TaskManager) {
    // signal: 2 s — page-level, high priority so the dashboard is current.
    tasks.add_periodic(
        "snapshot.signal",
        Duration::from_secs(2),
        Priority::High,
        Some(Duration::from_secs(5)),
        Box::new(|ctx| collect_signal(ctx)),
    );
    tasks.add_periodic(
        "snapshot.registration",
        Duration::from_secs(3),
        Priority::Normal,
        Some(Duration::from_secs(8)),
        Box::new(|ctx| collect_registration(ctx)),
    );
    tasks.add_periodic(
        "snapshot.network",
        Duration::from_secs(3),
        Priority::Normal,
        Some(Duration::from_secs(8)),
        Box::new(|ctx| collect_network(ctx)),
    );
    tasks.add_periodic(
        "snapshot.temperature",
        Duration::from_secs(5),
        Priority::Low,
        Some(Duration::from_secs(8)),
        Box::new(|ctx| collect_temperature(ctx)),
    );
    tasks.add_periodic(
        "snapshot.traffic",
        Duration::from_secs(5),
        Priority::Low,
        Some(Duration::from_secs(8)),
        Box::new(|ctx| collect_traffic(ctx)),
    );
    tasks.add_periodic(
        "snapshot.cell",
        Duration::from_secs(10),
        Priority::Normal,
        Some(Duration::from_secs(15)),
        Box::new(|ctx| collect_cell(ctx)),
    );
    tasks.add_periodic(
        "snapshot.sim",
        Duration::from_secs(30),
        Priority::Background,
        Some(Duration::from_secs(15)),
        Box::new(|ctx| collect_sim(ctx)),
    );
    tasks.add_periodic(
        "snapshot.modem_info",
        Duration::from_secs(60),
        Priority::Background,
        Some(Duration::from_secs(15)),
        Box::new(|ctx| collect_modem_info(ctx)),
    );
}

/// Write a topic to the cache and publish the update event.
fn store(ctx: &TaskCtx, topic: &str, event: &str, value: &Value) {
    ctx.cache.set(topic, value.clone(), "snapshot");
    ctx.bus.publish(topic, event, value.clone());
}

/// Non-fatal collector wrapper: modem/transport errors are expected (absent
/// modem, USB in upgrade mode, ...) and must never poison the scheduler.
/// Returns the trimmed raw response so callers can feed it to their parser.
fn soft(r: AtResult) -> Result<String, crate::error::BackendError> {
    r.map(|text| text.trim().to_string())
}

fn collect_signal(ctx: &TaskCtx) -> Result<Value, crate::error::BackendError> {
    let text = soft(ctx.query("AT^HCSQ?"))?;
    let value = parse_hcsq(&text);
    store(ctx, TOPIC_SIGNAL, "signal.updated", &value);
    Ok(value)
}

/// Parse `^HCSQ: "LTE",rssi,rsrp,sinr,rsrq,...` into the same field names the
/// dashboard already consumes (sysmode/rsrp/rsrq/sinr/rssi).
pub fn parse_hcsq(raw: &str) -> Value {
    let mut m = std::collections::BTreeMap::new();
    let Some(body) = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("^HCSQ:"))
    else {
        return Value::Obj(m);
    };
    let fields: Vec<String> = body
        .split(',')
        .map(|f| f.trim().trim_matches('"').to_string())
        .collect();
    if fields.is_empty() {
        return Value::Obj(m);
    }
    let valid = |v: &str| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) && v != "255";
    let to_num = |v: &str| v.parse::<u64>().unwrap_or(0);
    m.insert("sysmode".to_string(), json::str_val(&fields[0]));
    let get = |i: usize| fields.get(i).map(|s| s.as_str()).unwrap_or("");
    match fields[0].as_str() {
        "NR" => {
            if valid(get(1)) {
                let n = to_num(get(1)) as i64;
                m.insert("rsrp".to_string(), json::num_val(if n >= 97 { -44 } else { n - 141 }));
            }
            if valid(get(2)) {
                let n = to_num(get(2));
                m.insert(
                    "sinr".to_string(),
                    json::num_val(if n >= 251 { 30.0 } else { -20.2 + n as f64 * 0.2 }),
                );
            }
            if valid(get(3)) {
                let n = to_num(get(3));
                m.insert(
                    "rsrq".to_string(),
                    json::num_val(if n >= 34 { -3.0 } else { -20.0 + n as f64 * 0.5 }),
                );
            }
        }
        "LTE" => {
            if valid(get(1)) {
                m.insert("rssi".to_string(), json::num_val(to_num(get(1)) as i64 - 121));
            }
            if valid(get(2)) {
                let n = to_num(get(2)) as i64;
                m.insert("rsrp".to_string(), json::num_val(if n >= 97 { -44 } else { n - 141 }));
            }
            if valid(get(3)) {
                let n = to_num(get(3));
                m.insert(
                    "sinr".to_string(),
                    json::num_val(if n >= 251 { 30.0 } else { -20.2 + n as f64 * 0.2 }),
                );
            }
            if valid(get(4)) {
                let n = to_num(get(4));
                m.insert(
                    "rsrq".to_string(),
                    json::num_val(if n >= 34 { -3.0 } else { -20.0 + n as f64 * 0.5 }),
                );
            }
        }
        "WCDMA" => {
            if valid(get(1)) {
                m.insert("rssi".to_string(), json::num_val(to_num(get(1)) as i64 - 121));
            }
            if valid(get(2)) {
                let n = to_num(get(2)) as i64;
                m.insert("rscp".to_string(), json::num_val(if n >= 96 { -25 } else { n - 121 }));
            }
            if valid(get(3)) {
                let n = to_num(get(3));
                m.insert(
                    "ecio".to_string(),
                    json::num_val(-32.5 + n as f64 * 0.5),
                );
            }
        }
        "GSM" => {
            if valid(get(1)) {
                m.insert("rssi".to_string(), json::num_val(to_num(get(1)) as i64 - 121));
            }
        }
        _ => {}
    }
    Value::Obj(m)
}

fn collect_registration(ctx: &TaskCtx) -> Result<Value, crate::error::BackendError> {
    // C5GREG covers SA, CEREG covers LTE/NSA, CREG as the last resort.
    let mut value = Value::Null;
    for cmd in ["AT+C5GREG?", "AT+CEREG?", "AT+CREG?"] {
        if let Ok(text) = ctx.query(cmd) {
            if !text.trim().is_empty() {
                value = parse_registration(&text);
                break;
            }
        }
    }
    store(ctx, TOPIC_REGISTRATION, "registration.updated", &value);
    Ok(value)
}

/// Parse a `+CxxREG:` line: registration state + optional MCC/MNC/tac/ci.
/// `+CxxREG: <mode>,<stat>[,<mcc>,<mnc>,...]` — the first field is the
/// unsolicited-report mode, the actual registration state is the second;
/// some older replies carry only the stat (single field).
pub fn parse_registration(raw: &str) -> Value {
    let mut m = std::collections::BTreeMap::new();
    let mut best: Option<(u8, String)> = None;
    for line in raw.lines() {
        let t = line.trim();
        let Some(idx) = t.find("REG:") else { continue };
        let body = t[idx + 4..].trim().trim_matches(|c| c == '"' || c == ' ');
        let fields: Vec<&str> = body.split(',').map(|f| f.trim()).collect();
        let stat_field = fields.get(1).or_else(|| fields.first());
        let Some(state) = stat_field.and_then(|f| f.parse::<u8>().ok()) else {
            continue;
        };
        // Keep the most specific registration family (C5GREG > CEREG > CREG)
        // by preferring lines with more fields.
        let score = if t.contains("C5GREG") {
            3
        } else if t.contains("CEREG") {
            2
        } else {
            1
        };
        if best.as_ref().map(|(s, _)| *s < score).unwrap_or(true) {
            best = Some((score, body.to_string()));
        }
        m.insert("state".to_string(), json::num_val(state));
        if let Some(mcc) = fields.get(2) {
            m.insert("mcc".to_string(), json::str_val(mcc.trim().trim_matches('"')));
        }
        if let Some(mnc) = fields.get(3) {
            m.insert("mnc".to_string(), json::str_val(mnc.trim().trim_matches('"')));
        }
    }
    if best.is_none() {
        m.insert("state".to_string(), json::num_val(0));
    }
    Value::Obj(m)
}

fn collect_network(ctx: &TaskCtx) -> Result<Value, crate::error::BackendError> {
    let cops = soft(ctx.query("AT+COPS?"))?;
    let mut m = std::collections::BTreeMap::new();
    if let Some(op) = extract_cops_operator(&cops) {
        m.insert("operator".to_string(), json::str_val(&op));
    }
    if let Some(rat) = extract_cops_rat(&cops) {
        m.insert("sysmode".to_string(), json::str_val(&rat));
    }
    if let Ok(sysinfo) = ctx.query("AT^SYSINFOEX") {
        if let Some(mode) = extract_sysinfo_mode(&sysinfo) {
            m.insert("sysmode_detail".to_string(), json::str_val(&mode));
        }
    }
    let value = Value::Obj(m);
    store(ctx, TOPIC_NETWORK, "network.updated", &value);
    Ok(value)
}

/// Quoted or numeric MCC-MNC operator name from `+COPS:`.
fn extract_cops_operator(raw: &str) -> Option<String> {
    let line = raw.lines().find(|l| l.trim().starts_with("+COPS:"))?;
    let body = line.trim().strip_prefix("+COPS:")?.trim();
    if let Some(q1) = body.find('"') {
        if let Some(q2) = body[q1 + 1..].find('"') {
            let name = &body[q1 + 1..q1 + 1 + q2];
            if !name.is_empty() {
                return Some(name.trim_matches('"').to_string());
            }
        }
    }
    let fields: Vec<&str> = body.split(',').collect();
    for f in &fields {
        let t = f.trim();
        if t.len() >= 5 && t.len() <= 6 && t.bytes().all(|b| b.is_ascii_digit()) {
            return Some(t.to_string());
        }
    }
    fields
        .get(2)
        .map(|s| s.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
}

fn extract_cops_rat(raw: &str) -> Option<String> {
    let line = raw.lines().find(|l| l.trim().starts_with("+COPS:"))?;
    let cleaned: String = line
        .trim()
        .chars()
        .filter(|c| !matches!(c, ' ' | '"' | '\r'))
        .collect();
    let rat = cleaned.split(',').nth(3)?.trim();
    if rat.is_empty() {
        return None;
    }
    Some(normalize_rat(rat))
}

fn normalize_rat(rat: &str) -> String {
    match rat {
        "0" => "GSM".into(),
        "2" => "UTRAN".into(),
        "3" => "GSM EDGE".into(),
        "4" => "HSDPA".into(),
        "5" => "HSUPA".into(),
        "6" => "HSDPA/HSUPA".into(),
        "7" => "LTE".into(),
        "9" => "NR".into(),
        "10" => "LTE-M".into(),
        "11" => "NB-IoT".into(),
        "13" => "LTE".into(),
        "20" => "NR".into(),
        other => other.trim_matches('"').to_string(),
    }
}

fn extract_sysinfo_mode(raw: &str) -> Option<String> {
    let line = raw.lines().find(|l| l.trim().starts_with("^SYSINFOEX:"))?;
    let body = line.trim().strip_prefix("^SYSINFOEX:")?.trim();
    if let Some(q1) = body.find('"') {
        if let Some(q2) = body[q1 + 1..].find('"') {
            let mode = &body[q1 + 1..q1 + 1 + q2];
            if !mode.is_empty() {
                return Some(mode.trim_matches('"').to_string());
            }
        }
    }
    Some(body.trim_matches('"').to_string())
}

fn collect_temperature(ctx: &TaskCtx) -> Result<Value, crate::error::BackendError> {
    let text = soft(ctx.query("AT^CHIPTEMP?"))?;
    let value = parse_chiptemp(&text);
    store(ctx, TOPIC_TEMPERATURE, "temperature.updated", &value);
    Ok(value)
}

/// Parse `^CHIPTEMP: t0..t11` (tenths of degrees) into named fields plus a
/// rounded average. Mirrors the WebUI `parseCHIPTEMP`.
pub fn parse_chiptemp(raw: &str) -> Value {
    let names = [
        "sub3GPA", "sub6GPA", "mimoPa", "tcxo", "peri1", "peri2", "ap1", "ap2", "modem1",
        "modem2", "bbp1", "bbp2",
    ];
    let mut m = std::collections::BTreeMap::new();
    let Some(body) = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("^CHIPTEMP:"))
    else {
        return Value::Obj(m);
    };
    let fields: Vec<&str> = body.split(',').map(|f| f.trim()).collect();
    let mut sum: f64 = 0.0;
    let mut count: usize = 0;
    for (i, name) in names.iter().enumerate() {
        let raw_v = fields.get(i).and_then(|f| f.parse::<u64>().ok()).unwrap_or(0);
        let v = if raw_v >= 65535 || raw_v > 1500 {
            0.0
        } else {
            raw_v as f64 / 10.0
        };
        m.insert((*name).to_string(), json::num_val(v));
        if v > 0.0 {
            sum += v;
            count += 1;
        }
    }
    if count > 0 {
        let avg = sum / count as f64;
        m.insert("average".to_string(), json::num_val(avg));
    }
    Value::Obj(m)
}

fn collect_traffic(ctx: &TaskCtx) -> Result<Value, crate::error::BackendError> {
    let text = soft(ctx.query("AT^PDCPDATAINFO?"))?;
    let value = text
        .lines()
        .find_map(|l| crate::dispatcher::handle_pdcp(l))
        .unwrap_or(Value::Null);
    store(ctx, TOPIC_TRAFFIC, "traffic.updated", &value);
    Ok(value)
}

fn collect_cell(ctx: &TaskCtx) -> Result<Value, crate::error::BackendError> {
    // Serving cell: best-effort raw snapshot. Full per-carrier analysis stays
    // in the interactive WebUI flows (MONSC/HFREQINFO/MONSSC); the cache only
    // guarantees the dashboard has *something* instantly available.
    let mut m = std::collections::BTreeMap::new();
    if let Ok(text) = ctx.query("AT^MONSC") {
        m.insert("raw".to_string(), json::str_val(text.trim()));
        if let Some(body) = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("^MONSC:"))
        {
            let fields: Vec<&str> = body.split(',').map(|f| f.trim()).collect();
            if let Some(arfcn) = fields.first() {
                m.insert("arfcn".to_string(), json::str_val(arfcn));
            }
        }
    }
    let value = Value::Obj(m);
    store(ctx, TOPIC_CELL, "cell.updated", &value);
    Ok(value)
}

fn collect_sim(ctx: &TaskCtx) -> Result<Value, crate::error::BackendError> {
    let mut m = std::collections::BTreeMap::new();
    if let Ok(text) = ctx.query("AT+CPIN?") {
        for line in text.lines() {
            if let Some(rest) = line.trim().strip_prefix("+CPIN:") {
                m.insert(
                    "status".to_string(),
                    json::str_val(rest.trim().trim_matches('"')),
                );
                break;
            }
        }
    }
    if let Ok(text) = ctx.query("AT^ICCID?") {
        for line in text.lines() {
            if let Some(rest) = line.trim().strip_prefix("^ICCID:") {
                m.insert(
                    "iccid".to_string(),
                    json::str_val(rest.trim().trim_matches('"')),
                );
                break;
            }
        }
    }
    if let Ok(text) = ctx.query("AT+CIMI") {
        for line in text.lines() {
            let t = line.trim();
            if t.len() >= 10 && t.bytes().all(|b| b.is_ascii_digit()) {
                m.insert("imsi".to_string(), json::str_val(t));
                break;
            }
        }
    }
    let value = Value::Obj(m);
    store(ctx, TOPIC_SIM, "sim.updated", &value);
    Ok(value)
}

fn collect_modem_info(ctx: &TaskCtx) -> Result<Value, crate::error::BackendError> {
    let mut m = std::collections::BTreeMap::new();
    if let Ok(text) = ctx.query("ATI") {
        for (prefix, key) in [
            ("Manufacturer:", "manufacturer"),
            ("Model:", "model"),
            ("Revision:", "revision"),
        ] {
            if let Some(rest) = text.lines().find_map(|l| l.trim().strip_prefix(prefix)) {
                m.insert(
                    key.to_string(),
                    json::str_val(rest.trim().trim_matches('"')),
                );
            }
        }
    }
    if let Ok(text) = ctx.query("AT+CGSN") {
        for line in text.lines() {
            let t = line.trim();
            if t.len() == 15 && t.bytes().all(|b| b.is_ascii_digit()) {
                m.insert("imei".to_string(), json::str_val(t));
                break;
            }
        }
    }
    let value = Value::Obj(m);
    // modem_info is long-lived; give the cache a longer TTL.
    ctx.cache
        .set_ttl(TOPIC_MODEM, value.clone(), "snapshot", Duration::from_secs(60));
    ctx.bus.publish(TOPIC_MODEM, "modem.info", value.clone());
    Ok(value)
}

/// Total number of AT exchanges the periodic snapshot may produce per minute
/// (kept tiny: 20 + 20 + 20 + 12 + 12 + 6 + 2 + 1 ≈ 93 fast queries/min,
/// all deduplicated and background-priority — a fraction of one serial
/// exchange per second on average).
#[allow(dead_code)]
pub fn snapshot_rate_per_minute() -> usize {
    60 / 2 + 60 / 3 + 60 / 3 + 60 / 5 + 60 / 5 + 60 / 10 + 60 / 30 + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hcsq_lte_parse() {
        let v = parse_hcsq("^HCSQ: \"LTE\",62,52,22,30\r\nOK");
        assert_eq!(v.get("sysmode").and_then(|x| x.as_str()), Some("LTE"));
        assert_eq!(v.get("rssi").and_then(|x| x.as_i64()), Some(-59));
        assert_eq!(v.get("rsrp").and_then(|x| x.as_i64()), Some(-89));
        // sinr = -20.2 + 22 * 0.2 ≈ -15.8
        assert!((v.get("sinr").and_then(|x| x.as_f64()).unwrap_or(0.0) + 15.8).abs() < 1e-6);
        // rsrq = -20.0 + 30 * 0.5 = -5.0
        assert_eq!(v.get("rsrq").and_then(|x| x.as_f64()), Some(-5.0));
    }

    #[test]
    fn hcsq_nr_parse() {
        let v = parse_hcsq("^HCSQ: \"NR\",132,200,26\r\nOK");
        assert_eq!(v.get("sysmode").and_then(|x| x.as_str()), Some("NR"));
        // rsrp raw 132 >= 97 clamps to -44 (frontend convertRsrp behaviour).
        assert_eq!(v.get("rsrp").and_then(|x| x.as_i64()), Some(-44));
        // sinr = -20.2 + 200 * 0.2 = 19.8
        assert!((v.get("sinr").and_then(|x| x.as_f64()).unwrap_or(0.0) - 19.8).abs() < 1e-6);
        // rsrq = -20.0 + 26 * 0.5 = -7.0
        assert_eq!(v.get("rsrq").and_then(|x| x.as_f64()), Some(-7.0));
    }

    #[test]
    fn hcsq_unknown_ignored() {
        let v = parse_hcsq("^HCSQ: \"CDMA\",1,2,3\r\nOK");
        assert_eq!(v.get("sysmode").and_then(|x| x.as_str()), Some("CDMA"));
        assert!(v.get("rsrp").is_none());
    }

    #[test]
    fn registration_parse_cereg() {
        let v = parse_registration("+CEREG: 0,1,\"460\",\"00\",7A32,7\r\nOK");
        assert_eq!(v.get("state").and_then(|x| x.as_u64()), Some(1));
        assert_eq!(v.get("mcc").and_then(|x| x.as_str()), Some("460"));
    }

    #[test]
    fn chiptemp_parse() {
        let v = parse_chiptemp("^CHIPTEMP: 432,445,451,398,401,407,460,455,470,468,410,415\r\nOK");
        assert_eq!(v.get("tcxo").and_then(|x| x.as_f64()), Some(39.8));
        assert_eq!(v.get("ap1").and_then(|x| x.as_f64()), Some(46.0));
        assert!(v.get("average").and_then(|x| x.as_f64()).unwrap_or(0.0) > 0.0);
    }

    #[test]
    fn cops_operator_extraction() {
        assert_eq!(
            extract_cops_operator("+COPS: 0,0,\"CHN-UNICOM\",7\r\nOK").as_deref(),
            Some("CHN-UNICOM")
        );
        assert_eq!(
            extract_cops_rat("+COPS: 0,0,\"CHN-UNICOM\",7\r\nOK").as_deref(),
            Some("LTE")
        );
    }

    #[test]
    fn sysinfo_mode_extraction() {
        assert_eq!(
            extract_sysinfo_mode("^SYSINFOEX: \"NR\",...\r\nOK").as_deref(),
            Some("NR")
        );
    }
}
