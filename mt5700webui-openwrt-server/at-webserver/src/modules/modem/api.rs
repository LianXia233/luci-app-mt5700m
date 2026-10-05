//! Modem API routes.

use crate::api::registry::{ApiCtx, Route};
use crate::core::error::BackendError;
use crate::core::json::{self, Value};
use crate::modules::modem::service;
use crate::state::bus::TOPIC_MODEM;

/// Routes contributed by this module.
pub fn routes() -> Vec<Route> {
    vec![
        Route {
            name: "modem.get",
            handler: get,
        },
        Route {
            name: "modem.cached",
            handler: cached,
        },
        Route {
            name: "modem.txpower",
            handler: txpower,
        },
        Route {
            name: "modem.endc",
            handler: endc,
        },
        Route {
            name: "modem.nr_txpower",
            handler: nr_txpower,
        },
    ]
}

/// Identity: cache-first with one bounded refresh when cold.
fn get(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let cached = service::cached(ctx.cache);
    if let Some(st) = &cached {
        if !st.is_empty() {
            return Ok(st.to_json());
        }
    }
    let refresh = ctx.refresh();
    Ok(service::refresh_info(&refresh)?.to_json())
}

/// Cache-only identity read.
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
                crate::modules::modem::state::ModemState::default().to_json(),
            );
            m.insert("available".to_string(), Value::Bool(false));
        }
    }
    let (_, fresh) = ctx.cache.get(TOPIC_MODEM);
    m.insert("freshness".to_string(), json::str_val(fresh.name()));
    Ok(Value::Obj(m))
}

/// Transmit power: cache-first domain read.
fn txpower(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    if let Some(st) = service::cached_txpower(ctx.cache) {
        if !st.is_empty() {
            return Ok(st.to_json());
        }
    }
    let refresh = ctx.refresh();
    Ok(service::refresh_txpower(&refresh)?.to_json())
}

/// EN-DC status (always one bounded read; the topic is cheap to refresh).
fn endc(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    Ok(service::refresh_endc(&refresh)?.to_json())
}

/// NR per-carrier transmit power.
fn nr_txpower(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    Ok(service::refresh_nr_txpower(&refresh)?.to_json())
}
