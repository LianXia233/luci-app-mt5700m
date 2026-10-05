//! API layer: the single contract shared by LuCI, the WebUI and the CLI.
//!
//! `cli` adapts argv to the same registry the RPC/WebSocket transports call,
//! so no frontend ever re-implements backend behaviour.

pub mod cli;
