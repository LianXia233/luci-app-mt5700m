//! Unified state layer.
//!
//! One state source for the whole system: `cache` holds the last known value
//! per topic, `bus` fans changes out to every subscriber (WebUI WebSocket,
//! LuCI polling, CLI), and `collectors` is the legacy name of the periodic
//! refresh driver that modules now own.

pub mod activity;
pub mod bus;
pub mod cache;
pub mod refresh;
