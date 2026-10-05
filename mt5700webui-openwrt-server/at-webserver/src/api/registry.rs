//! Unified backend API registry.
//!
//! Every capability the frontends may use is a `Route` registered here by the
//! module that owns it. The RPC transport (WebSocket + newline-JSON control
//! socket, i.e. what uhttpd/rpcd bridge for LuCI) and the CLI both dispatch
//! through this table, so:
//!
//! * adding a feature = add a module + register its routes;
//! * no transport, and no frontend, ever re-implements backend behaviour;
//! * the frontends cannot see AT commands, the scheduler or the cache keys.
//!
//! ```text
//! LuCI ─┐                        ┌─ modules::signal::api::routes()
//!       ├─ api::dispatch ───────┤
//! WebUI ┘   (rpc / cli)         └─ modules::<name>::api::routes()
//! ```

use crate::core::channel::AtChannel;
use crate::core::error::BackendError;
use crate::core::json::Value;
use crate::state::bus::EventBus;
use crate::state::cache::StateCache;
use crate::state::refresh::RefreshCtx;
use std::sync::Arc;

/// Everything a route handler may use. Handlers never touch the serial port,
/// the arbiter or a frontend-specific format.
pub struct ApiCtx<'a> {
    pub channel: &'a dyn AtChannel,
    pub cache: &'a Arc<StateCache>,
    pub bus: &'a Arc<EventBus>,
}

impl<'a> ApiCtx<'a> {
    pub fn new(
        channel: &'a dyn AtChannel,
        cache: &'a Arc<StateCache>,
        bus: &'a Arc<EventBus>,
    ) -> Self {
        ApiCtx {
            channel,
            cache,
            bus,
        }
    }

    /// A refresh context for the module service backing this route.
    pub fn refresh(&self) -> RefreshCtx<'a> {
        RefreshCtx::new(self.channel, self.cache, self.bus).with_source("api")
    }
}

/// A route handler: `(ctx, params) -> domain JSON`.
pub type Handler = fn(&ApiCtx, &Value) -> Result<Value, BackendError>;

/// One named capability contributed by a module.
pub struct Route {
    /// Fully qualified method name, e.g. `signal.get`.
    pub name: &'static str,
    pub handler: Handler,
}

/// The complete route table, composed from the modules. Adding a module means
/// adding one line here.
pub fn routes() -> Vec<Route> {
    let mut v: Vec<Route> = Vec::new();
    v.extend(crate::modules::signal::api::routes());
    v.extend(crate::modules::network::api::routes());
    v.extend(crate::modules::cell::api::routes());
    v.extend(crate::modules::sim::api::routes());
    v.extend(crate::modules::modem::api::routes());
    v.extend(crate::modules::traffic::api::routes());
    v.extend(crate::modules::system::api::routes());
    v
}

/// All registered method names (diagnostics / tests).
pub fn route_names() -> Vec<&'static str> {
    routes().iter().map(|r| r.name).collect()
}

/// Dispatch one API call. Unknown methods are a client error, never a panic.
pub fn dispatch(ctx: &ApiCtx, method: &str, params: &Value) -> Result<Value, BackendError> {
    for route in routes() {
        if route.name == method {
            return (route.handler)(ctx, params);
        }
    }
    Err(BackendError::InvalidParameter(format!(
        "unknown API method: {}",
        method
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::channel::AtChannel;

    struct NoModem;
    impl AtChannel for NoModem {
        fn query(&self, _c: &str) -> Result<String, BackendError> {
            Err(BackendError::ModemUnavailable)
        }
        fn query_timeout(&self, _c: &str, _t: std::time::Duration) -> Result<String, BackendError> {
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
    }

    #[test]
    fn every_route_is_reachable_and_unknown_methods_fail_cleanly() {
        let cache = Arc::new(StateCache::new());
        let bus = EventBus::new();
        let ch = NoModem;
        let ctx = ApiCtx::new(&ch, &cache, &bus);
        for name in route_names() {
            // A cold cache with no modem must still answer with a domain
            // object (the frontends render placeholders), never panic.
            let out = dispatch(&ctx, name, &Value::Null);
            assert!(out.is_ok(), "route {} failed: {:?}", name, out.err());
        }
        let err = dispatch(&ctx, "nope.nope", &Value::Null).unwrap_err();
        assert_eq!(err.code(), "INVALID_PARAMETER");
    }
}
