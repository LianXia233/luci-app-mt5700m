//! System service: chip-temperature refresh.
//!
//! `^CHIPTEMP?` is fast but its *queue* time is long while slow commands hold
//! the port, so the read runs at `Low` priority with a 12 s AT timeout and no
//! retry (a retry would burn the port twice). A failed tick re-publishes the
//! last good reading instead of clearing the card — the historical behaviour
//! that kept temperature from dropping to "—".

use crate::core::error::BackendError;
use crate::core::json::Value;
use crate::core::task::Priority;
use crate::modules::system::commands::CHIPTEMP;
use crate::modules::system::parser::parse_chiptemp;
use crate::modules::system::state::TemperatureState;
use crate::scheduler::channel::run_in_task;
use crate::scheduler::jobs::TaskManager;
use crate::state::bus::{EVENT_TEMPERATURE_UPDATED, TOPIC_TEMPERATURE};
use crate::state::refresh::RefreshCtx;
use std::time::Duration;

const AT_TIMEOUT: Duration = Duration::from_secs(12);
const QUEUED_TIMEOUT: Duration = Duration::from_secs(8);
const PERIOD: Duration = Duration::from_secs(60);

/// Refresh the temperature topic (never fails on a slow/absent modem).
pub fn refresh(ctx: &RefreshCtx) -> Result<TemperatureState, BackendError> {
    let text = ctx.read(CHIPTEMP, AT_TIMEOUT, QUEUED_TIMEOUT, Priority::Low)?;
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
