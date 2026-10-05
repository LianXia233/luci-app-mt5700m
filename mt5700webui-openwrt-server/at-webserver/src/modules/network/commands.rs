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

#[cfg(test)]
mod tests {
    use super::*;

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
