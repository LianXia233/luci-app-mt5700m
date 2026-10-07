//! `^HFREQINFO?` / `^CASCELLINFO?` / `^MONSSC` decoders.
//!
//! Field offsets live here and nowhere else: the WebUI's carrier card, the LuCI
//! carrier panel and the CLI all render the state this module decoded.

use crate::modules::ca::state::{CaCarrier, CaSource, CaState, SecondaryCell, SecondaryLte, SecondaryNr};

/// LTE bandwidth code -> MHz. The `^CASCELLINFO` answer reports a code, not a
/// bandwidth, so the table is part of the decode.
pub fn lte_bandwidth(code: i64) -> f64 {
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

/// Strip spaces/CR/quotes from a response body before splitting on commas —
/// the modem pads `^HFREQINFO`/`^CASCELLINFO` answers inconsistently.
fn clean_body(rest: &str) -> String {
    rest.chars()
        .filter(|c| !matches!(c, ' ' | '\r' | '"'))
        .collect()
}

/// Decode every carrier group of every `^HFREQINFO:` line.
///
/// `field[1]` selects the RAT: `7` = NR (kHz frequencies, up to 4 groups),
/// `6` = LTE (100 kHz frequencies, 1 group). Each group is 7 numbers:
/// band, dl_arfcn, dl_freq, dl_bw, ul_arfcn, ul_freq, ul_bw — where the
/// bandwidth is in kHz in both cases (`/1000`).
///
/// A group whose fields are not all numeric (the modem pads empty carriers with
/// text) is skipped instead of aborting the whole answer, and the index
/// arithmetic is absolute — the 2.4.x builds indexed `field[i + (i + k)]` and
/// panicked on the 9-field line below, blanking the LuCI carrier panel.
pub fn parse_hfreqinfo(raw: &str) -> Vec<CaCarrier> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let Some(rest) = line.trim().strip_prefix("^HFREQINFO:") else {
            continue;
        };
        let clean = clean_body(rest);
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
            let numeric = (0..7).all(|k| {
                !field[i + k].is_empty()
                    && field[i + k].bytes().all(|b| b.is_ascii_digit())
            });
            if numeric {
                let num = |k: usize| field[k].parse::<f64>().unwrap_or(0.0);
                let band = if radio == "NR" {
                    format!("n{}", field[i])
                } else {
                    format!("B{}", field[i])
                };
                out.push(CaCarrier {
                    radio: radio.to_string(),
                    band,
                    dl_arfcn: field[i + 1].to_string(),
                    ul_arfcn: field[i + 4].to_string(),
                    dl_frequency_mhz: num(i + 2) / divisor,
                    ul_frequency_mhz: num(i + 5) / divisor,
                    dl_bandwidth_mhz: num(i + 3) / 1000.0,
                    ul_bandwidth_mhz: num(i + 6) / 1000.0,
                    source: CaSource::HfreqInfo,
                });
                parsed += 1;
                i += 7;
            } else {
                i += 7;
            }
        }
    }
    out
}

/// Decode one `^CASCELLINFO:` line into an LTE secondary cell.
///
/// Field map (manual 13.18): `[0]` index, `[1]` PCI, `[2]` RSSI, `[3]` RSRP,
/// `[4]` RSRQ, `[5]` band, `[6]` ul_arfcn, `[7]` dl_arfcn, `[8]` ul_freq (100 kHz),
/// `[9]` dl_freq (100 kHz), `[10]` ul bandwidth code, `[11]` dl bandwidth code.
/// This is the **only** field map for the reply: [`parse_cascellinfo`] derives
/// the aggregated carrier list from it, so the carriers and the per-cell signal
/// can never disagree.
pub fn parse_lte_secondary(line: &str) -> Option<SecondaryLte> {
    let rest = line.trim().strip_prefix("^CASCELLINFO:")?;
    let clean = clean_body(rest);
    let field: Vec<&str> = clean.split(',').collect();
    if field.len() < 12 {
        return None;
    }
    let nums: Option<Vec<i64>> = field[..12].iter().map(|f| f.parse::<i64>().ok()).collect();
    let nums = nums?;
    let mhz = |code: i64| {
        let value = lte_bandwidth(code);
        (value > 0.0).then_some(value)
    };
    Some(SecondaryLte {
        index: nums[0],
        pci: nums[1],
        rssi: Some(nums[2] as f64),
        rsrp: Some(nums[3] as f64),
        rsrq: Some(nums[4] as f64),
        band: nums[5],
        ul_arfcn: Some(nums[6]),
        dl_arfcn: Some(nums[7]),
        ul_frequency_mhz: Some(nums[8] as f64 / 10.0),
        dl_frequency_mhz: Some(nums[9] as f64 / 10.0),
        ul_bandwidth_mhz: mhz(nums[10]),
        dl_bandwidth_mhz: mhz(nums[11]),
    })
}

/// Decode `^CASCELLINFO:` LTE secondary cells (12 numeric fields).
pub fn parse_cascellinfo(raw: &str) -> Vec<CaCarrier> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let Some(cell) = parse_lte_secondary(line) else {
            continue;
        };
        out.push(CaCarrier {
            radio: "LTE".to_string(),
            band: format!("B{}", cell.band),
            dl_arfcn: cell.dl_arfcn.unwrap_or(0).to_string(),
            ul_arfcn: cell.ul_arfcn.unwrap_or(0).to_string(),
            dl_frequency_mhz: cell.dl_frequency_mhz.unwrap_or(0.0),
            ul_frequency_mhz: cell.ul_frequency_mhz.unwrap_or(0.0),
            dl_bandwidth_mhz: cell.dl_bandwidth_mhz.unwrap_or(0.0),
            ul_bandwidth_mhz: cell.ul_bandwidth_mhz.unwrap_or(0.0),
            source: CaSource::LteScell,
        });
    }
    out
}

/// 手册 13.27.3 `<MEASTYPE>`: 0 SSB 测量，1 CSI-RS 测量；其它值（无效测量）
/// 显示为手册里的 `—`。
fn meas_type_name(code: i64) -> String {
    match code {
        0 => "SSB".to_string(),
        1 => "CSI-RS".to_string(),
        _ => "—".to_string(),
    }
}

/// 手册 13.27.3 的无效值：RSRP -1256、RSRQ -348、SINR -188。
const NR_INVALID_RSRP: f64 = -1256.0;
const NR_INVALID_RSRQ: f64 = -348.0;
const NR_INVALID_SINR: f64 = -188.0;

/// 手册 13.27 的参数表与它自己的示例互相矛盾：表里说 RSRP `-156..-31`、
/// 无效值写成 `-1256`（`-157 * 8`），暗示上报值是 8 倍；示例给的却是 `-70`
/// 这种未放大的值。所以按量级判断——超出合法区间就当 8 倍值还原，否则原样用
/// （`^MONNC` 的解析也是这么处理的，前端 `modem/carrier.ts` 曾有一份同样的
/// 规则，现在只有这一份）。
fn descale(value: f64, min: f64, max: f64) -> f64 {
    if value < min || value > max {
        (value / 8.0 * 10.0).round() / 10.0
    } else {
        value
    }
}

fn signed(field: &str) -> Option<f64> {
    let t = field.trim();
    if t.is_empty() {
        None
    } else {
        t.parse::<f64>().ok()
    }
}

/// Decode one `^MONSSC:` line into an NR secondary cell (manual 13.27).
///
/// `NONE` (not in EN-DC) or the LTE branch (the manual marks it unsupported)
/// yields `None`, as does an unparsable ARFCN/PCI.
pub fn parse_nr_secondary(line: &str) -> Option<SecondaryNr> {
    let rest = line.trim().strip_prefix("^MONSSC:")?;
    let field: Vec<&str> = rest.split(',').map(|f| f.trim()).collect();
    let rat = field.first()?.trim_matches('"').to_ascii_uppercase();
    if rat != "NR" || field.len() < 3 {
        return None;
    }
    let arfcn: i64 = field[1].parse().ok()?;
    // 手册 13.27.3: <PCI> 十六进制，取值 0~0x3EF。
    let pci = i64::from_str_radix(field[2].trim(), 16).ok()?;
    let pick = |index: usize, invalid: f64, min: f64, max: f64| -> Option<f64> {
        let raw = signed(field.get(index).copied().unwrap_or(""))?;
        if raw == invalid {
            return None;
        }
        Some(descale(raw, min, max))
    };
    Some(SecondaryNr {
        arfcn,
        pci,
        rsrp: pick(3, NR_INVALID_RSRP, -156.0, -31.0),
        rsrq: pick(4, NR_INVALID_RSRQ, -43.0, 20.0),
        sinr: pick(5, NR_INVALID_SINR, -23.0, 40.0),
        meas_type: meas_type_name(
            signed(field.get(6).copied().unwrap_or(""))
                .map(|v| v as i64)
                .unwrap_or(-1),
        ),
    })
}

/// Every secondary cell reported by `^CASCELLINFO?` and `^MONSSC`.
pub fn parse_secondaries(cascellinfo: &str, monssc: &str) -> Vec<SecondaryCell> {
    let mut out: Vec<SecondaryCell> = cascellinfo
        .lines()
        .filter_map(parse_lte_secondary)
        .map(SecondaryCell::Lte)
        .collect();
    out.extend(
        monssc
            .lines()
            .filter_map(parse_nr_secondary)
            .map(SecondaryCell::Nr),
    );
    out
}

/// Count `^MONSSC:` lines that report an NR leg (`NR,`).
pub fn parse_monssc_secondary(raw: &str) -> usize {
    raw.lines()
        .filter(|l| l.trim().starts_with("^MONSSC:") && l.contains("NR,"))
        .count()
}

/// Full decode: the three answers into one [`CaState`].
pub fn parse(hfreqinfo: &str, cascellinfo: &str, monssc: &str) -> CaState {
    let mut carriers = parse_hfreqinfo(hfreqinfo);
    carriers.extend(parse_cascellinfo(cascellinfo));
    CaState {
        carriers,
        secondary: parse_secondaries(cascellinfo, monssc),
        secondary_connection_count: parse_monssc_secondary(monssc),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::json::Value;

    #[test]
    fn hfreqinfo_real_nr_carrier() {
        // Regression: the 2.4.1/2.4.2 num closure indexed field[i + (i + k)],
        // panicking on this exact 9-field line from the modem
        // ("index out of bounds: the len is 9 but the index is 9").
        let carriers = parse_hfreqinfo("^HFREQINFO: 0,7,41,513000,2565000,100000,513000,2565000,100000");
        assert_eq!(carriers.len(), 1);
        let c = &carriers[0];
        assert_eq!(c.radio, "NR");
        assert_eq!(c.band, "n41");
        assert_eq!(c.dl_arfcn, "513000");
        assert_eq!(c.dl_frequency_mhz, 2565.0);
        assert_eq!(c.dl_bandwidth_mhz, 100.0);
        let st = CaState {
            carriers,
            secondary: Vec::new(),
            secondary_connection_count: 0,
        };
        assert_eq!(st.nr_carriers(), 1);
        assert_eq!(st.dl_bandwidth_mhz(), 100.0);
    }

    #[test]
    fn hfreqinfo_non_numeric_group_skipped() {
        let carriers = parse_hfreqinfo("^HFREQINFO: 0,7,n41,513000,2565000,100000,513000,2565000,100000");
        assert!(carriers.is_empty());
    }

    #[test]
    fn hfreqinfo_lte_single_carrier() {
        let carriers = parse_hfreqinfo("^HFREQINFO: 0,6,3,1650,18400,20000,1850,17450,20000");
        assert_eq!(carriers.len(), 1);
        assert_eq!(carriers[0].radio, "LTE");
        assert_eq!(carriers[0].band, "B3");
        assert_eq!(carriers[0].dl_frequency_mhz, 1840.0);
        assert_eq!(carriers[0].ul_bandwidth_mhz, 20.0);
    }

    #[test]
    fn cascellinfo_decodes_codes_and_tenths() {
        let carriers =
            parse_cascellinfo("^CASCELLINFO: 1,0,0,0,0,3,1850,1650,1745,1840,5,5");
        assert_eq!(carriers.len(), 1);
        let c = &carriers[0];
        assert_eq!(c.band, "B3");
        assert_eq!(c.dl_arfcn, "1650");
        assert_eq!(c.ul_arfcn, "1850");
        assert_eq!(c.dl_frequency_mhz, 184.0);
        assert_eq!(c.ul_frequency_mhz, 174.5);
        assert_eq!(c.dl_bandwidth_mhz, 20.0);
        assert_eq!(c.ul_bandwidth_mhz, 20.0);
        assert!(matches!(c.source, CaSource::LteScell));
    }

    #[test]
    fn short_cascellinfo_line_is_ignored() {
        assert!(parse_cascellinfo("^CASCELLINFO: 1,2,3").is_empty());
    }

    #[test]
    fn monssc_secondary_counts_nr_lines_only() {
        let raw = "^MONSSC: NR,78,1\r\n^MONSSC: LTE,3,1\r\nOK\r\n";
        assert_eq!(parse_monssc_secondary(raw), 1);
    }

    #[test]
    fn parse_merges_all_three_answers_in_order() {
        let st = parse(
            "^HFREQINFO: 0,7,41,513000,2565000,100000,513000,2565000,100000",
            "^CASCELLINFO: 1,0,0,0,0,3,1850,1650,1745,1840,5,5",
            "^MONSSC: NR,78,1",
        );
        assert_eq!(st.carriers.len(), 2);
        assert_eq!(st.carriers[0].radio, "NR");
        assert_eq!(st.carriers[1].radio, "LTE");
        assert_eq!(st.secondary_connection_count, 1);
        assert_eq!(st.mode(), "EN-DC + CA");
        // 同一条 ^MONSSC 行既计入辅连接数，也进 secondary 列表。
        assert_eq!(st.secondary.len(), 2);
    }

    #[test]
    fn monssc_decodes_the_handbook_example() {
        // 手册 13.27 的示例：^MONSSC: NR,636648,64,-70,-20,-10,0
        let nr = parse_nr_secondary("^MONSSC: NR,636648,64,-70,-20,-10,0").unwrap();
        assert_eq!(nr.arfcn, 636648);
        assert_eq!(nr.pci, 0x64);
        assert_eq!(nr.rsrp, Some(-70.0));
        assert_eq!(nr.rsrq, Some(-20.0));
        assert_eq!(nr.sinr, Some(-10.0));
        assert_eq!(nr.meas_type, "SSB");
        // NONE(非 ENDC)/LTE 分支/字段不足都不产出小区。
        assert!(parse_nr_secondary("^MONSSC: NONE").is_none());
        assert!(parse_nr_secondary("^MONSSC: LTE,3,1").is_none());
        assert!(parse_nr_secondary("^MONSSC: NR,636648").is_none());
        // 十六进制 PCI 与 CSI-RS 测量。
        let nr = parse_nr_secondary("^MONSSC: NR,504990,1F,-68,-10,1,1").unwrap();
        assert_eq!(nr.pci, 0x1f);
        assert_eq!(nr.meas_type, "CSI-RS");
    }

    #[test]
    fn monssc_invalid_sentinels_and_the_eight_x_heuristic() {
        // 手册的无效值 → 该字段没有读数，不代表 -1256 dBm。
        let nr = parse_nr_secondary("^MONSSC: NR,78,1,-1256,-348,-188,2").unwrap();
        assert_eq!(nr.rsrp, None);
        assert_eq!(nr.rsrq, None);
        assert_eq!(nr.sinr, None);
        assert_eq!(nr.meas_type, "—");
        // 超出合法区间的值按 8 倍还原（RSRP -560 / 8 = -70；SINR -40 超出
        // -23..40，也按 8 倍还原成 -5）。区间内的值原样保留。
        let nr = parse_nr_secondary("^MONSSC: NR,78,1,-560,0,-40,0").unwrap();
        assert_eq!(nr.rsrp, Some(-70.0));
        assert_eq!(nr.rsrq, Some(0.0));
        assert_eq!(nr.sinr, Some(-5.0));
    }

    #[test]
    fn cascellinfo_decodes_the_signal_fields_too() {
        // 手册 13.18 的示例：^CASCELLINFO: 1,417,-60,-80,-5,3,23925,1650,8225,8675,5,5
        let cell = parse_lte_secondary(
            "^CASCELLINFO: 1,417,-60,-80,-5,3,23925,1650,8225,8675,5,5",
        )
        .unwrap();
        assert_eq!(cell.index, 1);
        assert_eq!(cell.pci, 417);
        assert_eq!(cell.rssi, Some(-60.0));
        assert_eq!(cell.rsrp, Some(-80.0));
        assert_eq!(cell.rsrq, Some(-5.0));
        assert_eq!(cell.band, 3);
        assert_eq!(cell.ul_arfcn, Some(23925));
        assert_eq!(cell.dl_arfcn, Some(1650));
        assert_eq!(cell.ul_frequency_mhz, Some(822.5));
        assert_eq!(cell.dl_frequency_mhz, Some(867.5));
        assert_eq!(cell.ul_bandwidth_mhz, Some(20.0));
        assert_eq!(cell.dl_bandwidth_mhz, Some(20.0));
        assert!(parse_lte_secondary("^CASCELLINFO: 1,2,3").is_none());
    }

    #[test]
    fn secondary_cells_reach_the_json_round_trip() {
        let st = parse(
            "",
            "^CASCELLINFO: 1,417,-60,-80,-5,3,23925,1650,8225,8675,5,5",
            "^MONSSC: NR,636648,64,-70,-20,-10,0",
        );
        assert_eq!(st.secondary.len(), 2);
        let dump = st.to_json().dump();
        assert!(dump.contains("\"radio\":\"NR\""), "{}", dump);
        assert!(dump.contains("\"measType\":\"SSB\""), "{}", dump);
        assert!(dump.contains("\"dlArfcn\":1650"), "{}", dump);
        // The cache round trip keeps both, signal values included.
        let Value::Obj(map) = st.to_json() else {
            panic!("object expected")
        };
        let back = CaState::from_json(&map);
        assert_eq!(back.secondary, st.secondary);
        assert_eq!(back.secondary_connection_count, 1);
    }
}
