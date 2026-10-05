//! Network API routes.
//!
//! `network.get` (operator + access technology + registration) and
//! `registration.get` are what the dashboard, the network page and the CLI
//! now read; `network.cached`/`registration.cached` never touch the modem.

use crate::api::registry::{ApiCtx, Route};
use crate::core::error::BackendError;
use crate::core::json::{self, Value};
use crate::core::task::Priority;
use crate::modules::network::commands::{CGREG_DETAILED, CGPADDR, DHCP_V4, DHCP_V6, IPV6CAP};
use crate::modules::network::parser::{
    parse_cgpaddr, parse_dhcp_v4, parse_dhcp_v6, parse_ipv6cap,
};
use crate::modules::network::service;
use crate::modules::network::state::NetworkState;
use crate::state::refresh::{ReadErrors, RefreshCtx};
use std::time::Duration;
use crate::state::bus::TOPIC_NETWORK;

/// Routes contributed by this module.
pub fn routes() -> Vec<Route> {
    vec![
        Route::display("network.get", get),
        Route::display("network.cached", cached),
        Route::display("registration.get", registration),
        Route::on_demand("network.pdp", pdp),
        Route::on_demand("network.dhcp", dhcp),
        Route::on_demand("network.registration_urc", registration_urc),
    ]
}

/// Cache-first domain read; one bounded refresh when the cache is cold.
fn get(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    if let Some(st) = service::cached(ctx.cache) {
        if !st.is_empty() {
            return Ok(st.to_json());
        }
    }
    let refresh = ctx.refresh();
    let st = service::refresh(&refresh)?;
    Ok(st.to_json())
}

/// Cache-only read: always answers immediately.
fn cached(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let mut m = std::collections::BTreeMap::new();
    match service::cached(ctx.cache) {
        Some(st) => {
            m.insert("value".to_string(), st.to_json());
            m.insert("available".to_string(), Value::Bool(true));
        }
        None => {
            m.insert("value".to_string(), NetworkState::default().to_json());
            m.insert("available".to_string(), Value::Bool(false));
        }
    }
    let (_, fresh) = ctx.cache.get(TOPIC_NETWORK);
    m.insert("freshness".to_string(), json::str_val(fresh.name()));
    Ok(Value::Obj(m))
}

/// Registration topic on its own (flat `{state,tac,ci,...}` object, the shape
/// both frontends already consume from the cache).
fn registration(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    if let Some(reg) = service::cached_registration(ctx.cache) {
        if !reg.is_empty() {
            return Ok(reg.to_json());
        }
    }
    let refresh = ctx.refresh();
    Ok(service::refresh_registration(&refresh).to_json())
}

/// Activated PDP addresses (`AT+CGPADDR`), decoded here and nowhere else.
///
/// On-demand read: the diagnostics panel asks when the user hits refresh, so
/// there is no topic and no polling — but the decode is the module's, which is
/// what keeps the manual refresh and the pushed topics in agreement.
fn pdp(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    let raw = refresh.read(
        CGPADDR,
        Duration::from_secs(8),
        Duration::from_secs(10),
        crate::core::task::Priority::Interactive,
    )?;
    let addresses: Vec<Value> = parse_cgpaddr(&raw).iter().map(|a| a.to_json()).collect();
    let mut m = std::collections::BTreeMap::new();
    m.insert("addresses".to_string(), Value::Arr(addresses));
    Ok(Value::Obj(m))
}

/// Bounded read of one DHCP/IPv6-capability command; the caller decides what
/// to do with a failure (partial answers are still answers).
fn read_lease(refresh: &RefreshCtx<'_>, command: &str) -> Result<String, BackendError> {
    refresh.read(
        command,
        Duration::from_secs(8),
        Duration::from_secs(10),
        Priority::Normal,
    )
}

/// Data-call addresses as the firmware's DHCP client reports them
/// (`AT^DHCP?` + `AT^DHCPV6?` + `AT^IPV6CAP?`).
///
/// On-demand: the Info page asks once per load. The hex decoding and the field
/// names of both families are the module's, the page only renders them. A
/// partially answering modem still produces a useful object; the error is only
/// reported when *nothing* could be read.
fn dhcp(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    let mut m = std::collections::BTreeMap::new();
    let mut errors = ReadErrors::new();
    let v4 = errors.note(read_lease(&refresh, DHCP_V4));
    let v6 = errors.note(read_lease(&refresh, DHCP_V6));
    let cap = errors.note(read_lease(&refresh, IPV6CAP));
    if let Some(lease) = v4.as_deref().and_then(parse_dhcp_v4) {
        m.insert("ipv4".to_string(), lease.to_json());
    }
    if let Some(lease) = v6.as_deref().and_then(parse_dhcp_v6) {
        m.insert("ipv6".to_string(), lease.to_json());
    }
    if let Some(code) = cap.as_deref().and_then(parse_ipv6cap) {
        m.insert("ipv6_capability".to_string(), json::num_val(code));
    }
    if m.is_empty() {
        if let Some(e) = errors.into_option() {
            return Err(e);
        }
    }
    Ok(Value::Obj(m))
}

/// Ask for detailed PS registration reports (idempotent write).
///
/// The page-load side effect the WebUI used to perform with a raw
/// `AT+CGREG=2`: it is modem configuration, so it belongs to the module that
/// owns registration.
fn registration_urc(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    refresh.action(CGREG_DETAILED)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("enabled".to_string(), Value::Bool(true));
    Ok(Value::Obj(m))
}
