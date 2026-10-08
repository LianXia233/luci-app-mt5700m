//! QoS module — data-session parameters that the two frontends show next to the
//! signal card: which PDP context is up (`+CGACT?`), what the subscription
//! allows (`^DSAMBR`), and the bearer's QoS class (`+CGEQOSRDP`).
//!
//! It exists because all three were parsed in more than one place: the WebUI's
//! `pages/network/Info.tsx` did its own `+CGACT`/`^DSAMBR`/`+CGEQOSRDP`
//! decoding, the LuCI CLI had `qos` and `subscription-rate` verbs with their own
//! copies, and `mt5700m-manager` forwarded `qci=`/`ambr_*` text.
//!
//! Reads are on demand (`qos.get`), then cached under the `qos` topic: page
//! loads are cache hits, and the slow `^DSAMBR` sequence only runs when the
//! value went stale or a user asks for a refresh.

pub mod api;
pub mod commands;
pub mod parser;
pub mod service;
pub mod state;
