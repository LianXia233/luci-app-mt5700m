//! Business modules. Every module owns its domain end to end:
//!
//! ```text
//! Module
//! ├── state      (domain model for this module)
//! ├── commands   (AT command construction)
//! ├── parser     (AT response -> domain model)
//! ├── service    (refresh / operate / cache / events)
//! └── api        (routes exposed to LuCI, WebUI and CLI)
//! ```
//!
//! Modules never reach into each other's internals: they talk through the
//! scheduler (AT), the state cache/bus (data) and the API registry (actions).

use crate::scheduler::jobs::TaskManager;
use std::time::Duration;

pub mod beam;
pub mod ca;
pub mod cell;
pub mod modem;
pub mod network;
pub mod qos;
pub mod signal;
pub mod sim;
pub mod sms;
pub mod system;
pub mod traffic;

/// Slow heartbeat cadence for the collectors that must keep running even when
/// no frontend is connected (temperature, registration). Everything else is
/// paused by the activity gate; these two stay warm for other daemons
/// (`mt5700m-manager` reads the temperature/registration cache directly).
pub const IDLE_KEEPALIVE: Duration = Duration::from_secs(60);

/// Register every module's periodic refresh jobs on the shared task manager.
///
/// This is the single place the daemon learns what background work exists; a
/// new module adds one line here (and its routes to the API registry).
/// NOTE: `ca` is missing here on purpose — `^HFREQINFO?`/`^CASCELLINFO?` cost
/// seconds each, so carrier aggregation is refreshed on demand (`ca.get`) and
/// then served from cache, never polled.
pub fn spawn_all(tasks: &TaskManager) {
    signal::service::spawn(tasks);
    network::service::spawn(tasks);
    cell::service::spawn(tasks);
    sim::service::spawn(tasks);
    modem::service::spawn(tasks);
    traffic::service::spawn(tasks);
    system::service::spawn(tasks);
}
