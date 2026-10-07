//! Single binary, dual frontend — the ONE backend for LuCI and the WebUI.
//!
//! ```text
//!                 ┌─────────────┐
//!                 │    LuCI     │
//!                 └──────┬──────┘
//!                        │ API / RPC
//!                 ┌──────▼──────┐
//!                 │  Backend    │  api/  modules/  state/  scheduler/  serial/
//!                 └──────▲──────┘
//!                        │ API / WebSocket
//!                 ┌──────┴──────┐
//!                 │   WebUI     │
//!                 └─────────────┘
//! ```
//!
//! Entry points:
//!   - `at-webserver` (or no args / `daemon`): WebSocket + RPC daemon on :8765
//!     for both frontends (WebUI WebSocket, LuCI ucode TCP RPC).
//!   - `at-webserver cli …` or argv[0] == `mt5700m-at`: argv-compatible client
//!     used by the LuCI shell manager. It is a *client* of the daemon — it
//!     never opens the AT port itself.
//!   - `atprobe`: field diagnostic that inspects the AT port out-of-band.

mod api;
mod core;
mod daemon;
mod modules;
mod scheduler;
mod serial;
mod state;
mod transport;

use std::path::Path;

fn basename(p: &str) -> String {
    Path::new(p)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| p.to_string())
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let exe = basename(argv.first().map(|s| s.as_str()).unwrap_or(""));
    let args: Vec<String> = argv.iter().skip(1).cloned().collect();

    let code = match exe.as_str() {
        "mt5700m-at" => api::cli::run(&args),
        _ => match args.first().map(|s| s.as_str()) {
            Some("cli") => api::cli::run(&args[1..]),
            Some("daemon") => daemon::run(&args[1..]),
            // Field diagnostic: drive the AT port directly, bypassing the
            // daemon, the arbiter and the snapshot collectors. Used to tell
            // "the modem is not answering" apart from "our channel is
            // starved" when a page renders empty.
            Some("atprobe") => serial::probe::run(&args[1..]),
            _ => daemon::run(&args),
        },
    };
    std::process::exit(code);
}
