//! `^CHIPTEMP?` / `^TDPCIELANCFG?` / `^TDPMCFG?` / FOTA decoders.

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

/// `^TDPCIELANCFG: <rate>` -> the NIC rate, when it is one the UI offers.
///
/// A modem reporting anything else keeps the page on the value it already has,
/// which is exactly what the page did when it parsed this itself.
pub fn parse_nic_rate(raw: &str) -> Option<i64> {
    let body = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("^TDPCIELANCFG:"))?
        .trim();
    let rate = body.split(',').next()?.trim().parse::<i64>().ok()?;
    if crate::modules::system::commands::NIC_RATES.contains(&rate) {
        Some(rate)
    } else {
        None
    }
}

/// `^TDPMCFG: <0|1>` -> whether power management is on.
pub fn parse_power_control(raw: &str) -> Option<bool> {
    let body = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("^TDPMCFG:"))?
        .trim();
    body.split(',').next()?.trim().parse::<i64>().ok().map(|n| n == 1)
}

/// Split a vendor payload into numbers, accepting spaces and commas.
fn numbers(body: &str) -> Vec<i64> {
    body.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|f| !f.is_empty())
        .filter_map(|f| f.parse::<i64>().ok())
        .collect()
}

/// `^THERMAUTOFUN: <enabled> <caMimo> <interval>`.
///
/// The three fields are whitespace-separated on this firmware and
/// comma-separated on others, so both are accepted.
pub fn parse_thermautofun(raw: &str) -> Option<(bool, bool, i64)> {
    let body = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("^THERMAUTOFUN:"))?;
    let v = numbers(body);
    if v.len() < 3 {
        return None;
    }
    Some((v[0] == 1, v[1] == 1, v[2]))
}

/// `^THERMLDLOGSW: <console> <file>` -> the two log switches.
pub fn parse_thermlogsw(raw: &str) -> Option<(bool, bool)> {
    let body = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("^THERMLDLOGSW:"))?;
    let v = numbers(body);
    if v.len() < 2 {
        return None;
    }
    Some((v[0] == 1, v[1] == 1))
}

/// `^THERMLDAUTOPARA: <nums…>` -> the threshold table.
pub fn parse_thermthresholds(raw: &str) -> Option<Vec<i64>> {
    let body = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("^THERMLDAUTOPARA:"))?;
    let v = numbers(body);
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

/// `^THERMLDAUTOSTATUS: <nums…>` -> the current protection level (field 6).
pub fn parse_thermlevel(raw: &str) -> Option<i64> {
    let body = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("^THERMLDAUTOSTATUS:"))?;
    let v = numbers(body);
    v.get(crate::modules::system::commands::THERM_STATUS_LEVEL_INDEX)
        .copied()
}

/// `^FOTASTATE: <code>` -> the code. `None` when the modem answered without a
/// state line (rejection or empty reply), which the caller reports as a failed
/// update rather than as code 0.
pub fn parse_fota_state(raw: &str) -> Option<i64> {
    let line = raw.lines().map(str::trim).find(|l| l.starts_with("^FOTASTATE:"))?;
    let body = line.strip_prefix("^FOTASTATE:")?.trim();
    body.split(|c: char| c == ',' || c.is_whitespace())
        .find_map(|f| f.parse::<i64>().ok())
}

/// `^FOTADLQ: …` -> `(total, received)` bytes.
///
/// The page took the *last two* numbers of the reply, whatever the firmware put
/// before them; that is the contract this keeps, so an extra leading field
/// cannot silently become the reported total.
pub fn parse_fota_progress(raw: &str) -> (i64, i64) {
    let digits: String = raw
        .chars()
        .map(|c| if c.is_ascii_digit() { c } else { ' ' })
        .collect();
    let nums: Vec<i64> = digits
        .split_whitespace()
        .filter_map(|t| t.parse::<i64>().ok())
        .collect();
    if nums.len() >= 2 {
        (nums[nums.len() - 2], nums[nums.len() - 1])
    } else {
        (0, 0)
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

    #[test]
    fn nic_rate_only_accepts_the_two_settings() {
        assert_eq!(parse_nic_rate("^TDPCIELANCFG: 1"), Some(1));
        assert_eq!(parse_nic_rate("^TDPCIELANCFG: 2\r\nOK"), Some(2));
        assert_eq!(parse_nic_rate("^TDPCIELANCFG: 0"), None);
        assert_eq!(parse_nic_rate("+CME ERROR: 3"), None);
    }

    #[test]
    fn power_control_flag() {
        assert_eq!(parse_power_control("^TDPMCFG: 1\nOK"), Some(true));
        assert_eq!(parse_power_control("^TDPMCFG: 0\nOK"), Some(false));
        assert_eq!(parse_power_control("OK"), None);
    }

    #[test]
    fn fota_state_and_progress_keep_the_page_semantics() {
        assert_eq!(parse_fota_state("^FOTASTATE: 30\r\nOK"), Some(30));
        assert_eq!(parse_fota_state("^FOTASTATE:10"), Some(10));
        assert_eq!(parse_fota_state("ERROR"), None);

        // The page read the *last* two numbers, so a stricter earlier field
        // must not change which pair is reported.
        let (total, received) = parse_fota_progress("^FOTADLQ: \"1234,5678\"\r\nOK");
        assert_eq!((total, received), (5678, 5678));
        let (total, received) = parse_fota_progress("^FOTADLQ: 0,0,1048576,262144");
        assert_eq!((total, received), (1048576, 262144));
        assert_eq!(parse_fota_progress("OK"), (0, 0));
    }

    #[test]
    fn thermal_queries_accept_spaces_and_commas() {
        assert_eq!(parse_thermautofun("^THERMAUTOFUN: 1 1 2\nOK"), Some((true, true, 2)));
        assert_eq!(
            parse_thermautofun("^THERMAUTOFUN: 0,0,30"),
            Some((false, false, 30))
        );
        assert_eq!(parse_thermautofun("OK"), None);
        assert_eq!(parse_thermlogsw("^THERMLDLOGSW: 1 0"), Some((true, false)));
        assert_eq!(
            parse_thermthresholds("^THERMLDAUTOPARA: 50,60,70,80,90,100"),
            Some(vec![50, 60, 70, 80, 90, 100])
        );
        // Level is field 6 (index 5): a short reply has no level at all.
        assert_eq!(
            parse_thermlevel("^THERMLDAUTOSTATUS: 1,2,3,4,5,3"),
            Some(3)
        );
        assert_eq!(parse_thermlevel("^THERMLDAUTOSTATUS: 1,2,3"), None);
    }
}