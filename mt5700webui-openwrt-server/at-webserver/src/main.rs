//! Single binary, dual frontend:
//!   - invoked as `at-webserver` (or with no args / `daemon`): WebSocket AT
//!     daemon on :8765 for the MT5700M WebUI
//!   - invoked as `mt5700m-at` (symlink) or via `cli`: LuCI shell-backend
//!     replacement with byte-compatible stdout
//!
//! Deployment on OpenWrt:
//!   /usr/bin/at-webserver        (this binary, real file)
//!   /usr/sbin/mt5700m-at -> /usr/bin/at-webserver   (symlink, LuCI path)

mod at;
mod cli;
mod daemon;
mod dispatcher;
mod json;
mod scheduler;
mod sha1;
mod ws;

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
        "mt5700m-at" => cli::run(&args),
        _ => match args.first().map(|s| s.as_str()) {
            Some("cli") => cli::run(&args[1..]),
            Some("daemon") => daemon::run(&args[1..]),
            _ => daemon::run(&args),
        },
    };
    std::process::exit(code);
}
