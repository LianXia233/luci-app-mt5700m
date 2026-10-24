//! Process-wide wire pacing.
//!
//! Contract: the *start* of any two AT writes on the single serial/TCP
//! transport is separated by at least [`MIN_SEND_GAP`] (25 ms). 25 ms is a
//! lower bound and **not** a command timeout: a slow command still gets its
//! full response budget; the pacer only delays the next write.
//!
//! The pacer is shared by the arbiter (which serialises whole transactions)
//! and by the transport (which writes the individual `CMGF`/`CMGS`/PDU lines
//! of an SMS transaction). Both call the same clock so an SMS multi-step
//! sequence and a following query cannot violate the gap either.
//!
//! Implemented with std only: a mutex held while sleeping guarantees that two
//! threads cannot both observe the same "last send" and race onto the wire.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Minimum gap between the starts of two AT writes.
pub const MIN_SEND_GAP: Duration = Duration::from_millis(25);

pub struct WirePacer {
    gap: Duration,
    last: Mutex<Option<Instant>>,
}

impl WirePacer {
    pub const fn new(gap: Duration) -> Self {
        WirePacer {
            gap,
            last: Mutex::new(None),
        }
    }

    /// Block until the wire is allowed to carry the next write, then claim
    /// the slot. Returns how long the caller waited (for audit/tests).
    ///
    /// The lock is held across the sleep on purpose: releasing it early would
    /// let two waiters both believe they own the next slot.
    pub fn wait_turn(&self) -> Duration {
        let mut last = self.last.lock().unwrap_or_else(|p| p.into_inner());
        let now = Instant::now();
        let waited = match *last {
            Some(prev) => {
                let elapsed = now.saturating_duration_since(prev);
                if elapsed < self.gap {
                    let wait = self.gap - elapsed;
                    std::thread::sleep(wait);
                    wait
                } else {
                    Duration::ZERO
                }
            }
            None => Duration::ZERO,
        };
        *last = Some(Instant::now());
        waited
    }

    /// Milliseconds since the last claimed send slot (None before the first).
    pub fn since_last_ms(&self) -> Option<u64> {
        let last = self.last.lock().unwrap_or_else(|p| p.into_inner());
        last.map(|t| t.elapsed().as_millis() as u64)
    }

    #[cfg(test)]
    fn reset(&self) {
        *self.last.lock().unwrap() = None;
    }
}

/// Process-wide pacer. There is exactly one AT wire per daemon, so a single
/// global is the correct scope (same reasoning as the duty window).
static PACER: OnceLock<WirePacer> = OnceLock::new();

pub fn global() -> &'static WirePacer {
    PACER.get_or_init(|| WirePacer::new(MIN_SEND_GAP))
}

/// Convenience: wait for the next wire slot.
pub fn wait_turn() -> Duration {
    global().wait_turn()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_turns_are_at_least_25ms_apart() {
        // A private pacer: the global one is shared by every test in this
        // process and would make the measured gaps nondeterministic.
        let pacer = WirePacer::new(MIN_SEND_GAP);
        pacer.reset();
        let mut stamps = Vec::new();
        for _ in 0..5 {
            pacer.wait_turn();
            stamps.push(Instant::now());
        }
        for pair in stamps.windows(2) {
            let gap = pair[1].duration_since(pair[0]);
            assert!(
                gap >= MIN_SEND_GAP.saturating_sub(Duration::from_millis(1)),
                "AT writes were {:?} apart, contract is >= 25ms",
                gap
            );
        }
    }

    #[test]
    fn slow_commands_do_not_shrink_the_gap() {
        let pacer = WirePacer::new(MIN_SEND_GAP);
        pacer.reset();
        pacer.wait_turn();
        // Simulate a command that took 80ms to answer: the next turn should
        // not add another 25ms on top (the gap already elapsed).
        std::thread::sleep(Duration::from_millis(80));
        let waited = pacer.wait_turn();
        assert!(
            waited < MIN_SEND_GAP,
            "pacer waited {:?} after an 80ms command; 25ms is a gap, not a timeout",
            waited
        );
    }

    #[test]
    fn first_turn_is_immediate() {
        let pacer = WirePacer::new(MIN_SEND_GAP);
        pacer.reset();
        assert_eq!(pacer.wait_turn(), Duration::ZERO);
        assert!(pacer.since_last_ms().is_some());
    }
}
