//! SIM module — card status, ICCID, IMSI and operator identity.
//!
//! Owns the AT knowledge for its domain, publishes its topics, registers its
//! refresh jobs and exposes its API routes; no other module and no frontend
//! parses these responses.

pub mod api;
pub mod commands;
pub mod parser;
pub mod service;
pub mod state;
