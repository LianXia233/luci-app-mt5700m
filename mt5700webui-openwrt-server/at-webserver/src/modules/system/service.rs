//! System service: chip-temperature refresh plus the board switches the
//! system page reads and writes (NIC rate, power management, factory reset).
//!
//! `^CHIPTEMP?` is fast but its *queue* time is long while slow commands hold
//! the port, so the read runs at `Low` priority with a 12 s AT timeout and no
//! retry (a retry would burn the port twice). A failed tick re-publishes the
//! last good reading instead of clearing the card — the historical behaviour
//! that kept temperature from dropping to "—".

use crate::core::error::BackendError;
use crate::core::json::Value;
use crate::core::task::Priority;
use crate::modules::system::commands::{
    self, CHIPTEMP, FACTORY_RESET, FOTAMODE_QUERY, LEDSWITCH_QUERY, NIC_RATES, NWTIME_QUERY,
    TDPCIELANCFG_QUERY, TDPMCFG_QUERY, THERMAUTOFUN_QUERY, THERMLDAUTOPARA_QUERY,
    THERMLDAUTOSTATUS_QUERY, THERMLDLOGSW_QUERY, VERSION_QUERY, valid_thermal_thresholds,
};
use crate::modules::system::parser::{
    parse_chiptemp, parse_fotamode, parse_ledswitch, parse_nic_rate, parse_nwtime,
    parse_power_control, parse_thermautofun, parse_thermlevel, parse_thermlogsw,
    parse_thermthresholds, parse_version,
};
use crate::modules::system::state::{
    DeviceControlState, TemperatureState, ThermalState, VersionState,
};
use crate::scheduler::channel::run_in_task;
use crate::scheduler::jobs::TaskManager;
use crate::state::bus::{EVENT_TEMPERATURE_UPDATED, TOPIC_TEMPERATURE};
use crate::state::refresh::{ReadErrors, RefreshCtx};
use std::time::Duration;

const AT_TIMEOUT: Duration = Duration::from_secs(12);
const QUEUED_TIMEOUT: Duration = Duration::from_secs(8);
const PERIOD: Duration = Duration::from_secs(60);
/// Board switches are fast reads; they do not need the temperature budget.
const CTRL_AT_TIMEOUT: Duration = Duration::from_secs(6);
const CTRL_QUEUED_TIMEOUT: Duration = Duration::from_secs(5);

/// Refresh the temperature topic (never fails on a slow/absent modem).
pub fn refresh(ctx: &RefreshCtx) -> Result<TemperatureState, BackendError> {
    // A missing or busy modem is "no answer this tick", not a failure: the
    // temperature card is on every dashboard and a display route must always
    // answer (see `Route::display`).
    let text = match ctx.read(CHIPTEMP, AT_TIMEOUT, QUEUED_TIMEOUT, Priority::Low) {
        Ok(text) => text,
        Err(_) => String::new(),
    };
    if text.trim().is_empty() {
        let _ = ctx.stale(TOPIC_TEMPERATURE, EVENT_TEMPERATURE_UPDATED);
        return Ok(TemperatureState::default());
    }
    let st = parse_chiptemp(text.trim());
    if st.is_empty() {
        let _ = ctx.stale(TOPIC_TEMPERATURE, EVENT_TEMPERATURE_UPDATED);
        return Ok(st);
    }
    ctx.store(TOPIC_TEMPERATURE, EVENT_TEMPERATURE_UPDATED, &st.to_json());
    Ok(st)
}

/// Cache-first read for the API.
pub fn cached(cache: &std::sync::Arc<crate::state::cache::StateCache>) -> Option<TemperatureState> {
    match cache.get(TOPIC_TEMPERATURE) {
        (Some(Value::Obj(m)), _) => Some(TemperatureState::from_json(&m)),
        _ => None,
    }
}

/// Read the board switches (`^TDPCIELANCFG?`, `^TDPMCFG?`).
///
/// Both are best-effort: whichever answered fills its field, and a modem that
/// answered neither reports the first error so the caller can decide between
/// "not now" (display route) and a failure.
pub fn read_device_control(ctx: &RefreshCtx) -> Result<DeviceControlState, BackendError> {
    let mut st = DeviceControlState::default();
    let mut errors = ReadErrors::new();
    let nic = errors.note(ctx.read(
        TDPCIELANCFG_QUERY,
        CTRL_AT_TIMEOUT,
        CTRL_QUEUED_TIMEOUT,
        Priority::Normal,
    ));
    if let Some(text) = nic {
        st.nic_rate = parse_nic_rate(&text);
    }
    let power = errors.note(ctx.read(
        TDPMCFG_QUERY,
        CTRL_AT_TIMEOUT,
        CTRL_QUEUED_TIMEOUT,
        Priority::Normal,
    ));
    if let Some(text) = power {
        st.power_control = parse_power_control(&text);
    }
    if st.is_empty() {
        if let Some(e) = errors.into_option() {
            return Err(e);
        }
    }
    Ok(st)
}

/// Set the NIC rate (`^TDPCIELANCFG=<1|2>`); the change needs a reboot.
pub fn set_nic_rate(ctx: &RefreshCtx, rate: i64) -> Result<(), BackendError> {
    if !NIC_RATES.contains(&rate) {
        return Err(BackendError::InvalidParameter(format!(
            "网卡速率只能是 1 或 2，收到 {}",
            rate
        )));
    }
    ctx.action(&commands::tdpcpcielancfg(rate))?;
    Ok(())
}

/// Enable/disable the PCIe controller power management (`^TDPMCFG=<0|1>`).
pub fn set_power_control(ctx: &RefreshCtx, on: bool) -> Result<(), BackendError> {
    ctx.action(&commands::tdpmcfg(on))?;
    Ok(())
}

/// Restore the AT configuration defaults (`AT&F`).
pub fn factory_reset(ctx: &RefreshCtx) -> Result<(), BackendError> {
    ctx.action(FACTORY_RESET)?;
    Ok(())
}

/// Read the thermal-protection settings (four queries).
///
/// Best-effort like the board switches: whichever query answered fills its
/// fields, and a modem that answered none reports the first error.
pub fn read_thermal(ctx: &RefreshCtx) -> Result<ThermalState, BackendError> {
    let mut st = ThermalState::default();
    let mut errors = ReadErrors::new();

    let fun = errors.note(ctx.read(
        THERMAUTOFUN_QUERY,
        CTRL_AT_TIMEOUT,
        CTRL_QUEUED_TIMEOUT,
        Priority::Normal,
    ));
    if let Some((enabled, ca_mimo, interval)) = fun.as_deref().and_then(parse_thermautofun) {
        st.enabled = Some(enabled);
        st.ca_mimo_switch = Some(ca_mimo);
        st.interval = Some(interval);
    }
    let log = errors.note(ctx.read(
        THERMLDLOGSW_QUERY,
        CTRL_AT_TIMEOUT,
        CTRL_QUEUED_TIMEOUT,
        Priority::Normal,
    ));
    if let Some((console, file)) = log.as_deref().and_then(parse_thermlogsw) {
        st.console_log = Some(console);
        st.file_log = Some(file);
    }
    let para = errors.note(ctx.read(
        THERMLDAUTOPARA_QUERY,
        CTRL_AT_TIMEOUT,
        CTRL_QUEUED_TIMEOUT,
        Priority::Normal,
    ));
    if let Some(thresholds) = para.as_deref().and_then(parse_thermthresholds) {
        st.thresholds = thresholds;
    }
    let status = errors.note(ctx.read(
        THERMLDAUTOSTATUS_QUERY,
        CTRL_AT_TIMEOUT,
        CTRL_QUEUED_TIMEOUT,
        Priority::Normal,
    ));
    if let Some(level) = status.as_deref().and_then(parse_thermlevel) {
        st.current_level = Some(level);
    }

    if st.is_empty() {
        if let Some(e) = errors.into_option() {
            return Err(e);
        }
    }
    Ok(st)
}

/// Write the thermal master switch (`^THERMAUTOFUN`).
///
/// The page sends all three fields because the command carries all three: the
/// switch, the CA/MIMO link and the detection interval.
pub fn set_thermal(
    ctx: &RefreshCtx,
    enabled: bool,
    ca_mimo: bool,
    interval: i64,
) -> Result<(), BackendError> {
    if !(1..=300).contains(&interval) {
        return Err(BackendError::InvalidParameter(format!(
            "温度检测间隔必须在 1-300 秒之间，收到 {}",
            interval
        )));
    }
    ctx.action(&commands::thermautofun(enabled, ca_mimo, interval))?;
    Ok(())
}

/// Read the module's status LED (`^LEDSWITCH?`).
///
/// A live read rather than a cached fact. An unanswered query is an error
/// *here*; the `system.led` route turns that into an absent field so the page
/// keeps the switch where the user left it.
pub fn read_led(ctx: &RefreshCtx) -> Result<bool, BackendError> {
    let text = ctx.read(
        LEDSWITCH_QUERY,
        CTRL_AT_TIMEOUT,
        CTRL_QUEUED_TIMEOUT,
        Priority::Normal,
    )?;
    // Same shape the other on-demand readers report (`^SYSCFGEX?`, `^SETMODE?`,
    // `+CPIN?`): keep the raw frame so the operator sees what came back.
    parse_ledswitch(&text).ok_or_else(|| {
        BackendError::AtRejected(format!(
            "^LEDSWITCH? answered an unknown shape: {}",
            text.trim()
        ))
    })
}

/// Write the status LED (`^LEDSWITCH=<0|1>`).
///
/// The module stores the setting; it takes effect after a restart, which is
/// what the page's confirmation dialog has always said.
pub fn set_led(ctx: &RefreshCtx, on: bool) -> Result<(), BackendError> {
    ctx.action(&commands::ledswitch(on))?;
    Ok(())
}

/// Read the network-published time (`^NWTIME?`).
///
/// Returns the string the page used to slice out of the frame. A modem without
/// registration has no time line; that is an error here, and the route answers
/// `{}` so the card renders its own placeholder instead of a stale time.
pub fn read_network_time(ctx: &RefreshCtx) -> Result<String, BackendError> {
    let text = ctx.read(
        NWTIME_QUERY,
        CTRL_AT_TIMEOUT,
        CTRL_QUEUED_TIMEOUT,
        Priority::Normal,
    )?;
    parse_nwtime(&text).ok_or_else(|| {
        BackendError::AtRejected(format!(
            "^NWTIME? answered an unknown shape: {}",
            text.trim()
        ))
    })
}

/// Read the module version block (`^VERSION?`).
///
/// Each line is independent, so a modem answering only part of the block is
/// answered back with what it gave — no field is invented and none is dropped.
pub fn read_version(ctx: &RefreshCtx) -> Result<VersionState, BackendError> {
    let text = ctx.read(
        VERSION_QUERY,
        CTRL_AT_TIMEOUT,
        CTRL_QUEUED_TIMEOUT,
        Priority::Normal,
    )?;
    parse_version(&text).ok_or_else(|| {
        BackendError::AtRejected(format!(
            "^VERSION? answered an unknown shape: {}",
            text.trim()
        ))
    })
}

/// Read the FOTA update mode (`^FOTAMODE?`).
///
/// The mode is returned as the raw `a,b,c,d` string: the name for it is UI copy
/// (`HTTP update mode` for `0,1,0,1`), so the decoding stays in the page.
pub fn read_fota_mode(ctx: &RefreshCtx) -> Result<String, BackendError> {
    let text = ctx.read(
        FOTAMODE_QUERY,
        CTRL_AT_TIMEOUT,
        CTRL_QUEUED_TIMEOUT,
        Priority::Normal,
    )?;
    parse_fotamode(&text).ok_or_else(|| {
        BackendError::AtRejected(format!(
            "^FOTAMODE? answered an unknown shape: {}",
            text.trim()
        ))
    })
}

/// Write the nine thermal thresholds (`^THERMLDAUTOPARA=…`).
///
/// The rule the page and the CLI both applied before sending now lives here, so
/// neither frontend has to keep a copy of the modem's ladder.
pub fn set_thermal_thresholds(ctx: &RefreshCtx, values: &[i64]) -> Result<(), BackendError> {
    if !valid_thermal_thresholds(values) {
        return Err(BackendError::InvalidParameter(
            "温度阈值需要 9 个 0-150°C 的值，且触发温度逐级升高、每一级恢复温度低于其触发温度".to_string(),
        ));
    }
    ctx.action(&commands::thermldautopara(values))?;
    Ok(())
}

/// Write the two thermal log switches (`^THERMLDLOGSW=<serial>,<file>`).
pub fn set_thermal_log(
    ctx: &RefreshCtx,
    serial: bool,
    file: bool,
) -> Result<(), BackendError> {
    ctx.action(&commands::thermldlogsw(serial, file))?;
    Ok(())
}

/// Register the periodic refresh job.
pub fn spawn(tasks: &TaskManager) {
    tasks.add_periodic(
        "system.temperature",
        PERIOD,
        Priority::Low,
        Some(Duration::from_secs(15)),
        Box::new(|ctx| run_in_task(ctx, |r| refresh(r).map(|st| st.to_json()))),
    );
}
