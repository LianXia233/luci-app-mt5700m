//! Beam module — the NR synchronization-signal-block view (`^NRSSBID?`).
//!
//! SSB is what a UE searches for when it looks for an NR cell, and the MT5700M
//! can report the beam strengths it measured: up to eight SSBs for the serving
//! cell and the SSBs of the neighbour cells. The Settings page used to decode
//! that reply in TypeScript with hard-coded offsets (6 + i*2 for the serving
//! beams, 23 + 12*n for each neighbour); the layout is AT knowledge, so it
//! lives here and the page renders the decoded object.
//!
//! On-demand: the "查询 SSB" button.

pub mod api;
pub mod commands;
pub mod parser;
pub mod service;
pub mod state;
