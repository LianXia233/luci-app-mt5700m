//! Signal service: refresh policy + state publication.
//!
//! The collector writes the domain JSON into the shared cache under the
//! `signal` topic and publishes `signal.updated`; the API then answers
//! `signal.get` from the cache (zero AT) or triggers one bounded refresh when
//! the cache is still cold.

use crate::core::error::BackendError;
use crate::core::json::Value;
use crate::core::task::Priority;
use crate::modules::signal::commands::HCSQ;
use crate::modules::signal::parser;
use crate::modules::signal::state::SignalState;
use crate::scheduler::channel::TaskChannel;
use crate::scheduler::jobs::TaskManager;
use crate::state::bus::{EVENT_SIGNAL_UPDATED, TOPIC_SIGNAL};
use crate::state::cache::StateCache;
use crate::state::refresh::RefreshCtx;
use std::sync::Arc;
use std::time::Duration;

/// `^HCSQ?` needs ~4 s on the MT5700M: a 3 s fast query would time out, retry
/// twice and hold the channel for the whole window.
const AT_TIMEOUT: Duration = Duration::from_secs(12);
const QUEUED_TIMEOUT: Duration = Duration::from_secs(8);
const BACKOFF: Duration = Duration::from_secs(120);
const PERIOD: Duration = Duration::from_secs(15);

/// Refresh the signal topic. Never fails on a slow/absent modem: on timeout it
/// re-publishes the last known value instead of clearing the card.
pub fn refresh(ctx: &RefreshCtx) -> Result<SignalState, BackendError> {
    let raw = match ctx.slow("signal", HCSQ, AT_TIMEOUT, QUEUED_TIMEOUT, BACKOFF) {
        Some(text) => text,
        None => {
            ctx.stale(TOPIC_SIGNAL, EVENT_SIGNAL_UPDATED);
            return Ok(SignalState::default());
        }
    };
    let state = parser::parse(&raw);
    if state.is_empty() {
        ctx.stale(TOPIC_SIGNAL, EVENT_SIGNAL_UPDATED);
        return Ok(state);
    }
    ctx.store(TOPIC_SIGNAL, EVENT_SIGNAL_UPDATED, &state.to_json());
    Ok(state)
}

/// Cache-first read for the API: the last domain value, if any.
pub fn cached(cache: &Arc<StateCache>) -> Option<SignalState> {
    match cache.get(TOPIC_SIGNAL) {
        (Some(Value::Obj(m)), _) => Some(from_json(&m)),
        _ => None,
    }
}

/// Rebuild the typed state from its JSON form (cache / API transport).
pub fn from_json(m: &std::collections::BTreeMap<String, Value>) -> SignalState {
    let mut st = SignalState::default();
    if let Some(v) = m.get("sysmode").and_then(|v| v.as_str()) {
        st.sysmode = v.to_string();
    }
    st.rssi = m.get("rssi").and_then(|v| v.as_i64());
    st.rsrp = m.get("rsrp").and_then(|v| v.as_i64());
    st.rsrq = m.get("rsrq").and_then(|v| v.as_f64());
    st.sinr = m.get("sinr").and_then(|v| v.as_f64());
    st.rscp = m.get("rscp").and_then(|v| v.as_i64());
    st.ecio = m.get("ecio").and_then(|v| v.as_f64());
    st
}

/// Register this module's periodic refresh on the shared task manager.
pub fn spawn(tasks: &TaskManager) {
    tasks.add_periodic(
        "signal.refresh",
        PERIOD,
        Priority::Normal,
        Some(PERIOD),
        Box::new(|ctx| {
            let channel = TaskChannel::new(ctx);
            let refresh = RefreshCtx::new(&channel, &ctx.cache, &ctx.bus);
            match refresh_signal(&refresh) {
                Ok(()) => Ok(Value::Null),
                Err(e) => Err(e),
            }
        }),
    );
}

/// Adapter used by the periodic job (returns the bare `TaskResult`).
fn refresh_signal(ctx: &RefreshCtx) -> Result<(), BackendError> {
    refresh(ctx).map(|_| ())
}
