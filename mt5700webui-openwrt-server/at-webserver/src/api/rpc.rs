//! Transport plumbing for the unified API.
//!
//! One registry call, three envelopes — because the WebSocket (WebUI), the
//! newline-JSON control socket (LuCI ucode bridge / CLI) and JSON-RPC each
//! have their own historical response shape. The *behaviour* is shared: no
//! transport ever re-implements a capability.

use crate::api::registry::{self, ApiCtx};
use crate::core::error::BackendError;
use crate::core::json::{self, Value};
use crate::scheduler::arbiter::AtArbiter;
use crate::scheduler::channel::DirectChannel;
use crate::state::bus::EventBus;
use crate::state::cache::StateCache;
use std::sync::Arc;

/// Run one API route against the live daemon state.
pub fn call(
    arbiter: &Arc<AtArbiter>,
    cache: &Arc<StateCache>,
    bus: &Arc<EventBus>,
    method: &str,
    params: &Value,
) -> Result<Value, BackendError> {
    let channel = DirectChannel::new(arbiter);
    let ctx = ApiCtx::new(&channel, cache, bus);
    registry::dispatch(&ctx, method, params)
}

fn error_object(err: &BackendError) -> Value {
    crate::core::error::error_json(err)
}

/// WebSocket envelope: `{success, data}` / `{success:false, error}`.
pub fn ws_response(
    arbiter: &Arc<AtArbiter>,
    cache: &Arc<StateCache>,
    bus: &Arc<EventBus>,
    method: &str,
    params: &Value,
) -> Value {
    let mut m = std::collections::BTreeMap::new();
    match call(arbiter, cache, bus, method, params) {
        Ok(data) => {
            m.insert("success".to_string(), Value::Bool(true));
            m.insert("data".to_string(), data);
        }
        Err(e) => {
            m.insert("success".to_string(), Value::Bool(false));
            m.insert("error".to_string(), json::str_val(&e.message()));
            m.insert("code".to_string(), json::str_val(e.code()));
            m.insert("retryable".to_string(), Value::Bool(e.retryable()));
        }
    }
    Value::Obj(m)
}

/// Control-socket envelope (also used by the CLI client): `{ok, result}`.
pub fn control_response(
    arbiter: &Arc<AtArbiter>,
    cache: &Arc<StateCache>,
    bus: &Arc<EventBus>,
    method: &str,
    params: &Value,
) -> Value {
    let mut m = std::collections::BTreeMap::new();
    match call(arbiter, cache, bus, method, params) {
        Ok(data) => {
            m.insert("ok".to_string(), Value::Bool(true));
            m.insert("result".to_string(), data);
        }
        Err(e) => {
            m.insert("ok".to_string(), Value::Bool(false));
            m.insert("error".to_string(), json::str_val(&e.message()));
            m.insert("code".to_string(), json::str_val(e.code()));
        }
    }
    Value::Obj(m)
}

/// Whether a CLI/WS command string addresses the API (`api.<module>.<verb>`).
pub fn is_api_method(command: &str) -> bool {
    command.trim().starts_with("api.")
}
