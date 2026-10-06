//! Network module — operator, access technology and registration.
//!
//! Owns the `+COPS` / `^SYSINFOEX` / `+CxxREG` knowledge end to end: command
//! construction, the AT -> domain parse, the two published topics (`network`
//! and `registration`), the periodic refresh and the API routes. Previously
//! these lived in the collector bundle (`state/collectors.rs`) and were parsed
//! a second time on each frontend; both now read one domain model.

pub mod api;
pub mod commands;
pub mod parser;
pub mod schedule;
pub mod service;
pub mod state;
