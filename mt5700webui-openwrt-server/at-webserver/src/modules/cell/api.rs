//! Cell API routes.

use crate::api::registry::{ApiCtx, Route};
use crate::core::error::BackendError;
use crate::core::json::{self, Value};
use crate::modules::cell::parser;
use crate::modules::cell::scan;
use crate::modules::cell::service;
use crate::modules::cell::state::CellState;
use crate::core::task::Priority;
use std::time::Duration;
use crate::state::bus::TOPIC_CELL;

/// Routes contributed by this module.
pub fn routes() -> Vec<Route> {
    vec![
        Route::display("cell.get", get),
        Route::display("cell.cached", cached),
        Route::on_demand("cell.neighbors", neighbors),
        // `AT^CELLSCAN` is a minutes-long exclusive task, not a read: the
        // routes start it (`cell.scan_start`), answer whether it is running
        // (`cell.scan_state`, which must not touch the modem) and abort it
        // (`cell.scan_abort`). Results arrive on the `cellscan` push topic.
        Route::on_demand("cell.scan_start", scan_start),
        Route::display("cell.scan_state", scan_state),
        Route::on_demand("cell.scan_abort", scan_abort),
    ]
}

/// `{started: true}` — validate the filter, then submit the exclusive scan.
fn scan_start(ctx: &ApiCtx, params: &Value) -> Result<Value, BackendError> {
    let tasks = ctx.require_tasks()?;
    let filter = scan::ScanFilter::from_params(params);
    scan::start(tasks, &filter)
}

/// `{running}` — task introspection only (a page reload must find out that a
/// scan outlived its mount even while the modem is busy). A transport without
/// tasks has no scan running, which is a true answer, not an error.
fn scan_state(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    match ctx.tasks {
        Some(tasks) => Ok(scan::state(tasks)),
        None => {
            let mut m = std::collections::BTreeMap::new();
            m.insert("running".to_string(), Value::Bool(false));
            Ok(Value::Obj(m))
        }
    }
}

/// `{aborted}` — cancel the scan task; the arbiter injects the abort token on
/// the wire for it. Idempotent by design (see `scan::abort`).
fn scan_abort(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    match ctx.tasks {
        Some(tasks) => Ok(scan::abort(tasks)),
        None => {
            let mut m = std::collections::BTreeMap::new();
            m.insert("aborted".to_string(), Value::Bool(false));
            Ok(Value::Obj(m))
        }
    }
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

/// Cache-only read: never touches the modem.
fn cached(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let mut m = std::collections::BTreeMap::new();
    match service::cached(ctx.cache) {
        Some(st) => {
            m.insert("value".to_string(), st.to_json());
            m.insert("available".to_string(), Value::Bool(true));
        }
        None => {
            m.insert("value".to_string(), CellState::default().to_json());
            m.insert("available".to_string(), Value::Bool(false));
        }
    }
    let (_, fresh) = ctx.cache.get(TOPIC_CELL);
    m.insert("freshness".to_string(), json::str_val(fresh.name()));
    Ok(Value::Obj(m))
}

/// Measured neighbour cells (`AT^MONNC`).
///
/// On-demand: the Settings page's scan button. The line layout, the hex PCI,
/// the band table and the 1/8-unit scaling of NR values all live in the
/// parser, so the table renders domain values instead of raw AT fields.
fn neighbors(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    let text = refresh.read(
        crate::modules::cell::commands::MONNC,
        Duration::from_secs(20),
        Duration::from_secs(10),
        Priority::Normal,
    )?;
    let cells: Vec<Value> = parser::parse_monnc(&text).iter().map(|c| c.to_json()).collect();
    let mut m = std::collections::BTreeMap::new();
    m.insert("cells".to_string(), Value::Arr(cells));
    Ok(Value::Obj(m))
}
