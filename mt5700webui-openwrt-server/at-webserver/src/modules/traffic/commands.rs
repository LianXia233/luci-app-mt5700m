//! AT commands owned by the traffic module.

/// PDCP statistics. The query form appends two cumulative byte counters after
/// the 14 traffic fields.
pub const PDCP: &str = "AT^PDCPDATAINFO?";
