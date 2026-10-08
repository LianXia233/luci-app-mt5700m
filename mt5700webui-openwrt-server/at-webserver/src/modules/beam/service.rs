//! Beam service: the on-demand SSB read policy.
//!
//! No topic and no polling — the report is only meaningful when the user asks
//! for it. The read goes through the shared channel with a bounded budget,
//! because `^NRSSBID?` is a slow vendor query.

use crate::core::error::BackendError;
use crate::core::task::Priority;
use crate::modules::beam::commands::NRSSBID;
use crate::modules::beam::parser;
use crate::modules::beam::state::SsbState;
use crate::state::refresh::RefreshCtx;
use std::time::Duration;

const AT_TIMEOUT: Duration = Duration::from_secs(20);
const QUEUED_TIMEOUT: Duration = Duration::from_secs(10);

/// Read the SSB report. An answer without a `^NRSSBID:` line is reported as a
/// modem rejection, so the caller keeps whatever it displayed before.
pub fn refresh(ctx: &RefreshCtx) -> Result<SsbState, BackendError> {
    let text = ctx.read(NRSSBID, AT_TIMEOUT, QUEUED_TIMEOUT, Priority::Normal)?;
    parser::parse_ssbid(&text).ok_or_else(|| BackendError::AtRejected(text.trim().to_string()))
}
