//! QoS service: one on-demand read of the data-session parameters.
//!
//! Every read goes through `RefreshCtx::slow`, so a modem that stops answering
//! is asked once per backoff window instead of once per page load, and the
//! route stays a display route (it always answers; a missing answer simply
//! leaves the previous value in place).

use crate::core::error::BackendError;
use crate::modules::qos::commands::{dsambr, cgeqosrdp, CGACT, CGEQOSRDP_ALL};
use crate::modules::qos::parser::{parse_cgeqosrdp, parse_cgact_active_cid, parse_dsambr};
use crate::modules::qos::state::QosState;
use crate::state::bus::{EVENT_QOS_UPDATED, TOPIC_QOS};
use crate::state::refresh::RefreshCtx;
use std::sync::Arc;
use std::time::Duration;

const AT_TIMEOUT: Duration = Duration::from_secs(10);
const QUEUED_TIMEOUT: Duration = Duration::from_secs(10);

/// Read the three answers into the `qos` topic.
pub fn refresh(ctx: &RefreshCtx) -> Result<QosState, BackendError> {
    let mut st = QosState::default();

    if let Some(text) = ctx.slow("qos_cgact", CGACT, AT_TIMEOUT, QUEUED_TIMEOUT, Duration::from_secs(60))
    {
        st.active_cid = parse_cgact_active_cid(&text);
    }

    // Candidate contexts, in the order the two frontends used them: the active
    // one first (WebUI), then 1 and 8 (the CLI's subscription-rate verb).
    let mut candidates: Vec<u32> = Vec::new();
    if let Some(cid) = st.active_cid {
        candidates.push(cid);
    }
    for cid in [1u32, 8] {
        if !candidates.contains(&cid) {
            candidates.push(cid);
        }
    }
    for cid in candidates {
        if st.ambr_down_kbps.is_some() {
            break;
        }
        let key = format!("qos_ambr{}", cid);
        if let Some(text) = ctx.slow(
            &key,
            &dsambr(cid),
            AT_TIMEOUT,
            QUEUED_TIMEOUT,
            Duration::from_secs(120),
        ) {
            if let Some(ambr) = parse_dsambr(&text) {
                st.ambr_down_kbps = Some(ambr.down_kbps);
                st.ambr_up_kbps = Some(ambr.up_kbps);
                st.ambr_apn = ambr.apn;
            }
        }
    }

    if let Some(text) = ctx.slow(
        "qos_cgeqosrdp",
        CGEQOSRDP_ALL,
        AT_TIMEOUT,
        QUEUED_TIMEOUT,
        Duration::from_secs(120),
    ) {
        st.qci = parse_cgeqosrdp(&text, st.active_cid);
    }
    if st.qci.is_none() {
        if let Some(cid) = st.active_cid {
            if let Some(text) = ctx.slow(
                "qos_cgeqosrdp_cid",
                &cgeqosrdp(cid),
                AT_TIMEOUT,
                QUEUED_TIMEOUT,
                Duration::from_secs(120),
            ) {
                st.qci = parse_cgeqosrdp(&text, Some(cid));
            }
        }
    }

    ctx.store(TOPIC_QOS, EVENT_QOS_UPDATED, &st.to_json());
    Ok(st)
}

/// Cache-first read for the API and the CLI.
pub fn cached(cache: &Arc<crate::state::cache::StateCache>) -> Option<QosState> {
    match cache.get(TOPIC_QOS) {
        (Some(crate::core::json::Value::Obj(m)), _) => Some(QosState::from_json(&m)),
        _ => None,
    }
}
