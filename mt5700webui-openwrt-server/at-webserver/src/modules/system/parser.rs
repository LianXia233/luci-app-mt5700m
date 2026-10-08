//! `^CHIPTEMP?` / `^TDPCIELANCFG?` / `^TDPMCFG?` / FOTA decoders.

use crate::modules::system::state::{TemperatureState, VersionState, SENSOR_NAMES};
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

/// `^LEDSWITCH: <0|1>` -> whether the module's status LED is enabled.
///
/// The page read this field as text and only compared it against `"1"`, so a
/// modem answering anything else leaves the dropdown where it was.
pub fn parse_ledswitch(raw: &str) -> Option<bool> {
    let body = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("^LEDSWITCH:"))?
        .trim();
    let v = body.split(',').next()?.trim().parse::<i64>().ok()?;
    match v {
        1 => Some(true),
        0 => Some(false),
        _ => None,
    }
}

/// `^NWTIME: …` -> the timestamp as the network published it.
///
/// Returned **verbatim** (minus the quotes some firmwares wrap it in and the
/// surrounding whitespace): the page displayed this string as-is and had no
/// interpretation to move into the module. `None` when the modem does not
/// answer with a time line — a registration-less modem has none.
pub fn parse_nwtime(raw: &str) -> Option<String> {
    let body = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("^NWTIME:"))?
        .trim();
    let text = body.trim_matches('"').trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// The module version block (`^VERSION?`).
///
/// Three independent lines the system page shows as build date, software and
/// hardware version. Each is taken verbatim after its `^VERSION:<tag>` prefix
/// (then trimmed, which also drops the `: ` separator) — none of them has an
/// interpretation to move into the module. None when no line answered.
pub fn parse_version(raw: &str) -> Option<VersionState> {
    let field = |tag: &str| -> Option<String> {
        let after = raw
            .lines()
            .find_map(|l| l.trim().strip_prefix(tag))?
            .trim_start_matches(':')
            .trim();
        let text = after.trim_matches('"').trim();
        if text.is_empty() {
            None
        } else {
            Some(text.to_string())
        }
    };
    let st = VersionState {
        build_date: field("^VERSION:BDT"),
        software: field("^VERSION:EXTS"),
        hardware: field("^VERSION:EXTH"),
    };
    if st.is_empty() {
        None
    } else {
        Some(st)
    }
}

/// FOTA update mode (`^FOTAMODE: <a>,<b>,<c>,<d>`).
///
/// Returned as the raw field string: calling `0,1,0,1` "HTTP update mode" is
/// the page's own decoding, and that wording is UI copy. The route must be able
/// to answer whatever mode the modem reported.
pub fn parse_fotamode(raw: &str) -> Option<String> {
    let body = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("^FOTAMODE:"))?
        .trim();
    let fields: Vec<&str> = body.split(',').map(|f| f.trim()).collect();
    if fields.is_empty() || fields.iter().any(|f| f.is_empty()) {
        return None;
    }
    Some(fields.join(","))
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
    fn led_switch_read() {
        assert_eq!(parse_ledswitch("^LEDSWITCH: 1\r\n\r\nOK"), Some(true));
        assert_eq!(parse_ledswitch("^LEDSWITCH: 0\r\nOK"), Some(false));
        // An out-of-range answer keeps the page's own value.
        assert_eq!(parse_ledswitch("^LEDSWITCH: 2"), None);
        assert_eq!(parse_ledswitch("+CME ERROR: 3"), None);
    }

    #[test]
    fn network_time_is_passed_through_verbatim() {
        assert_eq!(
            parse_nwtime("^NWTIME: 2025/08/15 12:00:00\r\nOK").as_deref(),
            Some("2025/08/15 12:00:00")
        );
        // Some firmwares quote the timestamp; the page stripped quotes too.
        assert_eq!(
            parse_nwtime("^NWTIME: \"2025/08/15 12:00:00\"").as_deref(),
            Some("2025/08/15 12:00:00")
        );
        assert_eq!(parse_nwtime("^NWTIME: "), None);
        assert_eq!(parse_nwtime("OK"), None);
    }

    #[test]
    fn fota_state_and_progress_keep_the_page_semantics() {
        assert_eq!(parse_fota_state("^FOTASTATE: 30\r\nOK"), Some(30));
        assert_eq!(parse_fota_state("^FOTASTATE:10"), Some(10));
        assert_eq!(parse_fota_state("ERROR"), None);

        // The page read the *last* two numbers as (total, received) — a
        // leading field must not shift which pair is reported.
        let (total, received) = parse_fota_progress("^FOTADLQ: \"1234,5678\"\r\nOK");
        assert_eq!((total, received), (1234, 5678));
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

    #[test]
    fn version_block_keeps_each_line_independent() {
        let st = parse_version(
            "^VERSION:BDT: Aug 15 2025 10:20:30\r\n\
             ^VERSION:EXTS: MT5700M-2.5.0\r\n\
             ^VERSION:EXTH: MT5700M-HW-1.0\r\nOK",
        )
        .expect("the three lines");
        assert_eq!(st.build_date.as_deref(), Some("Aug 15 2025 10:20:30"));
        assert_eq!(st.software.as_deref(), Some("MT5700M-2.5.0"));
        assert_eq!(st.hardware.as_deref(), Some("MT5700M-HW-1.0"));
        assert!(!st.is_empty());
        // A modem that answers only part of the block gets answered back
        // verbatim: no missing field is invented.
        let partial = parse_version("^VERSION:EXTS: MT5700M-2.5.0\r\nOK").expect("one line");
        assert_eq!(partial.software.as_deref(), Some("MT5700M-2.5.0"));
        assert_eq!(partial.build_date, None);
        assert_eq!(partial.hardware, None);
        assert_eq!(parse_version("OK"), None);
        assert_eq!(parse_version("^VERSION:BDT: \r\nOK"), None);
    }

    #[test]
    fn fota_mode_is_passed_through_undecoded() {
        // Calling this "HTTP update mode" is UI copy, so the decoder must not
        // take that interpretation away from the page.
        assert_eq!(
            parse_fotamode("^FOTAMODE: 0,1,0,1\r\nOK"),
            Some("0,1,0,1".to_string())
        );
        assert_eq!(
            parse_fotamode("^FOTAMODE: 1,2,3,4\r\nOK"),
            Some("1,2,3,4".to_string())
        );
        assert_eq!(parse_fotamode("OK"), None);
        assert_eq!(parse_fotamode("^FOTAMODE: \r\nOK"), None);
        assert_eq!(parse_fotamode("^FOTAMODE: 0,,1,1\r\nOK"), None);
    }
}