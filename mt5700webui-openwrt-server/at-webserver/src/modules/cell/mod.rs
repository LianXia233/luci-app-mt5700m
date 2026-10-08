//! Cell module — serving cell, band, ARFCN and cell identity.
//!
//! Owns the `^HFREQINFO?` / `^MONSC` knowledge; the per-RAT field offsets are
//! decoded here once (4 parser tests) instead of in the collector, the WebUI
//! and the CLI separately. Publishes the `cell` topic and exposes
//! `cell.get` / `cell.cached`.
//!
//! `scan.rs` is the same deal for `AT^CELLSCAN`: one command builder, one line
//! parser and one exclusive task behind `cell.scan_start` / `cell.scan_state`
//! / `cell.scan_abort`, so neither frontend drives the scan itself.

pub mod api;
pub mod commands;
pub mod parser;
pub mod scan;
pub mod service;
pub mod state;
