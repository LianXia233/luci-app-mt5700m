//! SIM API routes.
//!
//! The system page and the PIN dialog talk to these routes only: the slot
//! switch sequence, the hot-plug switch, the PIN/card answer and the PIN
//! operations are all module behaviour, never frontend steps.

use crate::api::params::{boolean, required_bool, required_num, required_text, text};
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
        Route::display("sim.slot", slot),
        Route::on_demand("sim.slot_set", slot_set),
        Route::on_demand("sim.hotplug_set", hotplug_set),
        Route::on_demand("sim.pin_status", pin_status),
        Route::on_demand("sim.pin_apply", pin_apply),
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
    // A rejected `+CNUM?` still carries an answer when the card has no MSISDN
    // stored (`+CME ERROR: 22`), exactly like `+CPIN?`'s "no card" branch — so
    // the reply text is kept and classified instead of being dropped.
    let raw = match refresh.query(CNUM) {
        Ok(text) => text,
        Err(BackendError::AtRejected(text)) => text,
        Err(e) => return Err(e),
    };
    let mut st = service::cached(ctx.cache).unwrap_or_default();
    st.number = parser::parse_cnum(&raw);
    st.number_state = parser::cnum_number_state(&raw);
    ctx.cache.set(TOPIC_SIM, st.to_json(), "api");
    ctx.bus
        .publish(TOPIC_SIM, crate::state::bus::EVENT_SIM_UPDATED, st.to_json());
    Ok(st.to_json())
}

/// Active slot and hot-plug switch, read from the periodic snapshot (with one
/// bounded refresh when it is still cold).
fn slot(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let st = match service::cached(ctx.cache) {
        Some(st) if st.slot.is_some() || st.hotplug.is_some() => st,
        _ => {
            let refresh = ctx.refresh();
            service::refresh(&refresh)?
        }
    };
    let mut m = std::collections::BTreeMap::new();
    if let Some(v) = st.slot {
        m.insert("slot".to_string(), json::num_val(v));
    }
    if let Some(v) = st.hotplug {
        m.insert("hotplug".to_string(), Value::Bool(v));
    }
    Ok(Value::Obj(m))
}

/// Switch the active SIM slot (`0` = external, `1` = internal).
fn slot_set(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let target = required_num(params, "slot")?;
    let refresh = ctx.refresh();
    service::switch_slot(&refresh, target)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("switched".to_string(), Value::Bool(true));
    m.insert("slot".to_string(), json::num_val(target));
    Ok(Value::Obj(m))
}

/// Enable/disable hot-plug detection.
fn hotplug_set(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let on = required_bool(params, "hotplug")?;
    let refresh = ctx.refresh();
    service::set_hotplug(&refresh, on)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("applied".to_string(), Value::Bool(true));
    m.insert("hotplug".to_string(), Value::Bool(on));
    Ok(Value::Obj(m))
}

/// On-demand PIN/card answer: `+CPIN?` plus the `^SIMSQ?` refinement and the
/// `+CLCK` PIN-lock state the PIN card renders.
fn pin_status(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    Ok(service::read_pin(&refresh)?.to_json())
}

/// Run one PIN operation.
///
/// Params: `{ "operation": "verify"|"unblock"|"enable"|"disable"|"change",
///           "pin": "…", "newPin": "…", "pin2": false }`.
/// The operation decides which AT command is sent, and the digit/difference
/// rules are validated here (manual 6.3.3, 5.7.3).
fn pin_apply(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let operation = required_text(params, "operation")?;
    let pin = required_text(params, "pin")?;
    let new_pin = text(params, "newPin");
    let pin2 = boolean(params, "pin2").unwrap_or(false);
    let refresh = ctx.refresh();
    service::apply_pin(&refresh, operation, pin, new_pin, pin2)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("applied".to_string(), Value::Bool(true));
    Ok(Value::Obj(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_are_named_after_the_module() {
        for route in routes() {
            assert!(
                route.name.starts_with("sim."),
                "unexpected route name: {}",
                route.name
            );
        }
    }

    #[test]
    fn param_readers_reject_missing_fields() {
        let mut p = std::collections::BTreeMap::new();
        p.insert("operation".to_string(), json::str_val("nope"));
        p.insert("pin".to_string(), json::str_val("1234"));
        let params = Value::Obj(p);
        assert!(required_text(&params, "pin").is_ok());
        assert!(required_num(&params, "slot").is_err());
        assert!(required_bool(&params, "hotplug").is_err());
    }
}
