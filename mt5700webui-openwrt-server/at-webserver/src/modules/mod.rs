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

pub mod sms;
