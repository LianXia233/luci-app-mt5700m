//! `^CHIPTEMP?` decoder.

use crate::modules::system::state::{TemperatureState, SENSOR_NAMES};
use crate::state::refresh::round1;

/// Parse `^CHIPTEMP: t0..t11` (tenths of degrees) into the 12 named sensors
/// plus a rounded average of the plausible ones.
///
/// `65535`/`>1500` mean "sensor absent" and become 0.0, exactly as the
/// collector did: the UI treats 0 as "not reported".
pub fn parse_chiptemp(raw: &str) -> TemperatureState {
    let Some(body) = raw.lines().find_map(|l| l.trim().strip_prefix("^CHIPTEMP:")) else {
        return TemperatureState::default();
    };
    let fields: Vec<&str> = body.split(',').map(|f| f.trim()).collect();
    let mut sensors = Vec::with_capacity(SENSOR_NAMES.len());
    let mut sum = 0.0;
    let mut count = 0usize;
    for i in 0..SENSOR_NAMES.len() {
        let raw_v = fields.get(i).and_then(|f| f.parse::<u64>().ok()).unwrap_or(0);
        let v = if raw_v >= 65535 || raw_v > 1500 {
            0.0
        } else {
            raw_v as f64 / 10.0
        };
        if v > 0.0 {
            sum += v;
            count += 1;
        }
        sensors.push(v);
    }
    TemperatureState {
        sensors,
        average: if count > 0 {
            Some(round1(sum / count as f64))
        } else {
            None
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tenths_and_averages_nonzero() {
        let st = parse_chiptemp("^CHIPTEMP: 300,0,65535,450,400,0,0,0,0,0,0,0");
        assert_eq!(st.sensor("sub3GPA"), 30.0);
        assert_eq!(st.sensor("mimoPa"), 0.0);
        assert_eq!(st.sensor("tcxo"), 45.0);
        assert_eq!(st.sensor("peri1"), 40.0);
        // (30 + 45 + 40) / 3
        assert_eq!(st.average, Some(38.3));
    }

    #[test]
    fn implausible_values_are_zeroed() {
        let st = parse_chiptemp("^CHIPTEMP: 2000,1600,0,0,0,0,0,0,0,0,0,0");
        assert!(st.is_empty());
        assert_eq!(st.average, None);
    }

    #[test]
    fn missing_line_yields_empty_state() {
        assert!(parse_chiptemp("OK").is_empty());
    }
}
