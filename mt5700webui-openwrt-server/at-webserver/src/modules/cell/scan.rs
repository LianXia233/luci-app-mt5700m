//! Cell scan (`AT^CELLSCAN`): the one implementation every caller shares.
//!
//! A scan takes minutes and owns the modem while it runs, so it cannot be a
//! blocking route. The module builds the command, submits an exclusive task and
//! publishes the decoded result on the `cellscan` push topic; `cell.scan_start`
//! starts it, `cell.scan_state` reports whether it runs and `cell.scan_abort`
//! cancels it.
//!
//! Both frontends used to drive the scan themselves — each built the AT
//! command (with the firmware's band-bitmap quirk), polled the modem for the
//! run state and parsed the `^CELLSCAN:` lines field by field. That work lives
//! here now: the WebUI renders `{state, cells, count}` and never sees an AT
//! command or a firmware field layout, and the raw `AT^CELLSCAN*` form that
//! still reaches the WS/`api` command path (`pseudo_command`, i.e. the
//! terminal console and legacy WS clients) funnels into the same task instead
//! of being a second scan.
//!
//! Still on the old path: `mt5700m-at cellscan` — the LuCI modal's data source
//! — goes through the daemon's *control socket*, whose `send` verb is a raw
//! passthrough with a 30 s window that cannot carry a minutes-long scan. Its
//! text contract is unchanged until the LuCI page moves to `cell.scan_start`
//! plus the push (see docs/architecture-v2/migration.md).

use crate::core::error::BackendError;
use crate::core::json::{self, Value};
use crate::core::task::{Priority, TaskKind};
use crate::scheduler::arbiter::AtRequestSpec;
use crate::scheduler::jobs::{TaskCtx, TaskManager};
use crate::state::bus::TOPIC_SCAN;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

/// A full-band scan needs minutes; the task budget is a little longer than the
/// AT request's so the request is what times out first.
const AT_TIMEOUT: Duration = Duration::from_secs(180);
const TASK_TIMEOUT: Duration = Duration::from_secs(190);
/// Abort token injected on the wire: the firmware ends an in-flight scan on any
/// byte, and `abcd` is what the manual's own example uses.
const ABORT_WIRE: &[u8] = b"abcd";
/// How long the finished scan stays readable (the LuCI modal and the CLI may
/// only look minutes after they asked for it).
const RESULT_TTL: u64 = 1800;
/// Task name, so state/abort address *this* scan and never another exclusive
/// task (reboot, flash) that happens to be running.
const TASK_NAME: &str = "scan.cell";
/// Reported band values above this are firmware noise, not band numbers (the
/// manual's own LTE example reports `10000`, i.e. 0x2710). Showing "—" beats
/// showing 65536 as a band.
const MAX_BAND: u32 = 512;
/// `<5GRSRP>`/`<5GRSRQ>`/`<5GRSINR>`/`<LTESINR>` use 99 for "no measurement".
const INVALID_MEASURE: f64 = 99.0;
/// `<rat>` values the firmware reports, by index (manual 5.35.3).
const RAT_NAMES: [&str; 4] = ["GSM", "WCDMA", "LTE", "NR"];

// ------------------------------------------------------------------ Parsing

/// One cell from a `^CELLSCAN:` line, decoded to the values the tables render.
#[derive(Debug, Clone, PartialEq)]
pub struct ScannedCell {
    pub rat: u32,
    pub rat_name: &'static str,
    pub plmn: String,
    pub freq: Option<i64>,
    pub pci: Option<i64>,
    pub band: Option<u32>,
    pub lac: String,
    pub cid: String,
    pub rxlev: Option<i64>,
    pub bsic: Option<i64>,
    pub psc: Option<i64>,
    pub scs: Option<i64>,
    pub rsrp: Option<f64>,
    pub rsrq: Option<f64>,
    pub sinr: Option<f64>,
    /// The line as the modem sent it (field debugging in the UI's tooltip).
    pub raw: String,
}

impl ScannedCell {
    pub fn to_json(&self) -> Value {
        let mut m: BTreeMap<String, Value> = BTreeMap::new();
        m.insert("rat".to_string(), json::num_val(self.rat));
        m.insert("ratName".to_string(), json::str_val(self.rat_name));
        m.insert("plmn".to_string(), json::str_val(&self.plmn));
        m.insert("freq".to_string(), opt_num(self.freq.map(|v| v as f64)));
        m.insert("pci".to_string(), opt_num(self.pci.map(|v| v as f64)));
        m.insert("band".to_string(), opt_num(self.band.map(|v| v as f64)));
        m.insert("lac".to_string(), json::str_val(&self.lac));
        m.insert("cid".to_string(), json::str_val(&self.cid));
        m.insert("rxlev".to_string(), opt_num(self.rxlev.map(|v| v as f64)));
        m.insert("bsic".to_string(), opt_num(self.bsic.map(|v| v as f64)));
        m.insert("psc".to_string(), opt_num(self.psc.map(|v| v as f64)));
        m.insert("scs".to_string(), opt_num(self.scs.map(|v| v as f64)));
        m.insert("rsrp".to_string(), opt_num(self.rsrp));
        m.insert("rsrq".to_string(), opt_num(self.rsrq));
        m.insert("sinr".to_string(), opt_num(self.sinr));
        m.insert("raw".to_string(), json::str_val(&self.raw));
        Value::Obj(m)
    }
}

fn opt_num(v: Option<f64>) -> Value {
    match v {
        Some(n) => json::num_val(n),
        None => Value::Null,
    }
}

/// Decimal field, `None` when empty or not a number (the UI shows "—").
fn dec(field: &str) -> Option<i64> {
    let t = field.trim();
    if t.is_empty() {
        return None;
    }
    t.parse::<i64>().ok()
}

/// Hex field, `None` when empty or not hex (the firmware mixes the two bases
/// in one line: `<band>`/`<lac>`/`<cid>` are hex, `<pci>`/`<psc>` decimal).
fn hex(field: &str) -> Option<u32> {
    let t = field.trim();
    if t.is_empty() {
        return None;
    }
    u32::from_str_radix(t, 16).ok()
}

/// A measurement field: `None` for empty and for the 99 placeholder.
fn measure(field: &str, scale: f64) -> Option<f64> {
    let n = dec(field)? as f64;
    if n == INVALID_MEASURE {
        return None;
    }
    Some(n * scale)
}

/// `<band>` is reported as a hex *value* (band 41 -> `29`); anything outside
/// 1..=512 is firmware noise and renders as "—".
fn parse_band(field: &str) -> Option<u32> {
    match hex(field) {
        Some(n) if n > 0 && n <= MAX_BAND => Some(n),
        _ => None,
    }
}

/// Decode every `^CELLSCAN:` line of a scan reply, in arrival order.
///
/// Two quirks the manual does not admit: it documents 15 fields but its own
/// example prints 14 (trailing empty fields are dropped), and the RAT index has
/// to be one the table knows.
pub fn parse_cellscan(text: &str) -> Vec<ScannedCell> {
    let mut out = Vec::new();
    // The reply may arrive with CRLF, bare LF or bare CR depending on how the
    // transport collected it; none of them may hide a cell.
    for line in text.split(|c| c == '\n' || c == '\r') {
        if line.trim().is_empty() {
            continue;
        }
        if let Some(cell) = parse_cellscan_line(line) {
            out.push(cell);
        }
    }
    out
}

/// One `^CELLSCAN:` line, or `None` when the line is not a cell.
pub fn parse_cellscan_line(line: &str) -> Option<ScannedCell> {
    let idx = line.find("^CELLSCAN:")?;
    let body = line[idx + "^CELLSCAN:".len()..].trim();
    if body.is_empty() || body.eq_ignore_ascii_case("STARTED") || body.eq_ignore_ascii_case("OK") {
        return None;
    }
    let mut fields: Vec<String> = body.split(',').map(|s| s.trim().to_string()).collect();
    while fields.len() < 15 {
        fields.push(String::new());
    }
    let rat = dec(&fields[0])?;
    let rat_name = RAT_NAMES.get(rat as usize)?;
    Some(ScannedCell {
        rat: rat as u32,
        rat_name,
        plmn: fields[1].replace('"', ""),
        freq: dec(&fields[2]),
        pci: dec(&fields[3]),
        band: parse_band(&fields[4]),
        lac: fields[5].clone(),
        cid: fields[6].clone(),
        rxlev: dec(&fields[7]),
        bsic: dec(&fields[8]),
        psc: dec(&fields[9]),
        scs: dec(&fields[10]),
        rsrp: measure(&fields[11], 1.0),
        // 23.040/27.007 units: <5GRSRQ> and <5GRSINR> are half-dB,
        // <LTESINR> is 1/8 dB.
        rsrq: measure(&fields[12], 0.5),
        sinr: if rat == 2 {
            measure(&fields[14], 0.125)
        } else {
            measure(&fields[13], 0.5)
        },
        raw: line.trim().to_string(),
    })
}

// ------------------------------------------------------------------ Command

/// The scan form's fields, exactly as the pages collect them.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct ScanFilter {
    /// `""` = every RAT the modem supports, else `"1"`/`"2"`/`"3"`.
    pub rat: String,
    pub plmn: String,
    pub freq: String,
    pub pci: String,
    pub band: String,
    pub scs: String,
}

/// A parameter as text: the form sends strings, but a numeric `band`/`rat`
/// (a CLI or a script) must read the same.
fn param_text(params: &Value, key: &str) -> String {
    match params.get(key) {
        Some(Value::Str(s)) => s.trim().to_string(),
        Some(Value::Num(n)) => n.trim().to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

impl ScanFilter {
    /// Read the filter from route parameters (missing keys are empty fields).
    ///
    /// `rat` and `plmn` are digit-only by construction, so a hand-written
    /// caller cannot inject an AT argument through them.
    pub fn from_params(params: &Value) -> ScanFilter {
        let digits = |key: &str| -> String {
            param_text(params, key)
                .chars()
                .filter(|c| c.is_ascii_digit())
                .collect()
        };
        ScanFilter {
            rat: digits("rat"),
            plmn: digits("plmn"),
            freq: param_text(params, "freq"),
            pci: param_text(params, "pci"),
            band: param_text(params, "band"),
            scs: param_text(params, "scs"),
        }
    }
}

/// `1 << (band - 1)` as hex — what the firmware's `<band>` *argument* wants
/// (manual 5.35.3: the reported band is a value like `4E`, the requested one is
/// a bitmap like `8000000000` for n78). Built nibble by nibble because the
/// shift runs past 64 bits for NR bands.
pub fn band_mask(band: u32) -> String {
    let bit = (band - 1) as usize;
    let digits = bit / 4 + 1;
    let mut out = vec!['0'; digits];
    let digit = 1u32 << (bit % 4);
    out[digits - 1 - bit / 4] = std::char::from_digit(digit, 16)
        .unwrap_or('0')
        .to_ascii_uppercase();
    out.into_iter().collect()
}

/// Build `AT^CELLSCAN[=…]`, rejecting what the manual says the firmware will
/// reject anyway — a scan takes minutes, so a bad filter must not cost one.
pub fn build_command(f: &ScanFilter) -> Result<String, BackendError> {
    if !f.rat.is_empty() && !matches!(f.rat.as_str(), "1" | "2" | "3") {
        return Err(BackendError::InvalidParameter(
            "接入技术只能是 LTE、NR 或 WCDMA".to_string(),
        ));
    }
    let rat = f.rat.as_str();
    let freq = f.freq.trim();
    let pci = f.pci.trim();
    let band = f.band.trim();
    let scs = f.scs.trim();

    if !freq.is_empty() && !freq.chars().all(|c| c.is_ascii_digit()) {
        return Err(BackendError::InvalidParameter("频点必须是数字".to_string()));
    }
    if !pci.is_empty() && !pci.chars().all(|c| c.is_ascii_digit()) {
        return Err(BackendError::InvalidParameter("PCI 必须是数字".to_string()));
    }

    // The manual's own constraints, in the order the form checks them.
    if (!freq.is_empty() || !pci.is_empty()) && rat.is_empty() {
        return Err(BackendError::InvalidParameter(
            "指定频点或 PCI 时必须选择接入技术".to_string(),
        ));
    }
    if !pci.is_empty() && freq.is_empty() {
        return Err(BackendError::InvalidParameter(
            "指定 PCI 时必须同时指定频点".to_string(),
        ));
    }
    if !band.is_empty() && !freq.is_empty() {
        return Err(BackendError::InvalidParameter(
            "频段与频点不能同时指定".to_string(),
        ));
    }
    if !pci.is_empty() && rat != "2" && rat != "3" {
        return Err(BackendError::InvalidParameter(
            "只有 LTE 与 NR 支持指定 PCI".to_string(),
        ));
    }
    if rat == "3" && (!freq.is_empty() || !pci.is_empty()) && scs.is_empty() {
        return Err(BackendError::InvalidParameter(
            "NR 指定频点或 PCI 时必须同时选择子载波间隔".to_string(),
        ));
    }

    let mut band_arg = String::new();
    if !band.is_empty() {
        let n: u32 = band
            .parse()
            .ok()
            .filter(|n| *n >= 1 && *n <= MAX_BAND)
            .ok_or_else(|| {
                BackendError::InvalidParameter(format!("频段号超出范围（1-{}）", MAX_BAND))
            })?;
        band_arg = band_mask(n);
    }

    let mut args: Vec<String> = vec![
        rat.to_string(),
        if f.plmn.is_empty() {
            String::new()
        } else {
            format!("\"{}\"", f.plmn)
        },
        freq.to_string(),
        pci.to_string(),
        band_arg,
        scs.to_string(),
    ];
    while args.last().map(|a| a.is_empty()).unwrap_or(false) {
        args.pop();
    }
    if args.is_empty() {
        Ok("AT^CELLSCAN".to_string())
    } else {
        Ok(format!("AT^CELLSCAN={}", args.join(",")))
    }
}

// ------------------------------------------------------------------- Tasks

/// The scan task, if one is running (or queued) right now.
fn active(tasks: &Arc<TaskManager>) -> Option<crate::core::task::TaskRecord> {
    tasks
        .list()
        .into_iter()
        .find(|r| r.name == TASK_NAME && !r.status.is_terminal())
}

/// Whether a scan is queued or running right now.
pub fn is_active(tasks: &Arc<TaskManager>) -> bool {
    active(tasks).is_some()
}

/// `{running}` — the pages ask this on load to restore a scan that outlived
/// their mount, so it must never touch the modem.
pub fn state(tasks: &Arc<TaskManager>) -> Value {
    let mut m: BTreeMap<String, Value> = BTreeMap::new();
    m.insert("running".to_string(), Value::Bool(is_active(tasks)));
    Value::Obj(m)
}

/// Start a scan, unless one is already running. Returns `{started: true}` and
/// publishes the result asynchronously on the `cellscan` topic.
pub fn start(tasks: &Arc<TaskManager>, filter: &ScanFilter) -> Result<Value, BackendError> {
    let command = build_command(filter)?;
    if active(tasks).is_some() {
        return Err(BackendError::Busy);
    }
    submit(tasks, command);
    let mut m: BTreeMap<String, Value> = BTreeMap::new();
    m.insert("started".to_string(), Value::Bool(true));
    Ok(Value::Obj(m))
}

/// Abort the running scan. `{aborted: false}` when there was none: the page's
/// cancel button can lose a race with the scan's own completion, and that is
/// not an error to show the user.
pub fn abort(tasks: &Arc<TaskManager>) -> Value {
    let mut m: BTreeMap<String, Value> = BTreeMap::new();
    match active(tasks) {
        Some(record) => {
            tasks.cancel(record.id);
            m.insert("aborted".to_string(), Value::Bool(true));
        }
        None => {
            m.insert("aborted".to_string(), Value::Bool(false));
        }
    }
    Value::Obj(m)
}

/// Publish `{state, cells, count, raw[, error]}` — the one scan push shape.
///
/// `raw` is the modem's reply text verbatim: the WebUI renders the decoded
/// `cells`, while the LuCI modal (and `mt5700m-at cellscan`) print the
/// handbook's `^CELLSCAN:` lines, exactly as the old blocking command did.
/// The terminal payload is also cached on the `scan` topic so a client that
/// was not connected while the scan ran can still fetch the result — the
/// modem is never re-asked for it.
fn publish(ctx: &TaskCtx, state: &str, cells: &[ScannedCell], raw: &str, error: Option<&str>) {
    let mut m: BTreeMap<String, Value> = BTreeMap::new();
    m.insert("state".to_string(), json::str_val(state));
    m.insert(
        "cells".to_string(),
        Value::Arr(cells.iter().map(|c| c.to_json()).collect()),
    );
    m.insert("count".to_string(), json::num_val(cells.len() as i64));
    if !raw.trim().is_empty() {
        m.insert("raw".to_string(), json::str_val(raw.trim()));
    }
    if let Some(e) = error {
        m.insert("error".to_string(), json::str_val(e));
    }
    let payload = Value::Obj(m);
    if state != "running" {
        ctx.cache.set_ttl(
            TOPIC_SCAN,
            payload.clone(),
            "scan",
            std::time::Duration::from_secs(RESULT_TTL),
        );
    }
    ctx.bus.publish_now(TOPIC_SCAN, "cellscan", payload);
}

/// The same shape as [`result`] for callers without a task manager (a
/// transport built in a unit test): nothing is running, and whatever the cache
/// holds is still reported.
pub fn result_without_tasks() -> Value {
    let mut m: BTreeMap<String, Value> = BTreeMap::new();
    m.insert("running".to_string(), Value::Bool(false));
    m.insert("state".to_string(), json::str_val("idle"));
    m.insert("count".to_string(), json::num_val(0));
    Value::Obj(m)
}

/// `{running, state, cells, count, raw?, error?}` — the last scan and whether
/// one is in flight. Registry + cache only: a page reload (or the CLI) never
/// queues behind a scan to read its result.
pub fn result(tasks: &Arc<TaskManager>, cache: &Arc<crate::state::cache::StateCache>) -> Value {
    let running = is_active(tasks);
    let mut m: BTreeMap<String, Value> = BTreeMap::new();
    m.insert("running".to_string(), Value::Bool(running));
    match cache.get(TOPIC_SCAN) {
        (Some(Value::Obj(prev)), _) => {
            for (k, v) in prev {
                m.insert(k.clone(), v.clone());
            }
        }
        _ => {
            m.insert("state".to_string(), json::str_val("idle"));
            m.insert("count".to_string(), json::num_val(0));
        }
    }
    m.insert("running".to_string(), Value::Bool(running));
    Value::Obj(m)
}

/// Submit the exclusive scan task for an already-built command.
fn submit(tasks: &Arc<TaskManager>, command: String) {
    let spec = AtRequestSpec::long_exclusive(&command, AT_TIMEOUT, ABORT_WIRE);
    tasks.submit(
        TASK_NAME,
        TaskKind::Exclusive,
        Priority::Critical,
        Some(TASK_TIMEOUT),
        Box::new(move |ctx: &TaskCtx| match ctx.at_request(spec) {
            Ok(text) => {
                let cells = parse_cellscan(&text);
                publish(ctx, "done", &cells, &text, None);
                let mut m: BTreeMap<String, Value> = BTreeMap::new();
                m.insert("state".to_string(), json::str_val("done"));
                m.insert(
                    "cells".to_string(),
                    Value::Arr(cells.iter().map(|c| c.to_json()).collect()),
                );
                m.insert("count".to_string(), json::num_val(cells.len() as i64));
                Ok(Value::Obj(m))
            }
            Err(BackendError::TaskCancelled) => {
                publish(ctx, "aborted", &[], "", None);
                let mut m: BTreeMap<String, Value> = BTreeMap::new();
                m.insert("state".to_string(), json::str_val("aborted"));
                m.insert("count".to_string(), json::num_val(0));
                Ok(Value::Obj(m))
            }
            Err(e) => {
                let message = e.message();
                publish(ctx, "error", &[], "", Some(&message));
                Err(e)
            }
        }),
    );
}

// ------------------------------------------------------------- Raw-AT compat

/// The legacy raw-AT surface (`AT^CELLSCAN`, `AT^CELLSCAN=STATE`,
/// `AT^CELLSCAN=ABORT`) on the WS/`api` command path: the `/at` terminal and
/// any legacy WS client. It funnels into the same task and the same parser —
/// no second scan implementation.
///
/// Returns `None` for commands this module does not own.
pub fn pseudo_command(tasks: &Arc<TaskManager>, command: &str) -> Option<Value> {
    let cmd = command.trim();
    if !cmd.starts_with("AT^CELLSCAN") {
        return None;
    }
    let suffix = cmd["AT^CELLSCAN".len()..].trim_start_matches('=').trim();
    let upper = suffix.to_ascii_uppercase();
    if upper == "ABORT" || upper == "ABORTED" {
        // The raw form keeps its old error text for the clients that read it.
        return Some(match abort(tasks).get("aborted").and_then(|v| v.as_bool()) {
            Some(true) => ok_response(""),
            _ => err_response("no scan in progress"),
        });
    }
    if upper == "STATE" {
        let state = if active(tasks).is_some() {
            "scanning"
        } else {
            "idle"
        };
        return Some(ok_response(&format!("+CELLSCAN: {}", state)));
    }
    if active(tasks).is_some() {
        return Some(err_response("scan already in progress"));
    }
    // A bare scan, or a filtered one from a client that still builds the
    // command itself: the arguments are forwarded verbatim (they are already
    // sanitized to the AT alphabet).
    let wire: String = cmd.chars().filter(|c| *c != '\r' && *c != '\n').collect();
    submit(tasks, wire);
    Some(ok_response(""))
}

fn ok_response(data: &str) -> Value {
    let mut m: BTreeMap<String, Value> = BTreeMap::new();
    m.insert("success".to_string(), Value::Bool(true));
    if !data.is_empty() {
        m.insert("data".to_string(), json::str_val(data));
    }
    Value::Obj(m)
}

fn err_response(msg: &str) -> Value {
    let mut m: BTreeMap<String, Value> = BTreeMap::new();
    m.insert("success".to_string(), Value::Bool(false));
    m.insert("error".to_string(), json::str_val(msg));
    Value::Obj(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The five cells the WebUI has always used as its demo fixtures — the
    /// parser must produce the same numbers the page used to compute itself.
    const SAMPLE: &str = "^CELLSCAN: 3,\"46000\",504990,334,29,5A01,1F23,,,,1,-85,-11,20,\n\
                          ^CELLSCAN: 3,\"46000\",633888,100,4E,5A01,1F24,,,,1,-95,-13,12,\n\
                          ^CELLSCAN: 3,\"46001\",627264,201,4E,5A03,3F02,,,,1,-102,-15,4,\n\
                          ^CELLSCAN: 2,\"46001\",41332,177,29,5A02,2F10,-98,,,,,,,60\n\
                          ^CELLSCAN: 2,\"46000\",1850,55,3,5A04,2F55,-105,,,,,,,32\n\
                          OK";

    #[test]
    fn parses_every_field_of_a_scan_reply() {
        let cells = parse_cellscan(SAMPLE);
        assert_eq!(cells.len(), 5);

        let nr = &cells[0];
        assert_eq!(nr.rat, 3);
        assert_eq!(nr.rat_name, "NR");
        assert_eq!(nr.plmn, "46000");
        assert_eq!(nr.freq, Some(504990));
        assert_eq!(nr.pci, Some(334));
        assert_eq!(nr.band, Some(41)); // 0x29
        assert_eq!(nr.lac, "5A01");
        assert_eq!(nr.cid, "1F23");
        assert_eq!(nr.rxlev, None);
        assert_eq!(nr.scs, Some(1));
        assert_eq!(nr.rsrp, Some(-85.0));
        assert_eq!(nr.rsrq, Some(-5.5)); // half-dB
        assert_eq!(nr.sinr, Some(10.0)); // half-dB
        assert_eq!(nr.raw, "^CELLSCAN: 3,\"46000\",504990,334,29,5A01,1F23,,,,1,-85,-11,20,");

        let lte = &cells[3];
        assert_eq!(lte.rat_name, "LTE");
        assert_eq!(lte.freq, Some(41332));
        assert_eq!(lte.band, Some(41));
        assert_eq!(lte.rxlev, Some(-98));
        assert_eq!(lte.rsrp, None);
        assert_eq!(lte.sinr, Some(7.5)); // 60 * 1/8 dB
        assert_eq!(lte.rsrq, None);
    }

    #[test]
    fn skips_lines_that_are_not_cells() {
        assert!(parse_cellscan("OK").is_empty());
        assert!(parse_cellscan("AT^CELLSCAN\r\r\nOK").is_empty());
        assert!(parse_cellscan("^CELLSCAN: STARTED\r\nOK").is_empty());
        assert!(parse_cellscan("^CELLSCAN: 9,\"46000\"\r\nOK").is_empty()); // no such RAT
        // The noise band of the manual's own LTE example (0x2710) is not a band
        // number: the cell still renders, with "—" where the band would be.
        let noise = parse_cellscan("^CELLSCAN: 2,\"46000\",1850,55,10000\r\n");
        assert_eq!(noise.len(), 1);
        assert_eq!(noise[0].band, None);
    }

    #[test]
    fn short_lines_are_padded_not_rejected() {
        // The manual documents 15 fields and prints 14; both must parse.
        let line = "^CELLSCAN: 2,\"46000\",1850,55,3,5A04,2F55,-105";
        let cell = parse_cellscan_line(line).expect("cell");
        assert_eq!(cell.band, Some(3));
        assert_eq!(cell.rxlev, Some(-105));
        assert_eq!(cell.sinr, None);
    }

    #[test]
    fn band_mask_is_the_firmware_bitmap() {
        assert_eq!(band_mask(1), "1");
        assert_eq!(band_mask(7), "40"); // the manual's own example
        assert_eq!(band_mask(40), "8000000000"); // the manual's other example
        assert_eq!(band_mask(78), "20000000000000000000"); // 1 << 77
    }

    #[test]
    fn build_command_matches_the_form() {
        let bare = build_command(&ScanFilter::default()).expect("bare");
        assert_eq!(bare, "AT^CELLSCAN");

        let full = ScanFilter {
            rat: "3".to_string(),
            plmn: "46000".to_string(),
            freq: "633888".to_string(),
            pci: String::new(),
            band: String::new(),
            scs: "1".to_string(),
        };
        assert_eq!(
            build_command(&full).expect("full"),
            "AT^CELLSCAN=3,\"46000\",633888,,,1"
        );

        let banded = ScanFilter {
            rat: "2".to_string(),
            band: "3".to_string(),
            ..Default::default()
        };
        assert_eq!(build_command(&banded).expect("band"), "AT^CELLSCAN=2,,,,4");
    }

    #[test]
    fn build_command_rejects_what_the_firmware_would() {
        let cases = [
            (
                ScanFilter { freq: "1850".to_string(), ..Default::default() },
                "指定频点或 PCI 时必须选择接入技术",
            ),
            (
                ScanFilter { rat: "2".to_string(), pci: "55".to_string(), ..Default::default() },
                "指定 PCI 时必须同时指定频点",
            ),
            (
                ScanFilter { rat: "2".to_string(), freq: "1850".to_string(), band: "3".to_string(), ..Default::default() },
                "频段与频点不能同时指定",
            ),
            (
                ScanFilter { rat: "1".to_string(), freq: "1850".to_string(), pci: "55".to_string(), ..Default::default() },
                "只有 LTE 与 NR 支持指定 PCI",
            ),
            (
                ScanFilter { rat: "3".to_string(), freq: "633888".to_string(), ..Default::default() },
                "NR 指定频点或 PCI 时必须同时选择子载波间隔",
            ),
            (
                ScanFilter { rat: "2".to_string(), band: "600".to_string(), ..Default::default() },
                "频段号超出范围（1-512）",
            ),
        ];
        for (filter, expected) in cases {
            let err = build_command(&filter).expect_err(expected);
            assert_eq!(err.code(), "INVALID_PARAMETER", "{}", expected);
            // `message()` is the log line; the pages show `detail()`, which is
            // the bare rejection — assert both so neither drifts.
            assert!(err.message().contains(expected), "{}", err.message());
            assert_eq!(err.detail(), expected.to_string());
        }
    }

    #[test]
    fn filter_params_are_read_as_strings_or_digits() {
        let mut m: BTreeMap<String, Value> = BTreeMap::new();
        m.insert("rat".to_string(), json::num_val(3));
        m.insert("plmn".to_string(), json::str_val("46000"));
        m.insert("band".to_string(), json::str_val("78"));
        let f = ScanFilter::from_params(&Value::Obj(m));
        assert_eq!(f.rat, "3");
        assert_eq!(f.plmn, "46000");
        assert_eq!(f.band, "78");
        // Nothing at all is the unfiltered scan, not an error.
        assert_eq!(build_command(&ScanFilter::from_params(&Value::Null)).unwrap(), "AT^CELLSCAN");
    }
}
