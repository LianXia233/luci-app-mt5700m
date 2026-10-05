//! Modem API routes.

use crate::api::params::{boolean, num, required_text};
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
        Route::display("modem.nr_capability", nr_capability),
        Route::on_demand("modem.nr_capability_set", nr_capability_set),
    ]
}

/// NR capability settings (`^NRRCCAPQRY=3/2/5`).
///
/// Display route: an ability the modem did not answer stays absent from the
/// payload, and the page keeps the value it is showing.
fn nr_capability(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    match service::read_nr_capability(&refresh) {
        Ok(st) => Ok(st.to_json()),
        Err(_) => Ok(crate::modules::modem::state::NrCapabilityState::default().to_json()),
    }
}

/// Write one or more NR capability settings.
///
/// Params: `{ "ca": bool, "vonr": 0..3, "dss": {"rateMatchingLTE": 0|1,
/// "additionalDMRS": 0|1} }` — every field optional, at least one required. The
/// page's 5G cards send exactly the one that changed.
fn nr_capability_set(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let ca = boolean(params, "ca");
    let vonr = num(params, "vonr");
    let dss = match params.get("dss") {
        Some(dss) => Some((
            num(dss, "rateMatchingLTE").ok_or_else(|| {
                BackendError::InvalidParameter("dss.rateMatchingLTE is required".to_string())
            })?,
            num(dss, "additionalDMRS").ok_or_else(|| {
                BackendError::InvalidParameter("dss.additionalDMRS is required".to_string())
            })?,
        )),
        None => None,
    };
    let refresh = ctx.refresh();
    let wrote = service::set_nr_capability(&refresh, ca, vonr, dss)?;
    let mut m = std::collections::BTreeMap::new();
    m.insert("applied".to_string(), Value::Bool(true));
    m.insert(
        "wrote".to_string(),
        Value::Arr(wrote.iter().map(|k| json::num_val(k)).collect()),
    );
    Ok(Value::Obj(m))
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
