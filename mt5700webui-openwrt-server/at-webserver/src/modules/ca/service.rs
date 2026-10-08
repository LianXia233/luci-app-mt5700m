//! Carrier-aggregation service: on-demand refresh, published as the `ca` topic.
//!
//! There is deliberately **no periodic job**. The three queries cost ~3.4 s
//! (`^HFREQINFO?`), ~12 s (`^CASCELLINFO?`) and up to 8 s (`^MONSSC`) on the
//! modem, so polling them would eat the duty-cycle budget the rest of the
//! backend needs. Instead the topic is filled when a frontend actually opens
//! the carrier panel (`ca.get`) and served from cache afterwards
//! (`ca.cached`), with a per-command backoff so repeated clicks cannot hammer a
//! modem that is not answering.

use crate::core::error::BackendError;
use crate::core::json::Value;
use crate::modules::ca::commands::{CASCELLINFO, HFREQINFO, MONSSC};
use crate::modules::ca::parser;
use crate::modules::ca::state::CaState;
use crate::state::bus::{EVENT_CA_UPDATED, TOPIC_CA};
use crate::state::refresh::RefreshCtx;
use std::sync::Arc;
use std::time::Duration;

/// Decode fresh answers into the `ca` topic. Failures degrade to a partial
/// picture (whatever answered) instead of dropping the panel.
pub fn refresh(ctx: &RefreshCtx) -> Result<CaState, BackendError> {
    let hfreq = ctx.slow(
        "ca_hfreq",
        HFREQINFO,
        Duration::from_secs(15),
        Duration::from_secs(10),
        Duration::from_secs(60),
    );
    let cascell = ctx.slow(
        "ca_cascell",
        CASCELLINFO,
        Duration::from_secs(20),
        Duration::from_secs(10),
        Duration::from_secs(120),
    );
    let monssc = ctx.slow(
        "ca_monssc",
        MONSSC,
        Duration::from_secs(12),
        Duration::from_secs(10),
        Duration::from_secs(300),
    );

    // Nothing answered: keep the previous picture rather than publishing an
    // empty carrier list over a good one.
    if hfreq.is_none() && cascell.is_none() && monssc.is_none() {
        let stale = ctx.stale(TOPIC_CA, EVENT_CA_UPDATED);
        return Ok(match stale {
            Value::Obj(m) => CaState::from_json(&m),
            _ => CaState::default(),
        });
    }

    let st = parser::parse(
        hfreq.as_deref().unwrap_or(""),
        cascell.as_deref().unwrap_or(""),
        monssc.as_deref().unwrap_or(""),
    );
    ctx.store(TOPIC_CA, EVENT_CA_UPDATED, &st.to_json());
    Ok(st)
}

/// Cache-first read for the API and the CLI.
pub fn cached(cache: &Arc<crate::state::cache::StateCache>) -> Option<CaState> {
    match cache.get(TOPIC_CA) {
        (Some(Value::Obj(m)), _) => Some(CaState::from_json(&m)),
        _ => None,
    }
}
