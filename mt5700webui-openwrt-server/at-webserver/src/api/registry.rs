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
use crate::scheduler::jobs::TaskManager;
use crate::state::bus::EventBus;
use crate::state::cache::StateCache;
use crate::state::refresh::RefreshCtx;
use std::sync::Arc;

/// Everything a route handler may use. Handlers never touch the serial port,
/// the arbiter or a frontend-specific format.
///
/// `tasks` is the daemon's task manager, handed to routes whose work is a
/// *task* rather than a read (`cell.scan_*`): long-running, cancellable and
/// observable, instead of a request that blocks its transport for minutes. It
/// is optional because transports without a task manager (unit tests, a future
/// reader-only client) still serve every other route.
pub struct ApiCtx<'a> {
    pub channel: &'a dyn AtChannel,
    pub cache: &'a Arc<StateCache>,
    pub bus: &'a Arc<EventBus>,
    pub tasks: Option<&'a Arc<TaskManager>>,
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
            tasks: None,
        }
    }

    /// Same context, with the daemon's task manager available to task routes.
    pub fn with_tasks(mut self, tasks: &'a Arc<TaskManager>) -> Self {
        self.tasks = Some(tasks);
        self
    }

    /// The task manager, or a clear error for a route that cannot work without
    /// one (the caller is a transport that never registered tasks).
    pub fn require_tasks(&self) -> Result<&'a Arc<TaskManager>, BackendError> {
        self.tasks.ok_or_else(|| {
            BackendError::Internal("this transport has no task manager".to_string())
        })
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
    /// Whether calling this route needs a reachable modem.
    ///
    /// A display route answers from the cache and, when the topic is cold,
    /// performs at most one bounded refresh — it must still return domain JSON
    /// (placeholders and all) when the modem is missing, because the frontends
    /// render it on every page load. An on-demand route is an explicit,
    /// user-triggered modem read (`sim.number`, `network.pdp`): failing with a
    /// modem error is the honest answer and the UI shows it as such.
    pub requires_modem: bool,
}

impl Route {
    /// Cache-first display route: always answers with domain JSON.
    pub const fn display(name: &'static str, handler: Handler) -> Route {
        Route {
            name,
            handler,
            requires_modem: false,
        }
    }

    /// Explicit on-demand modem read; may fail with a modem error.
    pub const fn on_demand(name: &'static str, handler: Handler) -> Route {
        Route {
            name,
            handler,
            requires_modem: true,
        }
    }
}

/// The complete route table, composed from the modules. Adding a module means
/// adding one line here.
pub fn routes() -> Vec<Route> {
    let mut v: Vec<Route> = Vec::new();
    v.extend(crate::modules::signal::api::routes());
    v.extend(crate::modules::network::api::routes());
    v.extend(crate::modules::ca::api::routes());
    v.extend(crate::modules::cell::api::routes());
    v.extend(crate::modules::beam::api::routes());
    v.extend(crate::modules::sim::api::routes());
    v.extend(crate::modules::modem::api::routes());
    v.extend(crate::modules::traffic::api::routes());
    v.extend(crate::modules::qos::api::routes());
    v.extend(crate::modules::system::api::routes());
    v.extend(crate::modules::sms::api::routes());
    v
}

/// All registered method names (diagnostics / tests).
pub fn route_names() -> Vec<&'static str> {
    routes().iter().map(|r| r.name).collect()
}

/// Dispatch one API call. Unknown methods are a client error, never a panic.
///
/// The WebSocket/LuCI command path spells a route as `api.<name>` (see
/// `api::rpc::is_api_method`), the control socket and the CLI pass `<name>`.
/// Both must reach the same route, so the prefix is stripped here — one place
/// instead of one per transport.
pub fn dispatch(ctx: &ApiCtx, method: &str, params: &Value) -> Result<Value, BackendError> {
    let method = method.strip_prefix("api.").unwrap_or(method);
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
    use crate::core::json;

    /// A transport that never answers: enough to build a real arbiter and task
    /// manager, which the task routes (`cell.scan_start`) require.
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
        fn send_sms_pdu(
            &self,
            _parts: &[crate::core::channel::SmsPart],
        ) -> Result<String, BackendError> {
            Err(BackendError::ModemUnavailable)
        }
    }

    /// Plausible parameters for the routes that take any.
    ///
    /// The sweep below has to reach the modem call, not stop at parameter
    /// validation: a route that only misbehaves when called properly would
    /// otherwise slip through (this is how the lock routes' required `rat`
    /// went unnoticed).
    fn sample_params(name: &str) -> Value {
        let pairs = |items: &[(&str, Value)]| -> Value {
            let mut m = std::collections::BTreeMap::new();
            for (k, v) in items {
                m.insert(k.to_string(), v.clone());
            }
            Value::Obj(m)
        };
        match name {
            "network.lock_get" => pairs(&[("rat", json::str_val("lte"))]),
            "network.lock_apply" => pairs(&[
                ("rat", json::str_val("lte")),
                ("lock_type", json::num_val(0)),
            ]),
            "network.c5goption_set" => pairs(&[
                ("nr_sa_support_flag", json::num_val(1)),
                ("nr_dc_mode", json::num_val(1)),
                ("gc_access_mode", json::num_val(1)),
            ]),
            "sim.slot_set" => pairs(&[("slot", json::num_val(0))]),
            "sim.activation_set" => pairs(&[("active", Value::Bool(true))]),
            "sim.hotplug_set" => pairs(&[("hotplug", Value::Bool(true))]),
            "sim.pin_apply" => pairs(&[
                ("operation", json::str_val("verify")),
                ("pin", json::str_val("1234")),
            ]),
            "system.nic_rate_set" => pairs(&[("rate", json::num_val(1))]),
            "system.led_set" => pairs(&[("enabled", Value::Bool(true))]),
            "system.thermal_thresholds_set" => pairs(&[(
                "thresholds",
                Value::Arr(vec![
                    json::num_val(60),
                    json::num_val(70),
                    json::num_val(65),
                    json::num_val(80),
                    json::num_val(75),
                    json::num_val(90),
                    json::num_val(85),
                    json::num_val(100),
                    json::num_val(95),
                ]),
            )]),
            "system.thermal_log_set" => pairs(&[
                ("serial", Value::Bool(true)),
                ("file", Value::Bool(true)),
            ]),
            "system.power_control_set" => pairs(&[("enabled", Value::Bool(true))]),
            "modem.imei_set" => pairs(&[("imei", json::str_val("861234567890123"))]),
            "sms.ussd_send" => pairs(&[("code", json::str_val("*133#"))]),
            "network.schedule_set" => pairs(&[("check_interval", json::num_val(60))]),
            "network.radio_set" => pairs(&[("airplane", Value::Bool(true))]),
            "network.syscfg_set" => pairs(&[
                ("acqorder", json::str_val("080302")),
                ("band", json::str_val("3FFFFFFF")),
                ("roam", json::num_val(1)),
                ("srvdomain", json::num_val(2)),
                ("lteband", json::str_val("7FFFFFFFFFFFFFFF")),
            ]),
            "system.thermal_set" => pairs(&[
                ("enabled", Value::Bool(true)),
                ("interval", json::num_val(2)),
            ]),
            "modem.nr_capability_set" => pairs(&[("ca", Value::Bool(true))]),
            "sms.send" => pairs(&[
                ("number", json::str_val("+8613800138000")),
                ("text", json::str_val("hello")),
            ]),
            "sms.delete" => pairs(&[("index", json::num_val(1))]),
            "sms.storage_set" => pairs(&[
                ("read", json::str_val("SM")),
                ("write", json::str_val("SM")),
                ("receive", json::str_val("SM")),
            ]),
            "sms.center_set" => pairs(&[("number", json::str_val("+8613800138000"))]),
            "sms.ims_set" => pairs(&[("enabled", Value::Bool(true))]),
            "sms.analyze" => pairs(&[("text", json::str_val("hello"))]),
            "system.fota_start" => pairs(&[("url", json::str_val("http://fota.example.com/"))]),
            "cell.scan_start" => pairs(&[
                ("rat", json::str_val("2")),
                ("plmn", json::str_val("46000")),
            ]),
            _ => Value::Null,
        }
    }

    #[test]
    fn every_route_is_reachable_and_unknown_methods_fail_cleanly() {
        let cache = Arc::new(StateCache::new());
        let bus = EventBus::new();
        let arbiter = crate::scheduler::arbiter::AtArbiter::new(Arc::new(NoTransport));
        let tasks = crate::scheduler::jobs::TaskManager::new(arbiter, cache.clone(), bus.clone());
        let ch = NoModem;
        let ctx = ApiCtx::new(&ch, &cache, &bus).with_tasks(&tasks);
        let mut broken: Vec<String> = Vec::new();
        for route in routes() {
            let out = dispatch(&ctx, route.name, &sample_params(route.name));
            match (route.requires_modem, out) {
                // A cold cache with no modem must still answer with a domain
                // object (the frontends render placeholders), never panic.
                (false, Err(e)) => broken.push(format!("{}: {} ({})", route.name, e.message(), e.code())),
                // On-demand reads may fail, but only with a modem error the UI
                // can explain — never a routing/parameter bug.
                (true, Err(e)) => {
                    if e.code() == "INVALID_PARAMETER" || e.code() == "INTERNAL" {
                        broken.push(format!("{}: unexpected {} ({})", route.name, e.message(), e.code()));
                    }
                }
                _ => {}
            }
        }
        assert!(broken.is_empty(), "routes misbehaving without a modem: {:#?}", broken);
        // The WebSocket/LuCI spelling must reach the same handler.
        assert!(dispatch(&ctx, "api.signal.get", &Value::Null).is_ok());
        let err = dispatch(&ctx, "nope.nope", &Value::Null).unwrap_err();
        assert_eq!(err.code(), "INVALID_PARAMETER");
    }
}
