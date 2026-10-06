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
use crate::scheduler::jobs::TaskManager;
use crate::state::bus::EventBus;
use crate::state::cache::StateCache;
use std::sync::Arc;

/// Run one API route against the live daemon state.
///
/// `tasks` is the daemon's task manager, optional because only task routes
/// (`cell.scan_*`) need it and every transport must still be able to serve the
/// rest. Callers that have it pass it in; there is no global to look it up in.
pub fn call(
    arbiter: &Arc<AtArbiter>,
    cache: &Arc<StateCache>,
    bus: &Arc<EventBus>,
    tasks: Option<&Arc<TaskManager>>,
    method: &str,
    params: &Value,
) -> Result<Value, BackendError> {
    let channel = DirectChannel::new(arbiter);
    let mut ctx = ApiCtx::new(&channel, cache, bus);
    if let Some(tasks) = tasks {
        ctx = ctx.with_tasks(tasks);
    }
    registry::dispatch(&ctx, method, params)
}

/// WebSocket envelope: `{success, data}` / `{success:false, error}`.
pub fn ws_response(
    arbiter: &Arc<AtArbiter>,
    cache: &Arc<StateCache>,
    bus: &Arc<EventBus>,
    tasks: Option<&Arc<TaskManager>>,
    method: &str,
    params: &Value,
) -> Value {
    let mut m = std::collections::BTreeMap::new();
    match call(arbiter, cache, bus, tasks, method, params) {
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
    tasks: Option<&Arc<TaskManager>>,
    method: &str,
    params: &Value,
) -> Value {
    let mut m = std::collections::BTreeMap::new();
    match call(arbiter, cache, bus, tasks, method, params) {
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

/// Split `api.<route> [<json params>]` into the route name and its parameters.
///
/// The WebSocket/LuCI command path is a plain text line, so parameters ride
/// along as a trailing JSON object: `api.ca.get {"refresh":true}`. Anything
/// unparsable (or absent) means "no parameters", never an error — a route that
/// takes no parameters must keep working when called as `api.signal.get`.
pub fn split_api_command(command: &str) -> (String, Value) {
    let trimmed = command.trim();
    match trimmed.split_once(char::is_whitespace) {
        None => (trimmed.to_string(), Value::Null),
        Some((method, tail)) => {
            let params = json::parse(tail.trim()).unwrap_or(Value::Null);
            (method.to_string(), params)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_command_splits_route_and_params() {
        let (m, p) = split_api_command("api.signal.get");
        assert_eq!(m, "api.signal.get");
        assert!(matches!(p, Value::Null));

        let (m, p) = split_api_command("api.ca.get {\"refresh\":true}");
        assert_eq!(m, "api.ca.get");
        assert_eq!(p.get("refresh").and_then(|v| v.as_bool()), Some(true));

        // Broken JSON must not turn into an error: the route decides.
        let (m, p) = split_api_command("api.modem.endc {oops");
        assert_eq!(m, "api.modem.endc");
        assert!(matches!(p, Value::Null));
    }
}
