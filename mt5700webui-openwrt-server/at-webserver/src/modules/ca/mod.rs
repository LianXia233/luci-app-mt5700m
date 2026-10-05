//! Carrier-aggregation module — the multi-carrier picture (`^HFREQINFO?` groups,
//! `^CASCELLINFO?` LTE secondary cells, `^MONSSC` NSA secondary cells).
//!
//! It exists because "which carriers are aggregated right now" is its own
//! question with its own answer shape (`carrier_1=NR|n78|…` lines plus the
//! `ca_*` summary), and because three surfaces wanted it: the WebUI band card,
//! the LuCI carrier panel (through `mt5700m-at carrier-aggregation`) and the
//! network diagnostics dump. All three read the one `CaState` this module
//! decodes: the CLI issues the three commands (declared in `commands.rs`) and
//! then prints `CaState::to_text()`, the two frontends read the `ca` topic
//! through `ca.get`/`ca.cached`. Nothing else in the crate splits `^HFREQINFO`
//! field groups.
//!
//! Unlike signal/cell/network there is **no periodic job**: `^HFREQINFO?` costs
//! ~3.4 s and `^CASCELLINFO?` ~12 s, so the topic is filled on demand
//! (`ca.get`) and then served from cache (`ca.cached`) until its TTL expires.

pub mod api;
pub mod commands;
pub mod parser;
pub mod service;
pub mod state;
