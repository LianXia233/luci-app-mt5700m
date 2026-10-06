//! Beam domain model: one SSB measurement and the cells it belongs to.
//!
//! The JSON keys are the ones the Settings page already rendered
//! (`servingCell`/`neighborCells`, `ssbId`/`rsrp`), so the page keeps its
//! markup while the offsets live in the parser.

use crate::core::json::{self, Value};

/// One measured synchronization-signal block (a beam).
#[derive(Debug, Clone, PartialEq)]
pub struct SsbBeam {
    pub ssb_id: i64,
    /// RSRP in dBm as the firmware reports it.
    pub rsrp: i64,
}

impl SsbBeam {
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        m.insert("ssbId".to_string(), json::num_val(self.ssb_id));
        m.insert("rsrp".to_string(), json::num_val(self.rsrp));
        Value::Obj(m)
    }
}

/// The serving NR cell of an `^NRSSBID` report.
///
/// `arfcn`/`cid`/`pci` stay strings because that is what the reply carried and
/// what the page interpolates; the measurement fields are numbers.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SsbServingCell {
    pub arfcn: Option<String>,
    pub cid: Option<String>,
    pub pci: Option<String>,
    /// Band number for `arfcn` (`core::radio`), so no frontend needs an ARFCN
    /// table of its own.
    pub band: Option<i64>,
    pub rsrp: Option<i64>,
    pub sinr: Option<i64>,
    pub ta: Option<i64>,
    pub ssbs: Vec<SsbBeam>,
}

impl SsbServingCell {
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        let mut put = |k: &str, v: Value| {
            m.insert(k.to_string(), v);
        };
        if let Some(v) = &self.arfcn {
            put("arfcn", json::str_val(v));
        }
        if let Some(v) = &self.cid {
            put("cid", json::str_val(v));
        }
        if let Some(v) = &self.pci {
            put("pci", json::str_val(v));
        }
        if let Some(v) = self.band {
            put("band", json::num_val(v));
        }
        if let Some(v) = self.rsrp {
            put("rsrp", json::num_val(v));
        }
        if let Some(v) = self.sinr {
            put("sinr", json::num_val(v));
        }
        if let Some(v) = self.ta {
            put("ta", json::num_val(v));
        }
        let beams: Vec<Value> = self.ssbs.iter().map(|b| b.to_json()).collect();
        m.insert("ssbs".to_string(), Value::Arr(beams));
        Value::Obj(m)
    }
}

/// One neighbour cell of an `^NRSSBID` report.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SsbNeighborCell {
    pub pci: Option<String>,
    pub arfcn: Option<String>,
    /// Band number for `arfcn` (`core::radio`).
    pub band: Option<i64>,
    pub rsrp: Option<i64>,
    pub sinr: Option<i64>,
    pub ssbs: Vec<SsbBeam>,
}

impl SsbNeighborCell {
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        let mut put = |k: &str, v: Value| {
            m.insert(k.to_string(), v);
        };
        if let Some(v) = &self.pci {
            put("pci", json::str_val(v));
        }
        if let Some(v) = &self.arfcn {
            put("arfcn", json::str_val(v));
        }
        if let Some(v) = self.band {
            put("band", json::num_val(v));
        }
        if let Some(v) = self.rsrp {
            put("rsrp", json::num_val(v));
        }
        if let Some(v) = self.sinr {
            put("sinr", json::num_val(v));
        }
        let beams: Vec<Value> = self.ssbs.iter().map(|b| b.to_json()).collect();
        m.insert("ssbs".to_string(), Value::Arr(beams));
        Value::Obj(m)
    }
}

/// A whole `^NRSSBID` report.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SsbState {
    pub serving_cell: Option<SsbServingCell>,
    pub neighbors: Vec<SsbNeighborCell>,
}

impl SsbState {
    pub fn is_empty(&self) -> bool {
        self.serving_cell.is_none() && self.neighbors.is_empty()
    }

    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        m.insert(
            "servingCell".to_string(),
            match &self.serving_cell {
                Some(c) => c.to_json(),
                None => Value::Null,
            },
        );
        let cells: Vec<Value> = self.neighbors.iter().map(|c| c.to_json()).collect();
        m.insert("neighborCells".to_string(), Value::Arr(cells));
        Value::Obj(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssb_json_keeps_the_page_key_names() {
        let st = SsbState {
            serving_cell: Some(SsbServingCell {
                arfcn: Some("636648".into()),
                cid: Some("1A2B3C".into()),
                pci: Some("506".into()),
                band: Some(78),
                rsrp: Some(85),
                sinr: Some(50),
                ta: Some(1),
                ssbs: vec![SsbBeam { ssb_id: 0, rsrp: 90 }],
            }),
            neighbors: vec![SsbNeighborCell {
                pci: Some("506".into()),
                arfcn: Some("632448".into()),
                band: Some(78),
                rsrp: Some(88),
                sinr: Some(45),
                ssbs: vec![SsbBeam { ssb_id: 1, rsrp: 80 }],
            }],
        };
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        assert!(m.contains_key("servingCell"));
        assert!(m.contains_key("neighborCells"));
        let Value::Obj(serving) = m.get("servingCell").unwrap() else {
            panic!("serving")
        };
        assert_eq!(serving.get("arfcn").and_then(|v| v.as_str()), Some("636648"));
        assert_eq!(serving.get("band").and_then(|v| v.as_i64()), Some(78));
        let Value::Arr(beams) = serving.get("ssbs").unwrap() else {
            panic!("beams")
        };
        let Value::Obj(beam) = &beams[0] else { panic!("beam") };
        assert!(beam.contains_key("ssbId"));
        assert!(!st.is_empty());
    }
}
