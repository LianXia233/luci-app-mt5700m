//! Traffic module — PDCP counters, interface counters and accounting.
//!
//! Owns the AT knowledge for its domain, publishes its topics, registers its
//! refresh jobs and exposes its API routes; no other module and no frontend
//! parses these responses.

pub mod api;
pub mod commands;
pub mod netrate;
pub mod parser;
pub mod service;
pub mod state;
