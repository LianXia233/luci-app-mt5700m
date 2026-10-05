//! AT commands owned by the traffic module.

/// PDCP statistics. The query form appends two cumulative byte counters after
/// the 14 traffic fields.
pub const PDCP: &str = "AT^PDCPDATAINFO?";

/// Reset the modem's data-flow counters (`AT^DSFLOWCLR`).
///
/// A write: it clears the firmware's own accounting, so it is only ever issued
/// by an explicit user action through the API.
pub const DSFLOWCLR: &str = "AT^DSFLOWCLR";
