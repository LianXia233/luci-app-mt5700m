//! AT commands owned by the carrier-aggregation module.
//!
//! `AT^HFREQINFO?` is also read by the cell module, which cares only about the
//! first field group (the serving cell's band/ARFCN/bandwidth). Both modules
//! declare the command they send: the string is a wire fact, and the module that
//! issues a command documents it next to the parser for its answer.

/// Per-carrier frequency/bandwidth groups; ~3.4 s on the MT5700M.
pub const HFREQINFO: &str = "AT^HFREQINFO?";

/// LTE secondary-cell list. Slow (~12 s), so it is only asked on demand.
pub const CASCELLINFO: &str = "AT^CASCELLINFO?";

/// NSA secondary-cell list (NR legs), one line per reported cell.
pub const MONSSC: &str = "AT^MONSSC";
