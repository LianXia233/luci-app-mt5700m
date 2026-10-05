//! The AT channel contract every module talks to.
//!
//! Modules never open a serial port, never touch a queue and never know
//! whether they run inside the daemon (submitting through the AT arbiter) or
//! inside a CLI client (forwarding over the control socket). They only see
//! this trait, so the transport can be replaced without touching a module:
//!
//! ```text
//! Module -> Command/Service -> AtChannel -> { arbiter (in-daemon)
//!                                         { control socket (CLI client)
//! ```
//!
//! There is exactly one AT owner: the daemon's `serial::manager` behind the
//! arbiter. Implementations live in `scheduler::channel` (in-daemon) and
//! `transport::channel` (out-of-process).

use crate::core::error::BackendError;
use crate::core::task::Priority;
use std::time::Duration;

/// Read/write access to the modem, already serialized by the AT scheduler.
pub trait AtChannel: Send + Sync {
    /// Read-only AT command. Implementations are expected to serve the cached
    /// answer when the read gate has a fresh one, so calling this on a render
    /// path costs no AT traffic.
    fn query(&self, command: &str) -> Result<String, BackendError>;

    /// Read-only AT command with an explicit timeout, for slow queries
    /// (`AT^HCSQ?` needs ~4 s on the MT5700M, `AT^NTXPOWER?` ~12 s).
    fn query_timeout(&self, command: &str, timeout: Duration) -> Result<String, BackendError>;

    /// Background read with an explicit scheduler priority: no retry, its own
    /// queue budget. Periodic collectors use it so they can never starve an
    /// interactive user; the priority is the only knob that differs between a
    /// routine snapshot (`Low`) and a state the UI waits on (`Normal`).
    fn query_prio(
        &self,
        command: &str,
        at_timeout: Duration,
        queued_timeout: Duration,
        priority: Priority,
    ) -> Result<String, BackendError>;

    /// Low-priority background read (the common collector case).
    fn query_background(
        &self,
        command: &str,
        at_timeout: Duration,
        queued_timeout: Duration,
    ) -> Result<String, BackendError> {
        self.query_prio(command, at_timeout, queued_timeout, Priority::Low)
    }

    /// Write/action AT command. Bypasses the read cache entirely — a write
    /// must always reach the modem.
    fn action(&self, command: &str) -> Result<String, BackendError>;

    /// True while the channel is oversubscribed (background work over its duty
    /// cycle or a user request just starved). Background collectors check this
    /// and skip a tick instead of fighting for the port.
    fn duty_gate(&self) -> bool {
        false
    }
}

/// Normalise a transport result for parsers: a timeout/busy channel means
/// "no answer right now", which is not an error worth failing a collector for.
pub fn soft(r: Result<String, BackendError>) -> Result<String, BackendError> {
    match r {
        Ok(t) => Ok(t),
        Err(BackendError::AtTimeout) | Err(BackendError::Busy) => Ok(String::new()),
        Err(e) => Err(e),
    }
}
