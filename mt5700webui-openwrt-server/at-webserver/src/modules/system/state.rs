//! Chip-temperature domain model.

use crate::core::json::{self, Value};

/// The 12 sensor names the modem returns, in field order.
pub const SENSOR_NAMES: [&str; 12] = [
    "sub3GPA", "sub6GPA", "mimoPa", "tcxo", "peri1", "peri2", "ap1", "ap2", "modem1", "modem2",
    "bbp1", "bbp2",
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
