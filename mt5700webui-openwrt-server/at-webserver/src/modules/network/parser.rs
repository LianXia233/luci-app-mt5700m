//! `+CxxREG` / `+COPS` / `^SYSINFOEX` decoders — the only place these
//! responses are turned into data.
//!
//! Both the periodic collector (published to the cache for the frontends) and
//! the CLI's `status`/`network` verbs derive their output from here, so a
//! parsing fix can never land on one surface only.

use crate::modules::network::state::{DhcpLease, PdpAddress, RegistrationState};

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
/// One `+CGPADDR` line: `1,"10.0.0.1"` or `1,"32.8.0.2.0.2..."`.
///
/// Port of the WebUI's `parseCgpaddr`/`formatPdpAddress` so the diagnostics
/// panel gets the same strings it used to build in TypeScript — including the
/// IPv6 case, where the modem reports the address as 16 dotted bytes and the
/// UI shows the compressed form.
pub fn parse_cgpaddr(raw: &str) -> Vec<PdpAddress> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let Some(rest) = line.trim().strip_prefix("+CGPADDR:") else {
            continue;
        };
        // <cid> then the first quoted field (the regex the WebUI used also
        // stopped at the first quote, so a dual-stack line contributes the IPv4
        // address only — same behaviour, on purpose).
        let mut parts = rest.splitn(2, ',');
        let cid = match parts.next().map(|c| c.trim().parse::<u32>()) {
            Some(Ok(cid)) => cid,
            _ => continue,
        };
        let tail = parts.next().unwrap_or("").trim();
        let raw_address = if let Some(stripped) = tail.strip_prefix('"') {
            stripped.split('"').next().unwrap_or("").trim()
        } else {
            tail.trim_matches('"').trim()
        };
        if raw_address.is_empty() {
            continue;
        }
        let (address, family) = format_pdp_address(raw_address);
        out.push(PdpAddress {
            cid,
            address,
            family,
        });
    }
    out
}

/// Dotted-bytes address -> display form (IPv4 as-is, IPv6 compressed).
fn format_pdp_address(raw: &str) -> (String, &'static str) {
    let parts: Vec<Option<u8>> = raw.split('.').map(|p| p.trim().parse::<u8>().ok()).collect();
    if parts.len() == 4 && parts.iter().all(|p| p.is_some()) {
        let bytes: Vec<u8> = parts.into_iter().flatten().collect();
        return (
            bytes
                .iter()
                .map(|b| b.to_string())
                .collect::<Vec<_>>()
                .join("."),
            "IPv4",
        );
    }
    if parts.len() == 16 && parts.iter().all(|p| p.is_some()) {
        let bytes: Vec<u8> = parts.into_iter().flatten().collect();
        let groups: Vec<String> = bytes
            .chunks(2)
            .map(|c| format!("{:x}", ((c[0] as u16) << 8) | c[1] as u16))
            .collect();
        return (compress_ipv6(&groups), "IPv6");
    }
    (
        raw.to_string(),
        if raw.contains(':') { "IPv6" } else { "未知" },
    )
}

/// Fold the longest run of zero groups into `::` (RFC 5952's usual spelling),
/// exactly like the TypeScript helper it replaces.
fn compress_ipv6(groups: &[String]) -> String {
    let (mut best_start, mut best_len) = (0usize, 0usize);
    let (mut start, mut len) = (usize::MAX, 0usize);
    for (i, g) in groups.iter().enumerate() {
        if g == "0" {
            if start == usize::MAX {
                start = i;
            }
            len += 1;
            if len > best_len {
                best_len = len;
                best_start = start;
            }
        } else {
            start = usize::MAX;
            len = 0;
        }
    }
    if best_len < 2 {
        return groups.join(":");
    }
    let head = groups[..best_start].join(":");
    let tail = groups[best_start + best_len..].join(":");
    format!("{}::{}", head, tail)
}

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

/// Decode one field block of `^DHCP` / `^DHCPV6` into a [`DhcpLease`].
///
/// Both commands answer `^DHCP: <a>,<b>,<c>,<d>,<e>,<f>` with `a`..`f` =
/// address, netmask, gateway, DHCP server, primary DNS, secondary DNS. IPv6
/// fields are already textual; IPv4 fields are hex-encoded little-endian
/// 32-bit values, exactly as the WebUI decoded them (byte-reversed dotted
/// quad). Fewer than six fields means the firmware answered something else, so
/// nothing is decoded rather than shifting values into the wrong slots.
fn parse_dhcp(raw: &str, marker: &str, ipv4: bool) -> Option<DhcpLease> {
    for line in raw.lines() {
        let t = line.trim();
        let Some(idx) = t.find(marker) else { continue };
        let body = t[idx + marker.len()..].trim_start_matches(':').trim();
        if body.is_empty() {
            continue;
        }
        let fields: Vec<&str> = body.split(',').map(|f| f.trim()).collect();
        if fields.len() < 6 {
            continue;
        }
        let conv = |s: &str| {
            let s = s.trim().trim_matches('"');
            if ipv4 {
                hex_to_ipv4(s)
            } else {
                s.to_string()
            }
        };
        return Some(DhcpLease {
            address: Some(conv(fields[0])),
            netmask: Some(conv(fields[1])),
            gateway: Some(conv(fields[2])),
            dhcp_server: Some(conv(fields[3])),
            primary_dns: Some(conv(fields[4])),
            secondary_dns: Some(conv(fields[5])),
        });
    }
    None
}

/// `^DHCP?` (IPv4, hex fields) -> [`DhcpLease`].
pub fn parse_dhcp_v4(raw: &str) -> Option<DhcpLease> {
    parse_dhcp(raw, "^DHCP:", true)
}

/// `^DHCPV6?` (IPv6, textual fields) -> [`DhcpLease`].
pub fn parse_dhcp_v6(raw: &str) -> Option<DhcpLease> {
    parse_dhcp(raw, "^DHCPV6:", false)
}

/// IPv6 capability code from `^IPV6CAP?` (single decimal value).
pub fn parse_ipv6cap(raw: &str) -> Option<u32> {
    for line in raw.lines() {
        let t = line.trim();
        let Some(idx) = t.find("^IPV6CAP:") else {
            continue;
        };
        let body = t[idx + "^IPV6CAP:".len()..].trim();
        if let Some(code) = body
            .split(|c: char| c == ',' || c.is_whitespace())
            .find_map(|f| f.trim().parse::<u32>().ok())
        {
            return Some(code);
        }
    }
    None
}

/// Hex-encoded little-endian IPv4 field -> dotted quad.
///
/// The firmware sends each 4-byte address as 8 hex digits in reverse byte
/// order, so `0100A8C0` is `192.168.0.1`. Unparseable input keeps the WebUI's
/// historical placeholder instead of dropping the row.
fn hex_to_ipv4(hex: &str) -> String {
    let clean = hex.trim().replace('\r', "").replace('\n', "");
    if clean.is_empty() || !clean.bytes().all(|b| b.is_ascii_hexdigit()) {
        return "0.0.0.0".to_string();
    }
    let mut padded = clean.to_ascii_uppercase();
    if padded.len() != 8 {
        while padded.len() < 8 {
            padded.push('0');
        }
        padded.truncate(8);
    }
    let mut bytes: Vec<u8> = Vec::new();
    for i in (0..8).step_by(2) {
        bytes.push(u8::from_str_radix(&padded[i..i + 2], 16).unwrap_or(0));
    }
    bytes.reverse();
    bytes
        .iter()
        .map(|b| b.to_string())
        .collect::<Vec<_>>()
        .join(".")
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
    fn cgpaddr_ipv4_and_dotted_ipv6() {
        // The IPv6 example is the one in the manual (7.8.5): the modem reports
        // 16 dotted bytes, the UI shows the standard spelling.
        let out = parse_cgpaddr(
            "+CGPADDR: 1,\"10.0.0.1\"\r\n\
             +CGPADDR: 2,\"32.8.0.2.0.2.0.1.255.255.255.255.255.255.255.255\"\r\nOK",
        );
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].cid, out[0].address.as_str(), out[0].family), (1, "10.0.0.1", "IPv4"));
        assert_eq!(out[1].address, "2008:2:2:1:ffff:ffff:ffff:ffff");
        assert_eq!(out[1].family, "IPv6");
    }

    #[test]
    fn cgpaddr_compresses_zero_runs_and_skips_empties() {
        let out = parse_cgpaddr("+CGPADDR: 1,\"\"\n+CGPADDR: 3,\"0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.1\"\n");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].cid, 3);
        assert_eq!(out[0].address, "::1");
    }

    #[test]
    fn cgpaddr_keeps_plain_ipv6_text_and_unquoted_lines() {
        let out = parse_cgpaddr("+CGPADDR: 4,2001:db8::1\n+AT+CGPADDR: nope\n");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].address, "2001:db8::1");
        assert_eq!(out[0].family, "IPv6");
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

    #[test]
    fn dhcp_v4_decodes_hex_little_endian_fields() {
        // 0100A8C0 == 192.168.0.1 (bytes reversed), the firmware's byte order.
        let raw = "^DHCP: 0100A8C0,00FFFFFF,0100A8C0,0100A8C0,08080808,08080404\nOK";
        let lease = parse_dhcp_v4(raw).expect("lease");
        assert_eq!(lease.address.as_deref(), Some("192.168.0.1"));
        assert_eq!(lease.netmask.as_deref(), Some("255.255.255.0"));
        assert_eq!(lease.primary_dns.as_deref(), Some("8.8.8.8"));
        assert_eq!(lease.secondary_dns.as_deref(), Some("4.4.8.8"));
    }

    #[test]
    fn dhcp_v4_invalid_hex_keeps_placeholder() {
        let lease = parse_dhcp_v4("^DHCP: zz,00FFFFFF,0100A8C0,0100A8C0,08080808,08080404").unwrap();
        assert_eq!(lease.address.as_deref(), Some("0.0.0.0"));
    }

    #[test]
    fn dhcp_v6_keeps_textual_fields_and_ignores_short_answers() {
        let raw = "^DHCPV6: 2409:8a00::1,64,2409:8a00::,2409:8a00::1,2400:3200::1,2400:3200:baba::1";
        let lease = parse_dhcp_v6(raw).expect("lease");
        assert_eq!(lease.address.as_deref(), Some("2409:8a00::1"));
        assert_eq!(lease.netmask.as_deref(), Some("64"));
        assert_eq!(lease.secondary_dns.as_deref(), Some("2400:3200:baba::1"));
        // Wrong slot count: nothing is decoded (no shifting).
        assert!(parse_dhcp_v6("^DHCPV6: 2409:8a00::1,64,2409:8a00::").is_none());
        // The other family's line must not match.
        assert!(parse_dhcp_v6("^DHCP: 0100A8C0,00FFFFFF,1,2,3,4").is_none());
    }

    #[test]
    fn ipv6cap_reads_single_code() {
        assert_eq!(parse_ipv6cap("^IPV6CAP: 7\nOK"), Some(7));
        assert_eq!(parse_ipv6cap("^IPV6CAP: 11"), Some(11));
        assert_eq!(parse_ipv6cap("OK"), None);
    }
}
