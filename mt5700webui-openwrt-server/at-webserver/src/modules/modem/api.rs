//! Modem API routes.

use crate::api::params::required_text;
use crate::api::registry::{ApiCtx, Route};
use crate::core::error::BackendError;
use crate::core::json::{self, Value};
use crate::modules::modem::commands::{MCS_DL, MCS_UL};
use crate::modules::modem::service;
use crate::state::bus::TOPIC_MODEM;

/// Routes contributed by this module.
pub fn routes() -> Vec<Route> {
    vec![
        Route::display("modem.get", get),
        Route::display("modem.cached", cached),
        Route::display("modem.txpower", txpower),
        Route::display("modem.endc", endc),
        Route::display("modem.nr_txpower", nr_txpower),
        Route::on_demand("modem.mcs", mcs),
        Route::on_demand("modem.reset", reset),
        Route::on_demand("modem.imei_set", imei_set),
    ]
}

/// Restart the modem (`AT^RESET`).
fn reset(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    service::reset(&refresh)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("rebooting".to_string(), Value::Bool(true));
    Ok(Value::Obj(m))
}

/// Write a new IMEI (`^PHYNUM=IMEI,<imei>`).
fn imei_set(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let imei = required_text(params, "imei")?;
    let refresh = ctx.refresh();
    service::set_imei(&refresh, imei)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("applied".to_string(), Value::Bool(true));
    m.insert("imei".to_string(), json::str_val(imei));
    Ok(Value::Obj(m))
}

/// Identity: cache-first with one bounded refresh when cold.
fn get(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let cached = service::cached(ctx.cache);
    if let Some(st) = &cached {
        if !st.is_empty() {
            return Ok(st.to_json());
        }
    }
    let refresh = ctx.refresh();
    Ok(service::refresh_info(&refresh)?.to_json())
}

/// Cache-only identity read.
fn cached(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let mut m = std::collections::BTreeMap::new();
    match service::cached(ctx.cache) {
        Some(st) => {
            m.insert("value".to_string(), st.to_json());
            m.insert("available".to_string(), Value::Bool(true));
        }
        None => {
            m.insert(
                "value".to_string(),
                crate::modules::modem::state::ModemState::default().to_json(),
            );
            m.insert("available".to_string(), Value::Bool(false));
        }
    }
    let (_, fresh) = ctx.cache.get(TOPIC_MODEM);
    m.insert("freshness".to_string(), json::str_val(fresh.name()));
    Ok(Value::Obj(m))
}

/// Transmit power: cache-first domain read.
fn txpower(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    if let Some(st) = service::cached_txpower(ctx.cache) {
        if !st.is_empty() {
            return Ok(st.to_json());
        }
    }
    let refresh = ctx.refresh();
    Ok(service::refresh_txpower(&refresh)?.to_json())
}

/// EN-DC status (always one bounded read; the topic is cheap to refresh).
fn endc(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    Ok(service::refresh_endc(&refresh)?.to_json())
}

/// NR per-carrier transmit power.
fn nr_txpower(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    Ok(service::refresh_nr_txpower(&refresh)?.to_json())
}

/// Modulation/coding scheme tables of both directions (`AT^MCS=1`/`=0`).
///
/// On-demand: the Info page reads it once per load. The three-value grouping,
/// the RAT field and the average are the module's; the page turns `code0` into
/// modulation names and colours. As with `network.dhcp`, a partial answer is
/// still an answer — the error is only reported when nothing could be read.
fn mcs(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    let mut m = std::collections::BTreeMap::new();
    let mut errors = crate::state::refresh::ReadErrors::new();
    if let Some(st) = errors.note(service::refresh_mcs(&refresh, MCS_DL)) {
        if !st.is_empty() {
            m.insert("downlink".to_string(), st.to_json());
        }
    }
    if let Some(st) = errors.note(service::refresh_mcs(&refresh, MCS_UL)) {
        if !st.is_empty() {
            m.insert("uplink".to_string(), st.to_json());
        }
    }
    if m.is_empty() {
        if let Some(e) = errors.into_option() {
            return Err(e);
        }
    }
    Ok(Value::Obj(m))
}
