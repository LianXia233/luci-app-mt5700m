//! Cell service: refresh policy for the `cell` topic.
//!
//! Cadence and backoffs are the ones the frontends were tuned against:
//! `^HFREQINFO?` every 120 s (15 s AT timeout, 120 s failure backoff) and
//! `^MONSC` on the same tick with a 600 s backoff because NR firmware answers
//! neither quickly nor reliably. `^MONSC` itself is now the only source of
//! LAC/CI; when it is unavailable they are filled from the network module's
//! registration state, which is what kept the cell card from showing "—".

use crate::core::error::BackendError;
use crate::core::json::Value;
use crate::core::task::Priority;
use crate::modules::cell::commands::{HFREQINFO, MONSC};
use crate::modules::cell::parser::{parse_hfreqinfo, parse_monsc};
use crate::modules::cell::state::CellState;
use crate::modules::network::{service as network_service, state::NetworkState};
use crate::scheduler::channel::run_in_task;
use crate::scheduler::jobs::TaskManager;
use crate::state::bus::{EVENT_CELL_UPDATED, TOPIC_CELL, TOPIC_NETWORK};
use crate::state::refresh::RefreshCtx;
use std::time::Duration;

const PERIOD: Duration = Duration::from_secs(120);
const HFREQ_BACKOFF: Duration = Duration::from_secs(120);
const MONSC_BACKOFF: Duration = Duration::from_secs(600);

/// Refresh the serving-cell topic.
pub fn refresh(ctx: &RefreshCtx) -> Result<CellState, BackendError> {
    let mut st = CellState::default();

    if let Some(text) = ctx.slow(
        "cell_hfreq",
        HFREQINFO,
        Duration::from_secs(15),
        Duration::from_secs(10),
        HFREQ_BACKOFF,
    ) {
        parse_hfreqinfo(&text, &mut st);
    }

    // Operator comes from the network module's state: the same COPS answer used
    // to be fetched a second time here under a different backoff key.
    if let (Some(Value::Obj(m)), _) = ctx.cache.get(TOPIC_NETWORK) {
        st.operator = NetworkState::from_json(&m).operator;
    }

    if let Some(text) = ctx.slow(
        "cell_monsc",
        MONSC,
        Duration::from_secs(12),
        Duration::from_secs(10),
        MONSC_BACKOFF,
    ) {
        st.raw = Some(text.trim().to_string());
        parse_monsc(&text, &mut st);
    }

    // Registration is the stable source of TAC/CI: fill LAC/CID when MONSC is
    // unavailable, without overwriting what MONSC reported.
    if let Some(reg) = network_service::cached_registration(ctx.cache) {
        if st.lac.is_none() {
            st.lac = reg.tac.clone();
        }
        if st.cid.is_none() {
            st.cid = reg.ci.clone();
        }
    }

    ctx.store(TOPIC_CELL, EVENT_CELL_UPDATED, &st.to_json());
    Ok(st)
}

/// Cache-first read for the API.
pub fn cached(cache: &std::sync::Arc<crate::state::cache::StateCache>) -> Option<CellState> {
    match cache.get(TOPIC_CELL) {
        (Some(Value::Obj(m)), _) => Some(CellState::from_json(&m)),
        _ => None,
    }
}

/// Register the periodic refresh job.
pub fn spawn(tasks: &TaskManager) {
    tasks.add_periodic(
        "cell.refresh",
        PERIOD,
        Priority::Low,
        Some(Duration::from_secs(20)),
        Box::new(|ctx| run_in_task(ctx, |r| refresh(r).map(|st| st.to_json()))),
    );
}
