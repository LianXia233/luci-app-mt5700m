//! Cell module — serving cell, band, ARFCN and cell identity.
//!
//! Owns the `^HFREQINFO?` / `^MONSC` knowledge; the per-RAT field offsets are
//! decoded here once (4 parser tests) instead of in the collector, the WebUI
//! and the CLI separately. Publishes the `cell` topic and exposes
//! `cell.get` / `cell.cached`.

pub mod api;
pub mod commands;
pub mod parser;
pub mod service;
pub mod state;
