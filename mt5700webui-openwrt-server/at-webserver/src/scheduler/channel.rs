//! `AtChannel` implementations that talk to the single AT arbiter.
//!
//! * `TaskChannel`  — used by periodic collectors; inherits the task's
//!                    cancellation flag and deadline.
//! * `DirectChannel`— used by request handlers that are not tasks (RPC, the
//!                    control socket, CLI-forwarded calls): submits with an
//!                    explicit budget and waits for the answer.
//!
//! Both keep every module on one path: module -> channel -> arbiter -> serial.
//! Nothing here or below opens a serial port directly.

use crate::core::channel::{AtChannel, SmsPart};
use crate::core::error::BackendError;
use crate::core::task::Priority;
use crate::scheduler::arbiter::{
    channel_budget_exhausted, channel_user_starved, AtArbiter, AtPayload, AtRequestSpec, AtResult,
    RetryPolicy,
};
use crate::scheduler::jobs::TaskCtx;
use std::sync::Arc;
use std::time::Duration;

/// Hard ceiling on how long a request handler waits for an AT answer
/// (queue time + command time + slack).
fn budget(spec: &AtRequestSpec) -> Duration {
    spec.queued_timeout + spec.timeout + Duration::from_secs(2)
}

/// Submit a spec and wait for the arbiter's answer.
pub fn await_spec(arbiter: &Arc<AtArbiter>, spec: AtRequestSpec) -> AtResult {
    let wait = budget(&spec);
    let rx = arbiter.submit(spec);
    match rx.recv_timeout(wait) {
        Ok(r) => r,
        Err(_) => Err(BackendError::TaskTimeout),
    }
}

/// Background read spec shared by every module collector: low priority, no
/// retry, generous timeout.
pub fn background_spec(
    command: &str,
    at_timeout: Duration,
    queued_timeout: Duration,
    priority: Priority,
) -> AtRequestSpec {
    AtRequestSpec {
        command: command.to_string(),
        priority,
        timeout: at_timeout,
        queued_timeout,
        exclusive: false,
        dedup_key: Some(command.to_string()),
        retry: RetryPolicy::none(),
        cancel: None,
        abort_wire: None,
        payload: AtPayload::None,
        label: format!("refresh {}", command),
    }
}

/// AT channel bound to a running task.
pub struct TaskChannel<'a> {
    ctx: &'a TaskCtx,
}

impl<'a> TaskChannel<'a> {
    pub fn new(ctx: &'a TaskCtx) -> Self {
        TaskChannel { ctx }
    }

    fn spec(&self, command: &str, timeout: Duration) -> AtRequestSpec {
        let mut spec = AtRequestSpec::ui_query(command);
        spec.timeout = timeout;
        spec.priority = Priority::High;
        spec
    }
}

impl AtChannel for TaskChannel<'_> {
    fn query(&self, command: &str) -> Result<String, BackendError> {
        self.ctx.query(command)
    }

    fn query_timeout(&self, command: &str, timeout: Duration) -> Result<String, BackendError> {
        self.ctx.at_request(self.spec(command, timeout))
    }

    fn query_prio(
        &self,
        command: &str,
        at_timeout: Duration,
        queued_timeout: Duration,
        priority: Priority,
    ) -> Result<String, BackendError> {
        self.ctx
            .at_request(background_spec(command, at_timeout, queued_timeout, priority))
    }

    fn action(&self, command: &str) -> Result<String, BackendError> {
        self.ctx.action(command)
    }

    fn send_sms_pdu(&self, parts: &[SmsPart]) -> Result<String, BackendError> {
        self.ctx
            .at_request(AtRequestSpec::sms_send(parts.to_vec()))
    }

    fn duty_gate(&self) -> bool {
        channel_budget_exhausted() || channel_user_starved()
    }
}

/// Run a module's refresh function inside a periodic task.
///
/// Every module service looks the same at the edges — build the task channel,
/// wrap it in a `RefreshCtx`, call one `refresh()` — so that adapter lives here
/// once instead of in each `service.rs`.
pub fn run_in_task<F>(ctx: &TaskCtx, f: F) -> Result<crate::core::json::Value, BackendError>
where
    F: FnOnce(&crate::state::refresh::RefreshCtx) -> Result<crate::core::json::Value, BackendError>,
{
    let channel = TaskChannel::new(ctx);
    let refresh = crate::state::refresh::RefreshCtx::new(&channel, &ctx.cache, &ctx.bus);
    f(&refresh)
}

/// AT channel for request handlers that are not tasks (RPC / control socket).
pub struct DirectChannel<'a> {
    arbiter: &'a Arc<AtArbiter>,
}

impl<'a> DirectChannel<'a> {
    pub fn new(arbiter: &'a Arc<AtArbiter>) -> Self {
        DirectChannel { arbiter }
    }
}

impl AtChannel for DirectChannel<'_> {
    fn query(&self, command: &str) -> Result<String, BackendError> {
        await_spec(self.arbiter, AtRequestSpec::fast_query(command))
    }

    fn query_timeout(&self, command: &str, timeout: Duration) -> Result<String, BackendError> {
        let mut spec = AtRequestSpec::ui_query(command);
        spec.timeout = timeout;
        spec.priority = Priority::High;
        await_spec(self.arbiter, spec)
    }

    fn query_prio(
        &self,
        command: &str,
        at_timeout: Duration,
        queued_timeout: Duration,
        priority: Priority,
    ) -> Result<String, BackendError> {
        await_spec(
            self.arbiter,
            background_spec(command, at_timeout, queued_timeout, priority),
        )
    }

    fn action(&self, command: &str) -> Result<String, BackendError> {
        await_spec(self.arbiter, AtRequestSpec::interactive(command))
    }

    fn send_sms_pdu(&self, parts: &[SmsPart]) -> Result<String, BackendError> {
        await_spec(self.arbiter, AtRequestSpec::sms_send(parts.to_vec()))
    }

    fn duty_gate(&self) -> bool {
        channel_budget_exhausted() || channel_user_starved()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::arbiter::AtTransport;
    use std::sync::atomic::AtomicBool;

    /// Transport double: echoes the command so the test can prove every call
    /// went through the arbiter exactly once.
    struct Echo;
    impl AtTransport for Echo {
        fn send(&self, command: &str, _t: Duration) -> AtResult {
            Ok(format!("OK:{}", command))
        }
        fn send_interruptible(
            &self,
            _c: &str,
            _t: Duration,
            _cancel: &AtomicBool,
            _abort: Option<&[u8]>,
        ) -> AtResult {
            Err(BackendError::TaskCancelled)
        }
        fn connected(&self) -> bool {
            true
        }
    }

    #[test]
    fn direct_channel_round_trips_through_the_arbiter() {
        let arbiter = AtArbiter::new(Arc::new(Echo));
        let ch = DirectChannel::new(&arbiter);
        assert_eq!(ch.query("ATI").unwrap(), "OK:ATI");
        assert_eq!(ch.action("AT+CFUN=1").unwrap(), "OK:AT+CFUN=1");
        assert_eq!(
            ch.query_timeout("AT^HCSQ?", Duration::from_secs(1)).unwrap(),
            "OK:AT^HCSQ?"
        );
        assert!(!ch.duty_gate());
    }

    #[test]
    fn background_specs_never_retry_and_stay_low_priority() {
        let spec = background_spec(
            "AT^NTXPOWER?",
            Duration::from_secs(12),
            Duration::from_secs(20),
            Priority::Low,
        );
        assert_eq!(spec.priority, Priority::Low);
        assert_eq!(spec.retry.max_retries, 0);
        assert_eq!(spec.dedup_key.as_deref(), Some("AT^NTXPOWER?"));
    }
}
