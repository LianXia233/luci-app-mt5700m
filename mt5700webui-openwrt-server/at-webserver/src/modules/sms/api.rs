//! SMS API routes.
//!
//! The SMS pages (list, conversation, centre number, storage, IMS switch) talk
//! to these routes only. `sms.status` is the page-load snapshot (never fails);
//! everything else is an explicit user action that may report a modem error.
//!
//! The one thing deliberately *not* here is the sent-message cache: the pages
//! label it 本地缓存 and let the user export/import it as a file, so it is
//! browser-local UI state, not modem state. It stays in the frontend.

use crate::api::params::{required_bool, required_num, required_text, text};
use crate::api::registry::{ApiCtx, Route};
use crate::core::error::BackendError;
use crate::core::json::{self, Value};
use crate::modules::sms::service;

/// Routes contributed by this module.
pub fn routes() -> Vec<Route> {
    vec![
        Route::display("sms.status", status),
        Route::on_demand("sms.storage", storage),
        Route::on_demand("sms.list", list),
        Route::on_demand("sms.send", send),
        Route::on_demand("sms.delete", delete),
        Route::on_demand("sms.clear_all", clear_all),
        Route::on_demand("sms.storage_set", storage_set),
        Route::on_demand("sms.center_set", center_set),
        Route::on_demand("sms.ims_set", ims_set),
        // 纯算术，不碰模组：显示类路由（冷启动/模组离线也必须给出统计）。
        Route::display("sms.analyze", analyze),
    ]
}

/// `{enabled, imsOn?, center?, storage?}` — the page-load snapshot.
///
/// Display route: a busy or absent modem yields `{enabled: false}` and the
/// pages keep their current values, exactly like their failed raw reads.
fn status(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    Ok(service::read_status(&refresh).to_json())
}

/// `{read, write, receive, storages}` — where messages are kept and how full
/// each plane is (`+CPMS?`). The settings page and the list header both read
/// this one shape.
fn storage(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    Ok(service::read_storage(&refresh)?.to_json())
}

/// `{messages: [...]}` — the stored messages, decoded.
fn list(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    let messages = service::list(&refresh)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert(
        "messages".to_string(),
        Value::Arr(messages.iter().map(|msg| msg.to_json()).collect()),
    );
    Ok(Value::Obj(m))
}

/// `{sent: true, parts}` — send `text` to `number`.
fn send(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let number = required_text(params, "number")?;
    let text = required_text(params, "text")?;
    let refresh = ctx.refresh();
    let parts = service::send(&refresh, number, text)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("sent".to_string(), Value::Bool(true));
    m.insert("parts".to_string(), json::num_val(parts as i64));
    Ok(Value::Obj(m))
}

/// `{deleted: true}` — delete the message at `index`.
fn delete(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let index = required_num(params, "index")?;
    let refresh = ctx.refresh();
    service::delete(&refresh, index)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("deleted".to_string(), Value::Bool(true));
    Ok(Value::Obj(m))
}

/// `{cleared: [storage...]}` — empty every storage plane the modem reports.
fn clear_all(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    let names = service::clear_all(&refresh)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert(
        "cleared".to_string(),
        Value::Arr(names.iter().map(|n| json::str_val(n)).collect()),
    );
    Ok(Value::Obj(m))
}

/// `{applied: true}` — select the read/write/receive storage planes.
fn storage_set(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let read = required_text(params, "read")?;
    let write = text(params, "write").unwrap_or(read);
    let receive = text(params, "receive").unwrap_or(read);
    let refresh = ctx.refresh();
    service::set_storage(&refresh, read, write, receive)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("applied".to_string(), Value::Bool(true));
    Ok(Value::Obj(m))
}

/// `{applied: true}` — set the service centre number.
fn center_set(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let number = required_text(params, "number")?;
    let refresh = ctx.refresh();
    service::set_center(&refresh, number)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("applied".to_string(), Value::Bool(true));
    Ok(Value::Obj(m))
}

/// `{applied: true}` — run the IMS enable/disable sequence.
fn ims_set(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let enabled = required_bool(params, "enabled")?;
    let refresh = ctx.refresh();
    service::set_ims(&refresh, enabled)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("applied".to_string(), Value::Bool(true));
    Ok(Value::Obj(m))
}

/// `{encoding, chars, parts}` — the compose hint, from the module's codec.
fn analyze(_ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let text = text(params, "text").unwrap_or("");
    Ok(service::analyze(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_needs_no_modem_and_handles_missing_params() {
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
        let Value::Obj(m) = analyze(&ctx, &Value::Null).unwrap() else {
            panic!("object")
        };
        assert_eq!(m.get("parts").and_then(|v| v.as_i64()), Some(0));
        // A status read without a modem is the "not enabled" snapshot.
        let Value::Obj(m) = status(&ctx, &Value::Null).unwrap() else {
            panic!("object")
        };
        assert_eq!(m.get("enabled").and_then(|v| v.as_bool()), Some(false));
        // Explicit actions report the modem error instead of a parameter bug.
        let err = list(&ctx, &Value::Null).unwrap_err();
        assert_ne!(err.code(), "INVALID_PARAMETER");
    }
}
