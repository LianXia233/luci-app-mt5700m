//! Traffic domain model: PDCP counters and interface rate counters.

use crate::core::json::{self, Value};

/// One `^PDCPDATAINFO:` snapshot (14 fields; `HighPriQueBuffPktNums` style
/// names preserved for the frontends).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PdcpState {
    pub fields: Vec<(String, Value)>,
    /// Cumulative UL/DL byte counters appended by the query response.
    pub ul_bytes: Option<u64>,
    pub dl_bytes: Option<u64>,
}

impl PdcpState {
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Domain JSON for the `traffic` topic: the flat 14-field map, exactly the
    /// shape the cache has always published (the cumulative byte counters are
    /// kept in the struct for the diagnostics route but deliberately not mixed
    /// into the topic, so both frontends keep reading the same object).
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        for (k, v) in &self.fields {
            m.insert(k.clone(), v.clone());
        }
        Value::Obj(m)
    }

    /// Diagnostics view: topic fields plus the cumulative counters.
    pub fn to_json_with_counters(&self) -> Value {
        let Value::Obj(mut m) = self.to_json() else {
            unreachable!()
        };
        if let Some(v) = self.ul_bytes {
            m.insert("ulBytes".to_string(), json::num_val(v));
        }
        if let Some(v) = self.dl_bytes {
            m.insert("dlBytes".to_string(), json::num_val(v));
        }
        Value::Obj(m)
    }
}

/// Interface byte counters plus the shared accounting report.
///
/// `traffic` is the parsed output of `mt5700m-traffic json` — the single
/// accounting writer on the box — carried through verbatim so the frontends
/// read the same numbers as the LuCI overview page.
#[derive(Debug, Clone, PartialEq)]
pub struct NetRateState {
    pub available: bool,
    pub device: Option<String>,
    pub reason: Option<String>,
    pub rx_bytes: Option<u64>,
    pub tx_bytes: Option<u64>,
    pub timestamp: f64,
    /// `mt5700m-traffic` provenance: `mt5700m-traffic` or `unavailable`.
    pub source: &'static str,
    pub traffic: Option<Value>,
}

impl Default for NetRateState {
    fn default() -> Self {
        NetRateState {
            available: false,
            device: None,
            reason: None,
            rx_bytes: None,
            tx_bytes: None,
            timestamp: 0.0,
            source: "unavailable",
            traffic: None,
        }
    }
}

impl NetRateState {
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        m.insert("available".to_string(), json::bool_val(self.available));
        if let Some(d) = &self.device {
            m.insert("device".to_string(), json::str_val(d));
        }
        if let Some(r) = &self.reason {
            m.insert("reason".to_string(), json::str_val(r));
        }
        m.insert(
            "rx_bytes".to_string(),
            self.rx_bytes.map(|v| json::num_val(v)).unwrap_or(Value::Null),
        );
        m.insert(
            "tx_bytes".to_string(),
            self.tx_bytes.map(|v| json::num_val(v)).unwrap_or(Value::Null),
        );
        m.insert("timestamp".to_string(), json::num_val(self.timestamp));
        m.insert("source".to_string(), json::str_val(self.source));
        if let Some(t) = &self.traffic {
            m.insert("traffic".to_string(), t.clone());
        }
        Value::Obj(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn netrate_json_matches_legacy_shape() {
        let st = NetRateState {
            available: true,
            device: Some("eth2".into()),
            rx_bytes: Some(1000),
            tx_bytes: Some(2000),
            timestamp: 1.5,
            source: "mt5700m-traffic",
            ..Default::default()
        };
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        assert_eq!(m.get("device").and_then(|v| v.as_str()), Some("eth2"));
        assert_eq!(m.get("source").and_then(|v| v.as_str()), Some("mt5700m-traffic"));
        assert!(!m.contains_key("reason"));
    }

    #[test]
    fn traffic_topic_stays_a_flat_field_map() {
        let st = PdcpState {
            fields: vec![("id".into(), json::num_val(1))],
            ul_bytes: Some(9),
            dl_bytes: Some(8),
        };
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        assert_eq!(m.get("id").and_then(|v| v.as_u64()), Some(1));
        assert!(!m.contains_key("ulBytes"));
        let Value::Obj(d) = st.to_json_with_counters() else {
            panic!("object")
        };
        assert_eq!(d.get("ulBytes").and_then(|v| v.as_u64()), Some(9));
    }
}
