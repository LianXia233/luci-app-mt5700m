//! Decoders for modem identity, transmit power and EN-DC status.

use crate::modules::modem::state::{
    EndcState, McsCarrier, McsState, ModemState, NrCarrier, NrTxPowerState, TxPowerState,
};
use crate::state::refresh::round1;

/// `ATI` lines `Manufacturer:` / `Model:` / `Revision:`.
pub fn parse_ati(raw: &str) -> ModemState {
    let mut st = ModemState::default();
    for (prefix, key) in [
        ("Manufacturer:", "manufacturer"),
        ("Model:", "model"),
        ("Revision:", "revision"),
    ] {
        if let Some(rest) = raw.lines().find_map(|l| l.trim().strip_prefix(prefix)) {
            let value = rest.trim().trim_matches('"').to_string();
            match key {
                "manufacturer" => st.manufacturer = Some(value),
                "model" => st.model = Some(value),
                _ => st.revision = Some(value),
            }
        }
    }
    st
}

/// `AT+CGSN` answers with the bare 15-digit IMEI.
pub fn parse_imei(raw: &str) -> Option<String> {
    raw.lines()
        .map(|l| l.trim())
        .find(|t| t.len() == 15 && t.bytes().all(|b| b.is_ascii_digit()))
        .map(|t| t.to_string())
}

/// `^TXPOWER: <total>,<pusch>,<pucch>,<srs>,<prach>`; `999` means "not valid".
pub fn parse_txpower(raw: &str) -> TxPowerState {
    let mut st = TxPowerState::default();
    let Some(body) = raw.lines().find_map(|l| l.trim().strip_prefix("^TXPOWER:")) else {
        return st;
    };
    let fields: Vec<&str> = body.split(',').map(|f| f.trim()).collect();
    if fields.len() < 5 {
        return st;
    }
    let power = |f: &str| -> Option<i64> {
        let n = f.parse::<i64>().ok()?;
        if n == 999 {
            None
        } else {
            Some(n)
        }
    };
    if let Some(total) = power(fields[0]) {
        // <stxpwr> is in 0.1 dBm
        st.total = Some(round1(total as f64 / 10.0));
    }
    st.pusch = power(fields[1]);
    st.pucch = power(fields[2]);
    st.srs = power(fields[3]);
    st.prach = power(fields[4]);
    st
}

/// `^NTXPOWER: <5 fields per carrier>`, up to four carriers.
pub fn parse_nr_txpower(raw: &str) -> NrTxPowerState {
    let mut st = NrTxPowerState::default();
    let Some(body) = raw.lines().find_map(|l| l.trim().strip_prefix("^NTXPOWER:")) else {
        return st;
    };
    let fields: Vec<&str> = body.split(',').map(|f| f.trim()).collect();
    let power = |f: &str| -> Option<i64> {
        let n = f.parse::<i64>().ok()?;
        if n == 999 {
            None
        } else {
            Some(n)
        }
    };
    let mut i = 0;
    while i + 4 < fields.len() && st.carriers.len() < 4 {
        let mut c = NrCarrier {
            pusch: power(fields[i]),
            pucch: power(fields[i + 1]),
            srs: power(fields[i + 2]),
            prach: power(fields[i + 3]),
            freq: fields[i + 4].parse::<i64>().ok().filter(|f| *f > 0),
        };
        if c.pusch.is_none() && c.pucch.is_none() && c.srs.is_none() && c.prach.is_none() {
            c = NrCarrier {
                freq: c.freq,
                ..Default::default()
            };
        }
        if c != NrCarrier::default() {
            st.carriers.push(c);
        }
        i += 5;
    }
    st
}

/// `^LENDC` reply. The query form carries an `<enable>` prefix (5+ fields), the
/// unsolicited form does not (4 fields) — same rule the frontends used.
pub fn parse_lendc(raw: &str) -> EndcState {
    let mut st = EndcState::default();
    let Some(body) = raw.lines().find_map(|l| l.trim().strip_prefix("^LENDC:")) else {
        return st;
    };
    let fields: Vec<u8> = body
        .split(',')
        .map(|f| f.trim().parse::<u8>().unwrap_or(0))
        .filter(|v| *v == 0 || *v == 1)
        .collect();
    let v: Vec<u8> = if fields.len() >= 5 {
        fields[1..].to_vec()
    } else {
        fields
    };
    if v.len() >= 4 {
        st.available = Some(v[0] as i64);
        st.plmn_available = Some(v[1] as i64);
        st.restricted = Some((v[2] == 0) as i64);
        st.established = Some(v[3] as i64);
    }
    st
}

/// `^MCS` table decode: one line per carrier group, three values per carrier.
///
/// Line shape `^MCS: <group>,<rat>,<table0>,<code0>,<code1>[,<table1>,<code1a>,
/// <code1b>...]`; `<rat>` is `1` for NR and `0` for LTE. Rows whose group did
/// not survive the regex-free split (fewer than three values left over) are
/// dropped, exactly as the WebUI's loop did — this decoder replaces that loop,
/// it does not change what the page shows.
pub fn parse_mcs(raw: &str) -> McsState {
    let mut st = McsState::default();
    for line in raw.lines() {
        let t = line.trim();
        let Some(idx) = t.find("^MCS:") else { continue };
        let body = t[idx + "^MCS:".len()..].trim();
        let mut head = body.splitn(3, ',');
        let group = head.next().map(|f| f.trim());
        let rat = head.next().map(|f| f.trim());
        let rest = head.next().map(|f| f.trim());
        let (Some(group), Some(rat), Some(rest)) = (group, rat, rest) else {
            continue;
        };
        if group.is_empty() || !group.bytes().all(|b| b.is_ascii_digit()) || rest.is_empty() {
            continue;
        }
        if rat == "1" {
            st.rat = "NR";
        } else if rat == "0" && st.rat == "UNKNOWN" {
            st.rat = "LTE";
        }
        // Unparseable codes are treated as "not in use" (255), the same value
        // the firmware uses, so one bad token cannot poison the average.
        let values: Vec<i64> = rest
            .split(',')
            .map(|v| v.trim().parse::<i64>().unwrap_or(255))
            .collect();
        let mut i = 0;
        while i + 2 < values.len() {
            st.carriers.push(McsCarrier {
                index: st.carriers.len() + 1,
                mcs_table_index: values[i],
                code0: values[i + 1],
                code1: values[i + 2],
            });
            i += 3;
        }
    }
    let valid: Vec<i64> = st
        .carriers
        .iter()
        .map(|c| c.code0)
        .filter(|c| *c != 255)
        .collect();
    st.avg_mcs = if valid.is_empty() {
        0
    } else {
        ((valid.iter().sum::<i64>() as f64) / (valid.len() as f64)).round() as i64
    };
    st
}

/// `^NRRCCAPQRY: <kind>,<values…>` -> the values after the kind.
///
/// The reply echoes the kind it answers (3 = CA, 2 = VoNR, 5 = DSS), so a page
/// that asks for one ability cannot read another one's numbers.
pub fn parse_nrrccap(raw: &str, kind: i64) -> Option<Vec<i64>> {
    for line in raw.lines() {
        let Some(rest) = line.trim().strip_prefix("^NRRCCAPQRY:") else {
            continue;
        };
        let mut fields = rest.split(',').map(|f| f.trim());
        let echoed = fields.next()?.parse::<i64>().ok()?;
        if echoed != kind {
            continue;
        }
        let values: Vec<i64> = fields.filter_map(|f| f.parse::<i64>().ok()).collect();
        if values.is_empty() {
            return None;
        }
        return Some(values);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ati_lines() {
        let st = parse_ati("ATI\r\nManufacturer: MT\r\nModel: MT5700M\r\nRevision: v1.2\r\nOK");
        assert_eq!(st.manufacturer.as_deref(), Some("MT"));
        assert_eq!(st.model.as_deref(), Some("MT5700M"));
        assert_eq!(st.revision.as_deref(), Some("v1.2"));
        assert_eq!(parse_imei("860000000000000\r\nOK").as_deref(), Some("860000000000000"));
    }

    #[test]
    fn txpower_tenths_and_invalid_marker() {
        let st = parse_txpower("^TXPOWER: 233,12,999,7,4");
        assert_eq!(st.total, Some(23.3));
        assert_eq!(st.pusch, Some(12));
        assert_eq!(st.pucch, None);
        assert_eq!(st.srs, Some(7));
        assert!(parse_txpower("^TXPOWER: 1,2").is_empty());
    }

    #[test]
    fn nr_txpower_groups_five_fields_per_carrier() {
        let st = parse_nr_txpower("^NTXPOWER: 10,11,12,13,500,20,21,22,23,600");
        assert_eq!(st.carriers.len(), 2);
        assert_eq!(st.carriers[0].freq, Some(500));
        assert_eq!(st.carriers[1].pusch, Some(20));
    }

    #[test]
    fn mcs_groups_three_values_per_carrier() {
        let st = parse_mcs("^MCS: 1,1,0,25,23,1,21,19\n^MCS: 2,0,0,18,17\nOK");
        assert_eq!(st.rat, "NR");
        assert_eq!(st.carriers.len(), 3);
        assert_eq!(st.carriers[0].index, 1);
        assert_eq!(st.carriers[0].mcs_table_index, 0);
        assert_eq!(st.carriers[0].code0, 25);
        assert_eq!(st.carriers[0].code1, 23);
        assert_eq!(st.carriers[2].code0, 18);
        // 25, 21 and 18 -> 64/3 = 21.33 -> 21
        assert_eq!(st.avg_mcs, 21);
    }

    #[test]
    fn mcs_lte_only_when_no_nr_row_seen() {
        let lte = parse_mcs("^MCS: 1,0,0,12,11");
        assert_eq!(lte.rat, "LTE");
        assert_eq!(lte.avg_mcs, 12);
        // An NR row anywhere wins, even after an LTE row.
        let mixed = parse_mcs("^MCS: 1,0,0,12,11\n^MCS: 2,1,0,25,23");
        assert_eq!(mixed.rat, "NR");
        // Unused carriers do not drag the average down.
        let unused = parse_mcs("^MCS: 1,1,0,255,23,1,20,19");
        assert_eq!(unused.avg_mcs, 20);
        assert_eq!(parse_mcs("OK").avg_mcs, 0);
        assert!(parse_mcs("OK").is_empty());
    }

    #[test]
    fn lendc_query_form_drops_enable_prefix() {
        let st = parse_lendc("^LENDC: 1,1,1,0,1");
        assert_eq!(st.available, Some(1));
        assert_eq!(st.plmn_available, Some(1));
        assert_eq!(st.restricted, Some(1)); // third field 0 -> restricted
        assert_eq!(st.established, Some(1));
        // unsolicited form: no prefix
        let st2 = parse_lendc("^LENDC: 1,1,0,1");
        assert_eq!(st2.available, Some(1));
        assert_eq!(st2.restricted, Some(1));
    }

    #[test]
    fn nr_capability_replies_are_matched_by_kind() {
        let raw = "^NRRCCAPQRY: 3,1\n^NRRCCAPQRY: 2,3\n^NRRCCAPQRY: 5,0,1\nOK";
        assert_eq!(parse_nrrccap(raw, 3), Some(vec![1]));
        assert_eq!(parse_nrrccap(raw, 2), Some(vec![3]));
        assert_eq!(parse_nrrccap(raw, 5), Some(vec![0, 1]));
        assert_eq!(parse_nrrccap(raw, 4), None);
        assert_eq!(parse_nrrccap("OK", 3), None);
    }
}
