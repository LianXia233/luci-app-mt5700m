//! SIM API routes.

use crate::api::registry::{ApiCtx, Route};
use crate::core::error::BackendError;
use crate::core::json::{self, Value};
use crate::modules::sim::commands::CNUM;
use crate::modules::sim::parser;
use crate::modules::sim::service;
use crate::modules::sim::state::SimState;
use crate::state::bus::TOPIC_SIM;

/// Routes contributed by this module.
pub fn routes() -> Vec<Route> {
    vec![
        Route::display("sim.get", get),
        Route::display("sim.cached", cached),
        Route::on_demand("sim.number", number),
    ]
}

/// Cache-first SIM read.
fn get(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    if let Some(st) = service::cached(ctx.cache) {
        if !st.is_empty() {
            return Ok(st.to_json());
        }
    }
    let refresh = ctx.refresh();
    Ok(service::refresh(&refresh)?.to_json())
}

/// Cache-only SIM read.
fn cached(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let mut m = std::collections::BTreeMap::new();
    match service::cached(ctx.cache) {
        Some(st) => {
            m.insert("value".to_string(), st.to_json());
            m.insert("available".to_string(), Value::Bool(true));
        }
        None => {
            m.insert("value".to_string(), SimState::default().to_json());
            m.insert("available".to_string(), Value::Bool(false));
        }
    }
    let (_, fresh) = ctx.cache.get(TOPIC_SIM);
    m.insert("freshness".to_string(), json::str_val(fresh.name()));
    Ok(Value::Obj(m))
}

/// On-demand subscriber number (`+CNUM`), persisted into the SIM topic so the
/// UI keeps it until the card changes.
fn number(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    let raw = refresh.query(CNUM)?;
    let mut st = service::cached(ctx.cache).unwrap_or_default();
    st.number = parser::parse_cnum(&raw);
    ctx.cache
        .set(TOPIC_SIM, st.to_json(), "api");
    ctx.bus
        .publish(TOPIC_SIM, crate::state::bus::EVENT_SIM_UPDATED, st.to_json());
    Ok(st.to_json())
}
