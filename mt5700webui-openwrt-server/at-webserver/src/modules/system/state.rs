//! Chip-temperature domain model.

use crate::core::json::{self, Value};

/// The 12 sensor names the modem returns, in field order.
pub const SENSOR_NAMES: [&str; 12] = [
    "sub3GPA", "sub6GPA", "mimoPa", "tcxo", "peri1", "peri2", "ap1", "ap2", "modem1", "modem2",
    "bbp1", "bbp2",
];

/// Legacy `mt5700m-at temperature` key for each sensor, in [`SENSOR_NAMES`]
/// order. Frozen: `mt5700m-manager` greps these keys out of the CLI text and
/// `scripts/tests` assert them, so they are not derived from the JSON names
/// (`sub3GPA` -> `sub3g_pa`, not `sub3_gpa`).
pub const SENSOR_TEXT_KEYS: [&str; 12] = [
    "sub3g_pa", "sub6g_pa", "mimo_pa", "tcxo", "peri1", "peri2", "ap1", "ap2", "modem1",
    "modem2", "bbp1", "bbp2",
];

/// One `^CHIPTEMP?` reading: every sensor plus the average of the non-zero
/// ones. Unreadable sensors are reported as 0.0 (legacy contract — the UI hides
/// them), and the JSON always carries all 12 fields.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TemperatureState {
    pub sensors: Vec<f64>,
    pub average: Option<f64>,
}

impl TemperatureState {
    /// Value for one named sensor (0.0 when absent/unknown).
    pub fn sensor(&self, name: &str) -> f64 {
        SENSOR_NAMES
            .iter()
            .position(|n| *n == name)
            .and_then(|i| self.sensors.get(i).copied())
            .unwrap_or(0.0)
    }

    /// True when no sensor reported a plausible temperature.
    pub fn is_empty(&self) -> bool {
        self.sensors.iter().all(|v| *v == 0.0)
    }

    /// Domain JSON: `<sensor>: <celsius>` for all 12, plus `average` and the
    /// `peak`/`peak_sensor` pair ([`Self::peak`]) the LuCI page's single
    /// temperature gauge reads — the same value the text form calls
    /// `temperature=`.
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        for (i, name) in SENSOR_NAMES.iter().enumerate() {
            let v = self.sensors.get(i).copied().unwrap_or(0.0);
            m.insert((*name).to_string(), json::num_val(v));
        }
        if let Some(avg) = self.average {
            m.insert("average".to_string(), json::num_val(avg));
        }
        if let Some((peak, key)) = self.peak() {
            m.insert("peak".to_string(), json::num_val(peak));
            m.insert("peak_sensor".to_string(), json::str_val(key));
        }
        Value::Obj(m)
    }

    /// Hottest plausible sensor reading, with its legacy text key.
    ///
    /// The rule the CLI text has always used — ignore a zero (unpopulated) or
    /// implausible (>150 °C) sensor, then take the maximum — and the value the
    /// LuCI page's single `temperature` gauge shows. Defined once so the text
    /// form and the JSON cannot disagree about which sensor is the hot one.
    pub fn peak(&self) -> Option<(f64, &'static str)> {
        let mut peak: Option<(f64, &'static str)> = None;
        for (i, key) in SENSOR_TEXT_KEYS.iter().copied().enumerate() {
            let v = self.sensors.get(i).copied().unwrap_or(0.0);
            if v <= 0.0 || v > 150.0 {
                continue;
            }
            if peak.map(|(p, _)| v > p).unwrap_or(true) {
                peak = Some((v, key));
            }
        }
        peak
    }

    /// The `mt5700m-at temperature` text contract: one `temp_<sensor>=<celsius>`
    /// line per reported sensor, then the hottest one as `temperature=` plus
    /// `temperature_sensor=`. Sensors at 0.0 are "not reported" and omitted.
    ///
    /// This is the ONE renderer of a reading; the CLI's cached path and its live
    /// `AT^CHIPTEMP?` path both end here, so both can never disagree.
    pub fn to_text(&self) -> String {
        use std::fmt::Write as _;
        let mut out = String::new();
        for (i, key) in SENSOR_TEXT_KEYS.iter().copied().enumerate() {
            let v = self.sensors.get(i).copied().unwrap_or(0.0);
            if v <= 0.0 || v > 150.0 {
                continue;
            }
            let _ = writeln!(out, "temp_{}={:.1}", key, v);
        }
        if let Some((peak, key)) = self.peak() {
            let _ = writeln!(out, "temperature={:.1}", peak);
            let _ = writeln!(out, "temperature_sensor={}", key);
        }
        out
    }

    /// Rebuild from cache JSON.
    pub fn from_json(m: &std::collections::BTreeMap<String, Value>) -> Self {
        let sensors = SENSOR_NAMES
            .iter()
            .map(|n| m.get(*n).and_then(|v| v.as_f64()).unwrap_or(0.0))
            .collect();
        TemperatureState {
            sensors,
            average: m.get("average").and_then(|v| v.as_f64()),
        }
    }
}

/// Board switches the system page renders (`^TDPCIELANCFG?`, `^TDPMCFG?`).
///
/// Absent fields mean "the modem did not answer that one yet"; the page keeps
/// whatever it is showing instead of blanking the control.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DeviceControlState {
    /// 1 = RTL8111 (1G), 2 = RTL8125 (2.5G).
    pub nic_rate: Option<i64>,
    pub power_control: Option<bool>,
}

impl DeviceControlState {
    /// True when neither switch was read.
    pub fn is_empty(&self) -> bool {
        self.nic_rate.is_none() && self.power_control.is_none()
    }

    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        let mut put = |k: &str, v: Value| {
            m.insert(k.to_string(), v);
        };
        if let Some(rate) = self.nic_rate {
            put("nic_rate", json::num_val(rate));
        }
        if let Some(on) = self.power_control {
            put("power_control", Value::Bool(on));
        }
        Value::Obj(m)
    }
}

/// Thermal protection settings (`^THERMAUTOFUN?`, `^THERMLDLOGSW?`,
/// `^THERMLDAUTOPARA?`, `^THERMLDAUTOSTATUS?`) — the "温度保护控制" card.
///
/// Field names are the page's (`enabled`, `caMimoSwitch`, `interval`,
/// `logSwitch.consoleLog`/`fileLog`, `thresholds`, `currentLevel`); a query that
/// did not answer leaves its fields absent and the card keeps showing them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThermalState {
    pub enabled: Option<bool>,
    pub ca_mimo_switch: Option<bool>,
    pub interval: Option<i64>,
    pub console_log: Option<bool>,
    pub file_log: Option<bool>,
    pub thresholds: Vec<i64>,
    pub current_level: Option<i64>,
}

impl ThermalState {
    pub fn is_empty(&self) -> bool {
        self.enabled.is_none()
            && self.console_log.is_none()
            && self.thresholds.is_empty()
            && self.current_level.is_none()
    }

    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        let mut put = |k: &str, v: Value| {
            m.insert(k.to_string(), v);
        };
        if let Some(on) = self.enabled {
            put("enabled", Value::Bool(on));
        }
        if let Some(on) = self.ca_mimo_switch {
            put("caMimoSwitch", Value::Bool(on));
        }
        if let Some(v) = self.interval {
            put("interval", json::num_val(v));
        }
        if let (Some(console), Some(file)) = (self.console_log, self.file_log) {
            let mut log = std::collections::BTreeMap::new();
            log.insert("consoleLog".to_string(), Value::Bool(console));
            log.insert("fileLog".to_string(), Value::Bool(file));
            put("logSwitch", Value::Obj(log));
        }
        if !self.thresholds.is_empty() {
            put(
                "thresholds",
                Value::Arr(self.thresholds.iter().map(|v| json::num_val(*v)).collect()),
            );
        }
        if let Some(level) = self.current_level {
            put("currentLevel", json::num_val(level));
        }
        Value::Obj(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_matches_the_cli_contract() {
        let mut st = TemperatureState {
            sensors: vec![0.0; 12],
            average: Some(43.6),
        };
        st.sensors[8] = 42.1; // modem1
        st.sensors[9] = 43.6; // modem2
        assert_eq!(
            st.to_text(),
            "temp_modem1=42.1\ntemp_modem2=43.6\ntemperature=43.6\ntemperature_sensor=modem2\n"
        );
        assert_eq!(TemperatureState::default().to_text(), "");
    }

    #[test]
    fn json_has_all_sensors_and_average() {
        let mut st = TemperatureState {
            sensors: vec![0.0; 12],
            average: Some(42.5),
        };
        st.sensors[3] = 45.0; // tcxo
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        assert_eq!(m.len(), 13);
        assert_eq!(m.get("tcxo").and_then(|v| v.as_f64()), Some(45.0));
        assert_eq!(m.get("average").and_then(|v| v.as_f64()), Some(42.5));
        assert_eq!(TemperatureState::from_json(&m), st);
    }

    #[test]
    fn device_control_omits_unread_switches() {
        let Value::Obj(m) = DeviceControlState::default().to_json() else {
            panic!("object")
        };
        assert!(m.is_empty());
        let st = DeviceControlState {
            nic_rate: Some(2),
            power_control: Some(false),
        };
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        assert_eq!(m.get("nic_rate").and_then(|v| v.as_i64()), Some(2));
        assert_eq!(m.get("power_control").and_then(|v| v.as_bool()), Some(false));
        assert!(!st.is_empty());
    }

    #[test]
    fn thermal_json_uses_the_page_fields() {
        let st = ThermalState {
            enabled: Some(true),
            ca_mimo_switch: Some(false),
            interval: Some(2),
            console_log: Some(true),
            file_log: Some(false),
            thresholds: vec![50, 60, 70],
            current_level: Some(3),
        };
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        assert_eq!(m.get("enabled").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(m.get("caMimoSwitch").and_then(|v| v.as_bool()), Some(false));
        assert_eq!(m.get("interval").and_then(|v| v.as_i64()), Some(2));
        assert_eq!(m.get("currentLevel").and_then(|v| v.as_i64()), Some(3));
        let Some(Value::Obj(log)) = m.get("logSwitch") else {
            panic!("logSwitch object")
        };
        assert_eq!(log.get("consoleLog").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(log.get("fileLog").and_then(|v| v.as_bool()), Some(false));
        let Some(Value::Arr(t)) = m.get("thresholds") else {
            panic!("thresholds array")
        };
        assert_eq!(t.len(), 3);
        // Half a log-switch pair is not reported.
        let half = ThermalState {
            console_log: Some(true),
            ..Default::default()
        };
        let Value::Obj(m) = half.to_json() else {
            panic!("object")
        };
        assert!(!m.contains_key("logSwitch"));
        assert!(ThermalState::default().is_empty());
    }
}
