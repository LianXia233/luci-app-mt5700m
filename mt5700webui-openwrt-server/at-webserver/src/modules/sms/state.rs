//! SMS domain model: one message, the storage/centre settings and the IMS
//! switch — the JSON the SMS pages render.
//!
//! Field names are the pages' (`index`, `content`, `number`, `time`, `type`,
//! `isConcatenated`/`concatenated*`); everything else the pages used to regex
//! out of `+CMGL`/`+CPMS`/`+CSCA`/`^IMSSWITCH` replies is decoded into the
//! structs below, once.

use crate::core::json::{self, Value};

/// One message from the store (or one local sent message).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SmsMessage {
    /// Storage index, `-1` for messages only kept on the page side.
    pub index: i64,
    pub content: String,
    pub number: String,
    /// `YY/MM/DD,HH:MM:SS`, the shape the UI parses and formats.
    pub time: String,
    /// `received` / `sent`.
    pub kind: String,
    pub is_concatenated: bool,
    pub concatenated_ref: Option<u8>,
    pub concatenated_seq: Option<u8>,
    pub concatenated_total: Option<u8>,
}

impl SmsMessage {
    pub fn received(index: i64) -> Self {
        SmsMessage {
            index,
            kind: "received".to_string(),
            ..Default::default()
        }
    }

    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        let mut put = |k: &str, v: Value| {
            m.insert(k.to_string(), v);
        };
        put("index", json::num_val(self.index));
        put("content", json::str_val(&self.content));
        put("number", json::str_val(&self.number));
        put("time", json::str_val(&self.time));
        put("type", json::str_val(&self.kind));
        if self.is_concatenated {
            put("isConcatenated", Value::Bool(true));
            if let Some(v) = self.concatenated_ref {
                put("concatenatedRef", json::num_val(v));
            }
            if let Some(v) = self.concatenated_seq {
                put("concatenatedSeq", json::num_val(v));
            }
            if let Some(v) = self.concatenated_total {
                put("concatenatedTotal", json::num_val(v));
            }
        }
        Value::Obj(m)
    }
}

/// `+CPMS?`: one storage plane (`"SM",3,50`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SmsStorageSlot {
    pub name: String,
    pub used: i64,
    pub total: i64,
}

impl SmsStorageSlot {
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        m.insert("name".to_string(), json::str_val(&self.name));
        m.insert("used".to_string(), json::num_val(self.used));
        m.insert("total".to_string(), json::num_val(self.total));
        Value::Obj(m)
    }
}

/// `+CPMS?` — the three planes the settings card shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SmsStorage {
    pub read: SmsStorageSlot,
    pub write: SmsStorageSlot,
    pub receive: SmsStorageSlot,
}

impl SmsStorage {
    /// True when the modem did not report all three planes.
    pub fn is_empty(&self) -> bool {
        self.read.name.is_empty() && self.write.name.is_empty() && self.receive.name.is_empty()
    }

    /// The distinct storage names in use, in the order the page clears them.
    pub fn names(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for slot in [&self.read, &self.write, &self.receive] {
            if !slot.name.is_empty() && !out.contains(&slot.name) {
                out.push(slot.name.clone());
            }
        }
        out
    }

    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        m.insert("read".to_string(), self.read.to_json());
        m.insert("write".to_string(), self.write.to_json());
        m.insert("receive".to_string(), self.receive.to_json());
        m.insert(
            "storages".to_string(),
            Value::Arr(self.names().iter().map(|n| json::str_val(n)).collect()),
        );
        Value::Obj(m)
    }
}

/// The SMS settings card: IMS switch, service centre, storage.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SmsSettings {
    /// `+CMGF?` answered — the modem is there and message mode is readable.
    /// The pages show "请先前往设置页开启短信" when this is false.
    pub enabled: bool,
    /// `^IMSSWITCH` first field: is SMS over IMS enabled.
    pub ims_on: Option<bool>,
    /// `+CSCA` centre number.
    pub center: Option<String>,
    pub storage: SmsStorage,
}

impl SmsSettings {
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        m.insert("enabled".to_string(), Value::Bool(self.enabled));
        if let Some(on) = self.ims_on {
            m.insert("imsOn".to_string(), Value::Bool(on));
        }
        if let Some(center) = &self.center {
            m.insert("center".to_string(), json::str_val(center));
        }
        if !self.storage.is_empty() {
            m.insert("storage".to_string(), self.storage.to_json());
        }
        Value::Obj(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_json_matches_the_page_contract() {
        let msg = SmsMessage {
            index: 12,
            content: "hello".into(),
            number: "8613800138000".into(),
            time: "26/08/26,14:28:36".into(),
            kind: "received".into(),
            is_concatenated: true,
            concatenated_ref: Some(1),
            concatenated_seq: Some(2),
            concatenated_total: Some(3),
        };
        let Value::Obj(m) = msg.to_json() else {
            panic!("object")
        };
        assert_eq!(m.get("index").and_then(|v| v.as_i64()), Some(12));
        assert_eq!(m.get("type").and_then(|v| v.as_str()), Some("received"));
        assert_eq!(m.get("isConcatenated").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(m.get("concatenatedSeq").and_then(|v| v.as_i64()), Some(2));
        // A single-part message carries no concatenation fields at all.
        let Value::Obj(m) = SmsMessage::received(3).to_json() else {
            panic!("object")
        };
        assert!(!m.contains_key("isConcatenated"));
        assert!(!m.contains_key("concatenatedRef"));
    }

    #[test]
    fn storage_names_are_deduplicated_in_plane_order() {
        let st = SmsStorage {
            read: SmsStorageSlot {
                name: "ME".into(),
                used: 3,
                total: 50,
            },
            write: SmsStorageSlot {
                name: "ME".into(),
                used: 3,
                total: 50,
            },
            receive: SmsStorageSlot {
                name: "SM".into(),
                used: 0,
                total: 30,
            },
        };
        assert_eq!(st.names(), vec!["ME".to_string(), "SM".to_string()]);
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        let Some(Value::Arr(names)) = m.get("storages") else {
            panic!("storages array")
        };
        assert_eq!(names.len(), 2);
        assert!(SmsStorage::default().is_empty());
        assert!(SmsStorage::default().names().is_empty());
    }

    #[test]
    fn settings_omit_what_the_modem_did_not_answer() {
        let Value::Obj(m) = SmsSettings::default().to_json() else {
            panic!("object")
        };
        // `enabled` is the one field that is always present: a cold/unanswered
        // modem must render as "短信未开启", never as a missing value.
        assert_eq!(m.get("enabled").and_then(|v| v.as_bool()), Some(false));
        assert!(!m.contains_key("imsOn"));
        assert!(!m.contains_key("center"));
        assert!(!m.contains_key("storage"));
        let st = SmsSettings {
            enabled: true,
            ims_on: Some(false),
            center: Some("+8613800138000".into()),
            storage: SmsStorage::default(),
        };
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        assert_eq!(m.get("enabled").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(m.get("imsOn").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(m.get("center").and_then(|v| v.as_str()), Some("+8613800138000"));
    }
}
