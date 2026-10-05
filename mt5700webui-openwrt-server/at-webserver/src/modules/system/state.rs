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

    /// Domain JSON: `<sensor>: <celsius>` for all 12 plus `average`.
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        for (i, name) in SENSOR_NAMES.iter().enumerate() {
            let v = self.sensors.get(i).copied().unwrap_or(0.0);
            m.insert((*name).to_string(), json::num_val(v));
        }
        if let Some(avg) = self.average {
            m.insert("average".to_string(), json::num_val(avg));
        }
        Value::Obj(m)
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
        let mut peak = 0.0f64;
        let mut peak_key = "";
        let mut found = false;
        for (i, key) in SENSOR_TEXT_KEYS.iter().copied().enumerate() {
            let v = self.sensors.get(i).copied().unwrap_or(0.0);
            if v <= 0.0 || v > 150.0 {
                continue;
            }
            let _ = writeln!(out, "temp_{}={:.1}", key, v);
            if !found || v > peak {
                peak = v;
                peak_key = key;
                found = true;
            }
        }
        if found {
            let _ = writeln!(out, "temperature={:.1}", peak);
            let _ = writeln!(out, "temperature_sensor={}", peak_key);
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
}
