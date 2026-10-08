//! Network API routes.
//!
//! `network.get` (operator + access technology + registration) and
//! `registration.get` are what the dashboard, the network page and the CLI
//! now read; `network.cached`/`registration.cached` never touch the modem.

use crate::api::params::{num, required_num, required_text};
use crate::api::registry::{ApiCtx, Route};
use crate::core::error::BackendError;
use crate::core::json::{self, Value};
use crate::core::task::Priority;
use crate::modules::network::commands::{
    self, CGREG_DETAILED, CGPADDR, DHCP_V4, DHCP_V6, IPV6CAP,
};
use crate::modules::network::parser::{
    self, parse_cgpaddr, parse_dhcp_v4, parse_dhcp_v6, parse_ipv6cap,
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
        Route::display("network.ims", ims),
        Route::on_demand("network.pdp", pdp),
        Route::on_demand("network.dhcp", dhcp),
        // The data-session snapshot (addresses, MTU, dual-stack mode, firmware
        // counters, per-cid sessions) and the counters reset. These replace the
        // `advanced session` text dump the two connection surfaces used to
        // regex, and the `flow-clear` CLI verb behind the "Clear counters"
        // button.
        Route::on_demand("network.session", session),
        Route::on_demand("network.flow_clear", flow_clear),
        Route::on_demand("network.registration_urc", registration_urc),
        Route::on_demand("network.rrc", rrc),
        Route::on_demand("network.lock_get", lock_get),
        Route::on_demand("network.lock_apply", lock_apply),
        Route::on_demand("network.c5goption", c5goption),
        Route::on_demand("network.c5goption_set", c5goption_set),
        Route::display("network.radio", radio),
        Route::on_demand("network.radio_set", radio_set),
        Route::display("network.syscfg", syscfg),
        Route::on_demand("network.syscfg_set", syscfg_set),
        // The dial page is read-only (LuCI owns the writes); these four answer
        // what it renders. `network.interface_cfg` covers the page's two
        // separate ^TDCFG? reads with one.
        Route::display("network.autodial", autodial),
        Route::display("network.usb_mode", usb_mode),
        Route::display("network.interface_cfg", interface_cfg),
        // The advanced page's two interface writes. SETMODE=7 (MBIM) stays
        // rejected — the CLI verb marked it temporarily unsupported and the
        // page's own selector never offered it.
        Route::on_demand("network.usb_mode_set", usb_mode_set),
        Route::on_demand("network.interface_mode_set", interface_mode_set),
        Route::display("network.pdp_contexts", pdp_contexts),
        // 定时锁频：配置在 UCI 里（`scheduler::plan` 每 15 s 读一次生效），
        // 读路由给页面快照，写路由只落盘不碰模组 —— LuCI 的 `AT+SCHED*`
        // 伪命令现在也只是这两条的别名。
        Route::display("network.schedule_get", schedule_get),
        Route::on_demand("network.schedule_set", schedule_set),
        Route::display("network.direct_ip", direct_ip),
        Route::on_demand("network.pdp_set", pdp_set),
        Route::on_demand("network.pdp_remove", pdp_remove),
        Route::on_demand("network.pdp_state", pdp_state),
        Route::on_demand("network.autodial_set", autodial_set),
        Route::on_demand("network.direct_ip_set", direct_ip_set),
        Route::on_demand("network.postroute_set", postroute_set),
        Route::on_demand("network.dmz_set", dmz_set),
    ]
}

/// `{enabled, check_interval, …, night, day, status}` — the schedule panel's
/// snapshot (`modules::network::schedule`).
fn schedule_get(_ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    Ok(crate::modules::network::schedule::read())
}

/// `{applied: true}` — validate and persist a schedule save.
fn schedule_set(_ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    crate::modules::network::schedule::write(params)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("applied".to_string(), Value::Bool(true));
    Ok(Value::Obj(m))
}

/// `{state?, camped?}` — connection/RRC state (`^RRCSTAT?`), the wireless
/// page's "Radio status" card.
///
/// On-demand: it is a live read with no cache of its own, and an unreachable
/// modem is reported as an error (the page shows `--` on that row, like every
/// other failed row).
fn rrc(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let st = crate::modules::network::service::read_rrc(ctx.channel)?;
    Ok(st.to_json())
}

/// Access-technology configuration (`^SYSCFGEX?`): the "网络系统配置" card.
///
/// Display route: an unanswered read (or an unexpected shape) is an empty
/// object, and the page keeps the values it shows.
fn syscfg(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    match service::read_syscfg(&refresh) {
        Ok(st) => Ok(st.to_json()),
        Err(_) => Ok(crate::modules::network::state::SysCfgState::default().to_json()),
    }
}

/// Write the access-technology configuration.
///
/// Params: `{acqorder, band, roam, srvdomain, lteband}` — the five values the
/// card edits; the manual's two reserved trailing arguments stay empty.
fn syscfg_set(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let acqorder = required_text(params, "acqorder")?;
    let band = required_text(params, "band")?;
    let lteband = required_text(params, "lteband")?;
    let roam = required_num(params, "roam")?;
    let srvdomain = required_num(params, "srvdomain")?;
    let refresh = ctx.refresh();
    service::apply_syscfg(&refresh, acqorder, band, roam, srvdomain, lteband)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("applied".to_string(), Value::Bool(true));
    Ok(Value::Obj(m))
}

/// Built-in autodial configuration (`^SETAUTODIAL?`, with the NDIS fallback).
///
/// Display route: unread fields stay absent, so the dial page keeps the values
/// it is showing instead of blanking its cards.
fn autodial(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    match service::read_autodial(&refresh) {
        Ok(st) => Ok(st.to_json()),
        Err(_) => Ok(crate::modules::network::state::AutodialState::default().to_json()),
    }
}

/// USB port mode (`^SETMODE?`).
fn usb_mode(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    match service::read_usb_mode(&refresh) {
        Ok(st) => Ok(st.to_json()),
        Err(_) => Ok(crate::modules::network::state::UsbModeState::default().to_json()),
    }
}

/// Interface configuration + DMZ host (`^TDCFG?`).
fn interface_cfg(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    match service::read_interface_cfg(&refresh) {
        Ok(st) => Ok(st.to_json()),
        Err(_) => Ok(crate::modules::network::state::InterfaceCfgState::default().to_json()),
    }
}

/// Set the USB network-driver profile (`AT^SETMODE=<0-6|8>`).
///
/// SETMODE=7 (MBIM) stays rejected: the CLI verb marked it temporarily
/// unsupported and the page's own selector never offered it.
fn usb_mode_set(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let mode = required_num(params, "mode")?;
    if !(0..=8).contains(&mode) || mode == 7 {
        return Err(BackendError::InvalidParameter(
            "network.usb_mode_set needs mode 0-6 or 8 (7 = MBIM, temporarily unsupported)"
                .to_string(),
        ));
    }
    ctx.refresh().action(&commands::set_mode(mode as u8))?;
    Ok(applied_json())
}

/// Set the interface operating mode (`AT^TDCFG="infcfg","mode",<1|2>`).
fn interface_mode_set(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let mode = required_num(params, "mode")?;
    let command = commands::tdcfg_mode(mode).ok_or_else(|| {
        BackendError::InvalidParameter(
            "network.interface_mode_set needs mode 1 or 2".to_string(),
        )
    })?;
    ctx.refresh().action(&command)?;
    Ok(applied_json())
}

/// PDP context table (`+CGDCONT?` + `+CGACT?`) — `{contexts: [...]}`.
fn pdp_contexts(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    let contexts: Vec<Value> = match service::read_pdp_contexts(&refresh) {
        Ok(list) => list.iter().map(|c| c.to_json()).collect(),
        Err(_) => Vec::new(),
    };
    let mut m = std::collections::BTreeMap::new();
    m.insert("contexts".to_string(), Value::Arr(contexts));
    Ok(Value::Obj(m))
}

/// IP passthrough flag (`^SETDIRECTIP?`) — the dial page's "IP passthrough"
/// select. A missing/unknown answer yields `{}`: the page disables the
/// control, exactly as it did when the frame section was unreadable.
fn direct_ip(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    let mut m = std::collections::BTreeMap::new();
    if let Ok(text) = refresh.query(commands::SETDIRECTIP_QUERY) {
        if let Some(enabled) = parser::parse_directip(&text) {
            m.insert("enabled".to_string(), Value::Bool(enabled));
        }
    }
    Ok(Value::Obj(m))
}

/// Define/overwrite a PDP profile (`AT+CGDCONT=<cid>,"<type>","<apn>"`).
fn pdp_set(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let cid = crate::api::params::required_num(params, "cid")?;
    let pdp_type = crate::api::params::required_text(params, "type")?;
    let apn = crate::api::params::required_text(params, "apn")?;
    let command = commands::cgdcont_set(u32::try_from(cid).unwrap_or(0), pdp_type, apn)
        .ok_or_else(|| {
            BackendError::InvalidParameter(
                "network.pdp_set needs cid 1-11, type IP|IPV6|IPV4V6 and a plain APN up to 99 chars"
                    .to_string(),
            )
        })?;
    ctx.refresh().action(&command)?;
    Ok(applied_json())
}

/// Delete a PDP profile (`AT+CGDCONT=<cid>` — the carrier default returns).
fn pdp_remove(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let cid = crate::api::params::required_num(params, "cid")?;
    let command = commands::cgdcont_remove(u32::try_from(cid).unwrap_or(0)).ok_or_else(|| {
        BackendError::InvalidParameter("network.pdp_remove needs cid 1-11".to_string())
    })?;
    ctx.refresh().action(&command)?;
    Ok(applied_json())
}

/// Activate/deactivate a PDP profile (`AT+CGACT=<state>,<cid>`).
fn pdp_state(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let cid = crate::api::params::required_num(params, "cid")?;
    let active = crate::api::params::required_bool(params, "active")?;
    let command = commands::cgact(active, u32::try_from(cid).unwrap_or(0)).ok_or_else(|| {
        BackendError::InvalidParameter("network.pdp_state needs cid 1-11".to_string())
    })?;
    ctx.refresh().action(&command)?;
    Ok(applied_json())
}

/// Module-side dialing (`AT^SETAUTODIAL=…`). `enabled=false` short-circuits
/// to `AT^SETAUTODIAL=0` and the other fields stay unchecked — the CLI
/// verb's exact behaviour.
fn autodial_set(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let enabled = crate::api::params::required_bool(params, "enabled")?;
    let dial_mode = crate::api::params::num(params, "dialMode").unwrap_or(1);
    let protocol = crate::api::params::text(params, "protocol").unwrap_or("IPV4V6");
    let apn = crate::api::params::text(params, "apn").unwrap_or("");
    let username = crate::api::params::text(params, "username").unwrap_or("");
    let password = crate::api::params::text(params, "password").unwrap_or("");
    let auth = crate::api::params::num(params, "auth").unwrap_or(0);
    let command = commands::setautodial(enabled, dial_mode, protocol, apn, username, password, auth)
        .ok_or_else(|| {
            BackendError::InvalidParameter(
                "network.autodial_set rejected its arguments (dialMode 0-2, protocol IP|IPV6|IPV4V6, auth 0-2, plain fields: apn<=99, username/password<=31)"
                    .to_string(),
            )
        })?;
    ctx.refresh().action(&command)?;
    Ok(applied_json())
}

/// IP passthrough (`AT^SETDIRECTIP=<0|1>`).
fn direct_ip_set(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let enabled = crate::api::params::required_bool(params, "enabled")?;
    ctx.refresh().action(&commands::setdirectip(enabled))?;
    Ok(applied_json())
}

/// Post-routing (`AT^TDCFG="infcfg","PostRoute",<mode>`). Turning it on also
/// clears the inbound filter (`AT^IPFILTERSWITCH=0`), like the CLI verb;
/// if the first write fails the second is not attempted.
fn postroute_set(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let mode = crate::api::params::required_num(params, "mode")?;
    let command =
        commands::tdcfg_postroute(mode).ok_or_else(|| {
            BackendError::InvalidParameter("network.postroute_set needs mode 1|2".to_string())
        })?;
    let refresh = ctx.refresh();
    refresh.action(&command)?;
    let mut filter_cleared = false;
    if mode == 1 {
        refresh.action(commands::IPFILTERSWITCH_OFF)?;
        filter_cleared = true;
    }
    let mut m = std::collections::BTreeMap::new();
    m.insert("applied".to_string(), Value::Bool(true));
    m.insert("filterCleared".to_string(), Value::Bool(filter_cleared));
    Ok(Value::Obj(m))
}

/// DMZ host (`AT^TDCFG="infcfg","dmz","<host>"`); host `"0"` disables.
fn dmz_set(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let host = crate::api::params::required_text(params, "host")?;
    let command = commands::tdcfg_dmz(host).ok_or_else(|| {
        BackendError::InvalidParameter(
            "network.dmz_set needs host \"0\" or a dotted-quad IPv4".to_string(),
        )
    })?;
    ctx.refresh().action(&command)?;
    Ok(applied_json())
}

fn applied_json() -> Value {
    let mut m = std::collections::BTreeMap::new();
    m.insert("applied".to_string(), Value::Bool(true));
    Value::Obj(m)
}

/// Airplane mode (`+CFUN?`): the switch the system page shows.
///
/// Display route: a missing/busy modem answers an empty object, which the page
/// reads as "keep the switch where it is" — the same as a failed raw read.
fn radio(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    let mut m = std::collections::BTreeMap::new();
    if let Ok(text) = refresh.query(commands::CFUN_QUERY) {
        if let Some(state) = parser::parse_cfun(&text) {
            m.insert("airplane".to_string(), Value::Bool(state == 0));
            m.insert("cfun".to_string(), json::num_val(state));
        }
    }
    Ok(Value::Obj(m))
}

/// Turn airplane mode on/off (`AT+CFUN=0|1`).
fn radio_set(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let airplane = crate::api::params::required_bool(params, "airplane")?;
    let refresh = ctx.refresh();
    refresh.action(&commands::cfun(if airplane { 0 } else { 1 }))?;
    // The radio state changed, so registration is stale either way.
    ctx.cache
        .invalidate_many(&[TOPIC_NETWORK, crate::state::bus::TOPIC_REGISTRATION]);
    let mut m = std::collections::BTreeMap::new();
    m.insert("applied".to_string(), Value::Bool(true));
    m.insert("airplane".to_string(), Value::Bool(airplane));
    Ok(Value::Obj(m))
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

/// Data-session snapshot (`^NDISSTATQRY?` + `^DHCP?`/`^DHCPV6?` + `+CGPADDR`
/// + `^IPV6CAP?` + `^CGMTU` + `^DSFLOWQRY` + `^DCONNSTAT?`).
///
/// On-demand: the overview page refreshes it with its poll and the connection
/// page asks once per load. A partially answering modem still produces the
/// object (each command is independent); an entirely silent one is an error so
/// the caller can keep its previous values.
fn session(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    Ok(service::read_session(&refresh)?.to_json())
}

/// Reset the firmware's data-flow counters (`AT^DSFLOWCLR`).
fn flow_clear(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    service::clear_flow(&refresh)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("cleared".to_string(), Value::Bool(true));
    Ok(Value::Obj(m))
}

/// `{enabled?, registered?}` — IMS registration, for the diagnostics block.
///
/// A display route: the page renders it on load, so an unreachable modem is an
/// empty answer (the page shows no value) rather than a failed request that
/// would replace the whole block with an error message.
fn ims(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    match service::read_ims(&refresh) {
        Ok(st) => Ok(st.to_json()),
        Err(_) => Ok(crate::modules::network::state::ImsState::default().to_json()),
    }
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
    let lock_type = num(row, "lock_type").unwrap_or(0);
    if !(0..=3).contains(&lock_type) {
        return Err(BackendError::InvalidParameter(format!(
            "lock_type out of range: {}",
            lock_type
        )));
    }
    let mobility = num(row, "mobility").unwrap_or(0).clamp(0, 1) as u8;
    let items: Vec<crate::modules::network::state::LockItem> = row
        .get("items")
        .and_then(|v| v.as_arr())
        .map(|list| {
            list.iter()
                .map(|item| crate::modules::network::state::LockItem {
                    band: num(item, "band"),
                    arfcn: num(item, "arfcn"),
                    pci: num(item, "pci"),
                    scs: num(item, "scs"),
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
///
/// `"verify": true` additionally polls the lock query the way the CLI's `lock`
/// verb does (8 attempts when the radio was cycled, one probe when it was
/// already off — the firmware publishes the new lock asynchronously) and adds
/// `verified` plus, on failure, `verify_error` to each result. That is what
/// lets LuCI's lock panel keep the old flow's outcome reporting without doing
/// the polling itself: verification is modem knowledge, so it stays here.
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
    let verify = params
        .get("verify")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // The CLI's attempts rule, from `api/cli.rs`'s `apply_frequency_lock`.
    let attempts = if report.cycled_radio { 8 } else { 1 };
    let mut results: Vec<Value> = Vec::new();
    for (i, o) in report.outcomes.iter().enumerate() {
        let mut m = std::collections::BTreeMap::new();
        m.insert("rat".to_string(), json::str_val(o.kind.as_str()));
        m.insert("applied".to_string(), Value::Bool(o.ok()));
        if let Some(e) = &o.error {
            m.insert("error".to_string(), json::str_val(&e.message()));
            m.insert("code".to_string(), json::str_val(e.code()));
        }
        if verify && o.ok() {
            let expected = requests
                .get(i)
                .map(|r| r.lock_type.to_string())
                .unwrap_or_default();
            let checked = crate::modules::network::service::verify_lock(
                ctx.channel,
                o.kind,
                &expected,
                attempts,
            );
            m.insert("verified".to_string(), Value::Bool(checked.ok));
            if !checked.ok {
                let observed = if checked.observed.is_empty() {
                    "no response".to_string()
                } else {
                    checked.observed
                };
                m.insert(
                    "verify_error".to_string(),
                    json::str_val(&format!(
                        "frequency lock verification failed: expected {}, got {}",
                        expected, observed
                    )),
                );
            }
        }
        results.push(Value::Obj(m));
    }
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
        let v = num(params, key).ok_or_else(|| {
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
