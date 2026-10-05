//! Network API routes.
//!
//! `network.get` (operator + access technology + registration) and
//! `registration.get` are what the dashboard, the network page and the CLI
//! now read; `network.cached`/`registration.cached` never touch the modem.

use crate::api::registry::{ApiCtx, Route};
use crate::core::error::BackendError;
use crate::core::json::{self, Value};
use crate::modules::network::service;
use crate::modules::network::state::NetworkState;
use crate::state::bus::TOPIC_NETWORK;

/// Routes contributed by this module.
pub fn routes() -> Vec<Route> {
    vec![
        Route {
            name: "network.get",
            handler: get,
        },
        Route {
            name: "network.cached",
            handler: cached,
        },
        Route {
            name: "registration.get",
            handler: registration,
        },
    ]
}

/// Cache-first domain read; one bounded refresh when the cache is cold.
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

/// Cache-only read: always answers immediately.
fn cached(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let mut m = std::collections::BTreeMap::new();
    match service::cached(ctx.cache) {
        Some(st) => {
            m.insert("value".to_string(), st.to_json());
            m.insert("available".to_string(), Value::Bool(true));
        }
        None => {
            m.insert("value".to_string(), NetworkState::default().to_json());
            m.insert("available".to_string(), Value::Bool(false));
        }
    }
    let (_, fresh) = ctx.cache.get(TOPIC_NETWORK);
    m.insert("freshness".to_string(), json::str_val(fresh.name()));
    Ok(Value::Obj(m))
}

/// Registration topic on its own (flat `{state,tac,ci,...}` object, the shape
/// both frontends already consume from the cache).
fn registration(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    if let Some(reg) = service::cached_registration(ctx.cache) {
        if !reg.is_empty() {
            return Ok(reg.to_json());
        }
    }
    let refresh = ctx.refresh();
    Ok(service::refresh_registration(&refresh).to_json())
}
