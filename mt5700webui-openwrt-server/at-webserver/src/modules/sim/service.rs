//! SIM service: refresh policy for the `sim` topic.
//!
//! Three independent reads (card status, ICCID, IMSI), each with a 60 s
//! failure backoff so a missing card cannot make the collector hold the port.
//! A card that answers nothing leaves the topic empty rather than stale, which
//! is what the UI shows as "no SIM".

use crate::core::error::BackendError;
use crate::core::json::Value;
use crate::core::task::Priority;
use crate::modules::sim::commands::{CIMI, CPIN, ICCID};
use crate::modules::sim::parser;
use crate::modules::sim::state::SimState;
use crate::scheduler::channel::run_in_task;
use crate::scheduler::jobs::TaskManager;
use crate::state::bus::{EVENT_SIM_UPDATED, TOPIC_SIM};
use crate::state::refresh::RefreshCtx;
use std::time::Duration;

const AT_TIMEOUT: Duration = Duration::from_secs(6);
const QUEUED_TIMEOUT: Duration = Duration::from_secs(5);
const BACKOFF: Duration = Duration::from_secs(60);
const PERIOD: Duration = Duration::from_secs(60);

/// Refresh the SIM topic.
pub fn refresh(ctx: &RefreshCtx) -> Result<SimState, BackendError> {
    let mut st = SimState::default();
    if let Some(text) = ctx.slow("sim_cpin", CPIN, AT_TIMEOUT, QUEUED_TIMEOUT, BACKOFF) {
        st.status = parser::parse_cpin(&text);
    }
    if let Some(text) = ctx.slow("sim_iccid", ICCID, AT_TIMEOUT, QUEUED_TIMEOUT, BACKOFF) {
        st.iccid = parser::parse_iccid(&text);
    }
    if let Some(text) = ctx.slow("sim_cimi", CIMI, AT_TIMEOUT, QUEUED_TIMEOUT, BACKOFF) {
        st.imsi = parser::parse_imsi(&text);
    }
    // Keep a number read earlier by the on-demand route.
    if let Some(prev) = cached(ctx.cache) {
        if st.number.is_none() {
            st.number = prev.number;
        }
    }
    ctx.store(TOPIC_SIM, EVENT_SIM_UPDATED, &st.to_json());
    Ok(st)
}

/// Cache-first read for the API.
pub fn cached(cache: &std::sync::Arc<crate::state::cache::StateCache>) -> Option<SimState> {
    match cache.get(TOPIC_SIM) {
        (Some(Value::Obj(m)), _) => Some(SimState::from_json(&m)),
        _ => None,
    }
}

/// Register the periodic refresh job.
pub fn spawn(tasks: &TaskManager) {
    tasks.add_periodic(
        "sim.refresh",
        PERIOD,
        Priority::Background,
        Some(Duration::from_secs(15)),
        Box::new(|ctx| run_in_task(ctx, |r| refresh(r).map(|st| st.to_json()))),
    );
}
