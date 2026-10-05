//! Traffic service: PDCP snapshot (30 s) and interface counters (5 s).
//!
//! The rate collector is pure sysfs + one shell-out, so it runs every 5 s
//! without touching the AT port at all.

use crate::core::error::BackendError;
use crate::core::json::Value;
use crate::core::task::Priority;
use crate::modules::traffic::commands::PDCP;
use crate::modules::traffic::netrate;
use crate::modules::traffic::parser;
use crate::modules::traffic::state::{NetRateState, PdcpState};
use crate::scheduler::channel::run_in_task;
use crate::scheduler::jobs::TaskManager;
use crate::state::bus::{EVENT_NETRATE_UPDATED, EVENT_TRAFFIC_UPDATED, TOPIC_NETRATE, TOPIC_TRAFFIC};
use crate::state::refresh::RefreshCtx;
use std::time::Duration;

const PERIOD: Duration = Duration::from_secs(30);
const NETRATE_PERIOD: Duration = Duration::from_secs(5);

/// Refresh the PDCP traffic topic.
pub fn refresh(ctx: &RefreshCtx) -> Result<PdcpState, BackendError> {
    let text = ctx.query(PDCP)?;
    let st = text
        .lines()
        .find_map(parser::parse_pdcp)
        .unwrap_or_default();
    ctx.store(TOPIC_TRAFFIC, EVENT_TRAFFIC_UPDATED, &st.to_json());
    Ok(st)
}

/// Refresh the interface-counter topic (no AT involved).
pub fn refresh_netrate(ctx: &RefreshCtx) -> Result<NetRateState, BackendError> {
    let st = netrate::collect();
    ctx.store(TOPIC_NETRATE, EVENT_NETRATE_UPDATED, &st.to_json());
    Ok(st)
}

/// Cache-first read for the API.
pub fn cached(cache: &std::sync::Arc<crate::state::cache::StateCache>) -> Option<PdcpState> {
    match cache.get(TOPIC_TRAFFIC) {
        (Some(Value::Obj(m)), _) => {
            let fields = m
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<Vec<_>>();
            Some(PdcpState {
                fields,
                ul_bytes: None,
                dl_bytes: None,
            })
        }
        _ => None,
    }
}

/// Cache-only netrate read.
pub fn cached_netrate(cache: &std::sync::Arc<crate::state::cache::StateCache>) -> Option<Value> {
    match cache.get(TOPIC_NETRATE) {
        (Some(v @ Value::Obj(_)), _) => Some(v),
        _ => None,
    }
}

/// Register both periodic jobs.
pub fn spawn(tasks: &TaskManager) {
    tasks.add_periodic(
        "traffic.refresh",
        PERIOD,
        Priority::Low,
        Some(Duration::from_secs(15)),
        Box::new(|ctx| run_in_task(ctx, |r| refresh(r).map(|st| st.to_json()))),
    );
    tasks.add_periodic(
        "traffic.netrate",
        NETRATE_PERIOD,
        Priority::Background,
        Some(Duration::from_secs(5)),
        Box::new(|ctx| run_in_task(ctx, |r| refresh_netrate(r).map(|st| st.to_json()))),
    );
}
