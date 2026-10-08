//! Signal domain model.
//!
//! One struct is the single source of truth for signal values; the JSON form
//! (both frontends) and the legacy `key=value` text form (CLI) are renderings
//! of it, so LuCI, the WebUI and the CLI can never disagree about the same
//! modem reading.

use crate::core::json::{self, Value};

/// Radio access technology reported by `^HCSQ`, normalised to the strings the
/// frontends already match on (`NR` / `LTE` / `WCDMA` / `GSM`).
#[derive(Debug, Clone, PartialEq)]
pub struct SignalState {
    pub sysmode: String,
    pub rssi: Option<i64>,
    pub rsrp: Option<i64>,
    pub rsrq: Option<f64>,
    pub sinr: Option<f64>,
    pub rscp: Option<i64>,
    pub ecio: Option<f64>,
}

impl Default for SignalState {
    fn default() -> Self {
        SignalState {
            sysmode: String::new(),
            rssi: None,
            rsrp: None,
            rsrq: None,
            sinr: None,
            rscp: None,
            ecio: None,
        }
    }
}

impl SignalState {
    /// True when the modem reported no usable field at all.
    pub fn is_empty(&self) -> bool {
        self.sysmode.is_empty()
            && self.rssi.is_none()
            && self.rsrp.is_none()
            && self.rsrq.is_none()
            && self.sinr.is_none()
            && self.rscp.is_none()
            && self.ecio.is_none()
    }

    /// Domain JSON — the wire contract for LuCI and the WebUI.
    ///
    /// Field names are the ones both frontends already consume from the
    /// signal cache topic (`sysmode`, `rsrp`, `rsrq`, `sinr`, `rssi`, `rscp`,
    /// `ecio`); absent fields are omitted rather than sent as null, exactly
    /// like the previous cache writer.
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        if !self.sysmode.is_empty() {
            m.insert("sysmode".to_string(), json::str_val(&self.sysmode));
        }
        if let Some(v) = self.rssi {
            m.insert("rssi".to_string(), json::num_val(v));
        }
        if let Some(v) = self.rsrp {
            m.insert("rsrp".to_string(), json::num_val(v));
        }
        if let Some(v) = self.rsrq {
            m.insert("rsrq".to_string(), json::num_val(v));
        }
        if let Some(v) = self.sinr {
            m.insert("sinr".to_string(), json::num_val(v));
        }
        if let Some(v) = self.rscp {
            m.insert("rscp".to_string(), json::num_val(v));
        }
        if let Some(v) = self.ecio {
            m.insert("ecio".to_string(), json::num_val(v));
        }
        Value::Obj(m)
    }

    /// Legacy CLI text (`key=value` lines) kept byte-compatible with the
    /// historical `mt5700m-at signal` output.
    pub fn to_text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        if self.sysmode.is_empty() {
            return out;
        }
        let _ = writeln!(out, "sysmode={}", self.sysmode);
        if let Some(v) = self.rssi {
            let _ = writeln!(out, "rssi={}", v);
        }
        if let Some(v) = self.rsrp {
            let _ = writeln!(out, "rsrp={}", v);
        }
        if let Some(v) = self.rscp {
            let _ = writeln!(out, "rscp={}", v);
        }
        if let Some(v) = self.sinr {
            let _ = writeln!(out, "sinr={:.1}", v);
        }
        if let Some(v) = self.rsrq {
            let _ = writeln!(out, "rsrq={:.1}", v);
        }
        if let Some(v) = self.ecio {
            let _ = writeln!(out, "ecio={:.1}", v);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_omits_absent_fields() {
        let s = SignalState {
            sysmode: "NR".into(),
            rsrp: Some(-86),
            sinr: Some(-20.0),
            ..Default::default()
        };
        let dump = s.to_json().dump();
        assert!(dump.contains("\"sysmode\":\"NR\""));
        assert!(dump.contains("\"rsrp\":-86"));
        assert!(dump.contains("\"sinr\":-20"));
        assert!(!dump.contains("rsrq"));
    }

    #[test]
    fn text_matches_legacy_shape() {
        let s = SignalState {
            sysmode: "LTE".into(),
            rssi: Some(-51),
            rsrp: Some(-86),
            sinr: Some(-20.0),
            rsrq: Some(-3.0),
            ..Default::default()
        };
        assert_eq!(
            s.to_text(),
            "sysmode=LTE\nrssi=-51\nrsrp=-86\nsinr=-20.0\nrsrq=-3.0\n"
        );
    }
}
