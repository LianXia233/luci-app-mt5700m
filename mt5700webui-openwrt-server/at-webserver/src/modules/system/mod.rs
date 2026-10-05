//! System module — board/modem environment facts the UI shows in the status
//! and system pages.
//!
//! Currently owns the chip-temperature sensor array (`^CHIPTEMP?`), the only
//! AT-sourced system value; USB state and load figures stay with the daemon's
//! sysfs monitor (`serial::presence`) because they need no AT access at all.

pub mod api;
pub mod commands;
pub mod parser;
pub mod service;
pub mod state;
