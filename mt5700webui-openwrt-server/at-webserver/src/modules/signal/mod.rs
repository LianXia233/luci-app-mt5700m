//! Signal module — radio signal quality.
//!
//! Single owner of the `^HCSQ` knowledge: command construction, the AT ->
//! domain parse, the JSON contract used by both frontends, the legacy text
//! rendering used by the CLI and the periodic refresh. Previously the same
//! response was parsed twice (once into the cache as JSON for the WebUI, once
//! into `key=value` text for LuCI); both now derive from `SignalState`.

pub mod api;
pub mod commands;
pub mod parser;
pub mod service;
pub mod state;
