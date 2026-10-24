//! Time sync API routes.
//!
//! `time.status` is the page snapshot: saved config, runtime state, provider
//! and the unified clock. It never sends an NTP packet and never fails.
//! Writes persist only; `apply` and `sync` are tasks so the request returns a
//! task id and the page observes progress without blocking.
//!
//! The timezone is always the device's; nothing here hardcodes an offset.

use crate::api::registry::{ApiCtx, Route};
use crate::core::error::BackendError;
use crate::core::json::Value;
use crate::modules::timesync::service;

pub fn routes() -> Vec<Route> {
    vec![
        Route::display("time.status", status),
        Route::display("time.config_get", config_get),
        Route::display("time.now", now),
        Route::on_demand("time.config_set", config_set),
        Route::on_demand("time.reset", reset),
        Route::on_demand("time.apply", apply),
        Route::on_demand("time.sync", sync),
    ]
}

fn status(_ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    Ok(service::status_json())
}

fn config_get(_ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    Ok(service::config_get())
}

/// Lightweight clock read for other pages/panels. No provider probing.
fn now(_ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let state = crate::modules::timesync::state::NtpState::load();
    let mut value = match service::time_json() {
        Value::Obj(m) => m,
        other => return Ok(other),
    };
    value.insert(
        "synced".to_string(),
        state
            .synced
            .map(Value::Bool)
            .unwrap_or(Value::Null),
    );
    value.insert(
        "trusted".to_string(),
        Value::Bool(state.trusted_at(crate::core::time::now_unix())),
    );
    value.insert(
        "lastSuccessAt".to_string(),
        state
            .last_success_at
            .map(crate::core::json::num_val)
            .unwrap_or(Value::Null),
    );
    Ok(Value::Obj(value))
}

fn config_set(_ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    service::save(params)
}

fn reset(_ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    service::reset_defaults()
}

fn apply(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let tasks = ctx.require_tasks()?;
    let id = service::start_apply(tasks)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("started".to_string(), Value::Bool(true));
    m.insert("task_id".to_string(), crate::core::json::num_val(id));
    Ok(Value::Obj(m))
}

fn sync(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let tasks = ctx.require_tasks()?;
    let id = service::start_sync(tasks)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("started".to_string(), Value::Bool(true));
    m.insert("task_id".to_string(), crate::core::json::num_val(id));
    Ok(Value::Obj(m))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

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

    #[test]
    fn status_has_the_fields_the_page_needs_without_a_modem() {
        let cache = Arc::new(crate::state::cache::StateCache::new());
        let bus = crate::state::bus::EventBus::new();
        let ch = NoModem;
        let ctx = ApiCtx::new(&ch, &cache, &bus);
        let out = status(&ctx, &Value::Null).unwrap();
        for key in [
            "saved",
            "defaults",
            "state",
            "provider",
            "time",
            "warnings",
            "trusted",
            "verified",
            "pending",
        ] {
            assert!(out.get(key).is_some(), "missing {}", key);
        }
        let saved = out.get("saved").unwrap();
        let servers = saved.get("servers").and_then(|v| v.as_arr()).unwrap();
        assert_eq!(servers.len(), 3);
        assert_eq!(
            servers[0].get("host").and_then(|v| v.as_str()),
            Some("ntp.tencent.com")
        );
        // No provider in the test environment, so nothing may claim sync.
        assert_eq!(out.get("trusted").unwrap(), &Value::Bool(false));
    }

    #[test]
    fn now_is_a_lightweight_clock_object() {
        let cache = Arc::new(crate::state::cache::StateCache::new());
        let bus = crate::state::bus::EventBus::new();
        let ch = NoModem;
        let ctx = ApiCtx::new(&ch, &cache, &bus);
        let out = now(&ctx, &Value::Null).unwrap();
        assert!(out.get("unix").and_then(|v| v.as_i64()).unwrap() > 1_500_000_000);
        assert!(out.get("unixMs").and_then(|v| v.as_i64()).unwrap() > 1_500_000_000_000);
        assert!(!out
            .get("timezone")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .is_empty());
        assert!(!out
            .get("local")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .is_empty());
    }

    struct NoTransport;

    impl crate::scheduler::arbiter::AtTransport for NoTransport {
        fn send(&self, _c: &str, _t: std::time::Duration) -> Result<String, BackendError> {
            Err(BackendError::ModemUnavailable)
        }
        fn send_interruptible(
            &self,
            _c: &str,
            _t: std::time::Duration,
            _cancel: &std::sync::atomic::AtomicBool,
            _abort: Option<&[u8]>,
        ) -> Result<String, BackendError> {
            Err(BackendError::ModemUnavailable)
        }
        fn connected(&self) -> bool {
            false
        }
    }

    #[test]
    fn writes_fail_cleanly_without_uci() {
        let cache = Arc::new(crate::state::cache::StateCache::new());
        let bus = crate::state::bus::EventBus::new();
        let arbiter = crate::scheduler::arbiter::AtArbiter::new(Arc::new(NoTransport));
        let tasks = crate::scheduler::jobs::TaskManager::new(arbiter, cache.clone(), bus.clone());
        let ch = NoModem;
        let ctx = ApiCtx::new(&ch, &cache, &bus).with_tasks(&tasks);
        if crate::core::sys::uci_available() {
            // On a real UCI host this test must not modify the system config.
            return;
        }
        let err = config_set(&ctx, &Value::Null).unwrap_err();
        assert_eq!(err.code(), "UNAVAILABLE");
        let err = sync(&ctx, &Value::Null).unwrap_err();
        assert_eq!(err.code(), "UNAVAILABLE");
        let err = apply(&ctx, &Value::Null).unwrap_err();
        assert_eq!(err.code(), "UNAVAILABLE");
    }
}
