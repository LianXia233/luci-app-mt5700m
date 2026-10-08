//! `^HCSQ` -> `SignalState`.
//!
//! The ONLY place in the codebase that knows how an MT5700M signal report is
//! decoded. Both the cache (WebUI) and the CLI text output (LuCI) used to
//! carry their own copy of these formulas; they are merged here so a single
//! reading can never produce two different numbers.

use crate::core::json;
use crate::modules::signal::state::SignalState;
use crate::state::refresh::round1;

/// Raw field is usable only when it is a plain decimal and not the modem's
/// "invalid" marker (255).
fn valid(v: &str) -> bool {
    !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()) && v != "255"
}

fn to_num(v: &str) -> u64 {
    v.parse::<u64>().unwrap_or(0)
}

/// RSRP (NR/LTE): index -> dBm, saturating at the top of the range.
fn rsrp_dbm(v: &str) -> i64 {
    let n = to_num(v) as i64;
    if n >= 97 {
        -44
    } else {
        n - 141
    }
}

/// RSCP (WCDMA): index -> dBm.
fn rscp_dbm(v: &str) -> i64 {
    let n = to_num(v) as i64;
    if n >= 96 {
        -25
    } else {
        n - 121
    }
}

/// SINR: index -> dB (0.2 dB per step from -20.2 dB).
fn sinr_db(v: &str) -> f64 {
    let n = to_num(v);
    if n >= 251 {
        30.0
    } else {
        round1(-20.2 + n as f64 * 0.2)
    }
}

/// RSRQ: index -> dB (0.5 dB per step from -20 dB).
fn rsrq_db(v: &str) -> f64 {
    let n = to_num(v);
    if n >= 34 {
        -3.0
    } else {
        round1(-20.0 + n as f64 * 0.5)
    }
}

/// Parse the modem answer of `AT^HCSQ?`.
///
/// `^HCSQ: "LTE",rssi,rsrp,sinr,rsrq,…` / `^HCSQ: "NR",rsrp,sinr,rsrq,…`
pub fn parse(raw: &str) -> SignalState {
    let mut st = SignalState::default();
    let Some(body) = raw.lines().find_map(|l| l.trim().strip_prefix("^HCSQ:")) else {
        return st;
    };
    let fields: Vec<String> = body
        .split(',')
        .map(|f| f.trim().trim_matches('"').to_string())
        .collect();
    if fields.is_empty() {
        return st;
    }
    let get = |i: usize| fields.get(i).map(|s| s.as_str()).unwrap_or("");
    st.sysmode = fields[0].clone();
    match fields[0].as_str() {
        "NR" => {
            if valid(get(1)) {
                st.rsrp = Some(rsrp_dbm(get(1)));
            }
            if valid(get(2)) {
                st.sinr = Some(sinr_db(get(2)));
            }
            if valid(get(3)) {
                st.rsrq = Some(rsrq_db(get(3)));
            }
        }
        "LTE" => {
            if valid(get(1)) {
                st.rssi = Some(to_num(get(1)) as i64 - 121);
            }
            if valid(get(2)) {
                st.rsrp = Some(rsrp_dbm(get(2)));
            }
            if valid(get(3)) {
                st.sinr = Some(sinr_db(get(3)));
            }
            if valid(get(4)) {
                st.rsrq = Some(rsrq_db(get(4)));
            }
        }
        "WCDMA" => {
            if valid(get(1)) {
                st.rssi = Some(to_num(get(1)) as i64 - 121);
            }
            if valid(get(2)) {
                st.rscp = Some(rscp_dbm(get(2)));
            }
            if valid(get(3)) {
                st.ecio = Some(round1(-32.5 + to_num(get(3)) as f64 * 0.5));
            }
        }
        "GSM" => {
            if valid(get(1)) {
                st.rssi = Some(to_num(get(1)) as i64 - 121);
            }
        }
        _ => {}
    }
    st
}

/// The cache/JSON shape historically written by the snapshot collector; kept
/// as a helper so existing call sites migrate without changing their contract.
pub fn parse_json(raw: &str) -> json::Value {
    parse(raw).to_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lte_report() {
        let st = parse("^HCSQ: \"LTE\",56,80,72,20\r\nOK");
        assert_eq!(st.sysmode, "LTE");
        assert_eq!(st.rssi, Some(56 - 121));
        assert_eq!(st.rsrp, Some(80 - 141));
        assert_eq!(st.sinr, Some(round1(-20.2 + 72.0 * 0.2)));
        assert_eq!(st.rsrq, Some(round1(-20.0 + 20.0 * 0.5)));
    }

    #[test]
    fn nr_report() {
        let st = parse("^HCSQ: \"NR\",87,160,12");
        assert_eq!(st.sysmode, "NR");
        assert_eq!(st.rsrp, Some(-54));
        assert_eq!(st.sinr, Some(round1(-20.2 + 160.0 * 0.2)));
        assert_eq!(st.rsrq, Some(round1(-20.0 + 12.0 * 0.5)));
        assert_eq!(st.rssi, None);
    }

    #[test]
    fn unknown_rat_is_kept_but_empty() {
        let st = parse("^HCSQ: \"CDMA\",1,2,3");
        assert_eq!(st.sysmode, "CDMA");
        assert!(st.rsrp.is_none() && st.rsrq.is_none());
    }

    #[test]
    fn invalid_marker_is_skipped() {
        let st = parse("^HCSQ: \"LTE\",255,255,255,255");
        assert_eq!(st.sysmode, "LTE");
        assert!(st.is_empty() == false);
        assert!(st.rssi.is_none() && st.rsrp.is_none());
        assert!(st.sinr.is_none() && st.rsrq.is_none());
    }

    #[test]
    fn missing_line_and_saturation() {
        assert!(parse("OK\r\n").is_empty());
        let st = parse("^HCSQ: \"LTE\",1,99,251,40");
        assert_eq!(st.rsrp, Some(-44));
        assert_eq!(st.sinr, Some(30.0));
        assert_eq!(st.rsrq, Some(-3.0));
    }
}
