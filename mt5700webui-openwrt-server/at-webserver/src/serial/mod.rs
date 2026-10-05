//! Serial layer — the one and only owner of the AT port.
//!
//! `manager` scans/opens the PCUI port exclusively (TIOCEXCL), `presence`
//! tracks USB (dis)connection and `probe` is the field diagnostic that talks
//! to the port out-of-band.

pub mod manager;
pub mod presence;
pub mod probe;
