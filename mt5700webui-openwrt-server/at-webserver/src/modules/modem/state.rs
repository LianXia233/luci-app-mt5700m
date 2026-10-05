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

/// One carrier row of an `^MCS` reading: an MCS table index plus the two
/// modulation/coding codes the firmware reports for it.
#[derive(Debug, Clone, PartialEq)]
pub struct McsCarrier {
    /// 1-based position, in the order the modem listed the carriers.
    pub index: usize,
    pub mcs_table_index: i64,
    /// Primary code (`255` = the carrier is not in use).
    pub code0: i64,
    pub code1: i64,
}

impl McsCarrier {
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        m.insert("index".to_string(), json::num_val(self.index));
        m.insert("mcs_table_index".to_string(), json::num_val(self.mcs_table_index));
        m.insert("code0".to_string(), json::num_val(self.code0));
        m.insert("code1".to_string(), json::num_val(self.code1));
        Value::Obj(m)
    }
}

/// One direction's `^MCS` reading (downlink or uplink).
///
/// `rat` is the access technology the modem attributed the table to: `NR`
/// when any row says so, otherwise `LTE` when the first row says so, otherwise
/// `UNKNOWN`. The frontends map the codes to modulation names and colours —
/// that part is presentation, so it stays out of here.
#[derive(Debug, Clone, PartialEq)]
pub struct McsState {
    pub rat: &'static str,
    pub carriers: Vec<McsCarrier>,
    pub avg_mcs: i64,
}

impl Default for McsState {
    fn default() -> Self {
        McsState {
            rat: "UNKNOWN",
            carriers: Vec::new(),
            avg_mcs: 0,
        }
    }
}

impl McsState {
    /// True when the reply carried no carrier row at all.
    pub fn is_empty(&self) -> bool {
        self.carriers.is_empty()
    }

    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        m.insert("rat".to_string(), json::str_val(self.rat));
        let carriers: Vec<Value> = self.carriers.iter().map(|c| c.to_json()).collect();
        m.insert("carriers".to_string(), Value::Arr(carriers));
        m.insert("avg_mcs".to_string(), json::num_val(self.avg_mcs));
        Value::Obj(m)
    }
}

/// NR capability settings the system page's 5G cards render
/// (`^NRRCCAPQRY=3/2/5`).
///
/// The JSON keys keep the page's camelCase spelling (`ca`, `vonr`,
/// `dss.rateMatchingLTE`, `dss.additionalDMRS`); an ability the modem did not
/// answer is omitted, which the page reads as "keep what I show".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NrCapabilityState {
    /// Carrier aggregation.
    pub ca: Option<bool>,
    /// VoNR mode, 0..=3.
    pub vonr: Option<i64>,
    /// DSS LTE-CRS rate matching.
    pub dss_rate_matching_lte: Option<i64>,
    /// DSS additional DMRS.
    pub dss_additional_dmrs: Option<i64>,
}

impl NrCapabilityState {
    pub fn is_empty(&self) -> bool {
        self.ca.is_none()
            && self.vonr.is_none()
            && self.dss_rate_matching_lte.is_none()
            && self.dss_additional_dmrs.is_none()
    }

    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        let mut put = |k: &str, v: Value| {
            m.insert(k.to_string(), v);
        };
        if let Some(ca) = self.ca {
            put("ca", Value::Bool(ca));
        }
        if let Some(vonr) = self.vonr {
            put("vonr", json::num_val(vonr));
        }
        if let (Some(rm), Some(dmrs)) = (self.dss_rate_matching_lte, self.dss_additional_dmrs) {
            let mut dss = std::collections::BTreeMap::new();
            dss.insert("rateMatchingLTE".to_string(), json::num_val(rm));
            dss.insert("additionalDMRS".to_string(), json::num_val(dmrs));
            put("dss", Value::Obj(dss));
        }
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
    fn mcs_json_lists_carriers_and_average() {
        let st = McsState {
            rat: "NR",
            carriers: vec![
                McsCarrier { index: 1, mcs_table_index: 0, code0: 25, code1: 23 },
                McsCarrier { index: 2, mcs_table_index: 1, code0: 255, code1: 21 },
            ],
            avg_mcs: 25,
        };
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        assert_eq!(m.get("rat").and_then(|v| v.as_str()), Some("NR"));
        assert_eq!(m.get("avg_mcs").and_then(|v| v.as_i64()), Some(25));
        assert_eq!(m.get("carriers").map(|v| v.as_arr().map(|a| a.len())), Some(Some(2)));
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

    #[test]
    fn nr_capability_json_uses_the_page_field_names() {
        let st = NrCapabilityState {
            ca: Some(true),
            vonr: Some(3),
            dss_rate_matching_lte: Some(1),
            dss_additional_dmrs: Some(0),
        };
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        assert_eq!(m.get("ca").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(m.get("vonr").and_then(|v| v.as_i64()), Some(3));
        let Some(Value::Obj(dss)) = m.get("dss") else {
            panic!("dss object")
        };
        assert_eq!(dss.get("rateMatchingLTE").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(dss.get("additionalDMRS").and_then(|v| v.as_i64()), Some(0));
        // Half a DSS pair is not reported: the card shows both or neither.
        let partial = NrCapabilityState {
            dss_rate_matching_lte: Some(1),
            ..Default::default()
        };
        let Value::Obj(m) = partial.to_json() else {
            panic!("object")
        };
        assert!(!m.contains_key("dss"));
        assert!(NrCapabilityState::default().is_empty());
    }
}
