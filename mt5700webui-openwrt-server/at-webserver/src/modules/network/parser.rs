//! `+CxxREG` / `+COPS` / `^SYSINFOEX` decoders — the only place these
//! responses are turned into data.
//!
//! Both the periodic collector (published to the cache for the frontends) and
//! the CLI's `status`/`network` verbs derive their output from here, so a
//! parsing fix can never land on one surface only.

use crate::modules::network::state::{
    SysCfgState,
    AutodialState, C5gOptionState, DhcpLease, ImsState, InterfaceCfgState, LockItem, LockKind,
    LockState, PdpAddress, PdpContext, RegistrationState, RrcState,
};

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

/// Decode `^C5GOPTION: <nr_sa_support_flag>,<nr_dc_mode>,<gc_access_mode>`.
///
/// An answer that does not carry all three fields leaves them `None`; the page
/// then keeps the value it already had instead of showing zeros.
pub fn parse_c5goption(raw: &str) -> C5gOptionState {
    let mut st = C5gOptionState::default();
    for line in raw.lines() {
        let t = line.trim();
        let Some(idx) = t.find("^C5GOPTION:") else { continue };
        let body = t[idx + "^C5GOPTION:".len()..].trim();
        let fields: Vec<&str> = body.split(',').map(|f| f.trim()).collect();
        if fields.len() < 3 {
            continue;
        }
        st.nr_sa_support_flag = fields[0].parse::<u8>().ok();
        st.nr_dc_mode = fields[1].parse::<u8>().ok();
        st.gc_access_mode = fields[2].parse::<u8>().ok();
        break;
    }
    st
}

/// Decode `^RRCSTAT?`.
///
/// The firmware answers with two or three fields (`^RRCSTAT: 1,1,98` and
/// `^RRCSTAT: 1,98` both occur). The page's rule — and the one kept here — is
/// that the trailing 98/99 is the camped flag and the field before it is the
/// state; without the flag the last field is the state. That reads both forms
/// correctly, where the old fixed indices (`fields[1]`, `fields[2]`) showed the
/// raw `98` as the state on the two-field form.
pub fn parse_rrcstat(raw: &str) -> RrcState {
    let mut st = RrcState::default();
    let Some(body) = raw.lines().find_map(|l| l.trim().strip_prefix("^RRCSTAT:")) else {
        return st;
    };
    let fields: Vec<&str> = body
        .split(',')
        .map(|f| f.trim().trim_matches('"'))
        .filter(|f| !f.is_empty())
        .collect();
    let num = |s: &str| s.parse::<i64>().ok();
    let mut idx = fields.len();
    if let Some(last) = fields.last().copied() {
        if matches!(num(last), Some(98 | 99)) {
            st.camped = num(last);
            idx -= 1;
        }
    }
    if idx > 0 {
        st.state = num(fields[idx - 1]);
    }
    st
}

/// Decode `^LTEFREQLOCK?` / `^NRFREQLOCK?`.
///
/// Shape (manual 13.12.1 / 13.13.1):
///
/// ```text
/// ^LTEFREQLOCK: <type>
/// <mobility>,<num>
/// <band>,<arfcn>,<pci>          (LTE, num rows)
///
/// ^NRFREQLOCK: <type>
/// <mobility>,<num>
/// <band>,<arfcn>,<scstype>,<pci> (NR, num rows)
/// ```
///
/// `type 0` (unlocked) has no rows at all. PCI is printed in hex by the
/// firmware and exposed in decimal — exactly what the page's parser did, since
/// the lock write takes a decimal PCI.
///
/// `None` means the reply did not carry a lock line at all: callers keep the
/// value they already had instead of showing "unlocked".
pub fn parse_freq_lock(raw: &str, kind: LockKind) -> Option<LockState> {
    let prefix = crate::modules::network::commands::lock_prefix(kind);
    let lines: Vec<&str> = raw
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.contains("OK") && !l.starts_with("AT"))
        .collect();
    let head = lines.iter().position(|l| l.starts_with(prefix))?;
    let lock_type = lines[head][prefix.len()..]
        .trim_start()
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|v| v.parse::<u8>().ok())?;
    if lock_type == 0 {
        return Some(LockState::default());
    }
    let (mobility, num) = match lines.get(head + 1) {
        Some(row) => {
            let mut parts = row.split(',');
            let mobility = parts.next().and_then(|v| v.trim().parse::<u8>().ok()).unwrap_or(0);
            let num = parts
                .next()
                .and_then(|v| v.trim().parse::<usize>().ok())
                .unwrap_or(0);
            (mobility, num)
        }
        None => (0, 0),
    };
    let mut items = Vec::new();
    for i in 0..num {
        let Some(row) = lines.get(head + i + 2) else { break };
        let parts: Vec<&str> = row.split(',').map(|p| p.trim()).collect();
        let num_at = |idx: usize, radix: u32| -> Option<i64> {
            let raw = parts.get(idx)?.trim_matches('"');
            if raw.is_empty() {
                return None;
            }
            i64::from_str_radix(raw, radix).ok()
        };
        let item = match kind {
            LockKind::Lte => LockItem {
                band: num_at(0, 10),
                arfcn: num_at(1, 10),
                pci: num_at(2, 16),
                scs: None,
            },
            LockKind::Nr => LockItem {
                band: num_at(0, 10),
                arfcn: num_at(1, 10),
                scs: num_at(2, 10),
                pci: num_at(3, 16),
            },
        };
        items.push(item);
    }
    Some(LockState {
        lock_type,
        mobility,
        items,
    })
}

/// Radio function level from `+CFUN: <0|1>`; `None` when the reply has no
/// parseable value (`apply` then treats the radio as on).
pub fn parse_cfun(raw: &str) -> Option<u8> {
    raw.lines()
        .find_map(|l| l.trim().strip_prefix("+CFUN:"))
        .and_then(|rest| rest.trim().parse::<u8>().ok())
}

/// `^SYSCFGEX: "acqorder",band,roam,srvdomain,lteband,,` -> the settings.
///
/// The first field is quoted by some firmware and bare by others, and the two
/// trailing reserves are ignored; both spellings are accepted here so the page
/// never sees a raw reply.
pub fn parse_syscfgex(raw: &str) -> Option<SysCfgState> {
    let body = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("^SYSCFGEX:"))?
        .trim();
    let fields: Vec<&str> = body.split(',').map(|f| f.trim()).collect();
    if fields.len() < 5 {
        return None;
    }
    let acqorder = fields[0].trim_matches('"');
    if acqorder.is_empty() {
        return None;
    }
    Some(SysCfgState {
        acqorder: Some(acqorder.to_string()),
        band: Some(fields[1].to_string()),
        roam: fields[2].parse::<i64>().ok(),
        srvdomain: fields[3].parse::<i64>().ok(),
        lteband: Some(fields[4].to_string()),
    })
}


// ------------------------------------------------------------ dial page reads

/// Split an AT argument list on commas that are not inside a quoted string.
fn split_at_args(payload: &str) -> Vec<String> {
    let mut fields: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in payload.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                cur.push(c);
            }
            ',' if !quoted => {
                fields.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    fields.push(cur.trim().to_string());
    fields
        .into_iter()
        .map(|f| f.trim_matches('"').to_string())
        .collect()
}

/// `^SETAUTODIAL: <on>,<mode>,"<proto>","<apn>","<user>","<pass>",<auth>`.
///
/// The trailing fields are absent when autodial is off (the firmware then sends
/// only the switch), so every field after `enable` is optional — that is what
/// the page's own parser did, and what the NDIS fallback exists for.
pub fn parse_autodial(raw: &str) -> AutodialState {
    let mut st = AutodialState::default();
    // The cleaned text must outlive the borrow `line` takes from it.
    let cleaned = raw.replace('\r', "");
    let Some(line) = cleaned
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("^SETAUTODIAL:"))
    else {
        return st;
    };
    let body = line.trim_start_matches("^SETAUTODIAL:").trim();
    let fields = split_at_args(body);
    if fields.is_empty() || !fields[0].chars().all(|c| c.is_ascii_digit()) || fields[0].is_empty() {
        return st;
    }
    st.enable = fields[0].parse::<i64>().ok();
    let digits = |i: usize| -> Option<i64> {
        fields
            .get(i)
            .filter(|f| !f.is_empty() && f.chars().all(|c| c.is_ascii_digit()))
            .and_then(|f| f.parse::<i64>().ok())
    };
    st.dial_mode = digits(1);
    let text = |i: usize| fields.get(i).cloned().unwrap_or_default();
    st.protocol = text(2);
    st.apn = text(3);
    st.username = text(4);
    st.password = text(5);
    st.auth_type = digits(6);
    st
}

/// `^SETMODE?` -> the mode number. The device answers a bare number; a
/// `^SETMODE: <n>` line is accepted too so a firmware variant cannot blank the
/// card.
pub fn parse_usb_mode(raw: &str) -> Option<i64> {
    for line in raw.replace('\r', "").lines().map(str::trim) {
        if line.is_empty() || line == "OK" {
            continue;
        }
        let body = line.trim_start_matches("^SETMODE:").trim();
        if body.chars().all(|c| c.is_ascii_digit()) && !body.is_empty() {
            return body.parse::<i64>().ok();
        }
    }
    None
}

/// `^TDCFG?` -> NIC mode, post-route flag and the DMZ host.
///
/// The reply is a small text table (`Mode : 1`, `PostRoute : 0`, `Dmz: 1.2.3.4`)
/// and `Dmz: not cfg` means the feature is off — the page's rule.
pub fn parse_interface_cfg(raw: &str) -> InterfaceCfgState {
    let mut st = InterfaceCfgState::default();
    for line in raw.replace('\r', "").lines() {
        let line = line.trim();
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        match key.as_str() {
            "mode" => st.mode = value.parse::<i64>().ok(),
            "postroute" => st.post_route = value.parse::<i64>().ok(),
            "dmz" => {
                st.dmz_enabled = value != "not cfg" && !value.is_empty();
                if st.dmz_enabled {
                    st.dmz_host = value.to_string();
                }
            }
            _ => {}
        }
    }
    st
}

/// `+CGDCONT?` + `+CGACT?` -> the PDP context table.
///
/// The definition line carries cid/type/apn/address; activation comes from the
/// second query, joined by cid. The dial page renders cids 1..20 (0 is the
/// modem's own always-on context) — `commands::PDP_CID_RANGE`.
pub fn parse_pdp_contexts(cgdcont: &str, cgact: &str) -> Vec<PdpContext> {
    let mut contexts: Vec<PdpContext> = Vec::new();
    let mut actives: Vec<(u32, bool)> = Vec::new();
    for line in cgdcont.replace('\r', "").lines() {
        let line = line.trim();
        let Some(body) = line.strip_prefix("+CGDCONT:") else {
            continue;
        };
        let fields = split_at_args(body.trim());
        let Some(cid) = fields.first().and_then(|f| f.parse::<u32>().ok()) else {
            continue;
        };
        let get = |i: usize| fields.get(i).cloned().unwrap_or_default();
        contexts.push(PdpContext {
            cid,
            apn_type: get(1),
            apn: get(2),
            pdp_addr: get(3),
            active: false,
        });
    }
    for line in cgact.replace('\r', "").lines() {
        let line = line.trim();
        let Some(body) = line.strip_prefix("+CGACT:") else {
            continue;
        };
        let mut parts = body.trim().split(',');
        let cid = parts.next().and_then(|p| p.trim().parse::<u32>().ok());
        let active = parts.next().map(|p| p.trim() == "1").unwrap_or(false);
        if let Some(cid) = cid {
            actives.push((cid, active));
        }
    }
    for ctx in contexts.iter_mut() {
        if let Some((_, active)) = actives.iter().find(|(cid, _)| *cid == ctx.cid) {
            ctx.active = *active;
        }
    }
    contexts.retain(|c| crate::modules::network::commands::PDP_CID_RANGE.contains(&c.cid));
    contexts
}
/// Decode `+CIREG: <n>,<reg_info>` (3GPP 27.007 §7.7).
///
/// The wireless page used to slice this line out of the CLI's
/// `radio-diagnostics` frame; the offsets live here now. A reply without the
/// line leaves both fields unset, so the page shows no value rather than
/// "Not registered" for a modem that never answered.
pub fn parse_cireg(raw: &str) -> ImsState {
    let mut st = ImsState::default();
    for line in raw.lines() {
        let t = line.trim();
        let Some(idx) = t.find("+CIREG:") else { continue };
        let body = t[idx + "+CIREG:".len()..].trim();
        let fields: Vec<&str> = body.split(',').map(|f| f.trim()).collect();
        st.enabled = fields.first().and_then(|f| f.parse::<i64>().ok());
        st.registered = fields.get(1).and_then(|f| f.parse::<i64>().ok());
        break;
    }
    st
}

    #[test]
    fn autodial_reads_the_pages_field_rules() {
        // The firmware sends the full tuple while autodial is on; a quoted APN
        // with a comma must survive the split.
        let st = parse_autodial(
            "^SETAUTODIAL: 1,2,\"IPV4V6\",\"cmnet, backup\",\"user\",\"pw\",0\r\nOK",
        );
        assert_eq!(st.enable, Some(1));
        assert_eq!(st.dial_mode, Some(2));
        assert_eq!(st.protocol, "IPV4V6");
        assert_eq!(st.apn, "cmnet, backup");
        assert_eq!(st.username, "user");
        assert_eq!(st.password, "pw");
        assert_eq!(st.auth_type, Some(0));

        // Autodial off: only the switch comes back, which is what the NDIS
        // fallback exists for.
        let st = parse_autodial("^SETAUTODIAL: 0\r\nOK");
        assert_eq!(st.enable, Some(0));
        assert_eq!(st.dial_mode, None);
        assert!(!st.is_empty());
        assert!(parse_autodial("OK").is_empty());
        // No space after the colon (the demo/mock form) is accepted too.
        assert_eq!(parse_autodial("^SETAUTODIAL:1,2,\"IP\"").enable, Some(1));
    }

    #[test]
    fn ndis_fallback_matches_the_pages_regex() {
        use crate::modules::network::commands::ndis_is_active;
        assert!(ndis_is_active("^NDISSTATQRY: 1,0\r\nOK"));
        assert!(ndis_is_active("^NDISSTATQRY:1,1"));
        assert!(!ndis_is_active("^NDISSTATQRY: 0,0\r\nOK"));
        assert!(!ndis_is_active("OK"));
        // A mode-less autodial reply + an active NDIS session == "the modem
        // dials itself" (dialMode 1), the value the page displayed.
        let mut st = parse_autodial("^SETAUTODIAL: 0");
        st.ndis_active = crate::modules::network::commands::ndis_is_active("^NDISSTATQRY: 1,0");
        assert!(st.to_json().dump().contains("\"dialMode\":1"));
    }

    #[test]
    fn usb_mode_accepts_the_bare_and_prefixed_forms() {
        assert_eq!(parse_usb_mode("2\r\nOK"), Some(2));
        assert_eq!(parse_usb_mode("^SETMODE: 1\r\nOK"), Some(1));
        assert_eq!(parse_usb_mode("OK"), None);
        assert_eq!(parse_usb_mode("ERROR"), None);
    }

    #[test]
    fn interface_cfg_keeps_the_dmz_semantics() {
        let st = parse_interface_cfg("Mode : 1\r\nPostRoute : 0\r\nDmz: 192.168.8.100\r\nOK");
        assert_eq!(st.mode, Some(1));
        assert_eq!(st.post_route, Some(0));
        assert!(st.dmz_enabled);
        assert_eq!(st.dmz_host, "192.168.8.100");

        let st = parse_interface_cfg("Mode: 2\nPostRoute: 1\nDmz: not cfg\nOK");
        assert_eq!(st.mode, Some(2));
        assert_eq!(st.post_route, Some(1));
        assert!(!st.dmz_enabled);
        assert_eq!(st.dmz_host, "");
    }

    #[test]
    fn pdp_contexts_join_definitions_with_activation() {
        let defs = "+CGDCONT: 1,\"IP\",\"cmnet\",\"10.0.0.1\",0,0\r\n\
                    +CGDCONT: 2,\"IPV6\",\"cmnet6\",\"\",0,0\r\n\
                    +CGDCONT: 0,\"IP\",\"\",\"\",0,0\r\nOK";
        let act = "+CGACT: 1,1\r\n+CGACT: 2,0\r\n+CGACT: 0,1\r\nOK";
        let list = parse_pdp_contexts(defs, act);
        // cid 0 is the modem's own context and is not shown.
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].cid, 1);
        assert_eq!(list[0].apn_type, "IP");
        assert_eq!(list[0].apn, "cmnet");
        assert_eq!(list[0].pdp_addr, "10.0.0.1");
        assert!(list[0].active);
        assert_eq!(list[1].cid, 2);
        assert!(!list[1].active);
        // Activation may be missing entirely: nobody is active then.
        assert!(!parse_pdp_contexts(defs, "OK")[0].active);
    }
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cireg_reports_ims_registration() {
        let st = parse_cireg("+CIREG: 0,1\nOK");
        assert_eq!(st.enabled, Some(0));
        assert_eq!(st.registered, Some(1));
        assert!(!st.is_empty());
        // A modem that rejects the query leaves both fields unset, so the page
        // shows nothing instead of "Not registered".
        let none = parse_cireg("+CME ERROR: 4\nOK");
        assert!(none.is_empty());
        assert_eq!(none.registered, None);
        let partial = parse_cireg("+CIREG: 1");
        assert_eq!(partial.enabled, Some(1));
        assert_eq!(partial.registered, None);
    }

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
    fn c5goption_triple_and_partial_replies() {
        let st = parse_c5goption("^C5GOPTION: 1,1,1\nOK");
        assert_eq!(st.nr_sa_support_flag, Some(1));
        assert_eq!(st.nr_dc_mode, Some(1));
        assert_eq!(st.gc_access_mode, Some(1));
        // Two fields: nothing is decoded (the page keeps its previous value).
        assert!(parse_c5goption("^C5GOPTION: 1,1").is_empty());
        assert!(parse_c5goption("OK").is_empty());
    }

    #[test]
    fn rrcstat_reads_both_reply_forms() {
        // Three fields: <x>,<state>,<camped>.
        let st = parse_rrcstat("^RRCSTAT: 1,1,98\r\nOK");
        assert_eq!(st.state, Some(1));
        assert_eq!(st.camped, Some(98));
        // Two fields: <x>,<camped> — the form the page's fixed indices read as
        // state 98. The state is the field before the flag.
        let st = parse_rrcstat("^RRCSTAT: 1,98");
        assert_eq!(st.state, Some(1));
        assert_eq!(st.camped, Some(99 - 1));
        // Without a flag the last field is the state (2 = Inactive).
        let st = parse_rrcstat("^RRCSTAT: 1,2");
        assert_eq!(st.state, Some(2));
        assert_eq!(st.camped, None);
        // Idle, not camped.
        let st = parse_rrcstat("^RRCSTAT: 0,0,99");
        assert_eq!(st.state, Some(0));
        assert_eq!(st.camped, Some(99));
        // Nothing to decode keeps the caller's value.
        assert!(parse_rrcstat("OK").is_empty());
        assert!(parse_rrcstat("^RRCSTAT:").is_empty());
    }

    #[test]
    fn freq_lock_lte_rows_and_hex_pci() {
        let raw = "^LTEFREQLOCK: 2\n0,1\n3,1850,64\nOK";
        let st = parse_freq_lock(raw, LockKind::Lte).expect("lock");
        assert_eq!(st.lock_type, 2);
        assert_eq!(st.mobility, 0);
        assert_eq!(st.items.len(), 1);
        assert_eq!(st.items[0].band, Some(3));
        assert_eq!(st.items[0].arfcn, Some(1850));
        assert_eq!(st.items[0].pci, Some(100)); // 0x64
        assert_eq!(st.items[0].scs, None);
    }

    #[test]
    fn freq_lock_nr_rows_carry_scs_before_pci() {
        let raw = "^NRFREQLOCK: 2\n0,2\n78,643456,1,10\n41,504990,0,1A\nOK";
        let st = parse_freq_lock(raw, LockKind::Nr).expect("lock");
        assert_eq!(st.items.len(), 2);
        assert_eq!(st.items[0].scs, Some(1));
        assert_eq!(st.items[0].pci, Some(0x10));
        assert_eq!(st.items[1].scs, Some(0));
        assert_eq!(st.items[1].pci, Some(26));
    }

    #[test]
    fn freq_lock_unlock_and_missing_line() {
        let unlocked = parse_freq_lock("^LTEFREQLOCK: 0\nOK", LockKind::Lte).unwrap();
        assert_eq!(unlocked.lock_type, 0);
        assert!(unlocked.items.is_empty());
        // A reply without the lock line keeps the caller's previous value.
        assert!(parse_freq_lock("OK", LockKind::Lte).is_none());
        // The other RAT's line must not match.
        assert!(parse_freq_lock("^NRFREQLOCK: 2\n0,1\n78,1,1,2", LockKind::Lte).is_none());
    }

    #[test]
    fn cfun_level_is_parsed() {
        assert_eq!(parse_cfun("+CFUN: 1\nOK"), Some(1));
        assert_eq!(parse_cfun("+CFUN: 0"), Some(0));
        assert_eq!(parse_cfun("OK"), None);
    }

    #[test]
    fn ipv6cap_reads_single_code() {
        assert_eq!(parse_ipv6cap("^IPV6CAP: 7\nOK"), Some(7));
        assert_eq!(parse_ipv6cap("^IPV6CAP: 11"), Some(11));
        assert_eq!(parse_ipv6cap("OK"), None);
    }

    #[test]
    fn syscfgex_parses_quoted_and_bare_orders() {
        let st = parse_syscfgex("^SYSCFGEX: \"08030201\",3FFFFFFF,1,2,7FFFFFFFFFFFFFFF,,\nOK").unwrap();
        assert_eq!(st.acqorder.as_deref(), Some("08030201"));
        assert_eq!(st.band.as_deref(), Some("3FFFFFFF"));
        assert_eq!(st.roam, Some(1));
        assert_eq!(st.srvdomain, Some(2));
        assert_eq!(st.lteband.as_deref(), Some("7FFFFFFFFFFFFFFF"));
        let bare = parse_syscfgex("^SYSCFGEX: 02,1,0,1,80,,").unwrap();
        assert_eq!(bare.acqorder.as_deref(), Some("02"));
        assert_eq!(bare.roam, Some(0));
        assert!(parse_syscfgex("OK").is_none());
        assert!(parse_syscfgex("^SYSCFGEX: \"\",1,0,1,80,,").is_none());
    }
}
