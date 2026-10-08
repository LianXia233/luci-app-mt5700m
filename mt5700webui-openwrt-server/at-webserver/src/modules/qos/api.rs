//! QoS API routes.

use crate::api::registry::{ApiCtx, Route};
use crate::core::error::BackendError;
use crate::core::json::{self, Value};
use crate::modules::qos::service;
use crate::modules::qos::state::QosState;
use crate::state::bus::TOPIC_QOS;

/// Routes contributed by this module.
pub fn routes() -> Vec<Route> {
    vec![
        Route::display("qos.get", get),
        Route::display("qos.cached", cached),
    ]
}

/// Cache-first domain read; one refresh when the cache is cold or empty.
fn get(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    if let Some(st) = service::cached(ctx.cache) {
        if !st.is_empty() {
            return Ok(st.to_json());
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
            m.insert("value".to_string(), QosState::default().to_json());
            m.insert("available".to_string(), Value::Bool(false));
        }
    }
    let (_, fresh) = ctx.cache.get(TOPIC_QOS);
    m.insert("freshness".to_string(), json::str_val(fresh.name()));
    Ok(Value::Obj(m))
}
