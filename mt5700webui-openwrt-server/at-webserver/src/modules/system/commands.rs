//! AT commands owned by the system module: board-level switches and the
//! factory reset. (Modem identity/restart live in `modules/modem`.)

/// Chip temperature array (12 sensors, tenths of a degree). Fast command
/// (~0.1 s) whose *queue* time is the problem: it shares the port with slow
/// commands, so it is read with a generous timeout and no retry.
pub const CHIPTEMP: &str = "AT^CHIPTEMP?";

/// NIC speed setting, query (`^TDPCIELANCFG: <1|2>`) and write.
pub const TDPCIELANCFG_QUERY: &str = "AT^TDPCIELANCFG?";
/// PCIe controller / power management, query (`^TDPMCFG: <0|1>`) and write.
pub const TDPMCFG_QUERY: &str = "AT^TDPMCFG?";

/// The NIC rate values the page offers: 1 = RTL8111 (1G), 2 = RTL8125 (2.5G).
pub const NIC_RATES: [i64; 2] = [1, 2];

/// `AT^TDPCIELANCFG=<1|2>`.
pub fn tdpcpcielancfg(rate: i64) -> String {
    format!("AT^TDPCIELANCFG={}", rate)
}

/// `AT^TDPMCFG=<0|1>`.
///
/// The CLI's older form appends the three unused sub-fields
/// (`AT^TDPMCFG=1,0,0,0`); the modem accepts the short form, and this is the
/// one the page has always used.
pub fn tdpmcfg(on: bool) -> String {
    format!("AT^TDPMCFG={}", if on { 1 } else { 0 })
}

/// Set the AT configuration back to factory defaults (`AT&F`).
pub const FACTORY_RESET: &str = "AT&F";

// --------------------------------------------------------------- FOTA
//
// The firmware-upgrade flow the WebUI's system page drives. Every command the
// page used to spell out itself lives here now and `fota.rs` runs them as one
// module-owned task; the page only calls `system.fota*`.

/// Download-state query (`^FOTASTATE: <code>` — 10 idle, 11 checking, 12
/// update available, 13 check failed, 14 no update, 20 download failed,
/// 30 downloading, 31 paused, 40 complete, 50 installing).
pub const FOTA_STATE_QUERY: &str = "AT^FOTASTATE?";
/// Downloaded/total byte counts (`^FOTADLQ: …`).
pub const FOTA_PROGRESS_QUERY: &str = "AT^FOTADLQ";
/// Stops the modem echoing every command, so the download poller does not
/// collect echoes as noise while it reads the state every second.
pub const FOTA_ECHO_OFF: &str = "ATE0";
/// HTTP update mode, exactly the argument list the page has always sent.
pub const FOTA_MODE_INIT: &str = "AT^FOTAMODE=0,1,0,1";
/// Resume a paused download.
pub const FOTA_DOWNLOAD_RESUME: &str = "AT^FOTADL=1";
/// Flash the downloaded image; the modem reboots.
pub const FOTA_UPGRADE: &str = "AT^FWUP";

/// FOTA server address (`AT^FOTAOEMDL="<url>"`).
///
/// The page's validation (http:// only, trailing slash) ran before the command
/// was built, so a rejected address never reached the modem; the same two rules
/// apply here now, plus a quote guard for the AT string argument.
pub fn fota_url_set(url: &str) -> Result<String, &'static str> {
    if !url.starts_with("http://") {
        return Err("仅支持 http 协议");
    }
    if url.contains('"') || url.contains('\r') || url.contains('\n') {
        return Err("地址不合法");
    }
    let normalized = if url.ends_with('/') {
        url.to_string()
    } else {
        format!("{}/", url)
    };
    Ok(format!("AT^FOTAOEMDL=\"{}\"", normalized))
}

/// State code -> the name both frontends show, identical to the strings the
/// WebUI's switch and LuCI's `parser.FOTA_STATE_NAMES` already use.
pub fn fota_state_name(code: i64) -> &'static str {
    match code {
        10 => "等待下载",
        11 => "正在查询新版本",
        12 => "发现新版本",
        13 => "查询新版本失败",
        14 => "服务器无新版本",
        20 => "固件下载失败",
        30 => "下载中",
        31 => "下载已挂起",
        40 => "固件下载完成",
        50 => "正在升级",
        _ => "未知状态",
    }
}

// ---------------------------------------------------- thermal protection
//
// `^THERMAUTOFUN` is the master switch (enabled, CA/MIMO switch, interval) and
// the three `^THERMLD*` queries report the log switches, the thresholds and the
// current protection level. The page reads all four and writes `^THERMAUTOFUN`.

/// Master thermal-protection query (`^THERMAUTOFUN: <on> <caMimo> <interval>`).
pub const THERMAUTOFUN_QUERY: &str = "AT^THERMAUTOFUN?";
/// Thermal log switches (`^THERMLDLOGSW: <console> <file>`).
pub const THERMLDLOGSW_QUERY: &str = "AT^THERMLDLOGSW?";
/// Thermal thresholds (`^THERMLDAUTOPARA: <nums…>`).
pub const THERMLDAUTOPARA_QUERY: &str = "AT^THERMLDAUTOPARA?";
/// Current protection level (`^THERMLDAUTOSTATUS: <nums…>`, level = field 6).
pub const THERMLDAUTOSTATUS_QUERY: &str = "AT^THERMLDAUTOSTATUS?";
/// Index of the current level inside the `^THERMLDAUTOSTATUS` numbers.
pub const THERM_STATUS_LEVEL_INDEX: usize = 5;

/// `AT^THERMAUTOFUN=<0|1>,<0|1>,<interval>`.
pub fn thermautofun(enabled: bool, ca_mimo: bool, interval: i64) -> String {
    format!(
        "AT^THERMAUTOFUN={},{},{}",
        if enabled { 1 } else { 0 },
        if ca_mimo { 1 } else { 0 },
        interval
    )
}

// ------------------------------------------------- LED / network time
//
// Two board-level facts the system page reads and one it writes. `^LEDSWITCH`
// is the module's status LED (the page's own wording: "the LED setting is
// stored by the module and takes effect after restart"); `^NWTIME` is the time
// the operator's network publishes, which the page shows verbatim.

/// Status LED query (`^LEDSWITCH: <0|1>`).
pub const LEDSWITCH_QUERY: &str = "AT^LEDSWITCH?";

/// `AT^LEDSWITCH=<0|1>`.
pub fn ledswitch(on: bool) -> String {
    format!("AT^LEDSWITCH={}", if on { 1 } else { 0 })
}

/// Network time query (`^NWTIME: …`).
pub const NWTIME_QUERY: &str = "AT^NWTIME?";

/// Module version block (`^VERSION:` with `BDT` / `EXTS` / `EXTH` lines).
pub const VERSION_QUERY: &str = "AT^VERSION?";

/// FOTA update mode (`^FOTAMODE?`).
pub const FOTAMODE_QUERY: &str = "AT^FOTAMODE?";

/// The nine thermal thresholds the page writes (`AT^THERMLDAUTOPARA=…`).
///
/// Order and meaning are the page's: normal, first derate, first recovery,
/// second derate, second recovery, continuous limit, its recovery, emergency
/// radio-off, emergency recovery.
pub fn thermldautopara(values: &[i64]) -> String {
    let body: Vec<String> = values.iter().map(|v| v.to_string()).collect();
    format!("AT^THERMLDAUTOPARA={}", body.join(","))
}

/// `AT^THERMLDLOGSW=<serial>,<file>`.
pub fn thermldlogsw(serial: bool, file: bool) -> String {
    format!(
        "AT^THERMLDLOGSW={},{}",
        if serial { 1 } else { 0 },
        if file { 1 } else { 0 }
    )
}

/// The threshold table's validation, moved here from the CLI verb.
///
/// The page validated before it built the command (and the CLI repeated the
/// check), so an impossible table never reached the modem. Nine values in
/// 0–150°C, with every trigger level rising and every recovery below its own
/// trigger. Kept as-is rather than reworded: this is the modem's rule, not a
/// formatting choice.
pub fn valid_thermal_thresholds(values: &[i64]) -> bool {
    if values.len() != 9 {
        return false;
    }
    if values.iter().any(|v| !(0..=150).contains(v)) {
        return false;
    }
    values[1] > values[0]
        && values[3] > values[1]
        && values[5] > values[3]
        && values[7] > values[5]
        && values[2] < values[1]
        && values[4] < values[3]
        && values[6] < values[5]
        && values[8] < values[7]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_forms() {
        assert_eq!(tdpcpcielancfg(1), "AT^TDPCIELANCFG=1");
        assert_eq!(tdpcpcielancfg(2), "AT^TDPCIELANCFG=2");
        assert_eq!(tdpmcfg(true), "AT^TDPMCFG=1");
        assert_eq!(tdpmcfg(false), "AT^TDPMCFG=0");
        assert!(NIC_RATES.contains(&1) && NIC_RATES.contains(&2));
    }

    #[test]
    fn thermal_master_write() {
        assert_eq!(
            thermautofun(true, true, 2),
            "AT^THERMAUTOFUN=1,1,2"
        );
        assert_eq!(thermautofun(false, false, 30), "AT^THERMAUTOFUN=0,0,30");
    }

    #[test]
    fn led_write_form_matches_the_cli_verb() {
        assert_eq!(ledswitch(true), "AT^LEDSWITCH=1");
        assert_eq!(ledswitch(false), "AT^LEDSWITCH=0");
        assert_eq!(LEDSWITCH_QUERY, "AT^LEDSWITCH?");
        assert_eq!(NWTIME_QUERY, "AT^NWTIME?");
    }

    #[test]
    fn thermal_table_write_joins_nine_fields() {
        // Same shape as the CLI's `advanced-set thermal-thresholds`, which
        // joined its arguments with commas.
        assert_eq!(
            thermldautopara(&[60, 70, 65, 80, 75, 90, 85, 100, 95]),
            "AT^THERMLDAUTOPARA=60,70,65,80,75,90,85,100,95"
        );
        assert_eq!(thermldlogsw(true, false), "AT^THERMLDLOGSW=1,0");
        assert_eq!(thermldlogsw(false, true), "AT^THERMLDLOGSW=0,1");
    }

    #[test]
    fn thermal_table_rule_is_the_clis() {
        let ok = [60, 70, 65, 80, 75, 90, 85, 100, 95];
        assert!(valid_thermal_thresholds(&ok));
        // Wrong length.
        assert!(!valid_thermal_thresholds(&[60, 70, 65]));
        assert!(!valid_thermal_thresholds(&[
            60, 70, 65, 80, 75, 90, 85, 100, 95, 120
        ]));
        // Out of range.
        assert!(!valid_thermal_thresholds(&[60, 151, 65, 80, 75, 90, 85, 100, 95]));
        assert!(!valid_thermal_thresholds(&[-1, 70, 65, 80, 75, 90, 85, 100, 95]));
        // Trigger levels must rise …
        let mut flat = ok;
        flat[1] = 60;
        assert!(!valid_thermal_thresholds(&flat));
        // … and every recovery must sit below its own trigger.
        let mut recov = ok;
        recov[2] = 75;
        assert!(!valid_thermal_thresholds(&recov));
    }
}
