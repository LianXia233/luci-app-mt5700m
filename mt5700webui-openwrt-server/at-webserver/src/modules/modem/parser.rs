//! Decoders for modem identity, transmit power and EN-DC status.

use crate::modules::modem::state::{EndcState, ModemState, NrCarrier, NrTxPowerState, TxPowerState};
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
}
