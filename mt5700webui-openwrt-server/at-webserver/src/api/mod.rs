//! API layer: the single contract shared by LuCI, the WebUI and the CLI.
//!
//! * `registry` — the route table modules register their capabilities in.
//! * `cli`      — argv adapter for the LuCI shell manager; it dispatches into
//!                the same registry, so it is a client, never a second backend.
//! * `rpc`      — transport-side plumbing that turns RPC/WebSocket/control
//!                frames into registry calls.
//! * `params`   — the shared request-parameter readers every module handler
//!                uses, so no module invents its own extraction/error text.

pub mod cli;
pub mod params;
pub mod registry;
pub mod rpc;
