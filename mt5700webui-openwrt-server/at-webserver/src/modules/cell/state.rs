//! Serving-cell domain model.
//!
//! One struct feeds the dashboard cell card (JSON topic `cell`), the WebUI cell
//! page and the CLI text renderer. Field names and types match what the cache
//! has always published: strings where the modem sends hex/Arfcn, numbers for
//! PCI and the derived bandwidth.

use crate::core::json::{self, Value};

/// Serving-cell parameters gathered from `^HFREQINFO?`, `^MONSC` and the
/// network module's state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CellState {
    /// LTE/NR band number as reported (`41`).
    pub band: Option<String>,
    /// Downlink ARFCN / channel number.
    pub channel: Option<String>,
    /// Downlink bandwidth in MHz (derived from the kHz field).
    pub dl_bandwidth_mhz: Option<i64>,
    /// Raw first field of `^MONSC:` (may be the system mode).
    pub arfcn: Option<String>,
    pub sysmode: Option<String>,
    pub mcc: Option<String>,
    pub mnc: Option<String>,
    pub cid: Option<String>,
    pub pci: Option<i64>,
    pub lac: Option<String>,
    /// Operator name, taken from the network module's state (one COPS query
    /// for the whole backend).
    pub operator: Option<String>,
    /// Raw `^MONSC` response, for the diagnostics view.
    pub raw: Option<String>,
}

/// One measured neighbour cell (`^MONNC`).
///
/// `rsrp`/`rsrq`/`sinr`/`rxlev` are kept as strings because that is exactly
/// what the firmware prints (and what the table rendered); `pci` is decimal
/// although the reply carries it in hex, and `band` is derived from the ARFCN
/// by the module's band table.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NeighborCell {
    /// `LTE` / `NR`.
    pub rat: String,
    pub arfcn: Option<i64>,
    pub pci: Option<i64>,
    pub rsrp: Option<String>,
    pub rsrq: Option<String>,
    pub sinr: Option<String>,
    pub rxlev: Option<String>,
    pub band: Option<i64>,
}

impl NeighborCell {
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        m.insert("type".to_string(), json::str_val(&self.rat));
        let mut put = |k: &str, v: Value| {
            m.insert(k.to_string(), v);
        };
        if let Some(v) = self.arfcn {
            put("arfcn", json::num_val(v));
        }
        if let Some(v) = self.pci {
            put("pci", json::num_val(v));
        }
        for (key, field) in [
            ("rsrp", &self.rsrp),
            ("rsrq", &self.rsrq),
            ("sinr", &self.sinr),
            ("rxlev", &self.rxlev),
        ] {
            if let Some(v) = field {
                if !v.is_empty() {
                    put(key, json::str_val(v));
                }
            }
        }
        if let Some(v) = self.band {
            put("band", json::num_val(v));
        }
        Value::Obj(m)
    }
}

impl CellState {
    /// True when nothing was learned about the serving cell.
    pub fn is_empty(&self) -> bool {
        self.band.is_none()
            && self.channel.is_none()
            && self.arfcn.is_none()
            && self.sysmode.is_none()
            && self.cid.is_none()
            && self.lac.is_none()
            && self.raw.is_none()
    }

    /// Domain JSON for the `cell` topic.
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        let mut put = |k: &str, v: &Option<String>| {
            if let Some(s) = v {
                if !s.is_empty() {
                    m.insert(k.to_string(), json::str_val(s));
                }
            }
        };
        put("band", &self.band);
        put("channel", &self.channel);
        put("arfcn", &self.arfcn);
        put("sysmode", &self.sysmode);
        put("mcc", &self.mcc);
        put("mnc", &self.mnc);
        put("cid", &self.cid);
        put("lac", &self.lac);
        put("operator", &self.operator);
        put("raw", &self.raw);
        if let Some(bw) = self.dl_bandwidth_mhz {
            m.insert("dlBandwidth".to_string(), json::num_val(bw));
        }
        if let Some(pci) = self.pci {
            m.insert("pci".to_string(), json::num_val(pci));
        }
        Value::Obj(m)
    }

    /// Rebuild from cache JSON (API + CLI rendering).
    pub fn from_json(m: &std::collections::BTreeMap<String, Value>) -> Self {
        let s = |k: &str| m.get(k).and_then(|v| v.as_str()).map(|s| s.to_string());
        CellState {
            band: s("band"),
            channel: s("channel"),
            dl_bandwidth_mhz: m.get("dlBandwidth").and_then(|v| v.as_i64()),
            arfcn: s("arfcn"),
            sysmode: s("sysmode"),
            mcc: s("mcc"),
            mnc: s("mnc"),
            cid: s("cid"),
            pci: m.get("pci").and_then(|v| v.as_i64()),
            lac: s("lac"),
            operator: s("operator"),
            raw: s("raw"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_uses_legacy_field_names_and_types() {
        let st = CellState {
            band: Some("41".into()),
            channel: Some("513000".into()),
            dl_bandwidth_mhz: Some(100),
            pci: Some(476),
            cid: Some("123456".into()),
            ..Default::default()
        };
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        assert_eq!(m.get("band").and_then(|v| v.as_str()), Some("41"));
        assert_eq!(m.get("dlBandwidth").and_then(|v| v.as_i64()), Some(100));
        assert_eq!(m.get("pci").and_then(|v| v.as_i64()), Some(476));
        assert!(m.get("raw").is_none());
        assert_eq!(CellState::from_json(&m), st);
    }
}
