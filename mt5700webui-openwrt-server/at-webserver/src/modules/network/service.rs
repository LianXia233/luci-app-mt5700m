//! Network service: refresh policy for the `network` and `registration`
//! topics, plus registration of the periodic jobs.
//!
//! Two jobs, matching the cadences the frontends were tuned against:
//!   * `network.registration`, 20 s — C5GREG first (tac/ci/AcT/NSSAI), falling
//!     back to CEREG then CREG; the topic is always written (Null when nothing
//!     parsed) so consumers keep a stable TTL;
//!   * `network.refresh`, 30 s — operator (`+COPS?`, 60 s failure backoff) and
//!     system mode (`^SYSINFOEX`, 300 s failure backoff), both non-fatal.

use crate::core::error::BackendError;
use crate::core::json::Value;
use crate::core::task::Priority;
use crate::modules::network::commands::{self, COPS, SYSINFOEX};
use crate::modules::network::parser;
use crate::modules::network::state::{NetworkState, RegistrationState};
use crate::scheduler::channel::run_in_task;
use crate::scheduler::jobs::TaskManager;
use crate::state::bus::{
    EVENT_NETWORK_UPDATED, EVENT_REGISTRATION_UPDATED, TOPIC_NETWORK, TOPIC_REGISTRATION,
};
use crate::state::cache::StateCache;
use crate::state::refresh::RefreshCtx;
use std::sync::Arc;
use std::time::Duration;

const REG_AT_TIMEOUT: Duration = Duration::from_secs(8);
const REG_QUEUED_TIMEOUT: Duration = Duration::from_secs(6);
const REG_PERIOD: Duration = Duration::from_secs(20);
const PERIOD: Duration = Duration::from_secs(30);
/// Backoff keys are part of the observable cache (`snapshot()` exposes them to
/// the diagnostics page), so they keep their historical names.
const COPS_BACKOFF_KEY: &str = "network_cops";
const SYSINFO_BACKOFF_KEY: &str = "network_sysinfo";

/// Refresh the registration topic (`+CxxREG`).
pub fn refresh_registration(ctx: &RefreshCtx) -> RegistrationState {
    let mut reg = RegistrationState::default();
    for cmd in commands::REG_QUERIES {
        if let Ok(text) = ctx.read(cmd, REG_AT_TIMEOUT, REG_QUEUED_TIMEOUT, Priority::Normal) {
            let t = text.trim();
            if !t.is_empty() && commands::has_registration_line(t) {
                reg = parser::parse_registration(t);
                break;
            }
        }
    }
    ctx.store(
        TOPIC_REGISTRATION,
        EVENT_REGISTRATION_UPDATED,
        &reg.to_json(),
    );
    reg
}

/// Refresh operator + access technology for the `network` topic.
pub fn refresh(ctx: &RefreshCtx) -> Result<NetworkState, BackendError> {
    let mut st = NetworkState {
        registration: cached_registration(ctx.cache).unwrap_or_default(),
        ..Default::default()
    };

    // COPS is slow on the MT5700M (8 s+), so it is backoff-protected instead of
    // retried: during the backoff window the previous value is kept.
    if let Some(cops) = ctx.slow(
        COPS_BACKOFF_KEY,
        COPS,
        Duration::from_secs(8),
        Duration::from_secs(6),
        Duration::from_secs(60),
    ) {
        st.operator = parser::parse_cops_operator(&cops);
        st.sysmode = parser::parse_cops_rat(&cops);
    }
    if let Some(sysinfo) = ctx.slow(
        SYSINFO_BACKOFF_KEY,
        SYSINFOEX,
        Duration::from_secs(6),
        Duration::from_secs(5),
        Duration::from_secs(300),
    ) {
        st.sysmode_detail = parser::parse_sysinfo_mode(&sysinfo);
    }

    // Publish even when empty: the previous value survives through the cache
    // TTL, and a genuine "no service" answer must be visible to the UI.
    ctx.store(TOPIC_NETWORK, EVENT_NETWORK_UPDATED, &st.to_json());
    Ok(st)
}

/// Cache-first read for the API: the last published domain value.
pub fn cached(cache: &Arc<StateCache>) -> Option<NetworkState> {
    let (value, _) = cache.get(TOPIC_NETWORK);
    match value {
        Some(Value::Obj(m)) => {
            let mut st = NetworkState::from_json(&m);
            st.registration = cached_registration(cache).unwrap_or_default();
            Some(st)
        }
        _ => None,
    }
}

/// The registration topic on its own (the cell card reads TAC/CI from it).
pub fn cached_registration(cache: &Arc<StateCache>) -> Option<RegistrationState> {
    match cache.get(TOPIC_REGISTRATION) {
        (Some(Value::Obj(m)), _) => Some(RegistrationState {
            state: m.get("state").and_then(|v| v.as_i64()).unwrap_or(0) as u8,
            tac: str_field(&m, "tac"),
            ci: str_field(&m, "ci"),
            act: str_field(&m, "act"),
            nssai: str_field(&m, "nssai"),
            mcc: str_field(&m, "mcc"),
            mnc: str_field(&m, "mnc"),
            lac: str_field(&m, "lac"),
        }),
        _ => None,
    }
}

fn str_field(m: &std::collections::BTreeMap<String, Value>, key: &str) -> Option<String> {
    m.get(key).and_then(|v| v.as_str()).map(|s| s.to_string())
}

/// Register both periodic jobs on the shared task manager.
pub fn spawn(tasks: &TaskManager) {
    tasks.add_periodic(
        "network.registration",
        REG_PERIOD,
        Priority::Normal,
        Some(REG_PERIOD),
        Box::new(|ctx| {
            run_in_task(ctx, |r| {
                refresh_registration(r);
                Ok(Value::Null)
            })
        }),
    );
    tasks.add_periodic(
        "network.refresh",
        PERIOD,
        Priority::Normal,
        Some(PERIOD),
        Box::new(|ctx| run_in_task(ctx, |r| refresh(r).map(|st| st.to_json()))),
    );
}
