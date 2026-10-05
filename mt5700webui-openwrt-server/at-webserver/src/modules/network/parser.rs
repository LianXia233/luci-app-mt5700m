//! `+CxxREG` / `+COPS` / `^SYSINFOEX` decoders — the only place these
//! responses are turned into data.
//!
//! Both the periodic collector (published to the cache for the frontends) and
//! the CLI's `status`/`network` verbs derive their output from here, so a
//! parsing fix can never land on one surface only.

use crate::modules::network::state::RegistrationState;

/// Parse a `+CxxREG:` line: registration state + family-specific fields.
///
/// `+CxxREG: <mode>,<stat>[,...]` — the first field is the unsolicited-report
/// mode, the actual registration state is the second; some older replies carry
/// only the stat (single field).
///
/// Field layout differs per family (3GPP 27.007):
///   * C5GREG: `<n>,<stat>[,<tac>,<ci>,<AcT>,<len>,<NSSAI>]`
///   * CEREG:  `<n>,<stat>[,<tac>,<ci>[,<AcT>]]`
///   * CREG:   `<n>,<stat>[,<lac>,<ci>]` or `<n>,<stat>[,<mcc>,<mnc>,<lac>,<ci>]`
///
/// The WebUI diagnostics panel renders tac/ci/nssai from the 5G family, so a
/// unified field set (tac/ci/act/nssai/mcc/mnc/lac) is exposed and each
/// consumer picks what it understands. Semantics are unchanged from the
/// original collector: later lines overwrite earlier ones, and the state of
/// the last parseable line wins.
pub fn parse_registration(raw: &str) -> RegistrationState {
    let mut st = RegistrationState::default();
    let mut seen = false;
    for line in raw.lines() {
        let t = line.trim();
        let Some(idx) = t.find("REG:") else { continue };
        let body = t[idx + 4..].trim().trim_matches(|c| c == '"' || c == ' ');
        let fields: Vec<&str> = body.split(',').map(|f| f.trim()).collect();
        let stat_field = fields.get(1).or_else(|| fields.first());
        let Some(state) = stat_field.and_then(|f| f.parse::<u8>().ok()) else {
            continue;
        };
        seen = true;
        st.state = state;
        // Keep the most specific registration family (C5GREG > CEREG > CREG)
        // by field count.
        let score = if t.contains("C5GREG") {
            3
        } else if t.contains("CEREG") {
            2
        } else {
            1
        };
        // A 3-digit decimal field is MCC, which identifies the vendor variants
        // that prefix mcc/mnc to the standard layout.
        let is_mcc = |s: &str| {
            let s = s.trim().trim_matches('"');
            s.len() == 3 && s.bytes().all(|b| b.is_ascii_digit())
        };
        match score {
            3 => {
                if fields.get(2).map(|s| is_mcc(s)).unwrap_or(false) && fields.len() >= 7 {
                    put(&mut st, "mcc", fields.get(2));
                    put(&mut st, "mnc", fields.get(3));
                    put(&mut st, "tac", fields.get(4));
                    put(&mut st, "ci", fields.get(5));
                    put(&mut st, "act", fields.get(6));
                    put(&mut st, "nssai", fields.get(8));
                } else {
                    put(&mut st, "tac", fields.get(2));
                    put(&mut st, "ci", fields.get(3));
                    put(&mut st, "act", fields.get(4));
                    put(&mut st, "nssai", fields.get(6));
                }
            }
            2 => {
                if fields.get(2).map(|s| is_mcc(s)).unwrap_or(false) {
                    put(&mut st, "mcc", fields.get(2));
                    put(&mut st, "mnc", fields.get(3));
                    put(&mut st, "tac", fields.get(4));
                    put(&mut st, "ci", fields.get(5));
                } else {
                    put(&mut st, "tac", fields.get(2));
                    put(&mut st, "ci", fields.get(3));
                    put(&mut st, "act", fields.get(4));
                }
            }
            _ => {
                if fields.len() >= 6 {
                    put(&mut st, "mcc", fields.get(2));
                    put(&mut st, "mnc", fields.get(3));
                    put(&mut st, "lac", fields.get(4));
                    put(&mut st, "ci", fields.get(5));
                } else {
                    put(&mut st, "lac", fields.get(2));
                    put(&mut st, "ci", fields.get(3));
                }
            }
        }
    }
    let _ = seen;
    st
}

fn put(st: &mut RegistrationState, key: &str, value: Option<&&str>) {
    let Some(v) = value else { return };
    let s = v.trim().trim_matches('"');
    if s.is_empty() {
        return;
    }
    let slot = match key {
        "tac" => &mut st.tac,
        "ci" => &mut st.ci,
        "act" => &mut st.act,
        "nssai" => &mut st.nssai,
        "mcc" => &mut st.mcc,
        "mnc" => &mut st.mnc,
        "lac" => &mut st.lac,
        _ => return,
    };
    *slot = Some(s.to_string());
}

/// Quoted or numeric MCC-MNC operator name from `+COPS:`.
pub fn parse_cops_operator(raw: &str) -> Option<String> {
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

/// Access technology from `+COPS: <mode>,<format>,"<name>",<act>`.
pub fn parse_cops_rat(raw: &str) -> Option<String> {
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

/// Map the 3GPP AcT code to the string the frontends display.
pub fn normalize_rat(rat: &str) -> String {
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

/// Mode string from `^SYSINFOEX` (quoted field when the firmware quotes it).
pub fn parse_sysinfo_mode(raw: &str) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_cereg_standard() {
        let st = parse_registration("+CEREG: 2,1,\"149002\",\"0000000C2840C001\",7");
        assert_eq!(st.state, 1);
        assert_eq!(st.tac.as_deref(), Some("149002"));
        assert_eq!(st.ci.as_deref(), Some("0000000C2840C001"));
        assert_eq!(st.act.as_deref(), Some("7"));
    }

    #[test]
    fn registration_c5greg_with_nssai() {
        let st = parse_registration("+C5GREG: 2,1,\"149002\",\"0000000C2840C001\",11,4,\"80.03ffff\"");
        assert_eq!(st.state, 1);
        assert_eq!(st.tac.as_deref(), Some("149002"));
        assert_eq!(st.ci.as_deref(), Some("0000000C2840C001"));
        assert_eq!(st.act.as_deref(), Some("11"));
        assert_eq!(st.nssai.as_deref(), Some("80.03ffff"));
    }

    #[test]
    fn registration_vendor_variant_with_mcc_mnc() {
        let st = parse_registration("+C5GREG: 2,1,\"460\",\"01\",\"149002\",\"0000000C2840C001\",7,4,\"80.03ffff\"");
        assert_eq!(st.mcc.as_deref(), Some("460"));
        assert_eq!(st.mnc.as_deref(), Some("01"));
        assert_eq!(st.tac.as_deref(), Some("149002"));
        assert_eq!(st.ci.as_deref(), Some("0000000C2840C001"));
    }

    #[test]
    fn registration_without_reg_line_is_empty() {
        let st = parse_registration("ERROR");
        assert!(st.is_empty());
        assert_eq!(st.state, 0);
    }

    #[test]
    fn cops_operator_and_rat() {
        let raw = "+COPS: 0,0,\"CHN-UNICOM\",7";
        assert_eq!(parse_cops_operator(raw).as_deref(), Some("CHN-UNICOM"));
        assert_eq!(parse_cops_rat(raw).as_deref(), Some("LTE"));
        assert_eq!(parse_cops_operator("+COPS: 0"), None);
    }

    #[test]
    fn sysinfo_mode_quoted_and_bare() {
        assert_eq!(
            parse_sysinfo_mode("^SYSINFOEX: 2,1,0,1,,,\"LTE/NR\"").as_deref(),
            Some("LTE/NR")
        );
        assert_eq!(
            parse_sysinfo_mode("^SYSINFOEX: 2,1,0,1").as_deref(),
            Some("2,1,0,1")
        );
        assert_eq!(parse_sysinfo_mode("OK"), None);
    }
}
