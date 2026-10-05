//! Signal API routes.
//!
//! `signal.get` is the shared contract for LuCI, the WebUI and the CLI:
//! cache-first (the collectors keep the topic warm), with one bounded refresh
//! when the cache is still cold. No frontend parses `^HCSQ` any more.

use crate::api::registry::{ApiCtx, Route};
use crate::core::error::BackendError;
use crate::core::json::{self, Value};
use crate::modules::signal::service;
use crate::modules::signal::state::SignalState;

/// Routes contributed by this module.
pub fn routes() -> Vec<Route> {
    vec![
        Route {
            name: "signal.get",
            handler: get,
        },
        Route {
            name: "signal.cached",
            handler: cached,
        },
    ]
}

/// Human-readable text rendering for the CLI (same registry, no second code
/// path: the text is formatted from the domain model).
pub fn render_text(state: &SignalState) -> String {
    state.to_text()
}

fn state_from_value(v: &Value) -> SignalState {
    match v {
        Value::Obj(m) => service::from_json(m),
        _ => SignalState::default(),
    }
}

/// Cache-first domain read.
fn get(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    if let Some(st) = service::cached(ctx.cache) {
        if !st.is_empty() {
            return Ok(st.to_json());
        }
    }
    let refresh = ctx.refresh();
    let st = service::refresh(&refresh)?;
    Ok(st.to_json())
}

/// Cache-only read: never touches the modem, always answers immediately.
fn cached(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let mut m = std::collections::BTreeMap::new();
    match service::cached(ctx.cache) {
        Some(st) => {
            m.insert("value".to_string(), st.to_json());
            m.insert("available".to_string(), Value::Bool(true));
        }
        None => {
            m.insert("value".to_string(), Value::Obj(Default::default()));
            m.insert("available".to_string(), Value::Bool(false));
        }
    }
    let (_, fresh) = ctx.cache.get(crate::state::bus::TOPIC_SIGNAL);
    m.insert(
        "freshness".to_string(),
        json::str_val(fresh.name()),
    );
    Ok(Value::Obj(m))
}

/// Helper for CLI text output (used by `api::cli`).
pub fn text_from_value(v: &Value) -> String {
    render_text(&state_from_value(v))
}
