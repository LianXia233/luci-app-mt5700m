//! Small shared runtime helpers for the async MT5700M backend.
//!
//! The backend stays **std-only** (see Cargo.toml): the OpenWrt buildroot
//! compiles it without a cargo index or vendored sources, and an async
//! *execution model* does not require an external runtime crate. Threads +
//! std channels (mpsc / oneshot / condvar) provide non-blocking UI, queued
//! AT access, caching, events, cancellation and timeouts without sacrificing
//! cross-compile stability or firmware size.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Monotonic id source (task ids, request seq numbers).
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Next unique monotonic id (never 0).
pub fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// Unix timestamp in milliseconds (for records/events).
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Unix timestamp in seconds (for WebSocket event envelopes).
pub fn now_secs() -> u64 {
    now_ms() / 1000
}

/// Spawn a named thread (helps debugging on resource-limited OpenWrt boxes).
/// `Builder::spawn` takes `&self`, so `f` is only ever moved once.
pub fn spawn_thread<F>(name: &str, f: F) -> std::thread::JoinHandle<()>
where
    F: FnOnce() + Send + 'static,
{
    let builder = std::thread::Builder::new()
        .name(format!("mt5700m-{}", name))
        .stack_size(256 * 1024);
    builder
        .spawn(f)
        .unwrap_or_else(|_| std::thread::spawn(|| {}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_nonzero() {
        let a = next_id();
        let b = next_id();
        assert!(a > 0 && b > a);
    }

    #[test]
    fn now_is_positive() {
        assert!(now_ms() > 1_500_000_000_000);
        assert!(now_secs() > 1_500_000_000);
    }
}
