//! Network API routes.
//!
//! `network.get` (operator + access technology + registration) and
//! `registration.get` are what the dashboard, the network page and the CLI
//! now read; `network.cached`/`registration.cached` never touch the modem.

use crate::api::registry::{ApiCtx, Route};
use crate::core::error::BackendError;
use crate::core::json::{self, Value};
use crate::core::task::Priority;
use crate::modules::network::commands::{self, CGREG_DETAILED, CGPADDR, DHCP_V4, DHCP_V6, IPV6CAP};
use crate::modules::network::parser::{
    parse_cgpaddr, parse_dhcp_v4, parse_dhcp_v6, parse_ipv6cap,
};
use crate::modules::network::state::LockKind;
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
        Route::on_demand("network.lock_get", lock_get),
        Route::on_demand("network.lock_apply", lock_apply),
        Route::on_demand("network.c5goption", c5goption),
        Route::on_demand("network.c5goption_set", c5goption_set),
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

// ------------------------------------------------------------- frequency lock

fn parse_kind(params: &Value) -> Option<LockKind> {
    match params.get("rat").and_then(|v| v.as_str()) {
        Some("lte") => Some(LockKind::Lte),
        Some("nr") => Some(LockKind::Nr),
        _ => None,
    }
}

fn num_param(params: &Value, key: &str) -> Option<i64> {
    params.get(key).and_then(|v| v.as_i64())
}

/// Current frequency lock of one RAT (`AT^LTEFREQLOCK?` / `AT^NRFREQLOCK?`).
///
/// On-demand: the Settings page reads both RATs when it opens and after every
/// change. The reply layout (the `mobility,num` row and the per-carrier rows)
/// and the hex PCI are decoded here, never in the page.
fn lock_get(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let kind = parse_kind(params).ok_or_else(|| {
        BackendError::InvalidParameter("network.lock_get needs rat=lte|nr".to_string())
    })?;
    let st = crate::modules::network::service::read_lock(ctx.channel, kind)?;
    Ok(st.to_json())
}

/// One lock request from the frontend, decoded from JSON.
struct LockRequest {
    kind: LockKind,
    lock_type: u8,
    mobility: u8,
    items: Vec<crate::modules::network::state::LockItem>,
}

fn parse_lock_request(row: &Value) -> Result<LockRequest, BackendError> {
    let kind = parse_kind(row).ok_or_else(|| {
        BackendError::InvalidParameter("network.lock_apply needs rat=lte|nr".to_string())
    })?;
    let lock_type = num_param(row, "lock_type").unwrap_or(0);
    if !(0..=3).contains(&lock_type) {
        return Err(BackendError::InvalidParameter(format!(
            "lock_type out of range: {}",
            lock_type
        )));
    }
    let mobility = num_param(row, "mobility").unwrap_or(0).clamp(0, 1) as u8;
    let items: Vec<crate::modules::network::state::LockItem> = row
        .get("items")
        .and_then(|v| v.as_arr())
        .map(|list| {
            list.iter()
                .map(|item| crate::modules::network::state::LockItem {
                    band: num_param(item, "band"),
                    arfcn: num_param(item, "arfcn"),
                    pci: num_param(item, "pci"),
                    scs: num_param(item, "scs"),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(LockRequest {
        kind,
        lock_type: lock_type as u8,
        mobility,
        items,
    })
}

/// Apply one or more frequency locks.
///
/// Params — either a single lock at the top level, or a list:
/// ```json
/// { "rat": "lte"|"nr", "lock_type": 0..3, "mobility": 0|1,
///   "items": [{ "band": 3, "arfcn": 1850, "pci": 100, "scs": 1 }] }
/// { "locks": [ { "rat": "lte", … }, { "rat": "nr", … } ] }
/// ```
/// The AT writes are assembled by the module (grouped CSVs and the SCS default
/// included) and the radio is cycled **once** around all of them by the shared
/// apply sequence — the page sends both RATs in one call exactly as it used to
/// cycle once and write twice, and the CLI/scheduler use the same sequence.
/// The answer reports per-RAT success so the page keeps its "applied anyway"
/// wording for the RAT that failed.
fn lock_apply(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let requests: Vec<LockRequest> = match params.get("locks").and_then(|v| v.as_arr()) {
        Some(list) => {
            let mut out = Vec::new();
            for row in list {
                out.push(parse_lock_request(row)?);
            }
            out
        }
        None => vec![parse_lock_request(params)?],
    };
    if requests.is_empty() {
        return Err(BackendError::InvalidParameter(
            "network.lock_apply needs at least one lock".to_string(),
        ));
    }
    let mut applies = Vec::new();
    for r in &requests {
        let command = commands::lock_command_for(r.kind, r.lock_type, r.mobility, &r.items)
            .ok_or_else(|| {
                BackendError::InvalidParameter(format!(
                    "invalid {} lock parameters",
                    r.kind.as_str()
                ))
            })?;
        applies.push(crate::modules::network::service::LockApply {
            kind: r.kind,
            command,
        });
    }
    let report = crate::modules::network::service::apply_lock(ctx.channel, &applies, true)?;
    let results: Vec<Value> = report
        .outcomes
        .iter()
        .map(|o| {
            let mut m = std::collections::BTreeMap::new();
            m.insert("rat".to_string(), json::str_val(o.kind.as_str()));
            m.insert("applied".to_string(), Value::Bool(o.ok()));
            if let Some(e) = &o.error {
                m.insert("error".to_string(), json::str_val(&e.message()));
                m.insert("code".to_string(), json::str_val(e.code()));
            }
            Value::Obj(m)
        })
        .collect();
    let mut m = std::collections::BTreeMap::new();
    m.insert("cycled_radio".to_string(), Value::Bool(report.cycled_radio));
    m.insert("results".to_string(), Value::Arr(results));
    Ok(Value::Obj(m))
}

/// 5G access mode (`AT^C5GOPTION?`).
///
/// On-demand: the Settings page reads it on load and after every change. The
/// triple is decoded here; the page only maps it to its label.
fn c5goption(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let st = crate::modules::network::service::read_c5goption(ctx.channel)?;
    Ok(st.to_json())
}

/// Write the 5G access mode, cycling the radio around the write.
///
/// Params: `{ "nr_sa_support_flag": 1, "nr_dc_mode": 1, "gc_access_mode": 1 }`
/// (all three are required — a defaulted flag would silently write a mode the
/// user did not choose).
fn c5goption_set(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let mut flags = [0u8; 3];
    for (i, key) in ["nr_sa_support_flag", "nr_dc_mode", "gc_access_mode"]
        .iter()
        .enumerate()
    {
        let v = num_param(params, key).ok_or_else(|| {
            BackendError::InvalidParameter(format!("network.c5goption_set needs {}", key))
        })?;
        if !(0..=15).contains(&v) {
            return Err(BackendError::InvalidParameter(format!(
                "{} out of range: {}",
                key, v
            )));
        }
        flags[i] = v as u8;
    }
    let command = commands::c5goption_write(flags[0], flags[1], flags[2]);
    let (_, cycled) = crate::modules::network::service::write_with_radio_cycle(ctx.channel, &command)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("applied".to_string(), Value::Bool(true));
    m.insert("cycled_radio".to_string(), Value::Bool(cycled));
    Ok(Value::Obj(m))
}
