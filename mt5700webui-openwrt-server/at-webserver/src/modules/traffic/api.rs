//! Traffic API routes.

use crate::api::registry::{ApiCtx, Route};
use crate::core::error::BackendError;
use crate::core::json::{self, Value};
use crate::modules::traffic::service;
use crate::state::bus::{TOPIC_NETRATE, TOPIC_TRAFFIC};

/// Routes contributed by this module.
pub fn routes() -> Vec<Route> {
    vec![
        Route::display("traffic.get", get),
        Route::display("traffic.cached", cached),
        Route::display("traffic.netrate", netrate),
    ]
}

/// Cache-first PDCP read.
fn get(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    if let Some(st) = service::cached(ctx.cache) {
        if !st.is_empty() {
            return Ok(st.to_json());
        }
    }
    let refresh = ctx.refresh();
    Ok(service::refresh(&refresh)?.to_json())
}

/// Cache-only PDCP read.
fn cached(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let mut m = std::collections::BTreeMap::new();
    match service::cached(ctx.cache) {
        Some(st) => {
            m.insert("value".to_string(), st.to_json());
            m.insert("available".to_string(), Value::Bool(true));
        }
        None => {
            m.insert(
                "value".to_string(),
                Value::Obj(Default::default()),
            );
            m.insert("available".to_string(), Value::Bool(false));
        }
    }
    let (_, fresh) = ctx.cache.get(TOPIC_TRAFFIC);
    m.insert("freshness".to_string(), json::str_val(fresh.name()));
    Ok(Value::Obj(m))
}

/// Interface counters + shared accounting report (cache-only, 5 s cadence).
fn netrate(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    if let Some(v) = service::cached_netrate(ctx.cache) {
        return Ok(v);
    }
    let refresh = ctx.refresh();
    Ok(service::refresh_netrate(&refresh)?.to_json())
}

/// Cache-only netrate read used by the CLI/diagnostics surfaces.
pub fn netrate_value(ctx: &ApiCtx) -> Value {
    match ctx.cache.get(TOPIC_NETRATE) {
        (Some(v @ Value::Obj(_)), _) => v,
        _ => Value::Null,
    }
}
