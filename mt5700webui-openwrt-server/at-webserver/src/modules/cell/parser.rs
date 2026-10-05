//! `^HFREQINFO?` / `^MONSC` decoders.
//!
//! The per-RAT field offsets below are the ones the WebUI's `parseMONSC` and
//! the CLI's cell printing already relied on; keeping them in one place is what
//! lets both surfaces (and the cache) agree.

use crate::modules::cell::state::{CellState, NeighborCell};

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

/// Band number for a downlink ARFCN, per the 3GPP band tables the MT5700M
/// reports (`LTE` E-UTRA bands, `NR` FR1/FR2 ranges).
///
/// This table used to live in the WebUI's `modem/parse.ts`; the neighbour scan
/// needs it for its band column, and it is radio knowledge, so it belongs next
/// to the cell decoders. `None` means the ARFCN falls outside every listed
/// band — the UI shows `—` rather than guessing.
pub fn arfcn_to_band(rat: &str, arfcn: i64) -> Option<i64> {
    const LTE: [(i64, i64, i64); 9] = [
        (0, 599, 1),
        (1200, 1949, 3),
        (2400, 2649, 5),
        (3450, 3799, 8),
        (36200, 36349, 34),
        (37750, 38249, 38),
        (38250, 38649, 39),
        (38650, 39649, 40),
        (39650, 41589, 41),
    ];
    const NR: [(i64, i64, i64); 9] = [
        (422000, 434000, 1),
        (361000, 376000, 3),
        (173800, 178800, 5),
        (185000, 192000, 8),
        (151600, 160600, 28),
        (499200, 537999, 41),
        (620000, 653333, 78),
        (653334, 680000, 77),
        (693334, 733333, 79),
    ];
    let table: &[(i64, i64, i64)] = match rat {
        "LTE" => &LTE,
        "NR" => &NR,
        _ => return None,
    };
    table
        .iter()
        .find(|(lo, hi, _)| arfcn >= *lo && arfcn <= *hi)
        .map(|(_, _, band)| *band)
}

/// Decode `^MONNC` neighbour lines (one per cell).
///
/// Shape: `^MONNC: <rat>[,<values…>]`; `NONE` (no neighbours) carries no
/// values. Field order per RAT comes from the manual (13.19):
///
/// ```text
/// LTE: <arfcn>,<pci(hex)>,<rsrp>,<rsrq>,<rxlev>
/// NR:  <arfcn>,<pci(hex)>,<rsrp>,<rsrq>,<sinr>
/// ```
///
/// Some firmware reports the NR values in 1/8 units; a value outside the
/// manual's legal range (-156..-31 RSRP, -43..20 RSRQ, -23..40 SINR) is scaled
/// back down, which is what the page used to do inline. The scaled values are
/// strings, so the table renders exactly the digits it rendered before.
pub fn parse_monnc(raw: &str) -> Vec<NeighborCell> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let t = line.trim();
        let Some(idx) = t.find("^MONNC:") else { continue };
        let rest = t[idx + "^MONNC:".len()..].trim();
        let mut fields = rest.splitn(2, ',');
        let rat = fields.next().unwrap_or("").trim().trim_matches('"');
        if rat.is_empty() || rat == "NONE" {
            continue;
        }
        let values: Vec<String> = fields
            .next()
            .unwrap_or("")
            .split(',')
            .map(|v| v.trim().trim_matches('"').to_string())
            .collect();
        if !matches!(rat, "LTE" | "NR") {
            continue;
        }
        let arfcn = values.first().and_then(|v| v.parse::<i64>().ok());
        let pci = values
            .get(1)
            .and_then(|v| i64::from_str_radix(v, 16).ok());
        let scale = |raw: Option<&String>, limit: f64| -> Option<String> {
            let text = raw?;
            let n: f64 = text.parse().ok()?;
            Some(if n.abs() > limit {
                format!("{:.1}", n / 8.0)
            } else {
                text.clone()
            })
        };
        let cell = match rat {
            "LTE" => NeighborCell {
                rat: rat.to_string(),
                arfcn,
                pci,
                rsrp: values.get(2).cloned(),
                rsrq: values.get(3).cloned(),
                sinr: None,
                rxlev: values.get(4).cloned(),
                band: arfcn.and_then(|a| arfcn_to_band("LTE", a)),
            },
            _ => NeighborCell {
                rat: rat.to_string(),
                arfcn,
                pci,
                rsrp: scale(values.get(2), 157.0),
                rsrq: scale(values.get(3), 43.5),
                sinr: scale(values.get(4), 40.0),
                rxlev: None,
                band: arfcn.and_then(|a| arfcn_to_band("NR", a)),
            },
        };
        out.push(cell);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arfcn_band_table_matches_manual_ranges() {
        assert_eq!(arfcn_to_band("LTE", 1850), Some(3));
        assert_eq!(arfcn_to_band("LTE", 100), Some(1));
        assert_eq!(arfcn_to_band("LTE", 41589), Some(41));
        assert_eq!(arfcn_to_band("LTE", 999999), None);
        assert_eq!(arfcn_to_band("NR", 643456), Some(78));
        assert_eq!(arfcn_to_band("NR", 653334), Some(77));
        assert_eq!(arfcn_to_band("NR", 504990), Some(41));
        assert_eq!(arfcn_to_band("NR", 1), None);
        assert_eq!(arfcn_to_band("WCDMA", 100), None);
    }

    #[test]
    fn monnc_parses_both_rasters_and_skips_none() {
        let raw = "^MONNC: NONE\n^MONNC: LTE,1850,64,-85,-12,30\n^MONNC: NR,643456,1A,-95,-11,-5\nOK";
        let cells = parse_monnc(raw);
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].rat, "LTE");
        assert_eq!(cells[0].arfcn, Some(1850));
        assert_eq!(cells[0].pci, Some(100)); // 0x64
        assert_eq!(cells[0].rsrp.as_deref(), Some("-85"));
        assert_eq!(cells[0].rxlev.as_deref(), Some("30"));
        assert_eq!(cells[0].band, Some(3));
        assert_eq!(cells[1].rat, "NR");
        assert_eq!(cells[1].pci, Some(26)); // 0x1A
        assert_eq!(cells[1].sinr.as_deref(), Some("-5"));
        assert_eq!(cells[1].band, Some(78));
        assert!(cells[1].rxlev.is_none());
    }

    #[test]
    fn monnc_scales_oversized_nr_values() {
        // 1/8-unit reporting: -760/8 = -95.0, -96/8 = -12.0
        let cells = parse_monnc("^MONNC: NR,643456,1A,-760,-96,-40");
        assert_eq!(cells[0].rsrp.as_deref(), Some("-95.0"));
        assert_eq!(cells[0].rsrq.as_deref(), Some("-12.0"));
        assert_eq!(cells[0].sinr.as_deref(), Some("-5"));
        // Values inside the legal range are passed through untouched.
        let cells = parse_monnc("^MONNC: NR,643456,1A,-95,-11,-5");
        assert_eq!(cells[0].rsrp.as_deref(), Some("-95"));
    }

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
