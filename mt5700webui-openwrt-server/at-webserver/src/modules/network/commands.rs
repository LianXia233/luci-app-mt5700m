//! AT commands owned by the network module.
//!
//! Read commands are tried in order: the first one that answers with a
//! `REG:` line wins. `C5GREG` carries the 5G fields (tac/ci/AcT/NSSAI), so it
//! is preferred; `CEREG` and `CREG` are the fallbacks for 4G/2G-only firmware.
//!
//! The frequency-lock builders live here too: the manual's grouped-CSV syntax
//! is AT knowledge, and the CLI, the day/night scheduler and the API route all
//! build the same write through these functions.

use crate::modules::network::state::{LockItem, LockKind};

/// Registration queries, most specific first.
pub const REG_QUERIES: [&str; 3] = ["AT+C5GREG?", "AT+CEREG?", "AT+CREG?"];

/// Operator name + access technology (`+COPS: <mode>,<format>,"<name>",<act>`).
pub const COPS: &str = "AT+COPS?";

/// IMS registration status (`+CIREG: <n>,<reg_info>`, 3GPP 27.007 §7.7).
///
/// On-demand only: the wireless page's diagnostics block shows it next to the
/// PS registration row.
pub const CIREG: &str = "AT+CIREG?";

/// Activated PDP context addresses (`+CGPADDR: <cid>,"<address>"`). On-demand
/// only: the diagnostics panel asks for it, nothing polls it.
pub const CGPADDR: &str = "AT+CGPADDR";

/// Detailed system mode (vendor command).
pub const SYSINFOEX: &str = "AT^SYSINFOEX";

/// IPv4 parameters of the data call as the firmware's DHCP client sees them.
/// Hex-encoded little-endian bytes, six fields (address, mask, gateway, DHCP
/// server, primary/secondary DNS).
pub const DHCP_V4: &str = "AT^DHCP?";

/// Same six fields for IPv6, already in colon form.
pub const DHCP_V6: &str = "AT^DHCPV6?";

/// IPv6 capability code (`^IPV6CAP`: 1 = IPv4 only, 2 = IPv6 only,
/// 7 = dual stack sharing one APN, 11 = dual stack with separate APNs).
pub const IPV6CAP: &str = "AT^IPV6CAP?";

/// Ask the modem for detailed PS registration reports (`AT+CGREG=2`).
///
/// The WebUI has always issued this once per page load so the registration
/// URCs carry the location/cell fields. It is modem configuration, not a
/// frontend concern, so the network module owns the verb and the frontend
/// calls this route.
pub const CGREG_DETAILED: &str = "AT+CGREG=2";

/// True when a response actually contains a registration line.
///
/// A failed response can still carry text without any `REG:` line; treating
/// that as an answer would record `state=0` and stop the fallback chain.
pub fn has_registration_line(text: &str) -> bool {
    text.lines().any(|l| l.trim().contains("REG:"))
}

/// Current LTE frequency lock (`^LTEFREQLOCK: <type>` …).
pub const LTEFREQLOCK_QUERY: &str = "AT^LTEFREQLOCK?";

/// Current NR frequency lock.
pub const NRFREQLOCK_QUERY: &str = "AT^NRFREQLOCK?";

/// Connection/RRC state (`^RRCSTAT?`), the "Radio status" card's first row.
pub const RRCSTAT_QUERY: &str = "AT^RRCSTAT?";

/// Current radio function level (`+CFUN: <0|1>`), read before a lock change.
pub const CFUN_QUERY: &str = "AT+CFUN?";

/// 5G access mode (`^C5GOPTION: <sa>,<dc>,<gc>`): SA support, EN-DC mode and
/// the 5G core access mode.
pub const C5GOPTION_QUERY: &str = "AT^C5GOPTION?";

/// Write the 5G access mode. The firmware only applies it after a radio
/// function-level cycle, so callers run it through the shared write helper.
pub fn c5goption_write(sa: u8, dc: u8, gc: u8) -> String {
    format!("AT^C5GOPTION={},{},{}", sa, dc, gc)
}

/// The lock query for one RAT.
pub fn lock_query(kind: LockKind) -> &'static str {
    match kind {
        LockKind::Lte => LTEFREQLOCK_QUERY,
        LockKind::Nr => NRFREQLOCK_QUERY,
    }
}

/// The reply prefix the query answers with (`^LTEFREQLOCK:` / `^NRFREQLOCK:`).
pub fn lock_prefix(kind: LockKind) -> &'static str {
    match kind {
        LockKind::Lte => "^LTEFREQLOCK:",
        LockKind::Nr => "^NRFREQLOCK:",
    }
}

// Longest band-lock form from the manual: 20 bands × up to 5 digits.
const MAX_LOCK_GROUPS: usize = 20;

fn clean_csv(value: &str) -> String {
    let no_space: String = value.chars().filter(|c| *c != ' ').collect();
    let trimmed = no_space.trim_matches(',');
    let parts: Vec<&str> = trimmed.split(',').filter(|p| !p.is_empty()).collect();
    parts.join(",")
}

fn csv_count(value: &str) -> usize {
    let v: String = value.chars().filter(|c| *c != ' ').collect();
    if v.is_empty() {
        0
    } else {
        v.split(',').count()
    }
}

fn is_numeric_csv(v: &str) -> bool {
    !v.is_empty() && v.split(',').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

fn numeric_csv_in_range(v: &str, min: u64, max: u64) -> bool {
    is_numeric_csv(v)
        && v.split(',')
            .all(|p| p.parse::<u64>().map(|n| n >= min && n <= max).unwrap_or(false))
}

fn valid_lock_count(count: usize) -> bool {
    (1..=MAX_LOCK_GROUPS).contains(&count)
}

/// LTE lock write (manual 13.12.3): type 0 unlock, 3 band-only, 1
/// band+ARFCN, 2 band+ARFCN+PCI. `None` means the caller passed something the
/// manual forbids (exit 64 in the CLI, a parameter error on the API).
fn lte_lock_write(
    lock_type: &str,
    mobility: u8,
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
            Some(format!(
                "AT^LTEFREQLOCK=3,{},{},\"{}\"",
                mobility, count, bands
            ))
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
                "AT^LTEFREQLOCK=1,{},{},\"{}\",\"{}\"",
                mobility, count, bands, arfcns
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
                "AT^LTEFREQLOCK=2,{},{},\"{}\",\"{}\",\"{}\"",
                mobility, count, bands, arfcns, pcis
            ))
        }
        _ => None,
    }
}

/// NR lock write (manual 13.13.3): NR carries one extra `scstype` group
/// between the ARFCNs and the PCIs.
fn nr_lock_write(
    lock_type: &str,
    mobility: u8,
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
            Some(format!("AT^NRFREQLOCK=3,{},{},\"{}\"", mobility, count, bands))
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
                "AT^NRFREQLOCK=1,{},{},\"{}\",\"{}\",\"{}\"",
                mobility, count, bands, arfcns, scs
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
                "AT^NRFREQLOCK=2,{},{},\"{}\",\"{}\",\"{}\",\"{}\"",
                mobility, count, bands, arfcns, scs, pcis
            ))
        }
        _ => None,
    }
}

/// The CLI's grouped-CSV form (mobility 0, as the shell interface has always
/// sent it).
pub fn lte_lock_command(lock_type: &str, bands: &str, arfcns: &str, pcis: &str) -> Option<String> {
    lte_lock_write(lock_type, 0, bands, arfcns, pcis)
}

/// The CLI's NR form (mobility 0).
pub fn nr_lock_command(
    lock_type: &str,
    bands: &str,
    arfcns: &str,
    scs: &str,
    pcis: &str,
) -> Option<String> {
    nr_lock_write(lock_type, 0, bands, arfcns, scs, pcis)
}

/// SCS code used when a lock item does not carry one: 30 kHz for the FR1 TDD
/// bands the MT5700M locks (n41/n77/n78/n79), 15 kHz otherwise.
///
/// This is the same rule the Settings page applied in its lock form, so a
/// request that omits `scs` writes exactly what the page used to write. The
/// day/night scheduler keeps its own port of the shell's `auto_detect_scs`
/// (FR2 -> 120 kHz) — two different inputs, two documented rules.
fn default_scs(band: i64) -> i64 {
    if matches!(band, 41 | 77 | 78 | 79) {
        1
    } else {
        0
    }
}

/// Build the lock write for one RAT from structured items (the API path).
///
/// Items without a band are ignored, exactly as the page filtered them. A
/// missing ARFCN/PCI becomes `-1`, which fails the numeric range check and
/// turns into a parameter error rather than a malformed AT write.
pub fn lock_command_for(
    kind: LockKind,
    lock_type: u8,
    mobility: u8,
    items: &[LockItem],
) -> Option<String> {
    let used: Vec<&LockItem> = items.iter().filter(|i| i.band.is_some()).collect();
    let list = |f: fn(&LockItem) -> Option<i64>| {
        used.iter()
            .map(|i| f(i).unwrap_or(-1).to_string())
            .collect::<Vec<_>>()
            .join(",")
    };
    let bands = list(|i| i.band);
    let arfcns = list(|i| i.arfcn);
    let pcis = list(|i| i.pci);
    let lock_type = lock_type.to_string();
    match kind {
        LockKind::Lte => lte_lock_write(&lock_type, mobility, &bands, &arfcns, &pcis),
        LockKind::Nr => {
            let scs = used
                .iter()
                .map(|i| {
                    i.scs
                        .unwrap_or_else(|| default_scs(i.band.unwrap_or(0)))
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join(",");
            nr_lock_write(&lock_type, mobility, &bands, &arfcns, &scs, &pcis)
        }
    }
}

/// AT command that selects `+CFUN` (radio on/off) when a caller needs to
/// re-register the modem.
pub fn cfun(state: u8) -> String {
    format!("AT+CFUN={}", state)
}

/// AT command that brings the data call up or down.
pub fn ndisdup(up: bool) -> String {
    format!("AT^NDISDUP=1,{}", if up { 1 } else { 0 })
}

/// USB port / network preference mode query (`^SETMODE: <n>`).
pub const SETMODE_QUERY: &str = "AT^SETMODE?";

/// AT command that sets the network preference mode.
pub fn set_mode(value: u8) -> String {
    format!("AT^SETMODE={}", value)
}

// ------------------------------------------------- access-technology config
//
// `AT^SYSCFGEX="<acqorder>",<band>,<roam>,<srvdomain>,<lteband>,,` — the seven
// arguments the manual requires (the last two are reserved and stay empty).
// `acqorder` is a concatenation of two-digit RAT codes, most preferred first.

/// Access-technology order (`^SYSCFGEX` first field). 3G / 4G / 5G and the
/// combinations the manual and the UI offer.
pub const ACQ_ORDERS: [&str; 6] = ["02", "03", "08", "0302", "0803", "080302"];

/// Query the current system configuration.
pub const SYSCFGEX_QUERY: &str = "AT^SYSCFGEX?";

/// Full write. `band`/`lteband` are the modem's hex bitmasks, passed through.
pub fn syscfgex(acqorder: &str, band: &str, roam: u8, srvdomain: u8, lteband: &str) -> String {
    format!(
        "AT^SYSCFGEX=\"{}\",{},{},{},{},,",
        acqorder, band, roam, srvdomain, lteband
    )
}

/// True when `acqorder` is one of the accepted values.
pub fn valid_acq_order(acqorder: &str) -> bool {
    ACQ_ORDERS.contains(&acqorder)
}

/// True when the string is a non-empty hex bitmask.
pub fn is_hex_mask(v: &str) -> bool {
    !v.is_empty() && v.bytes().all(|b| b.is_ascii_hexdigit())
}


// ------------------------------------------------------------ dial page reads
//
// The WebUI's dial page is read-only (LuCI owns the writes) and used to build
// these six queries and parse their replies itself. The module owns them now.

/// Built-in autodial configuration (`^SETAUTODIAL: <on>,<mode>,"<proto>",
/// "<apn>","<user>","<pass>",<auth>` — the trailing fields are absent when
/// autodial is off).
pub const SETAUTODIAL_QUERY: &str = "AT^SETAUTODIAL?";
/// USB data-session state (`^NDISSTATQRY: <up>,…`), used to tell "modem dials
/// itself" from "the host dials" when the autodial reply omits the mode.
///
/// The same command is the authority for "did this cid's data plane come up"
/// (`network.session`): the reply carries `<up>,…,<IPv4 state>,…,<IPv6 state>`
/// and the module-side answer outranks "there is an address", because the
/// MT5700M answers this query with an empty reply on some networks.
pub const NDISSTATQRY: &str = "AT^NDISSTATQRY?";
/// Data-flow counters (`^DSFLOWQRY: <cur_dur>,<cur_tx>,<cur_rx>,<total_dur>,
/// <total_tx>,<total_rx>`), six hex-encoded fields.
pub const DSFLOWQRY: &str = "AT^DSFLOWQRY";
/// Reset those counters (`network.flow_clear`, the connection page's "Clear
/// counters" button).
pub const DSFLOWCLR: &str = "AT^DSFLOWCLR";
/// Data-call MTU (`^CGMTU: <cid>,<mtu>`), asked for cid 1 exactly like the CLI
/// verb did (`AT^CGMTU=1` is a query on this firmware).
pub const CGMTU_QUERY: &str = "AT^CGMTU=1";
/// Detailed session list (`^DCONNSTAT: <cid>,"<apn>",<ipv4>,<ipv6>,<type>[,<ethernet>]`).
pub const DCONNSTAT: &str = "AT^DCONNSTAT?";
/// Interface configuration (`Mode:` / `PostRoute:` / `Dmz:` lines).
pub const TDCFG_QUERY: &str = "AT^TDCFG?";
/// PDP context definitions (`+CGDCONT: <cid>,"<type>","<apn>",…`).
pub const CGDCONT_QUERY: &str = "AT+CGDCONT?";
/// PDP context activation states (`+CGACT: <cid>,<active>`).
pub const CGACT_QUERY: &str = "AT+CGACT?";
/// The PDP context id range the dial page renders (0 is the modem's own
/// always-on context, 21+ is firmware-internal).
pub const PDP_CID_RANGE: std::ops::Range<u32> = 1..21;

/// Direct-IP passthrough (`^SETDIRECTIP?` / `^SETDIRECTIP=<0|1>`).
pub const SETDIRECTIP_QUERY: &str = "AT^SETDIRECTIP?";
/// Inbound filter toggle forced off when PostRoute is enabled
/// (`postroute` CLI verb appends it to the TDCFG write).
pub const IPFILTERSWITCH_OFF: &str = "AT^IPFILTERSWITCH=0";

/// AT-field guard shared with the CLI: no quotes, commas or line breaks may
/// reach a command argument (the historical `safe_at_field` in cli.rs — moved
/// here so the write routes validate with the same rule).
pub fn safe_at_field(v: &str) -> bool {
    !v.contains('"') && !v.contains(',') && !v.contains('\r') && !v.contains('\n')
}

/// The dial page renders/edits cids 1..=11 only; the CLI verb whitelist said
/// the same. (Reading accepts the wider `PDP_CID_RANGE`.)
pub fn valid_cid(cid: u32) -> bool {
    (1..=11).contains(&cid)
}

pub fn valid_pdp_type(t: &str) -> bool {
    matches!(t, "IP" | "IPV6" | "IPV4V6")
}

/// DMZ host: `0` disables, otherwise a strict dotted-quad IPv4 (all four
/// octets non-empty, numeric, <= 255) — the CLI verb's rule, word for word.
pub fn valid_dmz_host(v: &str) -> bool {
    if v == "0" {
        return true;
    }
    let parts: Vec<&str> = v.split('.').collect();
    parts.len() == 4
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_digit())
                && p.parse::<u32>().map(|n| n <= 255).unwrap_or(false)
        })
}

/// `AT+CGDCONT=<cid>,"<type>","<apn>"` — define/overwrite a PDP profile.
/// `None` when the tuple would not pass the CLI's validation.
pub fn cgdcont_set(cid: u32, pdp_type: &str, apn: &str) -> Option<String> {
    if !valid_cid(cid) || !valid_pdp_type(pdp_type) || !safe_at_field(apn) || apn.len() > 99 {
        return None;
    }
    Some(format!("AT+CGDCONT={cid},\"{pdp_type}\",\"{apn}\""))
}

/// `AT+CGDCONT=<cid>` — deleting the profile is the same command minus the
/// trailing fields (the modem restores the carrier default).
pub fn cgdcont_remove(cid: u32) -> Option<String> {
    if !valid_cid(cid) {
        return None;
    }
    Some(format!("AT+CGDCONT={cid}"))
}

/// `AT+CGACT=<state>,<cid>` — activate (`true`) / deactivate (`false`).
pub fn cgact(active: bool, cid: u32) -> Option<String> {
    if !valid_cid(cid) {
        return None;
    }
    Some(format!("AT+CGACT={},{}", if active { 1 } else { 0 }, cid))
}

/// `AT^SETAUTODIAL=…` with the CLI's trailing-empty-field rule: MT5700M
/// rejects trailing empty fields, so every omitted optional field shrinks the
/// command — otherwise `dial_mode` would not actually be applied.
///
/// Validation mirrors the `autodial` CLI verb: `enable=false` short-circuits
/// to `AT^SETAUTODIAL=0` (the other arguments stay unchecked, exactly like
/// the shell did); `None` means the tuple is invalid.
pub fn setautodial(
    enable: bool,
    dial_mode: i64,
    protocol: &str,
    apn: &str,
    username: &str,
    password: &str,
    auth: i64,
) -> Option<String> {
    if !enable {
        return Some("AT^SETAUTODIAL=0".to_string());
    }
    if !(0..=2).contains(&dial_mode) || !valid_pdp_type(protocol) || !(0..=2).contains(&auth) {
        return None;
    }
    if !safe_at_field(apn) || !safe_at_field(username) || !safe_at_field(password) {
        return None;
    }
    if apn.len() > 99 || username.len() > 31 || password.len() > 31 {
        return None;
    }
    // MT5700M rejects trailing empty fields; omit every optional
    // field when empty so dial_mode is actually applied.
    Some(if apn.is_empty() {
        format!("AT^SETAUTODIAL=1,{dial_mode},\"{protocol}\"")
    } else if username.is_empty() && password.is_empty() {
        format!("AT^SETAUTODIAL=1,{dial_mode},\"{protocol}\",\"{apn}\"")
    } else {
        format!(
            "AT^SETAUTODIAL=1,{dial_mode},\"{protocol}\",\"{apn}\",\"{username}\",\"{password}\",{auth}"
        )
    })
}

/// `AT^SETDIRECTIP=<0|1>` — IP passthrough.
pub fn setdirectip(enabled: bool) -> String {
    format!("AT^SETDIRECTIP={}", if enabled { 1 } else { 0 })
}

/// `AT^TDCFG="infcfg","PostRoute",<mode>` — mode 2 = off, 1 = on.
pub fn tdcfg_postroute(mode: i64) -> Option<String> {
    match mode {
        1 | 2 => Some(format!("AT^TDCFG=\"infcfg\",\"PostRoute\",{mode}")),
        _ => None,
    }
}

/// `AT^TDCFG="infcfg","mode",<mode>` — interface operating mode (1|2).
pub fn tdcfg_mode(mode: i64) -> Option<String> {
    match mode {
        1 | 2 => Some(format!("AT^TDCFG=\"infcfg\",\"mode\",{mode}")),
        _ => None,
    }
}

/// `AT^TDCFG="infcfg","dmz","<host>"` — host `0` disables, same command.
pub fn tdcfg_dmz(host: &str) -> Option<String> {
    if !valid_dmz_host(host) {
        return None;
    }
    Some(format!("AT^TDCFG=\"infcfg\",\"dmz\",\"{host}\""))
}

/// True when the modem's own NDIS data session is up (`^NDISSTATQRY: 1,…`) —
/// the page's fallback for telling "the modem dials itself" from "the host
/// dials", used when the autodial reply omits the mode.
pub fn ndis_is_active(text: &str) -> bool {
    text.replace('\r', "").lines().any(|l| {
        let l = l.trim();
        match l.strip_prefix("^NDISSTATQRY:") {
            Some(body) => body
                .trim_start()
                .split(',')
                .next()
                .map(str::trim)
                .map(|first| first == "1")
                .unwrap_or(false),
            None => false,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setautodial_keeps_the_cli_trailing_field_rule() {
        // disabled short-circuits, nothing else is validated
        assert_eq!(setautodial(false, 9, "XX", "a,b", "x", "y", 9).as_deref(), Some("AT^SETAUTODIAL=0"));
        // bare mode+protocol
        assert_eq!(setautodial(true, 1, "IPV4V6", "", "", "", 0).as_deref(),
            Some("AT^SETAUTODIAL=1,1,\"IPV4V6\""));
        // apn only
        assert_eq!(setautodial(true, 2, "IP", "cmnet", "", "", 0).as_deref(),
            Some("AT^SETAUTODIAL=1,2,\"IP\",\"cmnet\""));
        // full form
        assert_eq!(setautodial(true, 0, "IPV4V6", "apn", "user", "pass", 2).as_deref(),
            Some("AT^SETAUTODIAL=1,0,\"IPV4V6\",\"apn\",\"user\",\"pass\",2"));
        // validation: dial_mode / protocol / auth / field rules
        assert!(setautodial(true, 3, "IP", "", "", "", 0).is_none());
        assert!(setautodial(true, 1, "IPv4", "", "", "", 0).is_none());
        assert!(setautodial(true, 1, "IP", "", "", "", 3).is_none());
        assert!(setautodial(true, 1, "IP", "a,b", "", "", 0).is_none());
        assert!(setautodial(true, 1, "IP", "a\"b", "", "", 0).is_none());
        let long_apn = "a".repeat(100);
        assert!(setautodial(true, 1, "IP", &long_apn, "", "", 0).is_none());
        let long_user = "u".repeat(32);
        assert!(setautodial(true, 1, "IP", "apn", &long_user, "", 0).is_none());
    }

    #[test]
    fn pdp_writers_follow_the_cli_whitelist() {
        assert_eq!(cgdcont_set(3, "IPV4V6", "cmnet").as_deref(),
            Some("AT+CGDCONT=3,\"IPV4V6\",\"cmnet\""));
        assert_eq!(cgdcont_set(3, "IPV4V6", "").as_deref(),
            Some("AT+CGDCONT=3,\"IPV4V6\",\"\""));
        assert!(cgdcont_set(0, "IP", "apn").is_none());
        assert!(cgdcont_set(12, "IP", "apn").is_none());
        assert!(cgdcont_set(3, "IPv6", "apn").is_none());
        assert!(cgdcont_set(3, "IP", "a,b").is_none());
        assert_eq!(cgdcont_remove(11).as_deref(), Some("AT+CGDCONT=11"));
        assert!(cgdcont_remove(12).is_none());
        assert_eq!(cgact(true, 4).as_deref(), Some("AT+CGACT=1,4"));
        assert_eq!(cgact(false, 4).as_deref(), Some("AT+CGACT=0,4"));
        assert!(cgact(true, 0).is_none());
    }

    #[test]
    fn tdcfg_and_directip_writers_match_the_cli() {
        assert_eq!(setdirectip(true), "AT^SETDIRECTIP=1");
        assert_eq!(setdirectip(false), "AT^SETDIRECTIP=0");
        assert_eq!(tdcfg_postroute(2).as_deref(),
            Some("AT^TDCFG=\"infcfg\",\"PostRoute\",2"));
        assert!(tdcfg_postroute(0).is_none());
        assert!(tdcfg_postroute(3).is_none());
        assert_eq!(tdcfg_dmz("0").as_deref(), Some("AT^TDCFG=\"infcfg\",\"dmz\",\"0\""));
        assert_eq!(tdcfg_dmz("192.168.8.100").as_deref(),
            Some("AT^TDCFG=\"infcfg\",\"dmz\",\"192.168.8.100\""));
        // the dotted-quad rule rejects everything else the CLI refused
        assert!(tdcfg_dmz("1.2.3").is_none());
        assert!(tdcfg_dmz("1.2.3.256").is_none());
        assert!(tdcfg_dmz("1..3.4").is_none());
        assert!(tdcfg_dmz("a.b.c.d").is_none());
    }

    fn item(band: Option<i64>, arfcn: Option<i64>, pci: Option<i64>, scs: Option<i64>) -> LockItem {
        LockItem {
            band,
            arfcn,
            pci,
            scs,
        }
    }

    /// The route and the CLI must produce the same AT string.
    ///
    /// `network.lock_apply` takes typed items (`[{band, arfcn, pci, scs}]`) while
    /// the CLI takes the page's four CSV fields positionally; both funnel through
    /// [`lock_command_for`] / [`lte_lock_command`] / [`nr_lock_command`], and this
    /// test pins that they stay equal for exactly the inputs LuCI's lock panel
    /// sends (the shapes below are the panel's: band lock, ARFCN lock, cell lock,
    /// an NR item without an SCS code, and an unlock with no items).
    #[test]
    fn lock_items_build_the_cli_write() {
        // Band lock: only the bands are used by either form.
        let items = [item(Some(3), None, None, None), item(Some(7), None, None, None)];
        assert_eq!(
            lock_command_for(LockKind::Lte, 3, 0, &items).unwrap(),
            lte_lock_command("3", "3,7", "", "").unwrap()
        );
        assert_eq!(
            lock_command_for(LockKind::Nr, 3, 0, &items).unwrap(),
            nr_lock_command("3", "3,7", "", "", "").unwrap()
        );

        // ARFCN lock.
        let items = [item(Some(3), Some(1850), None, None)];
        assert_eq!(
            lock_command_for(LockKind::Lte, 1, 0, &items).unwrap(),
            lte_lock_command("1", "3", "1850", "").unwrap()
        );
        let items = [item(Some(78), Some(636648), None, Some(1))];
        assert_eq!(
            lock_command_for(LockKind::Nr, 1, 0, &items).unwrap(),
            nr_lock_command("1", "78", "636648", "1", "").unwrap()
        );

        // Cell lock: PCI (decimal, as the page holds it) in both forms.
        let items = [item(Some(3), Some(1850), Some(64), None)];
        assert_eq!(
            lock_command_for(LockKind::Lte, 2, 0, &items).unwrap(),
            lte_lock_command("2", "3", "1850", "64").unwrap()
        );
        let items = [item(Some(78), Some(636648), Some(10), Some(0))];
        assert_eq!(
            lock_command_for(LockKind::Nr, 2, 0, &items).unwrap(),
            nr_lock_command("2", "78", "636648", "0", "10").unwrap()
        );

        // An NR item without an SCS code takes the module's band default (the
        // CLI form cannot express that: an empty SCS is rejected there).
        let items = [item(Some(78), Some(636648), None, None)];
        assert_eq!(
            lock_command_for(LockKind::Nr, 1, 0, &items).unwrap(),
            nr_lock_command("1", "78", "636648", "1", "").unwrap()
        );

        // Unlock carries no items at all, and mobility is the caller's.
        assert_eq!(
            lock_command_for(LockKind::Lte, 0, 0, &[]).unwrap(),
            "AT^LTEFREQLOCK=0"
        );
        assert_eq!(
            lock_command_for(LockKind::Nr, 0, 1, &[]).unwrap(),
            "AT^NRFREQLOCK=0"
        );

        // Out-of-range input is refused by both paths.
        let items = [item(Some(3), Some(1850), Some(900), None)];
        assert!(lock_command_for(LockKind::Lte, 2, 0, &items).is_none());
        assert!(lte_lock_command("2", "3", "1850", "900").is_none());
    }

    #[test]
    fn tdcfg_mode_shape() {
        assert_eq!(tdcfg_mode(1).as_deref(), Some("AT^TDCFG=\"infcfg\",\"mode\",1"));
        assert_eq!(tdcfg_mode(2).as_deref(), Some("AT^TDCFG=\"infcfg\",\"mode\",2"));
        assert_eq!(tdcfg_mode(0), None);
        assert_eq!(tdcfg_mode(3), None);
    }

    #[test]
    fn syscfgex_write_and_validators() {
        assert_eq!(
            syscfgex("08030201", "3FFFFFFF", 1, 2, "7FFFFFFFFFFFFFFF"),
            "AT^SYSCFGEX=\"08030201\",3FFFFFFF,1,2,7FFFFFFFFFFFFFFF,,"
        );
        assert_eq!(syscfgex("02", "1", 0, 1, "80"), "AT^SYSCFGEX=\"02\",1,0,1,80,,");
        assert!(valid_acq_order("02") && valid_acq_order("080302"));
        assert!(!valid_acq_order("01") && !valid_acq_order(""));
        assert!(is_hex_mask("3FFFFFFF") && is_hex_mask("80"));
        assert!(!is_hex_mask("") && !is_hex_mask("3FG"));
    }
}
