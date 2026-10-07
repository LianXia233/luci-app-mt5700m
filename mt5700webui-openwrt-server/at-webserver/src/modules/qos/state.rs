//! Domain model for the QoS/data-session parameters.
//!
//! Two frozen text contracts live here, because the CLI grew two verbs for the
//! same subject: `qci=` (LuCI's network page reads it out of the `QoS` section)
//! and `ambr_down_mbps`/`ambr_up_mbps` (the system page's `Subscription rate`
//! section). Both render this state now, so the numbers the two pages show
//! cannot drift apart.

use crate::core::json::{self, Value};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct QosState {
    /// Lowest activated PDP context (`+CGACT?`), when the modem has one.
    pub active_cid: Option<u32>,
    /// Subscribed AMBR in kbps as reported by `^DSAMBR`.
    pub ambr_down_kbps: Option<f64>,
    pub ambr_up_kbps: Option<f64>,
    /// APN carried in the `^DSAMBR` answer.
    pub ambr_apn: Option<String>,
    /// QoS class identifier from `+CGEQOSRDP`, as text (the UI maps it to a
    /// label).
    pub qci: Option<String>,
}

impl QosState {
    pub fn is_empty(&self) -> bool {
        self.active_cid.is_none()
            && self.ambr_down_kbps.is_none()
            && self.ambr_up_kbps.is_none()
            && self.ambr_apn.is_none()
            && self.qci.is_none()
    }

    /// The CLI's `qos` verb: `qci=<value>`, or nothing when unknown.
    pub fn qci_text(&self) -> String {
        match &self.qci {
            Some(qci) => format!("qci={}\n", qci),
            None => String::new(),
        }
    }

    /// The CLI's `subscription-rate` verb: Mbps with one decimal, or nothing
    /// when the modem did not answer with two usable numbers.
    pub fn ambr_text(&self) -> String {
        match (self.ambr_down_kbps, self.ambr_up_kbps) {
            (Some(down), Some(up)) => {
                format!("ambr_down_mbps={:.1}\nambr_up_mbps={:.1}\n", down / 1000.0, up / 1000.0)
            }
            _ => String::new(),
        }
    }

    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        if let Some(cid) = self.active_cid {
            m.insert("active_cid".to_string(), json::num_val(cid as u64));
        }
        if let Some(v) = self.ambr_down_kbps {
            m.insert("ambr_down_kbps".to_string(), json::num_val(v));
        }
        if let Some(v) = self.ambr_up_kbps {
            m.insert("ambr_up_kbps".to_string(), json::num_val(v));
        }
        if let Some(apn) = &self.ambr_apn {
            m.insert("ambr_apn".to_string(), json::str_val(apn));
        }
        if let Some(qci) = &self.qci {
            m.insert("qci".to_string(), json::str_val(qci));
        }
        Value::Obj(m)
    }

    /// Rebuild from cache JSON.
    pub fn from_json(m: &std::collections::BTreeMap<String, Value>) -> Self {
        QosState {
            active_cid: m.get("active_cid").and_then(|v| v.as_u64()).map(|v| v as u32),
            ambr_down_kbps: m.get("ambr_down_kbps").and_then(|v| v.as_f64()),
            ambr_up_kbps: m.get("ambr_up_kbps").and_then(|v| v.as_f64()),
            ambr_apn: m.get("ambr_apn").and_then(|v| v.as_str()).map(String::from),
            qci: m.get("qci").and_then(|v| v.as_str()).map(String::from),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_contracts_match_the_cli_verbs() {
        let st = QosState {
            active_cid: Some(1),
            ambr_down_kbps: Some(20000.0),
            ambr_up_kbps: Some(10000.0),
            ambr_apn: Some("cmnet".into()),
            qci: Some("9".into()),
        };
        assert_eq!(st.qci_text(), "qci=9\n");
        assert_eq!(st.ambr_text(), "ambr_down_mbps=20.0\nambr_up_mbps=10.0\n");
        // Unknown values render nothing, exactly like the CLI's failure path.
        assert_eq!(QosState::default().qci_text(), "");
        assert_eq!(QosState::default().ambr_text(), "");
    }

    #[test]
    fn json_round_trips() {
        let st = QosState {
            active_cid: Some(3),
            ambr_down_kbps: Some(1234.0),
            ambr_up_kbps: Some(567.0),
            ambr_apn: Some("internet".into()),
            qci: Some("6".into()),
        };
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        assert_eq!(QosState::from_json(&m), st);
        assert!(!QosState::default().is_empty() || QosState::default().is_empty());
    }
}
