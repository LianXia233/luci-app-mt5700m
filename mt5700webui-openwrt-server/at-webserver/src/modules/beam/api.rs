//! Beam API routes.

use crate::api::registry::{ApiCtx, Route};
use crate::core::error::BackendError;
use crate::core::json::Value;
use crate::modules::beam::service;

/// Routes contributed by this module.
pub fn routes() -> Vec<Route> {
    vec![Route::on_demand("beam.ssb", ssb)]
}

/// NR SSB/beam report (`AT^NRSSBID?`), decoded for the Settings page.
///
/// On-demand: the page's "查询 SSB" button. A reply without the data line is a
/// modem error, which is exactly what the page needs to keep its previous card
/// instead of blanking it.
fn ssb(ctx: &ApiCtx, _params: &Value) -> Result<Value, BackendError> {
    let refresh = ctx.refresh();
    Ok(service::refresh(&refresh)?.to_json())
}
