//! System module — board/modem environment facts the UI shows in the status
//! and system pages.
//!
//! It owns the chip-temperature sensor array (`^CHIPTEMP?`), the board switches
//! and the FOTA firmware-upgrade flow (`fota.rs`, a module-owned task the pages
//! only start and observe). USB state and load figures stay with the daemon's
//! sysfs monitor (`serial::presence`) because they need no AT access at all.

pub mod api;
pub mod commands;
pub mod fota;
pub mod parser;
pub mod service;
pub mod state;
