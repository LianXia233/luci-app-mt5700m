//! Out-of-process `AtChannel`: the CLI is a client, not a second backend.
//!
//! `mt5700m-at` (used by the LuCI dial manager and by operators) must not open
//! the AT port — the daemon owns it. Instead this channel forwards to the
//! daemon over the control socket, where the same arbiter, read gate, cache
//! and modules run. If the daemon is not running there is no AT access at all,
//! which is exactly what "single AT owner" means.

use crate::core::channel::{AtChannel, SmsPart};
use crate::core::error::BackendError;
use crate::core::json::Value;
use crate::transport::control::{self, ControlError};
use std::time::Duration;

/// Default control-socket budget for a read (mirrors the ucode bridge: reads
/// 12 s, writes 25 s) plus slack.
const READ_TIMEOUT_S: u64 = 12;
const WRITE_TIMEOUT_S: u64 = 25;

fn to_backend(e: ControlError) -> BackendError {
    match e {
        ControlError::Unavailable => BackendError::ModemUnavailable,
        ControlError::BadResponse(m) => BackendError::TransportError(m),
    }
}

pub struct DaemonChannel;

impl DaemonChannel {
    pub fn new() -> Self {
        DaemonChannel
    }
}

impl Default for DaemonChannel {
    fn default() -> Self {
        Self::new()
    }
}

impl AtChannel for DaemonChannel {
    fn query(&self, command: &str) -> Result<String, BackendError> {
        control::daemon_send(command, READ_TIMEOUT_S).map_err(to_backend)
    }

    fn query_timeout(&self, command: &str, timeout: Duration) -> Result<String, BackendError> {
        let secs = timeout.as_secs().max(1);
        control::daemon_send(command, secs).map_err(to_backend)
    }

    fn query_prio(
        &self,
        command: &str,
        at_timeout: Duration,
        _queued_timeout: Duration,
        _priority: crate::core::task::Priority,
    ) -> Result<String, BackendError> {
        // The daemon decides priority; the client only bounds its own wait.
        let secs = at_timeout.as_secs().max(1) + 4;
        control::daemon_send(command, secs).map_err(to_backend)
    }

    fn action(&self, command: &str) -> Result<String, BackendError> {
        control::daemon_send(command, WRITE_TIMEOUT_S).map_err(to_backend)
    }

    fn send_sms_pdu(&self, _parts: &[SmsPart]) -> Result<String, BackendError> {
        // An SMS is one multi-phase `AT+CMGS` transaction that only the daemon
        // may run: this client can read and write single commands, but it has no
        // business holding the '>' prompt. Callers outside the daemon use the
        // `sms.send` route.
        Err(BackendError::TransportError(
            "SMS sends go through the daemon's sms.send route".into(),
        ))
    }
}

/// Call a unified API route (`signal.get`, `sms.send`, …) on the daemon.
///
/// This is how the CLI consumes the same contract as both frontends: one
/// registry, one implementation, no duplicated business logic in the shell.
pub fn api_call(method: &str, params: &Value, timeout_s: u64) -> Result<Value, BackendError> {
    control::daemon_api(method, params, timeout_s).map_err(to_backend)
}
