//! AT commands owned by the cell module.

/// Frequency/band information; ~3.4 s on the MT5700M, fast enough to refresh
/// on the cell card's cadence.
pub const HFREQINFO: &str = "AT^HFREQINFO?";

/// Serving-cell parameters. Not supported in NR mode on the MT5700M (the
/// attempt costs 8 s+), so it is queried with a long backoff.
pub const MONSC: &str = "AT^MONSC";
