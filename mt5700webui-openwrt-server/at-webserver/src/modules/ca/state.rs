//! Domain model for carrier aggregation: the carrier list plus the summary the
//! `mt5700m-at carrier-aggregation` text contract has always printed.
//!
//! `to_text()` is that contract and the only renderer of it — the CLI reads the
//! state through the `ca.get` route and prints `to_text()`, so the shell helper
//! and the WebUI can never disagree about how many carriers are up.

use crate::core::json::{self, Value};

/// Which query reported a carrier. `^HFREQINFO?` reports the serving/aggregated
/// frequency groups, `^CASCELLINFO?` the LTE secondary cells, and they print
/// with slightly different precision, so the renderer keeps the distinction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaSource {
    HfreqInfo,
    LteScell,
}

impl CaSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            CaSource::HfreqInfo => "hfreqinfo",
            CaSource::LteScell => "lte_scell",
        }
    }

    pub fn from_str(s: &str) -> CaSource {
        match s {
            "lte_scell" => CaSource::LteScell,
            _ => CaSource::HfreqInfo,
        }
    }
}

/// One aggregated carrier.
#[derive(Debug, Clone, PartialEq)]
pub struct CaCarrier {
    /// `"LTE"` or `"NR"`.
    pub radio: String,
    /// Band in display form, already prefixed: `n41`, `B3`.
    pub band: String,
    pub dl_arfcn: String,
    pub ul_arfcn: String,
    pub dl_frequency_mhz: f64,
    pub ul_frequency_mhz: f64,
    pub dl_bandwidth_mhz: f64,
    pub ul_bandwidth_mhz: f64,
    pub source: CaSource,
}

/// One `^MONSSC` NR secondary cell (manual 13.27).
///
/// The `^HFREQINFO` carrier list only carries frequency and bandwidth; the
/// per-carrier signal quality lives here, and the info page merges the two by
/// downlink ARFCN.
#[derive(Debug, Clone, PartialEq)]
pub struct SecondaryNr {
    pub arfcn: i64,
    /// 手册 13.27.3: `<PCI>` is hexadecimal.
    pub pci: i64,
    pub rsrp: Option<f64>,
    pub rsrq: Option<f64>,
    pub sinr: Option<f64>,
    /// 手册 13.27.3 `<MEASTYPE>`: `SSB` / `CSI-RS`, the manual's `—` otherwise.
    pub meas_type: String,
}

/// One `^CASCELLINFO` LTE secondary cell (manual 13.18).
#[derive(Debug, Clone, PartialEq)]
pub struct SecondaryLte {
    pub index: i64,
    pub pci: i64,
    pub band: i64,
    pub rssi: Option<f64>,
    pub rsrp: Option<f64>,
    pub rsrq: Option<f64>,
    pub ul_arfcn: Option<i64>,
    pub dl_arfcn: Option<i64>,
    pub ul_frequency_mhz: Option<f64>,
    pub dl_frequency_mhz: Option<f64>,
    pub ul_bandwidth_mhz: Option<f64>,
    pub dl_bandwidth_mhz: Option<f64>,
}

/// A secondary cell of either radio, tagged so the JSON is self-describing.
#[derive(Debug, Clone, PartialEq)]
pub enum SecondaryCell {
    Nr(SecondaryNr),
    Lte(SecondaryLte),
}

impl SecondaryCell {
    /// Domain JSON: the `radio` tag plus the fields the info page renders.
    /// Absent fields are omitted (the WebUI treats them as “no reading”).
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        match self {
            SecondaryCell::Nr(nr) => {
                m.insert("radio".to_string(), json::str_val("NR"));
                m.insert("arfcn".to_string(), json::num_val(nr.arfcn));
                m.insert("pci".to_string(), json::num_val(nr.pci));
                for (key, v) in [
                    ("rsrp", &nr.rsrp),
                    ("rsrq", &nr.rsrq),
                    ("sinr", &nr.sinr),
                ] {
                    if let Some(v) = v {
                        m.insert(key.to_string(), json::num_val(*v));
                    }
                }
                m.insert("measType".to_string(), json::str_val(&nr.meas_type));
            }
            SecondaryCell::Lte(lte) => {
                m.insert("radio".to_string(), json::str_val("LTE"));
                m.insert("index".to_string(), json::num_val(lte.index));
                m.insert("pci".to_string(), json::num_val(lte.pci));
                m.insert("band".to_string(), json::num_val(lte.band));
                for (key, v) in [
                    ("rssi", &lte.rssi),
                    ("rsrp", &lte.rsrp),
                    ("rsrq", &lte.rsrq),
                ] {
                    if let Some(v) = v {
                        m.insert(key.to_string(), json::num_val(*v));
                    }
                }
                if let Some(v) = lte.ul_arfcn {
                    m.insert("ulArfcn".to_string(), json::num_val(v));
                }
                if let Some(v) = lte.dl_arfcn {
                    m.insert("dlArfcn".to_string(), json::num_val(v));
                }
                for (key, v) in [
                    ("ulFreq", &lte.ul_frequency_mhz),
                    ("dlFreq", &lte.dl_frequency_mhz),
                    ("ulBandwidth", &lte.ul_bandwidth_mhz),
                    ("dlBandwidth", &lte.dl_bandwidth_mhz),
                ] {
                    if let Some(v) = v {
                        m.insert(key.to_string(), json::num_val(*v));
                    }
                }
            }
        }
        Value::Obj(m)
    }

    /// Inverse of [`Self::to_json`] for the cache round trip.
    pub fn from_json(v: &Value) -> Option<Self> {
        let Value::Obj(m) = v else { return None };
        let s = |k: &str| m.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let n = |k: &str| m.get(k).and_then(|v| v.as_i64());
        let f = |k: &str| m.get(k).and_then(|v| v.as_f64());
        match s("radio").as_str() {
            "NR" => Some(SecondaryCell::Nr(SecondaryNr {
                arfcn: n("arfcn")?,
                pci: n("pci")?,
                rsrp: f("rsrp"),
                rsrq: f("rsrq"),
                sinr: f("sinr"),
                meas_type: s("measType"),
            })),
            "LTE" => Some(SecondaryCell::Lte(SecondaryLte {
                index: n("index")?,
                pci: n("pci")?,
                band: n("band")?,
                rssi: f("rssi"),
                rsrp: f("rsrp"),
                rsrq: f("rsrq"),
                ul_arfcn: n("ulArfcn"),
                dl_arfcn: n("dlArfcn"),
                ul_frequency_mhz: f("ulFreq"),
                dl_frequency_mhz: f("dlFreq"),
                ul_bandwidth_mhz: f("ulBandwidth"),
                dl_bandwidth_mhz: f("dlBandwidth"),
            })),
            _ => None,
        }
    }
}

/// Everything known about the current carrier aggregation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CaState {
    pub carriers: Vec<CaCarrier>,
    /// `^MONSSC`/`^CASCELLINFO` reported secondary cells, signal quality and all.
    pub secondary: Vec<SecondaryCell>,
    /// `^MONSSC` reported NSA secondary cells (count of `NR,` lines).
    pub secondary_connection_count: usize,
}

impl CaState {
    pub fn nr_carriers(&self) -> usize {
        self.carriers.iter().filter(|c| c.radio == "NR").count()
    }

    pub fn lte_carriers(&self) -> usize {
        self.carriers.iter().filter(|c| c.radio == "LTE").count()
    }

    pub fn lte_scells(&self) -> usize {
        self.carriers
            .iter()
            .filter(|c| c.source == CaSource::LteScell)
            .count()
    }

    pub fn dl_bandwidth_mhz(&self) -> f64 {
        self.carriers.iter().map(|c| c.dl_bandwidth_mhz).sum()
    }

    pub fn ul_bandwidth_mhz(&self) -> f64 {
        self.carriers.iter().map(|c| c.ul_bandwidth_mhz).sum()
    }

    /// EN-DC: NR and LTE carriers are both up, or the NSA secondary-cell query
    /// confirmed an NR leg.
    pub fn dc_active(&self) -> bool {
        (self.nr_carriers() > 0 && self.lte_carriers() > 0)
            || self.secondary_connection_count > 0
    }

    /// Carrier aggregation proper: more than one NR carrier, or an LTE SCell.
    pub fn ca_active(&self) -> bool {
        self.nr_carriers() > 1 || self.lte_scells() > 0
    }

    pub fn mode(&self) -> &'static str {
        if self.dc_active() {
            if self.ca_active() {
                "EN-DC + CA"
            } else {
                "EN-DC"
            }
        } else if self.nr_carriers() > 0 {
            if self.nr_carriers() > 1 {
                "NR-CA"
            } else {
                "NR"
            }
        } else if self.lte_carriers() > 1 {
            "LTE-CA"
        } else {
            "LTE"
        }
    }

    pub fn is_empty(&self) -> bool {
        self.carriers.is_empty()
    }

    /// The `mt5700m-at carrier-aggregation` text contract. Frozen: LuCI's
    /// carrier panel renders these keys, and `mt5700m-manager` forwards them as
    /// the shell-side `carrier_*`/`ca_*` contract.
    pub fn to_text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        if self.carriers.is_empty() {
            return out;
        }
        for (i, c) in self.carriers.iter().enumerate() {
            match c.source {
                CaSource::HfreqInfo => {
                    let _ = writeln!(
                        out,
                        "carrier_{}={}|{}|{}|{:.2}|{:.1}|{}|{:.2}|{:.1}",
                        i + 1,
                        c.radio,
                        c.band,
                        c.dl_arfcn,
                        c.dl_frequency_mhz,
                        c.dl_bandwidth_mhz,
                        c.ul_arfcn,
                        c.ul_frequency_mhz,
                        c.ul_bandwidth_mhz
                    );
                }
                CaSource::LteScell => {
                    let _ = writeln!(
                        out,
                        "carrier_{}={}|{}|{}|{:.1}|{:.1}|{}|{:.1}|{:.1}",
                        i + 1,
                        c.radio,
                        c.band,
                        c.dl_arfcn,
                        c.dl_frequency_mhz,
                        c.dl_bandwidth_mhz,
                        c.ul_arfcn,
                        c.ul_frequency_mhz,
                        c.ul_bandwidth_mhz
                    );
                }
            }
        }
        let _ = writeln!(out, "carrier_count={}", self.carriers.len());
        let _ = writeln!(out, "ca_active={}", self.ca_active() as i32);
        let _ = writeln!(out, "dc_active={}", self.dc_active() as i32);
        let _ = writeln!(out, "nr_carrier_count={}", self.nr_carriers());
        let _ = writeln!(out, "lte_carrier_count={}", self.lte_carriers());
        let _ = writeln!(out, "lte_secondary_count={}", self.lte_scells());
        let _ = writeln!(
            out,
            "secondary_connection_count={}",
            self.secondary_connection_count
        );
        let _ = writeln!(out, "ca_mode={}", self.mode());
        let _ = writeln!(out, "ca_dl_bandwidth={:.1}", self.dl_bandwidth_mhz());
        let _ = writeln!(out, "ca_ul_bandwidth={:.1}", self.ul_bandwidth_mhz());
        out
    }

    /// Domain JSON for `ca.get` / `ca.cached`: the carrier array plus the same
    /// summary keys the text contract uses (booleans stay booleans here).
    pub fn to_json(&self) -> Value {
        let carriers = self
            .carriers
            .iter()
            .map(|c| {
                let mut m = std::collections::BTreeMap::new();
                m.insert("radio".to_string(), json::str_val(&c.radio));
                m.insert("band".to_string(), json::str_val(&c.band));
                m.insert("source".to_string(), json::str_val(c.source.as_str()));
                m.insert("dl_arfcn".to_string(), json::str_val(&c.dl_arfcn));
                m.insert("ul_arfcn".to_string(), json::str_val(&c.ul_arfcn));
                m.insert(
                    "dl_frequency_mhz".to_string(),
                    json::num_val(c.dl_frequency_mhz),
                );
                m.insert(
                    "ul_frequency_mhz".to_string(),
                    json::num_val(c.ul_frequency_mhz),
                );
                m.insert(
                    "dl_bandwidth_mhz".to_string(),
                    json::num_val(c.dl_bandwidth_mhz),
                );
                m.insert(
                    "ul_bandwidth_mhz".to_string(),
                    json::num_val(c.ul_bandwidth_mhz),
                );
                Value::Obj(m)
            })
            .collect();
        let secondary = self.secondary.iter().map(|c| c.to_json()).collect();
        let mut m = std::collections::BTreeMap::new();
        m.insert("carriers".to_string(), Value::Arr(carriers));
        m.insert("secondary".to_string(), Value::Arr(secondary));
        m.insert(
            "carrier_count".to_string(),
            json::num_val(self.carriers.len() as u64),
        );
        m.insert("ca_active".to_string(), Value::Bool(self.ca_active()));
        m.insert("dc_active".to_string(), Value::Bool(self.dc_active()));
        m.insert(
            "nr_carrier_count".to_string(),
            json::num_val(self.nr_carriers() as u64),
        );
        m.insert(
            "lte_carrier_count".to_string(),
            json::num_val(self.lte_carriers() as u64),
        );
        m.insert(
            "lte_secondary_count".to_string(),
            json::num_val(self.lte_scells() as u64),
        );
        m.insert(
            "secondary_connection_count".to_string(),
            json::num_val(self.secondary_connection_count as u64),
        );
        m.insert("ca_mode".to_string(), json::str_val(self.mode()));
        m.insert(
            "ca_dl_bandwidth".to_string(),
            json::num_val(self.dl_bandwidth_mhz()),
        );
        m.insert(
            "ca_ul_bandwidth".to_string(),
            json::num_val(self.ul_bandwidth_mhz()),
        );
        Value::Obj(m)
    }

    /// Rebuild from cache JSON (the `ca.cached` / CLI path).
    pub fn from_json(m: &std::collections::BTreeMap<String, Value>) -> Self {
        let carriers = match m.get("carriers") {
            Some(Value::Arr(items)) => items
                .iter()
                .filter_map(|v| match v {
                    Value::Obj(c) => {
                        let s = |k: &str| {
                            c.get(k)
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string()
                        };
                        let n = |k: &str| c.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
                        Some(CaCarrier {
                            radio: s("radio"),
                            band: s("band"),
                            dl_arfcn: s("dl_arfcn"),
                            ul_arfcn: s("ul_arfcn"),
                            dl_frequency_mhz: n("dl_frequency_mhz"),
                            ul_frequency_mhz: n("ul_frequency_mhz"),
                            dl_bandwidth_mhz: n("dl_bandwidth_mhz"),
                            ul_bandwidth_mhz: n("ul_bandwidth_mhz"),
                            source: CaSource::from_str(&s("source")),
                        })
                    }
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        let secondary = match m.get("secondary") {
            Some(Value::Arr(items)) => items.iter().filter_map(SecondaryCell::from_json).collect(),
            _ => Vec::new(),
        };
        CaState {
            carriers,
            secondary,
            secondary_connection_count: m
                .get("secondary_connection_count")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hfreq_nr() -> CaCarrier {
        CaCarrier {
            radio: "NR".into(),
            band: "n41".into(),
            dl_arfcn: "513000".into(),
            ul_arfcn: "513000".into(),
            dl_frequency_mhz: 2565.0,
            ul_frequency_mhz: 2565.0,
            dl_bandwidth_mhz: 100.0,
            ul_bandwidth_mhz: 100.0,
            source: CaSource::HfreqInfo,
        }
    }

    fn scell_lte() -> CaCarrier {
        CaCarrier {
            radio: "LTE".into(),
            band: "B3".into(),
            dl_arfcn: "1650".into(),
            ul_arfcn: "1850".into(),
            dl_frequency_mhz: 1840.0,
            ul_frequency_mhz: 1745.0,
            dl_bandwidth_mhz: 20.0,
            ul_bandwidth_mhz: 20.0,
            source: CaSource::LteScell,
        }
    }

    #[test]
    fn text_matches_the_frozen_cli_contract() {
        let mut st = CaState {
            carriers: vec![hfreq_nr(), scell_lte()],
            secondary: Vec::new(),
            secondary_connection_count: 1,
        };
        assert_eq!(
            st.to_text(),
            "carrier_1=NR|n41|513000|2565.00|100.0|513000|2565.00|100.0\n\
             carrier_2=LTE|B3|1650|1840.0|20.0|1850|1745.0|20.0\n\
             carrier_count=2\n\
             ca_active=1\n\
             dc_active=1\n\
             nr_carrier_count=1\n\
             lte_carrier_count=1\n\
             lte_secondary_count=1\n\
             secondary_connection_count=1\n\
             ca_mode=EN-DC + CA\n\
             ca_dl_bandwidth=120.0\n\
             ca_ul_bandwidth=120.0\n"
        );
        assert_eq!(CaState::default().to_text(), "");
        st.secondary_connection_count = 0;
        assert_eq!(st.mode(), "EN-DC + CA"); // still CA (an LTE SCell is up)
    }

    #[test]
    fn single_nr_carrier_is_plain_nr() {
        let st = CaState {
            carriers: vec![hfreq_nr()],
            secondary: Vec::new(),
            secondary_connection_count: 0,
        };
        assert!(!st.ca_active() && !st.dc_active());
        assert_eq!(st.mode(), "NR");
    }

    #[test]
    fn json_round_trips() {
        let st = CaState {
            carriers: vec![hfreq_nr(), scell_lte()],
            secondary: Vec::new(),
            secondary_connection_count: 2,
        };
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        assert_eq!(m.get("carrier_count").and_then(|v| v.as_u64()), Some(2));
        assert_eq!(CaState::from_json(&m), st);
    }
}
