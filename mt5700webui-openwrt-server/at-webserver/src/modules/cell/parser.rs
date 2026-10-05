//! `^HFREQINFO?` / `^MONSC` decoders.
//!
//! The per-RAT field offsets below are the ones the WebUI's `parseMONSC` and
//! the CLI's cell printing already relied on; keeping them in one place is what
//! lets both surfaces (and the cache) agree.

use crate::modules::cell::state::CellState;

/// Hex string to decimal; invalid/empty input yields None.
pub fn hex_dec(s: &str) -> Option<u64> {
    u64::from_str_radix(s.trim().trim_matches('"'), 16).ok()
}

/// Parse `^HFREQINFO: <grp>,<flags>,<band>,<dl_arfcn>,<dl_freq_khz>,<dl_bw_khz>,...`.
///
/// Real reply: `^HFREQINFO: 0,7,41,513000,2565000,100000,513000,2565000,100000`.
pub fn parse_hfreqinfo(raw: &str, st: &mut CellState) -> bool {
    let Some(body) = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("^HFREQINFO:"))
    else {
        return false;
    };
    let fields: Vec<&str> = body.split(',').map(|f| f.trim()).collect();
    if let Some(band) = fields.get(2).filter(|s| !s.is_empty()) {
        st.band = Some((*band).to_string());
    }
    if let Some(arfcn) = fields.get(3).filter(|s| !s.is_empty()) {
        st.channel = Some((*arfcn).to_string());
    }
    if let Some(bw) = fields.get(5).and_then(|s| s.parse::<u64>().ok()) {
        st.dl_bandwidth_mhz = Some(bw as i64 / 1000);
    }
    true
}

/// Parse `^MONSC: <sysmode>,<mcc>,<mnc>,<channel>,...`.
///
/// Cell parameters are offset per RAT: LTE carries cid/pci/lac at 4/5/6, NR at
/// 5/6/7, WCDMA's pci is decimal while cid/lac are hex.
pub fn parse_monsc(raw: &str, st: &mut CellState) -> bool {
    let Some(body) = raw.lines().find_map(|l| l.trim().strip_prefix("^MONSC:")) else {
        return false;
    };
    let fields: Vec<&str> = body.split(',').map(|f| f.trim()).collect();
    if let Some(arfcn) = fields.first() {
        st.arfcn = Some((*arfcn).to_string());
    }
    if fields.len() < 4 {
        return true;
    }
    let sysmode = fields[0].trim_matches('"');
    if !sysmode.is_empty() {
        st.sysmode = Some(sysmode.to_string());
    }
    let mut put = |slot: &mut Option<String>, i: usize| {
        if let Some(v) = fields.get(i) {
            let s = v.trim().trim_matches('"');
            if !s.is_empty() {
                *slot = Some(s.to_string());
            }
        }
    };
    put(&mut st.mcc, 1);
    put(&mut st.mnc, 2);
    put(&mut st.channel, 3);
    match sysmode {
        "LTE" => {
            if let Some(v) = fields.get(4).and_then(|s| hex_dec(s)) {
                st.cid = Some(v.to_string());
            }
            if let Some(v) = fields.get(5).and_then(|s| hex_dec(s)) {
                st.pci = Some(v as i64);
            }
            if let Some(v) = fields.get(6).and_then(|s| hex_dec(s)) {
                st.lac = Some(v.to_string());
            }
        }
        "NR" => {
            if let Some(v) = fields.get(5).and_then(|s| hex_dec(s)) {
                st.cid = Some(v.to_string());
            }
            if let Some(v) = fields.get(6).and_then(|s| hex_dec(s)) {
                st.pci = Some(v as i64);
            }
            if let Some(v) = fields.get(7).and_then(|s| hex_dec(s)) {
                st.lac = Some(v.to_string());
            }
        }
        "WCDMA" => {
            if let Some(v) = fields.get(4).and_then(|s| s.parse::<u64>().ok()) {
                st.pci = Some(v as i64);
            }
            if let Some(v) = fields.get(5).and_then(|s| hex_dec(s)) {
                st.cid = Some(v.to_string());
            }
            if let Some(v) = fields.get(6).and_then(|s| hex_dec(s)) {
                st.lac = Some(v.to_string());
            }
        }
        _ => {}
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hfreqinfo_nr_carrier() {
        let mut st = CellState::default();
        assert!(parse_hfreqinfo(
            "^HFREQINFO: 0,7,41,513000,2565000,100000,513000,2565000,100000",
            &mut st
        ));
        assert_eq!(st.band.as_deref(), Some("41"));
        assert_eq!(st.channel.as_deref(), Some("513000"));
        assert_eq!(st.dl_bandwidth_mhz, Some(100));
    }

    #[test]
    fn monsc_lte_offsets() {
        let mut st = CellState::default();
        // PCI and cid/lac arrive hex-encoded for LTE/NR (legacy contract).
        assert!(parse_monsc(
            "^MONSC: \"LTE\",460,01,1650,\"1A2B3C\",\"1DC\",\"1F2E\"",
            &mut st
        ));
        assert_eq!(st.sysmode.as_deref(), Some("LTE"));
        assert_eq!(st.mcc.as_deref(), Some("460"));
        assert_eq!(st.mnc.as_deref(), Some("01"));
        assert_eq!(st.channel.as_deref(), Some("1650"));
        assert_eq!(st.cid.as_deref(), Some("1715004")); // 0x1A2B3C
        assert_eq!(st.pci, Some(476));
        assert_eq!(st.lac.as_deref(), Some("7982")); // 0x1F2E
    }

    #[test]
    fn monsc_nr_offsets_shift_cid_pci_lac() {
        let mut st = CellState::default();
        assert!(parse_monsc(
            "^MONSC: \"NR\",460,01,513000,0,\"1A2B3C\",\"1DC\",\"1F2E\"",
            &mut st
        ));
        assert_eq!(st.cid.as_deref(), Some("1715004"));
        assert_eq!(st.pci, Some(476));
        assert_eq!(st.lac.as_deref(), Some("7982"));
    }

    #[test]
    fn non_monsc_text_is_ignored() {
        let mut st = CellState::default();
        assert!(!parse_monsc("ERROR", &mut st));
        assert!(st.is_empty());
    }
}
