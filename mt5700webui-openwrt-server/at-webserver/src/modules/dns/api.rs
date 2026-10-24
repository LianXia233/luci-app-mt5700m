//! DNS API routes.
//!
//! `dns.status` is the page-load snapshot: it never touches the modem and
//! never fails, because it is rendered on every visit. Writes validate the
//! whole address set before UCI is touched, and application is a task so the
//! request returns immediately with a task id.

use crate::api::registry::{ApiCtx, Route};
use crate::core::error::BackendError;
use crate::core::json::Value;
use crate::modules::dns::service;

pub fn routes() -> Vec<Route> {
    vec![
        // Read-only and platform-independent: always answers with the saved
        // config, last apply state and current interface runtime.
        Route::display("dns.status", status),
        Route::display("dns.config_get", config_get),
        // Writes. `config_set` persists only; `apply` mutates interfaces.
        Route::on_demand("dns.config_set", config_set),
        Route::on_demand("dns.reset", reset),
        Route::on_demand("dns.apply", apply),
    ]
}

fn status(_ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    Ok(service::status_json())
}

/// `{config, defaults, state, applying}` — no ubus/netifd calls, so the form
/// can load even while the network service is restarted.
fn config_get(_ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let config = crate::modules::dns::config::DnsConfig::load();
    let state = crate::modules::dns::state::ApplyState::load();
    let mut m = std::collections::BTreeMap::new();
    m.insert("config".to_string(), config.to_json());
    m.insert(
        "defaults".to_string(),
        crate::modules::dns::config::DnsConfig::defaults_json(),
    );
    m.insert("state".to_string(), state.to_json());
    m.insert(
        "applying".to_string(),
        Value::Bool(service::is_applying()),
    );
    Ok(Value::Obj(m))
}

/// `{saved, config, applied:false}` — params are the nested family blocks.
fn config_set(_ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    service::save(params)
}

/// `{saved, config, defaults:true}` — restore the documented defaults.
fn reset(_ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    service::reset_defaults()
}

/// `{started:true, task_id}` — apply the saved config asynchronously.
fn apply(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let tasks = ctx.require_tasks()?;
    let id = service::start_apply(tasks)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("started".to_string(), Value::Bool(true));
    m.insert("task_id".to_string(), crate::core::json::num_val(id));
    Ok(Value::Obj(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_shape_is_stable_without_a_modem_or_uci() {
        let cache = std::sync::Arc::new(crate::state::cache::StateCache::new());
        let bus = crate::state::bus::EventBus::new();
        struct NoModem;
        impl crate::core::channel::AtChannel for NoModem {
            fn query(&self, _c: &str) -> Result<String, BackendError> {
                Err(BackendError::ModemUnavailable)
            }
            fn query_timeout(
                &self,
                _c: &str,
                _t: std::time::Duration,
            ) -> Result<String, BackendError> {
                Err(BackendError::ModemUnavailable)
            }
            fn query_prio(
                &self,
                _c: &str,
                _t: std::time::Duration,
                _q: std::time::Duration,
                _p: crate::core::task::Priority,
            ) -> Result<String, BackendError> {
                Err(BackendError::ModemUnavailable)
            }
            fn action(&self, _c: &str) -> Result<String, BackendError> {
                Err(BackendError::ModemUnavailable)
            }
            fn send_sms_pdu(
                &self,
                _parts: &[crate::core::channel::SmsPart],
            ) -> Result<String, BackendError> {
                Err(BackendError::ModemUnavailable)
            }
        }
        let ch = NoModem;
        let ctx = ApiCtx::new(&ch, &cache, &bus);
        let out = status(&ctx, &Value::Null).unwrap();
        assert!(out.get("saved").is_some());
        assert!(out.get("verified").is_some());
        assert!(out.get("warnings").is_some());
        // Defaults must be visible even when the section is absent.
        let saved = out.get("saved").unwrap();
        assert_eq!(
            saved
                .get("ipv4")
                .and_then(|v| v.get("primary"))
                .and_then(|v| v.as_str()),
            Some(crate::modules::dns::config::DEFAULT_V4_PRIMARY)
        );
    }
}
