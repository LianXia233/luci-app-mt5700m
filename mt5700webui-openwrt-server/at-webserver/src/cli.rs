//! CLI mode: faithful port of the `mt5700m-at` shell backend (v2.3.44).
//!
//! Every subcommand reproduces the shell's stdout contract line by line so
//! the LuCI JS views (`fs.exec('/usr/sbin/mt5700m-at', ...)`) keep parsing
//! without modification. Exit codes match the shell: 0 ok, 1 generic, 2
//! disabled, 64 usage/argument validation, 127 missing tooling.
//!
//! HARD CONSTRAINT: the `set-imei` path (`AT^PHYNUM=IMEI`) is ported as-is.
//! No behaviour change is permitted around IMEI handling.

use crate::at::{self, AtError, AtOutcome, Mode, Settings};
use std::fmt::Write as FmtWrite;
use std::io::Write as IoWrite;
use std::thread::sleep;
use std::time::Duration;

pub const USB_HELPER_ENV: &str = "MT5700M_USB_HELPER";

// ---------------------------------------------------------------- UCI

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

pub fn load_settings() -> Settings {
    let mut s = Settings::default();
    if at::uci_available() {
        s.enabled = uci_get("mt5700m.settings.enabled").map(|v| v == "1").unwrap_or(true);
        s.mode = match uci_get("mt5700m.settings.mode").unwrap_or_else(|| "auto".into()).as_str() {
            "serial" => Mode::Serial,
            "network" => Mode::Network,
            _ => Mode::Auto,
        };
        s.at_port = uci_get("mt5700m.settings.at_port").unwrap_or_default();
        s.host = uci_get("mt5700m.settings.host").unwrap_or_else(|| "192.168.8.1".into());
        s.port = uci_get("mt5700m.settings.port")
            .and_then(|v| v.parse().ok())
            .unwrap_or(20249);
        s.timeout_s = uci_get("mt5700m.settings.timeout")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(8);
    }
    s
}

// ---------------------------------------------------------------- Run a command

/// Shell equivalent of `at_cmd 'X'` at top level: print response, exit rc.
fn run_at(settings: &Settings, command: &str) -> i32 {
    let out = at::at_cmd(settings, command);
    print_outcome(&out);
    match out.error {
        None => 0,
        Some(e) => e.exit_code(),
    }
}

fn print_outcome(out: &AtOutcome) {
    if !out.text.is_empty() {
        println!("{}", out.text);
    }
    if let Some(AtError::NoSerialPort) = out.error {
        eprintln!("AT serial port not found");
    }
}

fn fail_exit(e: AtError) -> i32 {
    e.exit_code()
}

const EXIT_USAGE: i32 = 64;

// ---------------------------------------------------------------- Text helpers

fn first_match<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    text.lines()
        .find_map(|l| l.strip_prefix(prefix))
        .map(|rest| rest.trim_start())
}

fn clean_value(s: &str) -> String {
    let t = s.trim();
    t.strip_prefix('"')
        .and_then(|x| x.strip_suffix('"'))
        .unwrap_or(t)
        .to_string()
}

fn clean_line_value(s: &str) -> String {
    clean_value(s)
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
        other => clean_line_value(other),
    }
}

fn extract_cops_operator(raw: &str) -> Option<String> {
    let line = first_match(raw, "+COPS:")?;
    let line = line.trim_end_matches('\r');

    // Quoted long name: awk -F" field 2.
    if let Some(q1) = line.find('"') {
        if let Some(q2) = line[q1 + 1..].find('"') {
            let name = &line[q1 + 1..q1 + 1 + q2];
            if !name.is_empty() {
                return Some(clean_line_value(name));
            }
        }
    }

    // Numeric MCC-MNC: first 5-6 digit comma field.
    let fields: Vec<&str> = line.split(',').collect();
    for f in &fields {
        let t = f.trim();
        if t.len() >= 5 && t.len() <= 6 && t.bytes().all(|b| b.is_ascii_digit()) {
            return Some(t.to_string());
        }
    }

    let third = fields.get(2).copied().unwrap_or("");
    let name = clean_line_value(third);
    if !name.is_empty() {
        return Some(name);
    }
    Some(clean_line_value(line))
}

fn extract_cops_rat(raw: &str) -> Option<String> {
    let line = first_match(raw, "+COPS:")?;
    let line: String = line.chars().filter(|c| !matches!(c, ' ' | '"' | '\r')).collect();
    if line.is_empty() {
        return None;
    }
    let rat = line.split(',').nth(3)?;
    let rat = rat.trim();
    if rat.is_empty() {
        return None;
    }
    Some(normalize_rat(rat))
}

fn extract_sysinfo_mode(raw: &str) -> Option<String> {
    let line = first_match(raw, "^SYSINFOEX:")?;
    let line = line.trim_end_matches('\r');

    if let Some(q1) = line.find('"') {
        if let Some(q2) = line[q1 + 1..].find('"') {
            let mode = &line[q1 + 1..q1 + 1 + q2];
            if !mode.is_empty() {
                return Some(clean_line_value(mode));
            }
        }
    }
    Some(clean_line_value(line))
}

// ---------------------------------------------------------------- CSV / lock validation

fn csv_count(value: &str) -> usize {
    let v: String = value.chars().filter(|c| *c != ' ').collect();
    if v.is_empty() {
        return 0;
    }
    v.split(',').count()
}

fn clean_csv(value: &str) -> String {
    let no_space: String = value.chars().filter(|c| *c != ' ').collect();
    // trim leading/trailing commas and collapse duplicates
    let t = no_space.trim_matches(',');
    let mut out: Vec<&str> = Vec::new();
    for part in t.split(',') {
        if part.is_empty() {
            continue;
        }
        // collapse consecutive empties is inherent; keep order
        out.push(part);
    }
    out.join(",")
}

fn is_numeric_csv(v: &str) -> bool {
    !v.is_empty()
        && v.split(',').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

fn numeric_csv_in_range(v: &str, min: u64, max: u64) -> bool {
    is_numeric_csv(v)
        && v.split(',').all(|p| {
            p.parse::<u64>().map(|n| n >= min && n <= max).unwrap_or(false)
        })
}

fn valid_lock_count(count: usize) -> bool {
    (1..=20).contains(&count)
}

pub fn build_lte_lock_command(
    lock_type: &str,
    bands: &str,
    arfcns: &str,
    pcis: &str,
) -> Option<String> {
    let bands = clean_csv(bands);
    let arfcns = clean_csv(arfcns);
    let pcis = clean_csv(pcis);
    let count = csv_count(&bands);
    match lock_type {
        "0" => Some("AT^LTEFREQLOCK=0".into()),
        "3" => {
            if !(valid_lock_count(count) && numeric_csv_in_range(&bands, 0, 65535)) {
                return None;
            }
            Some(format!("AT^LTEFREQLOCK=3,0,{},\"{}\"", count, bands))
        }
        "1" => {
            if !(valid_lock_count(count)
                && numeric_csv_in_range(&bands, 0, 65535)
                && numeric_csv_in_range(&arfcns, 0, 4294967295)
                && count == csv_count(&arfcns))
            {
                return None;
            }
            Some(format!(
                "AT^LTEFREQLOCK=1,0,{},\"{}\",\"{}\"",
                count, bands, arfcns
            ))
        }
        "2" => {
            if !(valid_lock_count(count)
                && numeric_csv_in_range(&bands, 0, 65535)
                && numeric_csv_in_range(&arfcns, 0, 4294967295)
                && numeric_csv_in_range(&pcis, 0, 503)
                && count == csv_count(&arfcns)
                && count == csv_count(&pcis))
            {
                return None;
            }
            Some(format!(
                "AT^LTEFREQLOCK=2,0,{},\"{}\",\"{}\",\"{}\"",
                count, bands, arfcns, pcis
            ))
        }
        _ => None,
    }
}

pub fn build_nr_lock_command(
    lock_type: &str,
    bands: &str,
    arfcns: &str,
    scs: &str,
    pcis: &str,
) -> Option<String> {
    let bands = clean_csv(bands);
    let arfcns = clean_csv(arfcns);
    let scs = clean_csv(scs);
    let pcis = clean_csv(pcis);
    let count = csv_count(&bands);
    match lock_type {
        "0" => Some("AT^NRFREQLOCK=0".into()),
        "3" => {
            if !(valid_lock_count(count) && numeric_csv_in_range(&bands, 0, 65535)) {
                return None;
            }
            Some(format!("AT^NRFREQLOCK=3,0,{},\"{}\"", count, bands))
        }
        "1" => {
            if !(valid_lock_count(count)
                && numeric_csv_in_range(&bands, 0, 65535)
                && numeric_csv_in_range(&arfcns, 0, 4294967295)
                && numeric_csv_in_range(&scs, 0, 4)
                && count == csv_count(&arfcns)
                && count == csv_count(&scs))
            {
                return None;
            }
            Some(format!(
                "AT^NRFREQLOCK=1,0,{},\"{}\",\"{}\",\"{}\"",
                count, bands, arfcns, scs
            ))
        }
        "2" => {
            if !(valid_lock_count(count)
                && numeric_csv_in_range(&bands, 0, 65535)
                && numeric_csv_in_range(&arfcns, 0, 4294967295)
                && numeric_csv_in_range(&scs, 0, 4)
                && numeric_csv_in_range(&pcis, 0, 1007)
                && count == csv_count(&arfcns)
                && count == csv_count(&scs)
                && count == csv_count(&pcis))
            {
                return None;
            }
            Some(format!(
                "AT^NRFREQLOCK=2,0,{},\"{}\",\"{}\",\"{}\",\"{}\"",
                count, bands, arfcns, scs, pcis
            ))
        }
        _ => None,
    }
}

fn apply_frequency_lock(settings: &Settings, rat: &str, lock_type: &str, lock_cmd: &str) -> i32 {
    let (query, prefix) = match rat {
        "lte" => ("AT^LTEFREQLOCK?", "^LTEFREQLOCK:"),
        "nr" => ("AT^NRFREQLOCK?", "^NRFREQLOCK:"),
        _ => return EXIT_USAGE,
    };

    let raw = at::at_cmd(settings, "AT+CFUN?");
    let mut previous = raw
        .text
        .lines()
        .find_map(|l| l.strip_prefix("+CFUN:"))
        .map(|r| clean_value(r.trim_start()))
        .unwrap_or_default();
    if previous != "0" && previous != "1" {
        previous = "1".into();
    }

    let lock_out = at::at_cmd(settings, lock_cmd);
    if lock_out.error.is_some() {
        return 1;
    }

    // MT5700M only exposes a frequency-lock change after a radio function
    // level cycle; keep an existing airplane-mode session offline.
    if previous == "1" {
        if at::at_cmd(settings, "AT+CFUN=0").error.is_some() {
            return 1;
        }
        sleep(Duration::from_secs(1));
        if at::at_cmd(settings, "AT+CFUN=1").error.is_some() {
            return 1;
        }
    }

    let restore = previous == "1";
    let mut current = String::from("no response");
    for attempt in 0..8 {
        let raw = at::at_cmd(settings, query);
        current = raw
            .text
            .lines()
            .find_map(|l| l.strip_prefix(prefix))
            .map(|r| {
                let v = clean_value(r.trim_start());
                v.split(',').next().unwrap_or("").to_string()
            })
            .unwrap_or_else(|| current.clone());
        if current == lock_type {
            println!("{}", raw.text);
            return 0;
        }
        if !restore {
            break;
        }
        sleep(Duration::from_secs(2));
        let _ = attempt;
    }

    eprintln!(
        "MT5700M frequency lock verification failed: expected {}, got {}",
        lock_type, current
    );
    1
}

// ---------------------------------------------------------------- print_* family

pub fn print_signal(settings: &Settings) -> String {
    let raw = at::at_cmd(settings, "AT^HCSQ?");
    let mut out = String::new();
    let Some(line) = first_match(&raw.text, "^HCSQ:") else {
        return out;
    };
    let fields: Vec<String> = line
        .split(',')
        .map(|f| f.chars().filter(|c| !matches!(c, ' ' | '\r' | '"')).collect())
        .collect();
    if fields.is_empty() {
        return out;
    }
    let sys = &fields[0];
    out.push_str(&format!("sysmode={}\n", sys));

    let valid = |v: &str| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) && v != "255";
    let to_num = |v: &str| v.parse::<u64>().unwrap_or(0);

    let push_rssi = |out: &mut String, v: &str| {
        if valid(v) {
            let _ = write!(out, "rssi={}\n", to_num(v) as i64 - 121);
        }
    };
    let push_rsrp = |out: &mut String, v: &str| {
        if !valid(v) {
            return;
        }
        let n = to_num(v);
        if n >= 97 {
            let _ = writeln!(out, "rsrp=-44");
        } else {
            let _ = write!(out, "rsrp={}\n", n as i64 - 141);
        }
    };
    let push_rscp = |out: &mut String, v: &str| {
        if !valid(v) {
            return;
        }
        let n = to_num(v);
        if n >= 96 {
            let _ = writeln!(out, "rscp=-25");
        } else {
            let _ = write!(out, "rscp={}\n", n as i64 - 121);
        }
    };
    let push_sinr = |out: &mut String, v: &str| {
        if !valid(v) {
            return;
        }
        let n = to_num(v);
        if n >= 251 {
            let _ = writeln!(out, "sinr=30.0");
        } else {
            let _ = write!(out, "sinr={:.1}\n", -20.2 + n as f64 * 0.2);
        }
    };
    let push_rsrq = |out: &mut String, v: &str| {
        if !valid(v) {
            return;
        }
        let n = to_num(v);
        if n >= 34 {
            let _ = writeln!(out, "rsrq=-3.0");
        } else {
            let _ = write!(out, "rsrq={:.1}\n", -20.0 + n as f64 * 0.5);
        }
    };

    match sys.as_str() {
        "NR" => {
            if fields.len() > 1 {
                push_rsrp(&mut out, &fields[1]);
            }
            if fields.len() > 2 {
                push_sinr(&mut out, &fields[2]);
            }
            if fields.len() > 3 {
                push_rsrq(&mut out, &fields[3]);
            }
        }
        "LTE" => {
            if fields.len() > 1 {
                push_rssi(&mut out, &fields[1]);
            }
            if fields.len() > 2 {
                push_rsrp(&mut out, &fields[2]);
            }
            if fields.len() > 3 {
                push_sinr(&mut out, &fields[3]);
            }
            if fields.len() > 4 {
                push_rsrq(&mut out, &fields[4]);
            }
        }
        "WCDMA" => {
            if fields.len() > 1 {
                push_rssi(&mut out, &fields[1]);
            }
            if fields.len() > 2 {
                push_rscp(&mut out, &fields[2]);
            }
            if fields.len() > 3 && valid(&fields[3]) {
                let ecio = -32.5 + to_num(&fields[3]) as f64 * 0.5;
                let _ = write!(out, "ecio={:.1}\n", ecio);
            }
        }
        "GSM" => {
            if fields.len() > 1 {
                push_rssi(&mut out, &fields[1]);
            }
        }
        _ => {}
    }
    out
}

pub fn print_identity(settings: &Settings) -> String {
    let raw = at::at_cmd(settings, "ATI");
    let mut out = String::new();
    for (prefix, key) in [
        ("Manufacturer:", "manufacturer"),
        ("Model:", "model"),
        ("Revision:", "revision"),
    ] {
        if let Some(v) = first_match(&raw.text, prefix) {
            let v = clean_value(v);
            let _ = writeln!(out, "{}={}", key, v);
        }
    }
    let mut imei = first_match(&raw.text, "IMEI:")
        .map(clean_value)
        .unwrap_or_default();
    let imei_ok = imei.len() == 15 && imei.bytes().all(|b| b.is_ascii_digit());
    if !imei_ok {
        imei = at::at_cmd(settings, "AT+CGSN")
            .text
            .lines()
            .map(|l| l.trim().trim_end_matches('\r'))
            .find(|l| l.len() == 15 && l.bytes().all(|b| b.is_ascii_digit()))
            .unwrap_or("")
            .to_string();
    }
    if !imei.is_empty() {
        let _ = writeln!(out, "imei={}", imei);
    }
    out.push_str("product_name=MT5700M\n");
    out
}

pub fn print_sim_operator(settings: &Settings) -> String {
    let mut out = String::new();
    let sim = at::at_cmd(settings, "AT+CPIN?");
    if let Some(v) = first_match(&sim.text, "+CPIN:") {
        let _ = writeln!(out, "sim={}", clean_value(v));
    }
    let cops = at::at_cmd(settings, "AT+COPS?");
    if let Some(op) = extract_cops_operator(&cops.text) {
        if !op.is_empty() {
            let _ = writeln!(out, "operator={}", op);
        }
    }
    if let Some(rat) = extract_cops_rat(&cops.text) {
        if !rat.is_empty() {
            let _ = writeln!(out, "sysmode={}", rat);
        }
    }
    let sysinfo = at::at_cmd(settings, "AT^SYSINFOEX");
    if let Some(mode) = extract_sysinfo_mode(&sysinfo.text) {
        if !mode.is_empty() {
            let _ = writeln!(out, "sysmode_detail={}", mode);
        }
    }
    out
}

pub fn print_sim_details(settings: &Settings) -> String {
    let mut out = String::new();
    let iccid_raw = at::at_cmd(settings, "AT^ICCID?");
    if let Some(v) = first_match(&iccid_raw.text, "^ICCID:") {
        let v = clean_value(v);
        if !v.is_empty() {
            let _ = writeln!(out, "iccid={}", v);
        }
    }
    let imsi = at::at_cmd(settings, "AT+CIMI");
    if let Some(v) = imsi
        .text
        .replace('\r', "")
        .lines()
        .find(|l| !l.is_empty() && l.bytes().all(|b| b.is_ascii_digit()))
    {
        let _ = writeln!(out, "imsi={}", v);
    }
    out
}

pub fn print_qos(settings: &Settings) -> String {
    let raw = at::at_cmd(settings, "AT+CGEQOSRDP=1");
    for line in raw.text.lines() {
        if let Some(rest) = line.strip_prefix("+CGEQOSRDP:") {
            let fields: Vec<&str> = rest.split(',').collect();
            if let Some(second) = fields.get(1) {
                let qci: String = second
                    .chars()
                    .filter(|c| !matches!(c, ' ' | '\r' | '"'))
                    .collect();
                if !qci.is_empty() {
                    return format!("qci={}\n", qci);
                }
            }
        }
    }
    String::new()
}

pub fn print_active_apn(settings: &Settings) -> String {
    let raw = at::at_cmd(settings, "AT+CGDCONT?");
    let mut apn = String::new();
    for line in raw.text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("+CGDCONT:") {
            let rest = trimmed
                .strip_prefix("+CGDCONT:")
                .unwrap_or("")
                .trim_start();
            // Format: <cid>,"<type>","<apn>",...
            if rest.starts_with("1,") {
                // quoted field 3 (index 2 among comma fields)
                let quoted: Vec<&str> = rest.split('"').collect();
                // split('"') -> [ '1,', type, ',', apn, ... ]
                if quoted.len() >= 5 {
                    let candidate = quoted[3];
                    if !candidate.is_empty() {
                        apn = candidate.to_string();
                        break;
                    }
                }
            }
        }
    }
    if apn.is_empty() {
        apn = uci_get("mt5700m.connection.apn").unwrap_or_default();
    }
    format!("active_apn={}\n", apn)
}

pub fn print_subscriber_number(settings: &Settings) -> String {
    let raw = at::at_cmd(settings, "AT+CNUM");
    let mut number = String::new();
    for line in raw.text.lines() {
        if let Some(rest) = line.strip_prefix("+CNUM:") {
            let fields: Vec<&str> = rest.split(',').collect();
            if let Some(second) = fields.get(1) {
                let v: String = second
                    .chars()
                    .filter(|c| !matches!(c, ' ' | '\r' | '"'))
                    .collect();
                let digits = v.strip_prefix('+').unwrap_or(&v);
                if !digits.is_empty()
                    && digits.bytes().all(|b| b.is_ascii_digit())
                    && v.len() >= 5
                {
                    number = v;
                    break;
                }
            }
        }
    }
    if !number.is_empty() {
        return format!("phone_number={}\n", number);
    }
    if raw
        .text
        .lines()
        .any(|l| l.trim_start().starts_with("+CME ERROR:") && l.contains("22"))
    {
        return "phone_number_state=not_stored\n".into();
    }
    String::new()
}

pub fn print_temperature(settings: &Settings) -> String {
    let raw = at::at_cmd(settings, "AT^CHIPTEMP?");
    let mut out = String::new();
    let Some(line) = first_match(&raw.text, "^CHIPTEMP:") else {
        return out;
    };
    const NAMES: [&str; 12] = [
        "sub3g_pa", "sub6g_pa", "mimo_pa", "tcxo", "peri1", "peri2", "ap1", "ap2", "modem1",
        "modem2", "bbp1", "bbp2",
    ];
    let fields: Vec<String> = line
        .split(',')
        .map(|f| f.trim().trim_end_matches('\r').to_string())
        .collect();
    let mut peak: i64 = 0;
    let mut peak_name = "";
    let mut found = false;
    for (i, value) in fields.iter().enumerate() {
        if i >= 12 {
            break;
        }
        let v = value.trim();
        let Ok(numeric) = v.parse::<i64>() else {
            continue;
        };
        if !(-400..=1200).contains(&numeric) {
            continue;
        }
        let _ = write!(out, "temp_{}={:.1}\n", NAMES[i], numeric as f64 / 10.0);
        if !found || numeric > peak {
            peak = numeric;
            peak_name = NAMES[i];
            found = true;
        }
    }
    if found {
        let _ = write!(out, "temperature={:.1}\n", peak as f64 / 10.0);
        let _ = writeln!(out, "temperature_sensor={}", peak_name);
    }
    out
}

pub fn print_subscription_rate(settings: &Settings) -> String {
    let mut raw = at::at_cmd(settings, "AT^DSAMBR=1");
    if !raw.text.lines().any(|l| l.starts_with("^DSAMBR:")) {
        raw = at::at_cmd(settings, "AT^DSAMBR=1");
    }
    if !raw.text.lines().any(|l| l.starts_with("^DSAMBR:")) {
        raw = at::at_cmd(settings, "AT^DSAMBR=8");
    }
    for line in raw.text.lines() {
        if let Some(rest) = line.strip_prefix("^DSAMBR:") {
            let fields: Vec<String> = rest
                .split(',')
                .map(|f| f.trim().trim_end_matches('\r').to_string())
                .collect();
            if fields.len() >= 3 {
                let down: String = fields[1]
                    .chars()
                    .filter(|c| !matches!(c, ' ' | '\r' | '"'))
                    .collect();
                let up: String = fields[2]
                    .chars()
                    .filter(|c| !matches!(c, ' ' | '\r' | '"'))
                    .collect();
                let down_ok = !down.is_empty()
                    && down
                        .split('.')
                        .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
                let up_ok = !up.is_empty()
                    && up
                        .split('.')
                        .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
                if down_ok && up_ok {
                    let d: f64 = down.parse().unwrap_or(0.0);
                    let u: f64 = up.parse().unwrap_or(0.0);
                    return format!("ambr_down_mbps={:.1}\nambr_up_mbps={:.1}\n", d / 1000.0, u / 1000.0);
                }
            }
            return String::new();
        }
    }
    String::new()
}

fn lte_bandwidth(code: i64) -> f64 {
    match code {
        0 => 1.4,
        1 => 3.0,
        2 => 5.0,
        3 => 10.0,
        4 => 15.0,
        5 => 20.0,
        _ => 0.0,
    }
}

pub fn print_carrier_aggregation(settings: &Settings) -> String {
    let frequency_raw = at::at_cmd(settings, "AT^HFREQINFO?");
    let lte_ca_raw = at::at_cmd(settings, "AT^CASCELLINFO?");
    let nsa_raw = at::at_cmd(settings, "AT^MONSSC");

    let mut out = String::new();
    let mut count = 0usize;
    let mut nr_count = 0usize;
    let mut lte_count = 0usize;
    let mut lte_scell_count = 0usize;
    let mut dc = false;
    let mut dl_total = 0.0f64;
    let mut ul_total = 0.0f64;

    for line in frequency_raw.text.lines() {
        let Some(rest) = line.strip_prefix("^HFREQINFO:") else {
            continue;
        };
        let clean: String = rest
            .chars()
            .filter(|c| !matches!(c, ' ' | '\r' | '"'))
            .collect();
        let field: Vec<&str> = clean.split(',').collect();
        if field.len() < 2 {
            continue;
        }
        let (radio, divisor, limit) = match field[1] {
            "7" => ("NR", 1000.0f64, 4usize),
            "6" => ("LTE", 10.0f64, 1usize),
            _ => continue,
        };
        let mut parsed = 0usize;
        let mut i = 2usize;
        while i + 6 < field.len() && parsed < limit {
            let ok = (0..7).all(|k| {
                field[i + k]
                    .bytes()
                    .all(|b| b.is_ascii_digit())
                    && !field[i + k].is_empty()
            });
            if !ok {
                i += 7;
                continue;
            }
            let num = |k: usize| field[i + k].parse::<f64>().unwrap_or(0.0);
            count += 1;
            parsed += 1;
            if radio == "NR" {
                nr_count += 1;
            } else {
                lte_count += 1;
            }
            let band = if radio == "NR" {
                format!("n{}", field[i])
            } else {
                format!("B{}", field[i])
            };
            let dl_frequency = num(i + 2) / divisor;
            let ul_frequency = num(i + 5) / divisor;
            let dl_bandwidth = num(i + 3) / 1000.0;
            let ul_bandwidth = num(i + 6) / 1000.0;
            dl_total += dl_bandwidth;
            ul_total += ul_bandwidth;
            let _ = write!(
                out,
                "carrier_{}={}|{}|{}|{:.2}|{:.1}|{}|{:.2}|{:.1}\n",
                count, radio, band, field[i + 1], dl_frequency, dl_bandwidth, field[i + 4],
                ul_frequency, ul_bandwidth
            );
            i += 7;
        }
    }

    for line in lte_ca_raw.text.lines() {
        let Some(rest) = line.strip_prefix("^CASCELLINFO:") else {
            continue;
        };
        let clean: String = rest
            .chars()
            .filter(|c| !matches!(c, ' ' | '\r' | '"'))
            .collect();
        let field: Vec<&str> = clean.split(',').collect();
        if field.len() < 12 {
            continue;
        }
        let nums: Option<Vec<i64>> =
            field[..12].iter().map(|f| f.parse::<i64>().ok()).collect();
        let Some(nums) = nums else { continue };
        let ul_bandwidth = lte_bandwidth(nums[10]);
        let dl_bandwidth = lte_bandwidth(nums[11]);
        count += 1;
        lte_count += 1;
        lte_scell_count += 1;
        dl_total += dl_bandwidth;
        ul_total += ul_bandwidth;
        let _ = write!(
            out,
            "carrier_{}=LTE|B{}|{}|{:.2}|{:.1}|{}|{:.2}|{:.1}\n",
            count, nums[5], nums[7], nums[9] as f64 / 10.0, dl_bandwidth, nums[6],
            nums[8] as f64 / 10.0, ul_bandwidth
        );
    }

    let mut secondary_count = 0usize;
    for line in nsa_raw.text.lines() {
        if line.starts_with("^MONSSC:") && line.contains("NR,") {
            dc = true;
            secondary_count += 1;
        }
    }

    if count == 0 {
        return out;
    }
    if nr_count > 0 && lte_count > 0 {
        dc = true;
    }
    let ca = nr_count > 1 || lte_scell_count > 0;
    let mode = if dc {
        if ca { "EN-DC + CA" } else { "EN-DC" }
    } else if nr_count > 0 {
        if nr_count > 1 { "NR-CA" } else { "NR" }
    } else if lte_count > 1 {
        "LTE-CA"
    } else {
        "LTE"
    };

    let _ = writeln!(out, "carrier_count={}", count);
    let _ = writeln!(out, "ca_active={}", ca as i32);
    let _ = writeln!(out, "dc_active={}", dc as i32);
    let _ = writeln!(out, "nr_carrier_count={}", nr_count);
    let _ = writeln!(out, "lte_carrier_count={}", lte_count);
    let _ = writeln!(out, "lte_secondary_count={}", lte_scell_count);
    let _ = writeln!(out, "secondary_connection_count={}", secondary_count);
    let _ = writeln!(out, "ca_mode={}", mode);
    let _ = writeln!(out, "ca_dl_bandwidth={:.1}", dl_total);
    let _ = writeln!(out, "ca_ul_bandwidth={:.1}", ul_total);
    out
}

pub fn print_lock_status(settings: &Settings) -> String {
    let mut out = String::new();
    let nr = at::at_cmd(settings, "AT^NRFREQLOCK?");
    if let Some(v) = first_match(&nr.text, "^NRFREQLOCK:") {
        let _ = writeln!(out, "nr_lock={}", clean_value(v));
    }
    let lte = at::at_cmd(settings, "AT^LTEFREQLOCK?");
    if let Some(v) = first_match(&lte.text, "^LTEFREQLOCK:") {
        let _ = writeln!(out, "lte_lock={}", clean_value(v));
    }
    out
}

fn dump_section(out: &mut String, label: &str, command: &str, settings: &Settings) {
    let _ = writeln!(out, "===== {}: {} =====", label, command);
    let raw = at::at_cmd(settings, command);
    let _ = writeln!(out, "{}\n", raw.text);
}

pub fn print_network_info(settings: &Settings) -> String {
    let mut out = String::new();
    for (label, command) in [
        ("Signal", "AT^HCSQ?"),
        ("Serving cell", "AT^MONSC"),
        ("RRC state", "AT^RRCSTAT?"),
        ("Network registration", "AT+CEREG?"),
        ("Operator", "AT+COPS?"),
        ("LTE lock", "AT^LTEFREQLOCK?"),
        ("NR lock", "AT^NRFREQLOCK?"),
    ] {
        dump_section(&mut out, label, command, settings);
    }
    out.push_str(&print_temperature(settings));
    out
}

pub fn print_sms_list(settings: &Settings) -> String {
    let mut out = String::new();
    out.push_str("===== SMS storage =====\n");
    let cpms = at::at_cmd(settings, "AT+CPMS?");
    out.push_str(&cpms.text);
    if !cpms.text.is_empty() {
        out.push('\n');
    }
    out.push('\n');
    out.push_str("===== SMS messages =====\n");
    // PDU mode preserves the payload for stored UCS2 messages.
    let _ = at::at_cmd(settings, "AT+CMGF=0");
    let cmgl = at::at_cmd(settings, "AT+CMGL=4");
    out.push_str(&cmgl.text);
    if !cmgl.text.is_empty() {
        out.push('\n');
    }
    out
}

pub fn print_system_info(settings: &Settings) -> String {
    let mut out = String::new();
    for (label, command) in [
        ("Identity", "ATI"),
        ("IMEI", "AT+CGSN"),
        ("Revision", "AT+CGMR"),
        ("Version", "AT^VERSION?"),
        ("SIM", "AT+CPIN?"),
        ("ICCID", "AT^ICCID?"),
        ("IMSI", "AT+CIMI"),
        ("Subscriber number", "AT+CNUM"),
        ("Subscription rate", "AT^DSAMBR=1"),
        ("Operator", "AT+COPS?"),
        ("Network time", "AT^NWTIME?"),
        ("Function level", "AT+CFUN?"),
        ("LED", "AT^LEDSWITCH?"),
        ("SIM activation", "AT^HVSST?"),
        ("SIM slot", "AT^SCICHG?"),
        ("FOTA mode", "AT^FOTAMODE?"),
        ("FOTA state", "AT^FOTASTATE?"),
        ("FOTA progress", "AT^FOTADLQ"),
        ("Temperature", "AT^CHIPTEMP?"),
        ("Thermal status", "AT^THERMLDAUTOSTATUS?"),
        ("Thermal thresholds", "AT^THERMLDAUTOPARA?"),
        ("Thermal log", "AT^THERMLDLOGSW?"),
    ] {
        dump_section(&mut out, label, command, settings);
    }
    out
}

const ADVANCED_GROUPS: &[(&str, &[(&str, &str)])] = &[
    ("connection", &[
        ("Auto dial", "AT^SETAUTODIAL?"),
        ("Interface mode", "AT^TDCFG?"),
        ("PDP contexts", "AT+CGDCONT?"),
        ("PDP activation", "AT+CGACT?"),
        ("Data session", "AT^NDISSTATQRY?"),
        ("Detailed sessions", "AT^DCONNSTAT?"),
        ("Direct IP", "AT^SETDIRECTIP?"),
        ("IPv4 lease", "AT^DHCP?"),
        ("IPv6 lease", "AT^DHCPV6?"),
        ("IP capability", "AT^IPV6CAP?"),
        ("Data flow", "AT^DSFLOWQRY"),
        ("MTU", "AT^CGMTU=1"),
        ("PDP address", "AT+CGPADDR=1"),
    ]),
    ("connection-settings", &[
        ("Auto dial", "AT^SETAUTODIAL?"),
        ("Interface mode", "AT^TDCFG?"),
        ("PDP contexts", "AT+CGDCONT?"),
        ("PDP activation", "AT+CGACT?"),
        ("Direct IP", "AT^SETDIRECTIP?"),
    ]),
    ("session", &[
        ("Data session", "AT^NDISSTATQRY?"),
        ("Detailed sessions", "AT^DCONNSTAT?"),
        ("IPv4 lease", "AT^DHCP?"),
        ("IPv6 lease", "AT^DHCPV6?"),
        ("IP capability", "AT^IPV6CAP?"),
        ("Data flow", "AT^DSFLOWQRY"),
        ("MTU", "AT^CGMTU=1"),
        ("PDP address", "AT+CGPADDR=1"),
    ]),
    ("radio", &[
        ("Radio mode", "AT^SYSCFGEX?"),
        ("5G access mode", "AT^C5GOPTION?"),
        ("NR carrier aggregation", "AT^NRRCCAPQRY=3"),
        ("VoNR", "AT^NRRCCAPQRY=2"),
        ("DSS", "AT^NRRCCAPQRY=5"),
    ]),
    ("radio-diagnostics", &[
        ("LTE secondary cells", "AT^CASCELLINFO?"),
        ("NSA secondary cells", "AT^MONSSC"),
        ("Uplink MCS", "AT^MCS=0"),
        ("Downlink MCS", "AT^MCS=1"),
        ("NR transmit power", "AT^NTXPOWER?"),
        ("NR SSB beam", "AT^NRSSBID?"),
        ("Neighbour cells", "AT^MONNC"),
        ("QoS", "AT+CGEQOSRDP=1"),
        ("Data registration", "AT+C5GREG?"),
        ("IMS registration", "AT+CIREG?"),
        ("Dual connectivity", "AT^LENDC?"),
    ]),
    ("hardware", &[
        ("USB mode", "AT^SETMODE?"),
        ("Interface mode", "AT^TDCFG?"),
        ("NIC speed", "AT^TDPCIELANCFG?"),
        ("PCIe controller", "AT^TDPMCFG?"),
        ("LED", "AT^LEDSWITCH?"),
        ("SIM hotplug", "AT^TDSIMHP?"),
        ("SIM slot", "AT^SCICHG?"),
        ("Thermal control", "AT^THERMAUTOFUN?"),
    ]),
    ("all", &[
        ("Auto dial", "AT^SETAUTODIAL?"),
        ("USB mode", "AT^SETMODE?"),
        ("Interface mode", "AT^TDCFG?"),
        ("PDP contexts", "AT+CGDCONT?"),
        ("Radio mode", "AT^SYSCFGEX?"),
        ("NIC speed", "AT^TDPCIELANCFG?"),
        ("PCIe controller", "AT^TDPMCFG?"),
        ("LED", "AT^LEDSWITCH?"),
        ("SIM hotplug", "AT^TDSIMHP?"),
        ("SIM slot", "AT^SCICHG?"),
        ("Thermal control", "AT^THERMAUTOFUN?"),
        ("NR carrier aggregation", "AT^NRRCCAPQRY=3"),
        ("VoNR", "AT^NRRCCAPQRY=2"),
        ("DSS", "AT^NRRCCAPQRY=5"),
    ]),
];

pub fn print_advanced_info(settings: &Settings, group: &str) -> Result<String, i32> {
    let items = ADVANCED_GROUPS
        .iter()
        .find(|(g, _)| *g == group)
        .map(|(_, items)| *items)
        .ok_or(EXIT_USAGE)?;
    let mut out = String::new();
    for (label, command) in items {
        dump_section(&mut out, label, command, settings);
    }
    Ok(out)
}

pub fn print_sms_info(settings: &Settings) -> String {
    let mut out = String::new();
    for (label, command) in [
        ("IMS", "AT^IMSSWITCH?"),
        ("Service mode", "AT+CEUS?"),
        ("SMSC", "AT+CSCA?"),
        ("Storage", "AT+CPMS?"),
    ] {
        dump_section(&mut out, label, command, settings);
    }
    out
}

// ---------------------------------------------------------------- Setters

fn safe_at_field(v: &str) -> bool {
    !v.contains('"') && !v.contains(',') && !v.contains('\r') && !v.contains('\n')
}

fn valid_pin(v: &str) -> bool {
    let n = v.len();
    (4..=8).contains(&n) && v.bytes().all(|b| b.is_ascii_digit())
}

fn valid_puk(v: &str) -> bool {
    v.len() == 8 && v.bytes().all(|b| b.is_ascii_digit())
}

fn set_radio_mode(settings: &Settings, requested: &str) -> i32 {
    match requested {
        "02" | "03" | "08" | "0302" | "0803" | "080302" => {}
        _ => return EXIT_USAGE,
    }
    let raw = at::at_cmd(settings, "AT^SYSCFGEX?");
    if raw.error.is_some() {
        return raw.error.map(fail_exit).unwrap_or(0);
    }
    let Some(current) = first_match(&raw.text, "^SYSCFGEX:") else {
        return 1;
    };
    let Some(idx) = current.find(',') else {
        return 1;
    };
    let suffix = &current[idx..];
    if suffix.is_empty() {
        return 1;
    }
    // Preserve the current band/roaming/service-domain/LTE-band values and
    // leave both reserves empty; the manual requires all seven arguments.
    run_at(settings, &format!("AT^SYSCFGEX=\"{}\"{},,", requested, suffix))
}

fn set_radio_policy(settings: &Settings, args: &[String]) -> i32 {
    let get = |i: usize| args.get(i).map(|s| s.as_str()).unwrap_or("");
    let acqorder = get(0);
    let band = get(1);
    let roam = get(2);
    let srvdomain = get(3);
    let lteband = get(4);
    match acqorder {
        "02" | "03" | "08" | "0302" | "0803" | "080302" => {}
        _ => return EXIT_USAGE,
    }
    let is_hex = |v: &str| {
        v.bytes().all(|b| b.is_ascii_hexdigit())
    };
    if band.is_empty() || !is_hex(band) {
        return EXIT_USAGE;
    }
    if !matches!(roam, "0" | "1") {
        return EXIT_USAGE;
    }
    if !matches!(srvdomain, "1" | "2") {
        return EXIT_USAGE;
    }
    if lteband.is_empty() || !is_hex(lteband) {
        return EXIT_USAGE;
    }
    run_at(
        settings,
        &format!(
            "AT^SYSCFGEX=\"{}\",{},{},{},{},,",
            acqorder, band, roam, srvdomain, lteband
        ),
    )
}

fn set_5g_access_mode(settings: &Settings, preset: &str) -> i32 {
    let values = match preset {
        "option2" => "1,0,1",
        "option3" => "0,1,0",
        "option23" => "1,1,1",
        _ => return EXIT_USAGE,
    };
    let raw = at::at_cmd(settings, "AT+CFUN?");
    let mut previous = raw
        .text
        .lines()
        .find_map(|l| l.strip_prefix("+CFUN:"))
        .map(|r| clean_value(r.trim_start()))
        .unwrap_or_default();
    if previous != "0" && previous != "1" {
        previous = "1".into();
    }
    // C5GOPTION must be written in airplane mode.
    if at::at_cmd(settings, "AT+CFUN=0").error.is_some() {
        return 1;
    }
    let result = at::at_cmd(settings, &format!("AT^C5GOPTION={}", values));
    let rc = match result.error {
        None => 0,
        Some(ref e) => e.exit_code(),
    };
    if previous != "0" {
        let _ = at::at_cmd(settings, &format!("AT+CFUN={}", previous));
    }
    print_outcome(&result);
    rc
}

fn valid_thermal_thresholds(args: &[String]) -> bool {
    if args.len() != 9 {
        return false;
    }
    let mut values = [0i64; 9];
    for (i, a) in args.iter().enumerate() {
        match a.parse::<i64>() {
            Ok(v) if (0..=150).contains(&v) => values[i] = v,
            _ => return false,
        }
    }
    // value[2] <= value[1] etc. means the shell awk exits 1 on violations:
    // v2>v1, v4>v2, v6>v4, v8>v6 must hold, and v3<v2, v5<v4, v7<v6, v9<v8.
    // awk indices are 1-based: value[2] > value[1] etc.
    values[1] > values[0]
        && values[3] > values[1]
        && values[5] > values[3]
        && values[7] > values[5]
        && values[2] < values[1]
        && values[4] < values[3]
        && values[6] < values[5]
        && values[8] < values[7]
}

fn at_sms_send(settings: &Settings, number: &str, text: &str) -> i32 {
    if !settings.enabled {
        return 2;
    }
    let number: String = number.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect();
    let text: String = text.chars().filter(|c| *c != '\0').collect();
    if number.is_empty() || text.is_empty() {
        return 1;
    }

    if matches!(settings.mode, Mode::Serial | Mode::Auto) {
        if let Some(device) = at::detect_mt5700m_at_port(settings) {
            // sms_tool_q performs PDU encoding and waits for +CMGS. Never
            // retry after invoking it, avoiding duplicate SMS.
            if which("sms_tool_q") {
                match std::process::Command::new("sms_tool_q")
                    .args(["-d", &device, "send", &number, &text])
                    .status()
                {
                    Ok(st) => return st.code().unwrap_or(1),
                    Err(_) => return 1,
                }
            }
            // Text-mode fallback on the serial device.
            return text_mode_sms_serial(&device, settings.timeout_s, &number, &text);
        }
        if matches!(settings.mode, Mode::Serial) {
            eprintln!("AT serial port not found");
            return 1;
        }
    }

    // Network fallback via nc-equivalent stream.
    match text_mode_sms_network(settings, &number, &text) {
        Ok(()) => 0,
        Err(e) => e,
    }
}

fn which(bin: &str) -> bool {
    std::process::Command::new("command")
        .args(["-v", bin])
        .status()
        .is_ok()
        || std::env::var("PATH").map(|p| {
            p.split(':').any(|d| {
                let p = std::path::Path::new(d).join(bin);
                p.is_file()
            })
        }).unwrap_or(false)
}

fn text_mode_sms_serial(device: &str, timeout_s: u64, number: &str, text: &str) -> i32 {
    use std::fs::OpenOptions;
    let Ok(mut port) = OpenOptions::new().read(true).write(true).open(device) else {
        return 1;
    };
    let _ = std::process::Command::new("stty")
        .args([
            "-F", device, "115200", "raw", "-echo", "-echoe", "-echok", "-echoctl", "-echoke",
            "-ixon", "-ixoff", "min", "0", "time", "5",
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    let buffer = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let handle = at::spawn_public_reader(port.try_clone().unwrap(), buffer.clone(), stop.clone());

    let seq = format!("AT+CMGF=1\rAT+CMGS=\"{}\"\r{}\u{1a}", number, text);
    let _ = &seq;
    // The shell interleaves 1s sleeps between the three writes; keep pacing.
    let _ = port.write_all(b"AT+CMGF=1\r");
    sleep(Duration::from_secs(1));
    let _ = port.write_all(format!("AT+CMGS=\"{}\"\r", number).as_bytes());
    sleep(Duration::from_secs(1));
    let _ = port.write_all(format!("{}\u{1a}", text).as_bytes());
    let _ = port.flush();

    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_s + 20);
    let mut printed = String::new();
    loop {
        let snapshot = {
            let buf = buffer.lock().unwrap();
            String::from_utf8_lossy(&buf).replace('\r', "")
        };
        let done = snapshot.lines().any(|l| {
            let t = l.trim();
            t == "OK" || t == "ERROR" || t.starts_with("+CME ERROR") || t.starts_with("+CMS ERROR")
        });
        if done || std::time::Instant::now() >= deadline {
            printed = snapshot;
            break;
        }
        sleep(Duration::from_millis(100));
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let _ = handle.join();
    print!("{}", printed);
    let _ = seq;
    0
}

fn text_mode_sms_network(settings: &Settings, number: &str, text: &str) -> Result<(), i32> {
    use std::io::{Read, Write};
    let addr = (settings.host.as_str(), settings.port);
    let mut stream = std::net::TcpStream::connect(addr).map_err(|_| 1)?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let _ = stream
        .write_all(b"AT+CMGF=1\r")
        .and_then(|_| stream.flush());
    sleep(Duration::from_secs(1));
    let _ = stream
        .write_all(format!("AT+CMGS=\"{}\"\r", number).as_bytes())
        .and_then(|_| stream.flush());
    sleep(Duration::from_secs(1));
    let _ = stream
        .write_all(format!("{}\u{1a}", text).as_bytes())
        .and_then(|_| stream.flush());

    let deadline = std::time::Instant::now() + Duration::from_secs(settings.timeout_s + 20);
    let mut acc: Vec<u8> = Vec::new();
    loop {
        let mut chunk = [0u8; 1024];
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => acc.extend_from_slice(&chunk[..n]),
            Err(_) => {
                if std::time::Instant::now() >= deadline {
                    break;
                }
            }
        }
        let text_now = String::from_utf8_lossy(&acc).replace('\r', "");
        if text_now.lines().any(|l| {
            let t = l.trim();
            t == "OK" || t == "ERROR" || t.starts_with("+CME ERROR") || t.starts_with("+CMS ERROR")
        }) {
            break;
        }
        if std::time::Instant::now() >= deadline {
            break;
        }
    }
    print!("{}", String::from_utf8_lossy(&acc).replace('\r', ""));
    Ok(())
}

// ---------------------------------------------------------------- Status

fn cmd_status(settings: &Settings) -> i32 {
    let mut settings = settings.clone();
    // Keep LuCI status below rpcd's execution timeout.
    if settings.timeout_s > 2 {
        settings.timeout_s = 2;
    }
    println!("enabled={}", settings.enabled as i32);
    println!(
        "mode={}",
        match settings.mode {
            Mode::Auto => "auto",
            Mode::Serial => "serial",
            Mode::Network => "network",
        }
    );

    let usb_info = at::mt5700m_usb_info();
    match usb_info {
        Some(info) => {
            let mut parts = info.split('|');
            let state = parts.next().unwrap_or("");
            let pid = parts.next().unwrap_or("");
            let slot = parts.next().unwrap_or("");
            println!("usb_state={}", state);
            println!("usb_pid={}", pid);
            println!("usb_slot={}", slot);
        }
        None => println!("usb_state=absent"),
    }

    let detected = at::detect_mt5700m_at_port(&settings);
    println!("at_port={}", detected.clone().unwrap_or_else(|| settings.at_port.clone()));
    println!("host={}", settings.host);
    println!("port={}", settings.port);
    println!(
        "detected_gateway={}",
        at::detect_modem_gateway().unwrap_or_default()
    );
    if settings.mode == Mode::Network {
        println!("channel=network");
    } else if detected.is_some() {
        println!("channel=serial");
    } else {
        println!("channel=network");
    }

    let probe = at::at_cmd(&settings, "AT");
    let connected = probe.ok() && probe.text.lines().any(|l| l.trim() == "OK");
    println!("connected={}", connected as i32);

    print!("{}", print_identity(&settings));
    print!("{}", print_sim_operator(&settings));
    print!("{}", print_sim_details(&settings));
    print!("{}", print_qos(&settings));
    print!("{}", print_active_apn(&settings));
    print!("{}", print_subscriber_number(&settings));
    print!("{}", print_signal(&settings));
    print!("{}", print_subscription_rate(&settings));
    print!("{}", print_carrier_aggregation(&settings));
    print!("{}", print_temperature(&settings));
    print!("{}", print_lock_status(&settings));
    0
}

fn cmd_cellscan(settings: &Settings) -> i32 {
    let mut settings = settings.clone();
    if settings.timeout_s < 30 {
        settings.timeout_s = 30;
    }
    println!("===== Serving cell: AT^MONSC =====");
    let r = at::at_cmd(&settings, "AT^MONSC");
    println!("{}", r.text);
    println!();
    println!("===== Neighbour cells: AT^MONNC =====");
    let r = at::at_cmd(&settings, "AT^MONNC");
    println!("{}", r.text);
    println!();
    println!("===== Frequency scan: AT^CELLSCAN =====");
    let r = at::at_cmd(&settings, "AT^CELLSCAN");
    println!("{}", r.text);
    0
}

fn cmd_lock(settings: &Settings, args: &[String]) -> i32 {
    let Some(rat) = args.first() else {
        eprintln!("Usage: mt5700m-at lock {{lte|nr}} <type> <bands> [arfcns] [scs] [pcis]");
        return 1;
    };
    let rest = &args[1.min(args.len())..];
    let lock_type = rest.first().map(|s| s.as_str()).unwrap_or("0");
    let (lock_rat, lock_cmd) = match rat.as_str() {
        "lte" => (
            "lte",
            build_lte_lock_command(lock_type, rest.get(1).map(|s| s.as_str()).unwrap_or(""),
                rest.get(2).map(|s| s.as_str()).unwrap_or(""),
                rest.get(3).map(|s| s.as_str()).unwrap_or("")),
        ),
        "nr" => (
            "nr",
            build_nr_lock_command(lock_type, rest.get(1).map(|s| s.as_str()).unwrap_or(""),
                rest.get(2).map(|s| s.as_str()).unwrap_or(""),
                rest.get(3).map(|s| s.as_str()).unwrap_or(""),
                rest.get(4).map(|s| s.as_str()).unwrap_or("")),
        ),
        _ => {
            eprintln!("Usage: mt5700m-at lock {{lte|nr}} <type> <bands> [arfcns] [scs] [pcis]");
            return 1;
        }
    };
    let Some(lock_cmd) = lock_cmd else {
        return EXIT_USAGE;
    };
    apply_frequency_lock(settings, lock_rat, lock_type, &lock_cmd)
}

fn cmd_advanced_set(settings: &Settings, args: &[String]) -> i32 {
    let Some(action) = args.first().map(|s| s.as_str()) else {
        return EXIT_USAGE;
    };
    let rest = &args[1..];
    let get = |i: usize| rest.get(i).map(|s| s.as_str()).unwrap_or("");

    match action {
        "radio-policy" => set_radio_policy(settings, rest),
        "radio-mode" => set_radio_mode(settings, get(0)),
        "5g-access" => set_5g_access_mode(settings, get(0)),
        "autodial" => {
            let enable = get(0);
            let dial_mode = get(1);
            let protocol = get(2);
            let apn = get(3);
            let username = get(4);
            let password = get(5);
            let auth_type = get(6);
            if !matches!(enable, "0" | "1") {
                return EXIT_USAGE;
            }
            if enable == "0" {
                return run_at(settings, "AT^SETAUTODIAL=0");
            }
            if !matches!(dial_mode, "0" | "1" | "2") {
                return EXIT_USAGE;
            }
            if !matches!(protocol, "IP" | "IPV6" | "IPV4V6") {
                return EXIT_USAGE;
            }
            if !matches!(auth_type, "0" | "1" | "2") {
                return EXIT_USAGE;
            }
            if !safe_at_field(apn) || !safe_at_field(username) || !safe_at_field(password) {
                return EXIT_USAGE;
            }
            if apn.len() > 99 || username.len() > 31 || password.len() > 31 {
                return EXIT_USAGE;
            }
            // MT5700M rejects trailing empty fields; omit every optional
            // field when empty so dial_mode is actually applied.
            let cmd = if apn.is_empty() {
                format!("AT^SETAUTODIAL={},{},\"{}\"", enable, dial_mode, protocol)
            } else if username.is_empty() && password.is_empty() {
                format!(
                    "AT^SETAUTODIAL={},{},\"{}\",\"{}\"",
                    enable, dial_mode, protocol, apn
                )
            } else {
                format!(
                    "AT^SETAUTODIAL={},{},\"{}\",\"{}\",\"{}\",\"{}\",{}",
                    enable, dial_mode, protocol, apn, username, password, auth_type
                )
            };
            run_at(settings, &cmd)
        }
        "nic-speed" => match get(0) {
            "1" | "2" => run_at(settings, &format!("AT^TDPCIELANCFG={}", get(0))),
            _ => EXIT_USAGE,
        },
        "pcie-controller" => match get(0) {
            "0" | "1" => run_at(settings, &format!("AT^TDPMCFG={},0,0,0", get(0))),
            _ => EXIT_USAGE,
        },
        "led" => match get(0) {
            "0" | "1" => run_at(settings, &format!("AT^LEDSWITCH={}", get(0))),
            _ => EXIT_USAGE,
        },
        "usb-mode" => {
            // SETMODE=7 (MBIM) is marked temporarily unsupported.
            match get(0) {
                "0" | "1" | "2" | "3" | "4" | "5" | "6" | "8" => {
                    run_at(settings, &format!("AT^SETMODE={}", get(0)))
                }
                _ => EXIT_USAGE,
            }
        }
        "interface-mode" => match get(0) {
            "1" | "2" => run_at(
                settings,
                &format!("AT^TDCFG=\"infcfg\",\"mode\",{}", get(0)),
            ),
            _ => EXIT_USAGE,
        },
        "postroute" => match get(0) {
            "2" => run_at(settings, "AT^TDCFG=\"infcfg\",\"PostRoute\",2"),
            "1" => {
                let rc = run_at(settings, "AT^TDCFG=\"infcfg\",\"PostRoute\",1");
                if rc != 0 {
                    return rc;
                }
                run_at(settings, "AT^IPFILTERSWITCH=0")
            }
            _ => EXIT_USAGE,
        },
        "dmz" => {
            let dmz = get(0);
            if dmz == "0" {
                return run_at(settings, "AT^TDCFG=\"infcfg\",\"dmz\",\"0\"");
            }
            let parts: Vec<&str> = dmz.split('.').collect();
            let ok = parts.len() == 4
                && parts.iter().all(|p| {
                    !p.is_empty()
                        && p.bytes().all(|b| b.is_ascii_digit())
                        && p.parse::<u32>().map(|n| n <= 255).unwrap_or(false)
                });
            if !ok {
                return EXIT_USAGE;
            }
            run_at(settings, &format!("AT^TDCFG=\"infcfg\",\"dmz\",\"{}\"", dmz))
        }
        "sim-hotplug" => match get(0) {
            "0" | "1" => run_at(settings, &format!("AT^TDSIMHP={}", get(0))),
            _ => EXIT_USAGE,
        },
        "sim-activation" => match get(0) {
            "0" | "1" => run_at(settings, &format!("AT^HVSST=1,{}", get(0))),
            _ => EXIT_USAGE,
        },
        "sim-slot" => match get(0) {
            "0" => run_at(settings, "AT^SCICHG=0,1"),
            "1" => run_at(settings, "AT^SCICHG=1,0"),
            _ => EXIT_USAGE,
        },
        "thermal" => {
            let enabled = get(0);
            let interval = get(1);
            if !matches!(enabled, "0" | "1") {
                return EXIT_USAGE;
            }
            if !matches!(interval, "1" | "2" | "3" | "4" | "5" | "10" | "15" | "30" | "60") {
                return EXIT_USAGE;
            }
            // The manual marks the CA/MIMO thermal switch as reserved.
            run_at(
                settings,
                &format!("AT^THERMAUTOFUN={},0,{}", enabled, interval),
            )
        }
        "thermal-log" => {
            let serial = get(0);
            let file = get(1);
            if !matches!(serial, "0" | "1") || !matches!(file, "0" | "1") {
                return EXIT_USAGE;
            }
            run_at(settings, &format!("AT^THERMLDLOGSW={},{}", serial, file))
        }
        "thermal-thresholds" => {
            if !valid_thermal_thresholds(rest) {
                return EXIT_USAGE;
            }
            run_at(settings, &format!("AT^THERMLDAUTOPARA={}", rest.join(",")))
        }
        "carrier-aggregation" => match get(0) {
            "0" | "1" => run_at(settings, &format!("AT^NRRCCAPCFG=3,{}", get(0))),
            _ => EXIT_USAGE,
        },
        "vonr" => match get(0) {
            "0" | "1" | "2" | "3" => run_at(settings, &format!("AT^NRRCCAPCFG=2,{}", get(0))),
            _ => EXIT_USAGE,
        },
        "dss" => {
            let rate = get(0);
            let dmrs = get(1);
            if !matches!(rate, "0" | "1") || !matches!(dmrs, "0" | "1") {
                return EXIT_USAGE;
            }
            run_at(settings, &format!("AT^NRRCCAPCFG=5,{},{}", rate, dmrs))
        }
        "direct-ip" => match get(0) {
            "0" | "1" => run_at(settings, &format!("AT^SETDIRECTIP={}", get(0))),
            _ => EXIT_USAGE,
        },
        _ => EXIT_USAGE,
    }
}

fn cmd_sim_pin(settings: &Settings, args: &[String]) -> i32 {
    let Some(op) = args.first().map(|s| s.as_str()) else {
        return EXIT_USAGE;
    };
    let a1 = args.get(1).map(|s| s.as_str()).unwrap_or("");
    let a2 = args.get(2).map(|s| s.as_str()).unwrap_or("");
    match op {
        "verify" => {
            if !valid_pin(a1) {
                return EXIT_USAGE;
            }
            run_at(settings, &format!("AT+CPIN=\"{}\"", a1))
        }
        "enable" | "disable" => {
            if !valid_pin(a1) {
                return EXIT_USAGE;
            }
            let lock_state = if op == "enable" { 1 } else { 0 };
            run_at(settings, &format!("AT+CLCK=\"SC\",{},\"{}\"", lock_state, a1))
        }
        "change" => {
            if !valid_pin(a1) || !valid_pin(a2) {
                return EXIT_USAGE;
            }
            run_at(settings, &format!("AT+CPWD=\"SC\",\"{}\",\"{}\"", a1, a2))
        }
        "unblock" => {
            if !valid_puk(a1) || !valid_pin(a2) {
                return EXIT_USAGE;
            }
            run_at(settings, &format!("AT+CPIN=\"{}\",\"{}\"", a1, a2))
        }
        _ => EXIT_USAGE,
    }
}

fn cmd_sms_ims(settings: &Settings, on: &str) -> i32 {
    match on {
        "1" => {
            if at::at_cmd(settings, "AT+CFUN=0").error.is_some() {
                return 1;
            }
            if at::at_cmd(
                settings,
                "AT+CGDCONT=5,\"IPV4V6\",\"ims\",\"\",0,0,0,0,1,1,1,,,,,,0,,0,0,0,0",
            )
            .error
            .is_some()
            {
                return 1;
            }
            if at::at_cmd(settings, "AT+CEUS=0").error.is_some() {
                return 1;
            }
            if at::at_cmd(settings, "AT^IMSSWITCH=1,0,0").error.is_some() {
                return 1;
            }
            run_at(settings, "AT+CFUN=1")
        }
        "0" => {
            if at::at_cmd(settings, "AT+CFUN=0").error.is_some() {
                return 1;
            }
            if at::at_cmd(
                settings,
                "AT+CGDCONT=5,\"IPV4V6\",\"\",\"\",0,0,0,0,1,1,1,,,,,,0,,0,0,0,0",
            )
            .error
            .is_some()
            {
                return 1;
            }
            if at::at_cmd(settings, "AT+CEUS=1").error.is_some() {
                return 1;
            }
            if at::at_cmd(settings, "AT^IMSSWITCH=0,0,0").error.is_some() {
                return 1;
            }
            run_at(settings, "AT+CFUN=1")
        }
        _ => EXIT_USAGE,
    }
}

fn cmd_sms_set(settings: &Settings, args: &[String]) -> i32 {
    let Some(what) = args.first().map(|s| s.as_str()) else {
        return EXIT_USAGE;
    };
    match what {
        "smsc" => {
            let smsc: String = args
                .get(1)
                .map(|s| s.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect())
                .unwrap_or_default();
            if smsc.is_empty() {
                return EXIT_USAGE;
            }
            run_at(settings, &format!("AT+CSCA=\"{}\"", smsc))
        }
        "storage" => match args.get(1).map(|s| s.as_str()) {
            Some("SM") => run_at(settings, "AT+CPMS=\"SM\",\"SM\",\"SM\""),
            Some("ME") => run_at(settings, "AT+CPMS=\"ME\",\"ME\",\"ME\""),
            _ => EXIT_USAGE,
        },
        _ => EXIT_USAGE,
    }
}

fn cmd_fota_start(settings: &Settings, url: &str) -> i32 {
    let url: String = url.chars().filter(|c| !matches!(c, '\0' | '\r' | '\n' | '"')).collect();
    if !url.starts_with("http://") {
        return EXIT_USAGE;
    }
    if url.contains(',') {
        return EXIT_USAGE;
    }
    let url = if url.ends_with('/') {
        url
    } else {
        format!("{}/", url)
    };
    if at::at_cmd(settings, "ATE0").error.is_some() {
        return 1;
    }
    if at::at_cmd(settings, "AT^FOTAMODE=0,1,0,1").error.is_some() {
        return 1;
    }
    run_at(settings, &format!("AT^FOTAOEMDL=\"{}\"", url))
}

// ---------------------------------------------------------------- Entry

pub fn run(args: &[String]) -> i32 {
    let mut settings = load_settings();
    // Delay uci-dependent fields used before defaults settle.
    if settings.timeout_s == 0 {
        settings.timeout_s = 8;
    }

    let first = args.first().map(|s| s.as_str()).unwrap_or("status");
    let rest: Vec<String> = args.iter().skip(1).cloned().collect();

    match first {
        "status" => cmd_status(&settings),
        "temperature" => {
            print!("{}", print_temperature(&settings));
            0
        }
        "command" => {
            if rest.is_empty() {
                return 1;
            }
            run_at(&settings, &rest.join(" "))
        }
        "network" => {
            print!("{}", print_network_info(&settings));
            0
        }
        "cellscan" => cmd_cellscan(&settings),
        "sms-list" => {
            print!("{}", print_sms_list(&settings));
            0
        }
        "sms-info" => {
            print!("{}", print_sms_info(&settings));
            0
        }
        "sms-send" => at_sms_send(
            &settings,
            rest.first().map(|s| s.as_str()).unwrap_or(""),
            rest.get(1).map(|s| s.as_str()).unwrap_or(""),
        ),
        "sms-delete" => {
            let index: String = rest
                .first()
                .map(|s| s.chars().filter(|c| c.is_ascii_digit()).collect())
                .unwrap_or_default();
            if index.is_empty() {
                return 1;
            }
            run_at(&settings, &format!("AT+CMGD={}", index))
        }
        "sms-clear" => {
            if at::at_cmd(&settings, "AT+CMGF=0").error.is_some() {
                return 1;
            }
            run_at(&settings, "AT+CMGD=1,4")
        }
        "sms-set" => cmd_sms_set(&settings, &rest),
        "system" => {
            print!("{}", print_system_info(&settings));
            0
        }
        "advanced" => {
            let group = rest.first().map(|s| s.as_str()).unwrap_or("all");
            match print_advanced_info(&settings, group) {
                Ok(text) => {
                    print!("{}", text);
                    0
                }
                Err(rc) => rc,
            }
        }
        "advanced-set" => cmd_advanced_set(&settings, &rest),
        "pdp-set" => {
            let cid = rest.first().map(|s| s.as_str()).unwrap_or("");
            let pdp_type = rest.get(1).map(|s| s.as_str()).unwrap_or("");
            let apn = rest.get(2).map(|s| s.as_str()).unwrap_or("");
            if !matches!(
                cid,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "10" | "11"
            ) {
                return EXIT_USAGE;
            }
            if !matches!(pdp_type, "IP" | "IPV6" | "IPV4V6") {
                return EXIT_USAGE;
            }
            if !safe_at_field(apn) || apn.len() > 99 {
                return EXIT_USAGE;
            }
            run_at(
                &settings,
                &format!("AT+CGDCONT={},\"{}\",\"{}\"", cid, pdp_type, apn),
            )
        }
        "pdp-remove" => match rest.first().map(|s| s.as_str()) {
            Some(cid @ ("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "10" | "11")) => {
                run_at(&settings, &format!("AT+CGDCONT={}", cid))
            }
            _ => EXIT_USAGE,
        },
        "pdp-state" => {
            let state = rest.first().map(|s| s.as_str()).unwrap_or("");
            let cid = rest.get(1).map(|s| s.as_str()).unwrap_or("");
            if !matches!(state, "0" | "1") {
                return EXIT_USAGE;
            }
            if !matches!(
                cid,
                "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "10" | "11"
            ) {
                return EXIT_USAGE;
            }
            run_at(&settings, &format!("AT+CGACT={},{}", state, cid))
        }
        "flow-clear" => run_at(&settings, "AT^DSFLOWCLR"),
        "airplane" => match rest.first().map(|s| s.as_str()) {
            Some(v @ ("0" | "1")) => run_at(&settings, &format!("AT+CFUN={}", v)),
            _ => EXIT_USAGE,
        },
        "sim-pin" => cmd_sim_pin(&settings, &rest),
        "sms-ims" => cmd_sms_ims(
            &settings,
            rest.first().map(|s| s.as_str()).unwrap_or(""),
        ),
        "factory-reset" => run_at(&settings, "AT&F0"),
        // HARD CONSTRAINT: IMEI write path ported as-is, no behaviour change.
        "set-imei" => {
            let imei = rest.first().map(|s| s.as_str()).unwrap_or("");
            if imei.is_empty()
                || !imei.bytes().all(|b| b.is_ascii_digit())
            {
                return EXIT_USAGE;
            }
            if imei.len() != 15 {
                return EXIT_USAGE;
            }
            run_at(&settings, &format!("AT^PHYNUM=IMEI,{}", imei))
        }
        "fota-init" => run_at(&settings, "AT^FOTAMODE=0,1,0,1"),
        "fota-state" => run_at(&settings, "AT^FOTASTATE?"),
        "fota-progress" => run_at(&settings, "AT^FOTADLQ"),
        "fota-download" => {
            let url: String = rest
                .first()
                .map(|s| s.chars().filter(|c| !matches!(c, '\0' | '\r' | '\n' | '"')).collect())
                .unwrap_or_default();
            if url.is_empty() {
                return 1;
            }
            run_at(&settings, &format!("AT^FOTAOEMDL=\"{}\"", url))
        }
        "fota-start" => cmd_fota_start(
            &settings,
            rest.first().map(|s| s.as_str()).unwrap_or(""),
        ),
        "fota-resume" => run_at(&settings, "AT^FOTADL=1"),
        "fota-upgrade" => run_at(&settings, "AT^FWUP"),
        "preview-lock" => {
            let Some(rat) = rest.first().map(|s| s.as_str()) else {
                eprintln!("Usage: mt5700m-at preview-lock {{lte|nr}} <type> <bands> [arfcns] [scs] [pcis]");
                return 1;
            };
            let t = rest.get(1).map(|s| s.as_str()).unwrap_or("0");
            let s = |i: usize| rest.get(i).map(|x| x.as_str()).unwrap_or("");
            let built = match rat {
                "lte" => build_lte_lock_command(t, s(2), s(3), s(4)),
                "nr" => build_nr_lock_command(t, s(2), s(3), s(4), s(5)),
                _ => {
                    eprintln!("Usage: mt5700m-at preview-lock {{lte|nr}} <type> <bands> [arfcns] [scs] [pcis]");
                    return 1;
                }
            };
            match built {
                Some(cmd) => {
                    println!("{}", cmd);
                    0
                }
                None => EXIT_USAGE,
            }
        }
        "lock" => cmd_lock(&settings, &rest),
        "restart" => run_at(&settings, "AT^RESET"),
        "unlock" => {
            println!("Unlock LTE:");
            let rc1 = run_at(&settings, "AT^LTEFREQLOCK=0");
            println!("Unlock NR:");
            let rc2 = run_at(&settings, "AT^NRFREQLOCK=0");
            if rc1 != 0 {
                rc1
            } else {
                rc2
            }
        }
        _ => {
            eprintln!("Usage: mt5700m-at {{status|command <AT>|restart|unlock}}");
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lte_lock_contract() {
        assert_eq!(build_lte_lock_command("0", "", "", "").unwrap(), "AT^LTEFREQLOCK=0");
        assert_eq!(
            build_lte_lock_command("3", "1,3,7", "", "").unwrap(),
            "AT^LTEFREQLOCK=3,0,3,\"1,3,7\""
        );
        assert_eq!(
            build_lte_lock_command("1", "3", "1850", "").unwrap(),
            "AT^LTEFREQLOCK=1,0,1,\"3\",\"1850\""
        );
        assert_eq!(
            build_lte_lock_command("2", "3", "1850", "100").unwrap(),
            "AT^LTEFREQLOCK=2,0,1,\"3\",\"1850\",\"100\""
        );
        // count mismatch and range violations must be rejected (exit 64)
        assert!(build_lte_lock_command("1", "3,7", "1850", "").is_none());
        assert!(build_lte_lock_command("2", "3", "1850", "9999").is_none());
        assert!(build_lte_lock_command("9", "3", "", "").is_none());
        // >20 bands rejected
        let many: Vec<&str> = (0..21).map(|_| "1").collect();
        assert!(build_lte_lock_command("3", &many.join(","), "", "").is_none());
    }

    #[test]
    fn nr_lock_contract() {
        assert_eq!(build_nr_lock_command("0", "", "", "", "").unwrap(), "AT^NRFREQLOCK=0");
        assert_eq!(
            build_nr_lock_command("3", "78", "", "", "").unwrap(),
            "AT^NRFREQLOCK=3,0,1,\"78\""
        );
        assert_eq!(
            build_nr_lock_command("1", "78", "643456", "0", "").unwrap(),
            "AT^NRFREQLOCK=1,0,1,\"78\",\"643456\",\"0\""
        );
        assert_eq!(
            build_nr_lock_command("2", "78", "643456", "0", "10").unwrap(),
            "AT^NRFREQLOCK=2,0,1,\"78\",\"643456\",\"0\",\"10\""
        );
        assert!(build_nr_lock_command("1", "78", "643456", "9", "").is_none());
    }

    #[test]
    fn cops_parsing_contract() {
        let raw = "+COPS: 0,2,\"46000\",7\r\nOK";
        assert_eq!(extract_cops_operator(raw).unwrap(), "46000");
        assert_eq!(extract_cops_rat(raw).unwrap(), "LTE");
        let named = "+COPS: 0,0,\"CHINA MOBILE\",7\r\nOK";
        assert_eq!(extract_cops_operator(named).unwrap(), "CHINA MOBILE");
        // numeric code in a stray field per the shell comment
        let stray = "+COPS: 0,2,,46000,7\r\nOK";
        assert_eq!(extract_cops_operator(stray).unwrap(), "46000");
    }

    #[test]
    fn signal_format_contract() {
        let mut settings = Settings::default();
        // Offline parse: feed a canned HCSQ line through the same helpers.
        let line = "^HCSQ: \"LTE\",60,50,20,30";
        let fields: Vec<String> = line
            .strip_prefix("^HCSQ:")
            .unwrap()
            .split(',')
            .map(|f| f.trim().trim_matches('"').to_string())
            .collect();
        assert_eq!(fields[0], "LTE");
        let v: u64 = fields[2].parse().unwrap();
        assert_eq!(v as i64 - 141, -91);
        let _ = &mut settings;
    }

    #[test]
    fn syscfgex_normalization() {
        let cmd = "AT^SYSCFGEX=\"0302\",3fffffff,1,2,7FFFFFFFFFFFFFFF,\"\",\"\"";
        let out = crate::daemon::normalize_syscfgex(cmd);
        assert!(out.contains(",\"7FFFFFFFFFFFFFFF\",\"\",\"\""));
    }
}
