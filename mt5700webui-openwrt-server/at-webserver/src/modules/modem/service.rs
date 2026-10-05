//! Modem service: identity, transmit power and EN-DC refresh policies.
//!
//! Cadences preserved from the collectors: identity 60 s (`Background`, 60 s
//! failure backoff, 60 s cache TTL so the UI never blanks), TXPOWER and LENDC
//! 300 s (both are unsupported in NR mode and cost 8–12 s when tried), NTXPOWER
//! 180 s (its reply takes ~12 s).

use crate::core::error::BackendError;
use crate::core::json::Value;
use crate::core::task::Priority;
use crate::modules::modem::commands::{
    self, ATI, CGSN, LENDC, NRRCCAP_CA, NRRCCAP_DSS, NRRCCAP_VONR, NTXPOWER, RESET, TXPOWER, VONR_MAX,
};
use crate::modules::modem::parser;
use crate::modules::modem::state::{
    EndcState, McsState, ModemState, NrCapabilityState, NrTxPowerState, TxPowerState,
};
use crate::scheduler::channel::run_in_task;
use crate::scheduler::jobs::TaskManager;
use crate::state::bus::{
    EVENT_ENDC_UPDATED, EVENT_MODEM_INFO, EVENT_NR_TXPOWER_UPDATED, EVENT_TXPOWER_UPDATED,
    TOPIC_ENDC, TOPIC_MODEM, TOPIC_NR_TXPOWER, TOPIC_TXPOWER,
};
use crate::state::refresh::RefreshCtx;
use std::time::Duration;

const AT_TIMEOUT: Duration = Duration::from_secs(6);
const QUEUED_TIMEOUT: Duration = Duration::from_secs(5);
const SLOW_AT_TIMEOUT: Duration = Duration::from_secs(15);
const SLOW_QUEUED_TIMEOUT: Duration = Duration::from_secs(12);
const BACKOFF: Duration = Duration::from_secs(60);
const SLOW_BACKOFF: Duration = Duration::from_secs(600);
const INFO_TTL: Duration = Duration::from_secs(60);
/// Diagnostic reads the Info page triggers on load: bounded, interactive-ish.
const DIAG_AT_TIMEOUT: Duration = Duration::from_secs(8);
const DIAG_QUEUED_TIMEOUT: Duration = Duration::from_secs(10);

/// Identity refresh (`ATI` + `AT+CGSN`).
pub fn refresh_info(ctx: &RefreshCtx) -> Result<ModemState, BackendError> {
    let mut st = ModemState::default();
    if let Some(text) = ctx.slow("modem_ati", ATI, AT_TIMEOUT, QUEUED_TIMEOUT, BACKOFF) {
        st = parser::parse_ati(&text);
    }
    if let Some(text) = ctx.slow("modem_cgsn", CGSN, AT_TIMEOUT, QUEUED_TIMEOUT, BACKOFF) {
        st.imei = parser::parse_imei(&text);
    }
    // Long-lived data: set an explicit TTL and publish (the daemon's URC bridge
    // watches `modem.info`).
    let value = st.to_json();
    ctx.cache.set_ttl(TOPIC_MODEM, value.clone(), ctx.source, INFO_TTL);
    ctx.bus.publish(TOPIC_MODEM, EVENT_MODEM_INFO, value);
    Ok(st)
}

/// Transmit power (`^TXPOWER?`), empty on firmware that does not support it.
pub fn refresh_txpower(ctx: &RefreshCtx) -> Result<TxPowerState, BackendError> {
    let mut st = TxPowerState::default();
    if let Some(text) = ctx.slow("txpower", TXPOWER, SLOW_AT_TIMEOUT, SLOW_QUEUED_TIMEOUT, SLOW_BACKOFF) {
        st = parser::parse_txpower(&text);
    }
    ctx.store(TOPIC_TXPOWER, EVENT_TXPOWER_UPDATED, &st.to_json());
    Ok(st)
}

/// Per-carrier NR transmit power (`^NTXPOWER?`, ~12 s reply).
pub fn refresh_nr_txpower(ctx: &RefreshCtx) -> Result<NrTxPowerState, BackendError> {
    let mut st = NrTxPowerState::default();
    if let Some(text) = ctx.slow("nr_txpower", NTXPOWER, SLOW_AT_TIMEOUT, SLOW_QUEUED_TIMEOUT, SLOW_BACKOFF) {
        st = parser::parse_nr_txpower(&text);
    }
    ctx.store(TOPIC_NR_TXPOWER, EVENT_NR_TXPOWER_UPDATED, &st.to_json());
    Ok(st)
}

/// EN-DC status (`^LENDC?`).
pub fn refresh_endc(ctx: &RefreshCtx) -> Result<EndcState, BackendError> {
    let mut st = EndcState::default();
    if let Some(text) = ctx.slow("endc_lendc", LENDC, SLOW_AT_TIMEOUT, SLOW_QUEUED_TIMEOUT, SLOW_BACKOFF) {
        st = parser::parse_lendc(&text);
    }
    ctx.store(TOPIC_ENDC, EVENT_ENDC_UPDATED, &st.to_json());
    Ok(st)
}

/// Modulation/coding scheme of one direction (`AT^MCS=1` downlink,
/// `AT^MCS=0` uplink).
///
/// A page-load diagnostic read: no topic, no polling, no cache — the reply is
/// only meaningful at the moment the user looks at it.
pub fn refresh_mcs(ctx: &RefreshCtx, command: &str) -> Result<McsState, BackendError> {
    let text = ctx.read(command, DIAG_AT_TIMEOUT, DIAG_QUEUED_TIMEOUT, Priority::Normal)?;
    Ok(parser::parse_mcs(&text))
}

/// Cache-first reads for the API.
pub fn cached(cache: &std::sync::Arc<crate::state::cache::StateCache>) -> Option<ModemState> {
    match cache.get(TOPIC_MODEM) {
        (Some(Value::Obj(m)), _) => Some(ModemState::from_json(&m)),
        _ => None,
    }
}

pub fn cached_txpower(
    cache: &std::sync::Arc<crate::state::cache::StateCache>,
) -> Option<TxPowerState> {
    match cache.get(TOPIC_TXPOWER) {
        (Some(Value::Obj(m)), _) => Some(TxPowerState::from_json(&m)),
        _ => None,
    }
}

/// Restart the modem (`AT^RESET`).
///
/// The daemon keeps running and the modem comes back on its own; the snapshot is
/// dropped so the next read is fresh rather than the pre-restart identity.
pub fn reset(ctx: &RefreshCtx) -> Result<(), BackendError> {
    ctx.action(RESET)?;
    ctx.cache.invalidate(TOPIC_MODEM);
    Ok(())
}

/// Write a new IMEI (`^PHYNUM=IMEI,<15 digits>`).
///
/// Manual: the argument is exactly 15 digits. The value is validated here so the
/// page can show the backend's rejection instead of keeping a copy of the rule.
pub fn set_imei(ctx: &RefreshCtx, imei: &str) -> Result<(), BackendError> {
    let imei = imei.trim();
    if imei.len() != 15 || !imei.bytes().all(|b| b.is_ascii_digit()) {
        return Err(BackendError::InvalidParameter(
            "IMEI必须是15位数字".to_string(),
        ));
    }
    ctx.action(&commands::phynum_imei(imei))?;
    ctx.cache.invalidate(TOPIC_MODEM);
    Ok(())
}

/// Read the NR capability settings (`^NRRCCAPQRY=3/2/5`).
///
/// Each ability is one vendor query that carries its kind as the argument, so
/// they are read one by one; whichever answers fills its field.
pub fn read_nr_capability(ctx: &RefreshCtx) -> Result<NrCapabilityState, BackendError> {
    let mut st = NrCapabilityState::default();
    let mut errors = crate::state::refresh::ReadErrors::new();
    let ca = errors.note(ctx.read(
        &commands::nrrccapqry(NRRCCAP_CA),
        DIAG_AT_TIMEOUT,
        DIAG_QUEUED_TIMEOUT,
        Priority::Normal,
    ));
    if let Some(values) = ca.as_deref().and_then(|t| parser::parse_nrrccap(t, NRRCCAP_CA)) {
        st.ca = Some(values.first() == Some(&1));
    }
    let vonr = errors.note(ctx.read(
        &commands::nrrccapqry(NRRCCAP_VONR),
        DIAG_AT_TIMEOUT,
        DIAG_QUEUED_TIMEOUT,
        Priority::Normal,
    ));
    if let Some(values) = vonr
        .as_deref()
        .and_then(|t| parser::parse_nrrccap(t, NRRCCAP_VONR))
    {
        st.vonr = values.first().copied();
    }
    let dss = errors.note(ctx.read(
        &commands::nrrccapqry(NRRCCAP_DSS),
        DIAG_AT_TIMEOUT,
        DIAG_QUEUED_TIMEOUT,
        Priority::Normal,
    ));
    if let Some(values) = dss.as_deref().and_then(|t| parser::parse_nrrccap(t, NRRCCAP_DSS)) {
        if values.len() >= 2 {
            st.dss_rate_matching_lte = Some(values[0]);
            st.dss_additional_dmrs = Some(values[1]);
        }
    }
    if st.is_empty() {
        if let Some(e) = errors.into_option() {
            return Err(e);
        }
    }
    Ok(st)
}

/// Write one or more NR capability settings.
///
/// The page sends whichever card changed (`ca`, `vonr`, `dss` — all optional);
/// each one becomes its own `^NRRCCAPCFG` write, in the order the CLI and the
/// page have always used, and abilities that were not asked for are left alone.
pub fn set_nr_capability(
    ctx: &RefreshCtx,
    ca: Option<bool>,
    vonr: Option<i64>,
    dss: Option<(i64, i64)>,
) -> Result<Vec<i64>, BackendError> {
    if ca.is_none() && vonr.is_none() && dss.is_none() {
        return Err(BackendError::InvalidParameter(
            "modem.nr_capability_set needs ca, vonr or dss".to_string(),
        ));
    }
    let mut wrote = Vec::new();
    if let Some(enabled) = ca {
        ctx.action(&commands::nrrccapcfg(NRRCCAP_CA, &[if enabled { 1 } else { 0 }]))?;
        wrote.push(NRRCCAP_CA);
    }
    if let Some(mode) = vonr {
        if !(0..=VONR_MAX).contains(&mode) {
            return Err(BackendError::InvalidParameter(format!(
                "VoNR 模式只能是 0-{}，收到 {}",
                VONR_MAX, mode
            )));
        }
        ctx.action(&commands::nrrccapcfg(NRRCCAP_VONR, &[mode]))?;
        wrote.push(NRRCCAP_VONR);
    }
    if let Some((rate_matching, dmrs)) = dss {
        for v in [rate_matching, dmrs] {
            if !(0..=1).contains(&v) {
                return Err(BackendError::InvalidParameter(format!(
                    "DSS 取值只能是 0 或 1，收到 {}",
                    v
                )));
            }
        }
        ctx.action(&commands::nrrccapcfg(NRRCCAP_DSS, &[rate_matching, dmrs]))?;
        wrote.push(NRRCCAP_DSS);
    }
    Ok(wrote)
}

/// Register the four periodic jobs.
pub fn spawn(tasks: &TaskManager) {
    tasks.add_periodic(
        "modem.info",
        Duration::from_secs(60),
        Priority::Background,
        Some(Duration::from_secs(15)),
        Box::new(|ctx| run_in_task(ctx, |r| refresh_info(r).map(|st| st.to_json()))),
    );
    tasks.add_periodic(
        "modem.txpower",
        Duration::from_secs(300),
        Priority::Low,
        Some(Duration::from_secs(20)),
        Box::new(|ctx| run_in_task(ctx, |r| refresh_txpower(r).map(|st| st.to_json()))),
    );
    tasks.add_periodic(
        "modem.nr_txpower",
        Duration::from_secs(180),
        Priority::Low,
        Some(Duration::from_secs(20)),
        Box::new(|ctx| run_in_task(ctx, |r| refresh_nr_txpower(r).map(|st| st.to_json()))),
    );
    tasks.add_periodic(
        "modem.endc",
        Duration::from_secs(300),
        Priority::Low,
        Some(Duration::from_secs(20)),
        Box::new(|ctx| run_in_task(ctx, |r| refresh_endc(r).map(|st| st.to_json()))),
    );
}
