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
}
