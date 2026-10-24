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

/// One encoded SMS-SUBMIT PDU, ready for the two-phase `AT+CMGS` transaction.
///
/// The module's PDU codec produces these; the channel (and below it the
/// transport) only puts the octets on the wire. Encoding stays business logic,
/// the wire stays infrastructure — and a multipart message is *one* request, so
/// the parts cannot interleave with another module's AT traffic.
#[derive(Debug, Clone, PartialEq)]
pub struct SmsPart {
    /// Octet count of the TPDU — the argument of `AT+CMGS=<length>`. The
    /// service-centre field in `hex` is not counted (3GPP 27.005).
    pub length: usize,
    /// The complete PDU in hex — service-centre field then TPDU — without the
    /// trailing `Ctrl-Z`.
    pub hex: String,
}

/// RAII guard for an SMS transaction.
///
/// Opening one (via `AtChannel::begin_sms_transaction`) tells the scheduler
/// that every AT step of the operation belongs to SMS: commands issued by the
/// owning thread are upgraded to P0 and non-SMS requests are not started until
/// the guard is dropped. `Drop` is the single release path, so success,
/// failure, timeout and cancellation all resume ordinary traffic — there is
/// no flag a caller can forget to clear.
pub struct SmsTransaction {
    release: Option<Box<dyn FnOnce() + Send + 'static>>,
}

impl SmsTransaction {
    /// A guard that owns no scheduler state (non-daemon channels, tests).
    pub fn inactive() -> Self {
        SmsTransaction { release: None }
    }

    /// Wrap a scheduler release callback.
    pub fn new(release: impl FnOnce() + Send + 'static) -> Self {
        SmsTransaction {
            release: Some(Box::new(release)),
        }
    }
}

impl Drop for SmsTransaction {
    fn drop(&mut self) {
        if let Some(release) = self.release.take() {
            release();
        }
    }
}

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
        self.query_prio(command, at_timeout, queued_timeout, Priority::Maintenance)
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

    /// Send an already-encoded SMS (one `AT+CMGS` transaction per part), on the
    /// single arbiter thread so nothing else can interleave with the prompt.
    ///
    /// Modules never build the wire sequence themselves: the module's PDU codec
    /// gives the parts, this method performs them, and the transport below owns
    /// the serial port. Required rather than defaulted so every channel states
    /// its answer — an out-of-process client has to go through the daemon's
    /// `sms.send` route instead of pretending it can write the tty.
    fn send_sms_pdu(&self, parts: &[SmsPart]) -> Result<String, BackendError>;

    /// Open an SMS transaction. Default: no scheduler state (out-of-process
    /// client / test channel). The in-daemon channels override it.
    fn begin_sms_transaction(&self) -> SmsTransaction {
        SmsTransaction::inactive()
    }

    /// Scheduler audit trail (`scheduler.audit` route). Default: absent.
    fn scheduler_audit(&self) -> crate::core::json::Value {
        crate::core::json::Value::Null
    }

    /// Scheduler state (`scheduler.status` route). Default: absent.
    fn scheduler_status(&self) -> crate::core::json::Value {
        crate::core::json::Value::Null
    }

    /// Bounded scheduler audit trail (newest first).
    fn scheduler_audit_with_limit(&self, _limit: usize) -> crate::core::json::Value {
        self.scheduler_audit()
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
