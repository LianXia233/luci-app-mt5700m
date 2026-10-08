//! Frontend activity gate.
//!
//! The backend must not keep collecting modem state when nobody is watching:
//! periodic collectors pause while no frontend is connected and resume (loading
//! the UI-critical topics first) the moment one shows up. This gate is the
//! single source of truth for "is a frontend in use".
//!
//! Two independent sources feed it:
//!   * **WebSocket sessions** (WebUI) — one live reader per connection,
//!   * **the 8765 newline-JSON RPC** (LuCI ucode) — a timestamp touched on
//!     every request, because LuCI keeps no persistent connection.
//!
//! The Unix control socket (`mt5700m-at` / `mt5700m-manager`) is deliberately
//! *not* counted: the dial manager polls it forever and would otherwise keep
//! the backend awake permanently.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;

/// How long after the last frontend touch the backend still considers itself
/// "in use". Covers LuCI's 15 s snapshot poll plus scheduling jitter.
const IDLE_GRACE_MS: u64 = 20_000;

pub struct ActivityGate {
    readers: AtomicUsize,
    last_touch_ms: AtomicU64,
}

impl ActivityGate {
    pub fn new() -> Arc<Self> {
        Arc::new(ActivityGate {
            readers: AtomicUsize::new(0),
            last_touch_ms: AtomicU64::new(0),
        })
    }

    /// A persistent frontend reader (WebSocket) connected.
    pub fn reader_enter(&self) {
        self.readers.fetch_add(1, Ordering::SeqCst);
        self.touch();
    }

    /// A persistent frontend reader disconnected (saturating: an unbalanced
    /// leave must never wrap the counter below zero).
    pub fn reader_leave(&self) {
        let _ = self
            .readers
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                Some(n.saturating_sub(1))
            });
    }

    /// Mark a one-shot frontend request (LuCI RPC).
    pub fn touch(&self) {
        self.last_touch_ms
            .store(crate::core::runtime::now_ms(), Ordering::Relaxed);
    }

    /// True while a frontend is connected, or was active within the grace
    /// window.
    pub fn is_active(&self) -> bool {
        if self.readers.load(Ordering::Relaxed) > 0 {
            return true;
        }
        let last = self.last_touch_ms.load(Ordering::Relaxed);
        last != 0 && crate::core::runtime::now_ms().saturating_sub(last) < IDLE_GRACE_MS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_before_any_touch() {
        assert!(!ActivityGate::new().is_active());
    }

    #[test]
    fn readers_keep_it_active() {
        let g = ActivityGate::new();
        g.reader_enter();
        assert!(g.is_active());
        g.reader_leave();
        // The enter already stamped the grace window, so it stays active.
        assert!(g.is_active());
    }

    #[test]
    fn a_stale_stamp_is_inactive() {
        let g = ActivityGate::new();
        g.last_touch_ms.store(
            crate::core::runtime::now_ms().saturating_sub(IDLE_GRACE_MS + 1),
            Ordering::Relaxed,
        );
        assert!(!g.is_active());
    }
}
