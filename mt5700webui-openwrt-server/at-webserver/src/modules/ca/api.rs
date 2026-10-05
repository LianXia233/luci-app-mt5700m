//! Carrier-aggregation API routes.

use crate::api::registry::{ApiCtx, Route};
use crate::core::error::BackendError;
use crate::core::json::{self, Value};
use crate::modules::ca::service;
use crate::modules::ca::state::CaState;
use crate::state::bus::TOPIC_CA;

/// Routes contributed by this module.
pub fn routes() -> Vec<Route> {
    vec![
        Route {
            name: "ca.get",
            handler: get,
        },
        Route {
            name: "ca.cached",
            handler: cached,
        },
    ]
}

/// Cache-first read; one bounded refresh when the cache is cold or empty.
///
/// The explicit `refresh` parameter lets the carrier panel's refresh button ask
/// for live numbers without a second route.
fn get(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let force = params
        .get("refresh")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !force {
        if let Some(st) = service::cached(ctx.cache) {
            if !st.is_empty() {
                return Ok(st.to_json());
            }
        }
    }
    let refresh = ctx.refresh();
    Ok(service::refresh(&refresh)?.to_json())
}

/// Cache-only read: never touches the modem.
fn cached(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let mut m = std::collections::BTreeMap::new();
    match service::cached(ctx.cache) {
        Some(st) => {
            m.insert("value".to_string(), st.to_json());
            m.insert("available".to_string(), Value::Bool(true));
        }
        None => {
            m.insert("value".to_string(), CaState::default().to_json());
            m.insert("available".to_string(), Value::Bool(false));
        }
    }
    let (_, fresh) = ctx.cache.get(TOPIC_CA);
    m.insert("freshness".to_string(), json::str_val(fresh.name()));
    Ok(Value::Obj(m))
}
