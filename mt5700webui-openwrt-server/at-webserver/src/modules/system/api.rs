//! System API routes.
//!
//! The system page's cards talk to these routes: the temperature array, the
//! board switches (NIC rate, power management), the factory reset and the
//! serial/network link kind.

use crate::api::params::{required_bool, required_num};
use crate::api::registry::{ApiCtx, Route};
use crate::core::error::BackendError;
use crate::core::json::{self, Value};
use crate::modules::system::service;
use crate::modules::system::state::{DeviceControlState, TemperatureState};
use crate::state::bus::TOPIC_TEMPERATURE;

/// Routes contributed by this module.
pub fn routes() -> Vec<Route> {
    vec![
        Route::display("system.temperature", temperature),
        Route::display("system.temperature.cached", temperature_cached),
        Route::display("system.device_control", device_control),
        Route::on_demand("system.nic_rate_set", nic_rate_set),
        Route::on_demand("system.power_control_set", power_control_set),
        Route::on_demand("system.factory_reset", factory_reset),
        Route::display("system.service_mode", service_mode),
    ]
}

/// Cache-first temperature read.
fn temperature(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    if let Some(st) = service::cached(ctx.cache) {
        if !st.is_empty() {
            return Ok(st.to_json());
        }
    }
    let refresh = ctx.refresh();
    Ok(service::refresh(&refresh)?.to_json())
}

/// Cache-only temperature read.
fn temperature_cached(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let mut m = std::collections::BTreeMap::new();
    match service::cached(ctx.cache) {
        Some(st) => {
            m.insert("value".to_string(), st.to_json());
            m.insert("available".to_string(), Value::Bool(true));
        }
        None => {
            m.insert("value".to_string(), TemperatureState::default().to_json());
            m.insert("available".to_string(), Value::Bool(false));
        }
    }
    let (_, fresh) = ctx.cache.get(TOPIC_TEMPERATURE);
    m.insert("freshness".to_string(), json::str_val(fresh.name()));
    Ok(Value::Obj(m))
}

/// Board switches (`^TDPCIELANCFG?`, `^TDPMCFG?`).
///
/// Display route: when the modem is missing or busy the answer is an empty
/// object, which the page reads as "keep showing what you have" — the same thing
/// it did when it could not parse the replies.
fn device_control(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    match service::read_device_control(&refresh) {
        Ok(st) => Ok(st.to_json()),
        Err(_) => Ok(DeviceControlState::default().to_json()),
    }
}

/// Set the NIC rate; takes effect after a reboot.
fn nic_rate_set(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let rate = required_num(params, "rate")?;
    let refresh = ctx.refresh();
    service::set_nic_rate(&refresh, rate)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("applied".to_string(), Value::Bool(true));
    m.insert("nic_rate".to_string(), json::num_val(rate));
    Ok(Value::Obj(m))
}

/// Enable/disable the PCIe controller power management.
fn power_control_set(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let on = required_bool(params, "enabled")?;
    let refresh = ctx.refresh();
    service::set_power_control(&refresh, on)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("applied".to_string(), Value::Bool(true));
    m.insert("power_control".to_string(), Value::Bool(on));
    Ok(Value::Obj(m))
}

/// Restore the AT configuration defaults (`AT&F`); the modem is not restarted.
fn factory_reset(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    service::factory_reset(&refresh)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("restored".to_string(), Value::Bool(true));
    Ok(Value::Obj(m))
}

/// How the daemon reaches the modem (`serial` / `network`).
///
/// The fact is recorded once at startup by the transport layer
/// (`core::modem`), so no page has to probe the modem with `AT+CONNECT?` to
/// learn whether AT is served over the serial port or over the network.
fn service_mode(_ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let mut m = std::collections::BTreeMap::new();
    m.insert("mode".to_string(), json::str_val(crate::core::modem::link()));
    Ok(Value::Obj(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_are_named_after_the_module() {
        for route in routes() {
            assert!(
                route.name.starts_with("system."),
                "unexpected route name: {}",
                route.name
            );
        }
    }
}
