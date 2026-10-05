//! Modem domain model: identity, transmit power and EN-DC status.

use crate::core::json::{self, Value};

/// Modem identity (`ATI` + `AT+CGSN`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModemState {
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    pub revision: Option<String>,
    pub imei: Option<String>,
}

impl ModemState {
    pub fn is_empty(&self) -> bool {
        self.manufacturer.is_none()
            && self.model.is_none()
            && self.revision.is_none()
            && self.imei.is_none()
    }

    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        let mut put = |k: &str, v: &Option<String>| {
            if let Some(s) = v {
                if !s.is_empty() {
                    m.insert(k.to_string(), json::str_val(s));
                }
            }
        };
        put("manufacturer", &self.manufacturer);
        put("model", &self.model);
        put("revision", &self.revision);
        put("imei", &self.imei);
        Value::Obj(m)
    }

    pub fn from_json(m: &std::collections::BTreeMap<String, Value>) -> Self {
        let s = |k: &str| m.get(k).and_then(|v| v.as_str()).map(|s| s.to_string());
        ModemState {
            manufacturer: s("manufacturer"),
            model: s("model"),
            revision: s("revision"),
            imei: s("imei"),
        }
    }
}

/// `^TXPOWER?` reading. `total` is in dBm (rounded to 0.1), the per-channel
/// values are the raw units the firmware reports.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TxPowerState {
    pub total: Option<f64>,
    pub pusch: Option<i64>,
    pub pucch: Option<i64>,
    pub srs: Option<i64>,
    pub prach: Option<i64>,
}

impl TxPowerState {
    pub fn is_empty(&self) -> bool {
        self.total.is_none()
            && self.pusch.is_none()
            && self.pucch.is_none()
            && self.srs.is_none()
            && self.prach.is_none()
    }

    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        if let Some(v) = self.total {
            m.insert("total".to_string(), json::num_val(v));
        }
        let mut put = |k: &str, v: Option<i64>| {
            if let Some(v) = v {
                m.insert(k.to_string(), json::num_val(v));
            }
        };
        put("pusch", self.pusch);
        put("pucch", self.pucch);
        put("srs", self.srs);
        put("prach", self.prach);
        Value::Obj(m)
    }

    pub fn from_json(m: &std::collections::BTreeMap<String, Value>) -> Self {
        TxPowerState {
            total: m.get("total").and_then(|v| v.as_f64()),
            pusch: m.get("pusch").and_then(|v| v.as_i64()),
            pucch: m.get("pucch").and_then(|v| v.as_i64()),
            srs: m.get("srs").and_then(|v| v.as_i64()),
            prach: m.get("prach").and_then(|v| v.as_i64()),
        }
    }
}

/// One NR carrier's transmit power (`^NTXPOWER?`, 5 fields per carrier).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NrCarrier {
    pub pusch: Option<i64>,
    pub pucch: Option<i64>,
    pub srs: Option<i64>,
    pub prach: Option<i64>,
    pub freq: Option<i64>,
}

impl NrCarrier {
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        let mut put = |k: &str, v: Option<i64>| {
            if let Some(v) = v {
                m.insert(k.to_string(), json::num_val(v));
            }
        };
        put("pusch", self.pusch);
        put("pucch", self.pucch);
        put("srs", self.srs);
        put("prach", self.prach);
        put("freq", self.freq);
        Value::Obj(m)
    }
}

/// `^NTXPOWER?` reading: up to four carriers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NrTxPowerState {
    pub carriers: Vec<NrCarrier>,
}

impl NrTxPowerState {
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        m.insert(
            "carriers".to_string(),
            Value::Arr(self.carriers.iter().map(|c| c.to_json()).collect()),
        );
        Value::Obj(m)
    }
}

/// `^LENDC` EN-DC status.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EndcState {
    pub available: Option<i64>,
    pub plmn_available: Option<i64>,
    pub restricted: Option<i64>,
    pub established: Option<i64>,
}

impl EndcState {
    pub fn is_empty(&self) -> bool {
        self.available.is_none()
            && self.plmn_available.is_none()
            && self.restricted.is_none()
            && self.established.is_none()
    }

    /// Field names unchanged from the collector: `plmnAvailable`/`restricted`
    /// keep their camelCase spelling, `restricted` is inverted (0 = allowed).
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        let mut put = |k: &str, v: Option<i64>| {
            if let Some(v) = v {
                m.insert(k.to_string(), json::num_val(v));
            }
        };
        put("available", self.available);
        put("plmnAvailable", self.plmn_available);
        put("restricted", self.restricted);
        put("established", self.established);
        Value::Obj(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_json_round_trip() {
        let st = ModemState {
            manufacturer: Some("MT".into()),
            model: Some("MT5700M".into()),
            revision: Some("v1.2".into()),
            imei: None,
        };
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        assert!(!m.contains_key("imei"));
        assert_eq!(ModemState::from_json(&m), st);
    }

    #[test]
    fn endc_json_keeps_legacy_key_names() {
        let st = EndcState {
            available: Some(1),
            plmn_available: Some(0),
            restricted: Some(1),
            established: Some(1),
        };
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        assert!(m.contains_key("plmnAvailable"));
        assert!(m.contains_key("restricted"));
    }
}
