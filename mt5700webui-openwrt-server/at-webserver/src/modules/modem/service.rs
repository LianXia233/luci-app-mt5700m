//! Modem service: identity, transmit power and EN-DC refresh policies.
//!
//! Cadences preserved from the collectors: identity 60 s (`Background`, 60 s
//! failure backoff, 60 s cache TTL so the UI never blanks), TXPOWER and LENDC
//! 300 s (both are unsupported in NR mode and cost 8–12 s when tried), NTXPOWER
//! 180 s (its reply takes ~12 s).

use crate::core::error::BackendError;
use crate::core::json::Value;
use crate::core::task::Priority;
use crate::modules::modem::commands::{ATI, CGSN, LENDC, NTXPOWER, TXPOWER};
use crate::modules::modem::parser;
use crate::modules::modem::state::{EndcState, McsState, ModemState, NrTxPowerState, TxPowerState};
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
