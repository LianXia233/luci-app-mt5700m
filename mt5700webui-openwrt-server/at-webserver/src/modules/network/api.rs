//! Network API routes.
//!
//! `network.get` (operator + access technology + registration) and
//! `registration.get` are what the dashboard, the network page and the CLI
//! now read; `network.cached`/`registration.cached` never touch the modem.

use crate::api::registry::{ApiCtx, Route};
use crate::core::error::BackendError;
use crate::core::json::{self, Value};
use crate::modules::network::commands::CGPADDR;
use crate::modules::network::parser::parse_cgpaddr;
use crate::modules::network::service;
use crate::modules::network::state::NetworkState;
use std::time::Duration;
use crate::state::bus::TOPIC_NETWORK;

/// Routes contributed by this module.
pub fn routes() -> Vec<Route> {
    vec![
        Route::display("network.get", get),
        Route::display("network.cached", cached),
        Route::display("registration.get", registration),
        Route::on_demand("network.pdp", pdp),
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

/// Activated PDP addresses (`AT+CGPADDR`), decoded here and nowhere else.
///
/// On-demand read: the diagnostics panel asks when the user hits refresh, so
/// there is no topic and no polling — but the decode is the module's, which is
/// what keeps the manual refresh and the pushed topics in agreement.
fn pdp(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    let raw = refresh.read(
        CGPADDR,
        Duration::from_secs(8),
        Duration::from_secs(10),
        crate::core::task::Priority::Interactive,
    )?;
    let addresses: Vec<Value> = parse_cgpaddr(&raw).iter().map(|a| a.to_json()).collect();
    let mut m = std::collections::BTreeMap::new();
    m.insert("addresses".to_string(), Value::Arr(addresses));
    Ok(Value::Obj(m))
}
