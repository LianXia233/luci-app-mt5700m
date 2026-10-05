//! `^HFREQINFO?` / `^CASCELLINFO?` / `^MONSSC` decoders.
//!
//! Field offsets live here and nowhere else: the WebUI's carrier card, the LuCI
//! carrier panel and the CLI all render the state this module decoded.

use crate::modules::ca::state::{CaCarrier, CaSource, CaState};

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

/// Decode `^CASCELLINFO:` LTE secondary cells (12 numeric fields).
///
/// Field map: `[5]` band, `[6]` ul_arfcn, `[7]` dl_arfcn, `[8]` ul_freq/10,
/// `[9]` dl_freq/10, `[10]` ul bandwidth code, `[11]` dl bandwidth code.
pub fn parse_cascellinfo(raw: &str) -> Vec<CaCarrier> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let Some(rest) = line.trim().strip_prefix("^CASCELLINFO:") else {
            continue;
        };
        let clean = clean_body(rest);
        let field: Vec<&str> = clean.split(',').collect();
        if field.len() < 12 {
            continue;
        }
        let nums: Option<Vec<i64>> = field[..12].iter().map(|f| f.parse::<i64>().ok()).collect();
        let Some(nums) = nums else { continue };
        out.push(CaCarrier {
            radio: "LTE".to_string(),
            band: format!("B{}", nums[5]),
            dl_arfcn: nums[7].to_string(),
            ul_arfcn: nums[6].to_string(),
            dl_frequency_mhz: nums[9] as f64 / 10.0,
            ul_frequency_mhz: nums[8] as f64 / 10.0,
            dl_bandwidth_mhz: lte_bandwidth(nums[11]),
            ul_bandwidth_mhz: lte_bandwidth(nums[10]),
            source: CaSource::LteScell,
        });
    }
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
        secondary_connection_count: parse_monssc_secondary(monssc),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
