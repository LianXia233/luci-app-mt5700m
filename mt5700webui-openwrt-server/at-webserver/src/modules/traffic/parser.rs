//! `^PDCPDATAINFO:` decoder — the single implementation of the traffic field
//! table. The URC dispatcher (`transport::urc`) delegates here, and so does the
//! periodic traffic refresh; `api/cli.rs` reads the cached JSON.

use crate::core::json::{self, Value};
use crate::modules::traffic::state::PdcpState;

/// Field map of `^PDCPDATAINFO:`, port of the Python `PDCP_FIELDS` table.
/// The bool flag marks "report in tenths" (value / 10). Query responses
/// (`AT^PDCPDATAINFO?`) append two extra cumulative byte counters after these
/// 14 fields.
pub const PDCP_FIELDS: [(&str, bool); 14] = [
    ("id", false),
    ("pduSessionId", false),
    ("discardTimerLen", false),
    ("avgDelay", true),
    ("minDelay", true),
    ("maxDelay", true),
    ("highPriQueMaxBuffTime", true),
    ("lowPriQueMaxBuffTime", true),
    ("highPriQueBuffPktNums", false),
    ("lowPriQueBuffPktNums", false),
    ("ulPdcpRate", false),
    ("dlPdcpRate", false),
    ("ulDiscardCnt", false),
    ("dlDiscardCnt", false),
];

/// Parse one `^PDCPDATAINFO:` line into the traffic payload.
pub fn parse_pdcp(line: &str) -> Option<PdcpState> {
    let body = line.strip_prefix("^PDCPDATAINFO:")?.trim();
    let parts: Vec<&str> = body.split(',').map(|p| p.trim()).collect();
    if parts.len() < PDCP_FIELDS.len() {
        return None;
    }
    let mut fields = Vec::with_capacity(PDCP_FIELDS.len());
    for (i, (name, tenth)) in PDCP_FIELDS.iter().enumerate() {
        let v: f64 = parts[i].parse().ok()?;
        let value = if *tenth {
            json::num_val(v / 10.0)
        } else {
            json::num_val(v as u64)
        };
        fields.push(((*name).to_string(), value));
    }
    let ul_bytes = parts
        .get(PDCP_FIELDS.len())
        .and_then(|s| s.parse::<u64>().ok());
    let dl_bytes = parts
        .get(PDCP_FIELDS.len() + 1)
        .and_then(|s| s.parse::<u64>().ok());
    Some(PdcpState {
        fields,
        ul_bytes,
        dl_bytes,
    })
}

/// Convenience wrapper for the URC dispatcher: the flat JSON event payload.
pub fn parse_pdcp_value(line: &str) -> Option<Value> {
    parse_pdcp(line).map(|st| {
        let mut m = std::collections::BTreeMap::new();
        for (k, v) in st.fields {
            m.insert(k, v);
        }
        Value::Obj(m)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_fourteen_fields() {
        let st = parse_pdcp("^PDCPDATAINFO: 1,5,65535,0,0,0,70,50,2380,168,512,1024,3,9").unwrap();
        assert_eq!(st.fields.len(), 14);
        // avgDelay is reported in tenths
        assert_eq!(
            st.fields
                .iter()
                .find(|(k, _)| k == "avgDelay")
                .and_then(|(_, v)| v.as_f64()),
            Some(0.0)
        );
        assert!(st.ul_bytes.is_none());
    }

    #[test]
    fn query_form_keeps_cumulative_counters() {
        let st = parse_pdcp(
            "^PDCPDATAINFO: 1,5,65535,0,0,0,0,0,0,0,0,512,0,0,562533856,562533859",
        )
        .unwrap();
        assert_eq!(st.ul_bytes, Some(562533856));
        assert_eq!(st.dl_bytes, Some(562533859));
    }

    #[test]
    fn short_line_is_rejected() {
        assert!(parse_pdcp("^PDCPDATAINFO: 1,5").is_none());
    }
}
