//! Signal AT commands (read path only — signal quality is never written).

/// Signal quality: RAT-dependent fields (RSRP/RSRQ/SINR/RSSI/RSCP/Ec/Io).
pub const HCSQ: &str = "AT^HCSQ?";

/// All read commands this module is allowed to issue, for the diagnostics page
/// and for the read gate's classification tests.
pub const READ_COMMANDS: [&str; 1] = [HCSQ];
