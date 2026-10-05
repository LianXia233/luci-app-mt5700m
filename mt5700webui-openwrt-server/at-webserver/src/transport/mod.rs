//! Transport layer: everything that carries bytes between the backend and a
//! client, never business logic.
//!
//! * `ws`      — RFC6455 framing for the WebUI WebSocket.
//! * `control` — newline-JSON client used by the CLI / rpcd bridge.
//! * `client`  — AT transport client (serial/TCP) plus port discovery.
//! * `urc`     — unsolicited-result-code translation into bus events.

pub mod client;
pub mod control;
pub mod urc;
pub mod ws;
