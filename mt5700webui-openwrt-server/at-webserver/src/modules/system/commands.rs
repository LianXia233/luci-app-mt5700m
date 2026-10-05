//! AT commands owned by the system module.

/// Chip temperature array (12 sensors, tenths of a degree). Fast command
/// (~0.1 s) whose *queue* time is the problem: it shares the port with slow
/// commands, so it is read with a generous timeout and no retry.
pub const CHIPTEMP: &str = "AT^CHIPTEMP?";
