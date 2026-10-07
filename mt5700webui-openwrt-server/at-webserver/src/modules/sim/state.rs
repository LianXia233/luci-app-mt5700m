//! SIM domain model.
//!
//! `SimState` is the periodic snapshot (card status, identity, active slot,
//! hot-plug switch); `SimPinState` is the on-demand PIN/card answer the system
//! page and the PIN dialog need. Both are the single source of truth for the
//! frontends — the JSON keys are the ones the pages always rendered.

use crate::core::json::{self, Value};

/// SIM card state: readiness, card id, subscriber identity, slot wiring.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SimState {
    /// `+CPIN:` status (`READY`, `SIM PIN`, `SIM PUK`, ...).
    pub status: Option<String>,
    pub iccid: Option<String>,
    pub imsi: Option<String>,
    /// Phone number from `+CNUM` (not part of the periodic snapshot, filled by
    /// the on-demand route).
    pub number: Option<String>,
    /// `not_stored` when `+CNUM` answered `+CME ERROR: 22` — the card exists but
    /// carries no MSISDN. The pages show their own copy for it; only the module
    /// can tell it apart from a failed read.
    pub number_state: Option<String>,
    /// Active slot (`^SCICHG`, 0 = external, 1 = internal).
    pub slot: Option<i64>,
    /// Hot-plug detection switch (`^TDSIMHP`).
    pub hotplug: Option<bool>,
}

impl SimState {
    /// True when nothing was learned about the card.
    pub fn is_empty(&self) -> bool {
        self.status.is_none() && self.iccid.is_none() && self.imsi.is_none()
    }

    /// Domain JSON for the `sim` topic (absent fields omitted).
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        let mut put = |k: &str, v: Value| {
            m.insert(k.to_string(), v);
        };
        for (key, value) in [
            ("status", &self.status),
            ("iccid", &self.iccid),
            ("imsi", &self.imsi),
            ("number", &self.number),
        ] {
            if let Some(s) = value {
                if !s.is_empty() {
                    put(key, json::str_val(s));
                }
            }
        }
        if let Some(state) = &self.number_state {
            put("numberState", json::str_val(state));
        }
        if let Some(v) = self.slot {
            put("slot", json::num_val(v));
        }
        if let Some(v) = self.hotplug {
            put("hotplug", json::bool_val(v));
        }
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
            number_state: s("numberState"),
            slot: m.get("slot").and_then(|v| v.as_i64()),
            hotplug: m.get("hotplug").and_then(|v| v.as_bool()),
        }
    }
}

/// On-demand PIN/card answer (`+CPIN?` + `^SIMSQ?` + `+CLCK="SC",2`).
///
/// `code`/`lock`/`blocked`/`needs_new_pin` are the semantics the UI branches
/// on; the display text for a code stays in the frontend.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SimPinState {
    /// `+CPIN` code, `ABSENT` when no card is in the slot.
    pub code: Option<String>,
    /// `ready`/`pin`/`puk`/`pin2`/`puk2`/`network`/`absent`/`unknown`.
    pub lock: Option<String>,
    pub blocked: bool,
    pub needs_new_pin: bool,
    /// `^SIMSQ` status code.
    pub card_status: Option<i64>,
    pub dead: bool,
    pub present: bool,
    /// `+CLCK="SC",2` — whether the PIN lock is enabled. `None` when the card is
    /// not ready (the query is meaningless then, and the modem rejects it).
    pub pin_enabled: Option<bool>,
}

impl SimPinState {
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        let mut put = |k: &str, v: Value| {
            m.insert(k.to_string(), v);
        };
        if let Some(code) = &self.code {
            put("code", json::str_val(code));
        }
        if let Some(lock) = &self.lock {
            put("lock", json::str_val(lock));
        }
        put("blocked", Value::Bool(self.blocked));
        put("needsNewPin", Value::Bool(self.needs_new_pin));
        if let Some(status) = self.card_status {
            let mut card = std::collections::BTreeMap::new();
            card.insert("status".to_string(), json::num_val(status));
            card.insert("dead".to_string(), Value::Bool(self.dead));
            card.insert("present".to_string(), Value::Bool(self.present));
            put("card", Value::Obj(card));
        }
        if let Some(enabled) = self.pin_enabled {
            put("pinEnabled", Value::Bool(enabled));
        }
        Value::Obj(m)
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
            number_state: None,
            slot: None,
            hotplug: None,
        };
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        assert!(!m.contains_key("imsi"));
        assert!(!m.contains_key("numberState"));
        assert_eq!(SimState::from_json(&m), st);
    }

    /// The "card has no MSISDN stored" state must survive the topic round trip:
    /// the page renders its own copy for it, so dropping it would turn "Not
    /// stored" into a blank row.
    #[test]
    fn number_state_round_trips_as_camel_case() {
        let st = SimState {
            number_state: Some("not_stored".into()),
            ..Default::default()
        };
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        assert_eq!(m.get("numberState").and_then(|v| v.as_str()), Some("not_stored"));
        assert_eq!(SimState::from_json(&m), st);
    }

    #[test]
    fn slot_and_hotplug_survive_the_round_trip() {
        let st = SimState {
            slot: Some(1),
            hotplug: Some(true),
            ..Default::default()
        };
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        assert_eq!(m.get("slot").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(m.get("hotplug").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(SimState::from_json(&m), st);
    }

    #[test]
    fn pin_state_json_matches_the_page_contract() {
        let st = SimPinState {
            code: Some("SIM PUK".into()),
            lock: Some("puk".into()),
            blocked: true,
            needs_new_pin: true,
            card_status: Some(2),
            dead: false,
            present: true,
            pin_enabled: Some(false),
        };
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        assert_eq!(m.get("code").and_then(|v| v.as_str()), Some("SIM PUK"));
        assert_eq!(m.get("lock").and_then(|v| v.as_str()), Some("puk"));
        assert_eq!(m.get("blocked").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(m.get("needsNewPin").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(m.get("pinEnabled").and_then(|v| v.as_bool()), Some(false));
        let Some(Value::Obj(card)) = m.get("card") else {
            panic!("card object")
        };
        assert_eq!(card.get("status").and_then(|v| v.as_i64()), Some(2));
        assert_eq!(card.get("dead").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(card.get("present").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn pin_state_omits_unknown_card_and_lock_fields() {
        let Value::Obj(m) = SimPinState::default().to_json() else {
            panic!("object")
        };
        assert!(!m.contains_key("code"));
        assert!(!m.contains_key("lock"));
        assert!(!m.contains_key("card"));
        assert!(!m.contains_key("pinEnabled"));
        assert_eq!(m.get("blocked").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(m.get("needsNewPin").and_then(|v| v.as_bool()), Some(false));
    }
}
