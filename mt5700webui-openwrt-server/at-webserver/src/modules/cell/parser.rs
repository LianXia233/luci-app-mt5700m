//! `^HFREQINFO?` / `^MONSC` decoders.
//!
//! The per-RAT field offsets below are the ones the WebUI's `parseMONSC` and
//! the CLI's cell printing already relied on; keeping them in one place is what
//! lets both surfaces (and the cache) agree.

use crate::core::json::Value;
use crate::core::radio::arfcn_to_band;
use crate::modules::cell::state::{CellState, NeighborCell};

/// Values some firmware prints in place of a measurement. The LuCI parser
/// blanked exactly this set (`cleanSignal`), and so did the WebUI's table
/// renderer: a neighbour with no reading shows an empty cell, never `32767`.
const NO_VALUE: [f64; 5] = [-1256.0, -348.0, -188.0, 32767.0, 255.0];

/// Decode one `^MONNC` measurement field.
///
/// Two firmware quirks, both of them radio knowledge and both handled only
/// here: sentinel values that mean "no measurement" become `None` (the field is
/// then omitted from the JSON, like every other absent reading), and the 1/8
/// units some NR firmware uses are restored — detected by the value falling
/// outside the manual's legal range (`limit`) rather than by a flag.
fn measurement(text: Option<&String>, limit: Option<f64>) -> Option<String> {
    let t = text?;
    let n: f64 = t.parse().ok()?;
    if NO_VALUE.contains(&n) {
        return None;
    }
    match limit {
        Some(limit) if n.abs() > limit => Some(format!("{:.1}", n / 8.0)),
        _ => Some(t.clone()),
    }
}

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
            // Field 4 is the subcarrier-spacing code (0 = 15 kHz); the page maps
            // it to kHz, so the code travels and the label stays display.
            if let Some(v) = fields.get(4).and_then(|s| s.trim().parse::<i64>().ok()) {
                st.scs = Some(v);
            }
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
        let cell = match rat {
            "LTE" => NeighborCell {
                rat: rat.to_string(),
                arfcn,
                pci,
                rsrp: measurement(values.get(2), None),
                rsrq: measurement(values.get(3), None),
                sinr: None,
                rxlev: measurement(values.get(4), None),
                band: arfcn.and_then(|a| arfcn_to_band("LTE", a)),
            },
            _ => NeighborCell {
                rat: rat.to_string(),
                arfcn,
                pci,
                rsrp: measurement(values.get(2), Some(157.0)),
                rsrq: measurement(values.get(3), Some(43.5)),
                sinr: measurement(values.get(4), Some(40.0)),
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
        // 1/8-unit reporting: -760/8 = -95.0, -96/8 = -12.0, -320/8 = -40.0
        let cells = parse_monnc("^MONNC: NR,643456,1A,-760,-96,-320");
        assert_eq!(cells[0].rsrp.as_deref(), Some("-95.0"));
        assert_eq!(cells[0].rsrq.as_deref(), Some("-12.0"));
        assert_eq!(cells[0].sinr.as_deref(), Some("-40.0"));
        // At the limit the value is already in dB, so it is passed through
        // untouched (the -40 dB SINR check) — the same rule the WebUI had.
        let cells = parse_monnc("^MONNC: NR,643456,1A,-95,-11,-40");
        assert_eq!(cells[0].rsrp.as_deref(), Some("-95"));
        assert_eq!(cells[0].rsrq.as_deref(), Some("-11"));
        assert_eq!(cells[0].sinr.as_deref(), Some("-40"));
    }

    #[test]
    fn monnc_drops_sentinel_measurements() {
        // The firmware's "no reading" sentinels: the field is omitted from the
        // domain value (and therefore from the JSON), which is what the page's
        // empty bar means. -348 is RSRQ's sentinel and also inside the NR
        // 1/8-unit window, so the sentinel check must run first.
        let cells = parse_monnc("^MONNC: NR,643456,1A,-1256,-348,-188");
        assert_eq!(cells[0].rsrp, None);
        assert_eq!(cells[0].rsrq, None);
        assert_eq!(cells[0].sinr, None);
        let cells = parse_monnc("^MONNC: LTE,1850,64,255,32767,-12");
        assert_eq!(cells[0].rsrp, None);
        assert_eq!(cells[0].rsrq, None);
        assert_eq!(cells[0].rxlev.as_deref(), Some("-12"));
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
    fn monsc_nr_carries_the_scs_code() {
        let mut st = CellState::default();
        parse_monsc("^MONSC: NR,460,00,636648,1,10321,1FA,2F01,-82,-9", &mut st);
        assert_eq!(st.scs, Some(1));
        assert_eq!(st.cid.as_deref(), Some("66337")); // 0x10321
        assert_eq!(st.pci, Some(0x1FA));
        let Value::Obj(m) = st.to_json() else { panic!("object") };
        assert_eq!(m.get("scs").and_then(|v| v.as_i64()), Some(1));
        // LTE has no SCS field.
        let mut lte = CellState::default();
        parse_monsc("^MONSC: LTE,460,00,1650,10321,64,2F01,-82,-9,-70", &mut lte);
        assert_eq!(lte.scs, None);
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
