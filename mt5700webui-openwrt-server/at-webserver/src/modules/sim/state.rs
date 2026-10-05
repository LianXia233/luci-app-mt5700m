//! SIM domain model.

use crate::core::json::{self, Value};

/// SIM card state: readiness, card id and subscriber identity.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SimState {
    /// `+CPIN:` status (`READY`, `SIM PIN`, `SIM PUK`, ...).
    pub status: Option<String>,
    pub iccid: Option<String>,
    pub imsi: Option<String>,
    /// Phone number from `+CNUM` (not part of the periodic snapshot, filled by
    /// the on-demand route).
    pub number: Option<String>,
}

impl SimState {
    /// True when nothing was learned about the card.
    pub fn is_empty(&self) -> bool {
        self.status.is_none() && self.iccid.is_none() && self.imsi.is_none()
    }

    /// Domain JSON for the `sim` topic (absent fields omitted).
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        let mut put = |k: &str, v: &Option<String>| {
            if let Some(s) = v {
                if !s.is_empty() {
                    m.insert(k.to_string(), json::str_val(s));
                }
            }
        };
        put("status", &self.status);
        put("iccid", &self.iccid);
        put("imsi", &self.imsi);
        put("number", &self.number);
        Value::Obj(m)
    }

    /// Rebuild from cache JSON.
    pub fn from_json(m: &std::collections::BTreeMap<String, Value>) -> Self {
        let s = |k: &str| m.get(k).and_then(|v| v.as_str()).map(|s| s.to_string());
        SimState {
            status: s("status"),
            iccid: s("iccid"),
            imsi: s("imsi"),
            number: s("number"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip_keeps_absent_fields_absent() {
        let st = SimState {
            status: Some("READY".into()),
            iccid: Some("89860012345678901234".into()),
            imsi: None,
            number: None,
        };
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        assert!(!m.contains_key("imsi"));
        assert_eq!(SimState::from_json(&m), st);
    }
}
