//! Unified task model for the async MT5700M backend.
//!
//! Every unit of work — a fast cache query, a background refresh, an
//! interactive user action, a long-running scan, an exclusive modem
//! operation — is a `Task` with a well-defined lifecycle:
//!
//!   Queued -> Running -> Progress -> (Completed | Failed | Cancelled | Timeout)
//!
//! Tasks carry an id, kind, priority, timeout and optional progress/result
//! so callers (WebSocket, control socket, LuCI) can observe them
//! independently of the AT channel.

use crate::core::json::Value;
use std::time::Instant;

/// Monotonic task id.
pub type TaskId = u64;

/// Task categories. Used for scheduling decisions (fast queries never wait
/// behind scans, background refreshes never block interactive actions).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TaskKind {
    /// Read-from-cache only, never touches the modem.
    FastQuery,
    /// Background refresh of cached state (signal, network, ...).
    BackgroundQuery,
    /// User-initiated short action (APN change, PDP, SMS send).
    Interactive,
    /// Long-running operation that reports progress (scan, beam scan).
    LongRunning,
    /// Requires exclusive ownership of the AT channel (scan, reboot, flash).
    Exclusive,
    /// Periodic job (temperature, traffic, ...).
    Periodic,
}

/// Scheduling priority — the five contract levels, highest first.
///
/// P0 is reserved for SMS. It is not merely "a bit higher": while an SMS
/// transaction is open the arbiter pauses every not-yet-started non-SMS
/// request (see `scheduler::arbiter`). The other four levels are ordinary
/// ordering and are never allowed to starve each other indefinitely because
/// every request also carries a queue deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// P0 — SMS (list/read/send/delete/USSD and every command inside the
    /// transaction). Highest, and the only class allowed while an SMS
    /// transaction holds the channel.
    Sms,
    /// P1 — key control: dial/disconnect/recovery, radio/USB operations and
    /// the status transitions they depend on.
    Control,
    /// P2 — user-visible queries (page reads, explicit on-demand reads).
    UserQuery,
    /// P3 — routine background polling (signal/network/cell collectors).
    Polling,
    /// P4 — low-value maintenance (telemetry, optional diagnostics).
    Maintenance,
}

impl Priority {
    /// Numeric rank used by the AT arbiter heap (larger = more urgent).
    pub fn rank(&self) -> i64 {
        match self {
            Priority::Sms => 100,
            Priority::Control => 80,
            Priority::UserQuery => 60,
            Priority::Polling => 30,
            Priority::Maintenance => 10,
        }
    }

    /// Contract-level name (P0..P4), used by audit logs and the task API.
    pub fn name(&self) -> &'static str {
        match self {
            Priority::Sms => "P0",
            Priority::Control => "P1",
            Priority::UserQuery => "P2",
            Priority::Polling => "P3",
            Priority::Maintenance => "P4",
        }
    }

    /// Human-readable class name for diagnostics.
    pub fn label(&self) -> &'static str {
        match self {
            Priority::Sms => "sms",
            Priority::Control => "control",
            Priority::UserQuery => "user_query",
            Priority::Polling => "polling",
            Priority::Maintenance => "maintenance",
        }
    }

    /// True for the SMS class (the arbiter's exclusive-dispatch level).
    pub fn is_sms(&self) -> bool {
        *self == Priority::Sms
    }

    /// True for work a frontend/user is directly waiting on (P0..P2). Used
    /// for the background-collector starvation signal.
    pub fn is_user_visible(&self) -> bool {
        self.rank() >= Priority::UserQuery.rank()
    }
}

/// Task lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Queued,
    Running,
    Progress,
    Completed,
    Failed,
    Cancelled,
    Timeout,
}

impl TaskStatus {
    pub fn name(&self) -> &'static str {
        match self {
            TaskStatus::Queued => "queued",
            TaskStatus::Running => "running",
            TaskStatus::Progress => "progress",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Cancelled => "cancelled",
            TaskStatus::Timeout => "timeout",
        }
    }

    /// Terminal statuses (the record is final and no longer running).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskStatus::Completed
                | TaskStatus::Failed
                | TaskStatus::Cancelled
                | TaskStatus::Timeout
        )
    }
}

/// Immutable snapshot of a task, safe to hand to WebSocket/CLI callers.
#[derive(Debug, Clone)]
pub struct TaskRecord {
    pub id: TaskId,
    pub name: String,
    pub kind: TaskKind,
    pub priority: Priority,
    pub status: TaskStatus,
    /// 0..=100 for LongRunning/Progress tasks.
    pub progress: u8,
    pub error: Option<String>,
    pub result: Option<Value>,
    /// Millisecond timestamp (Unix epoch) — set at creation.
    pub created_at_ms: u64,
    pub started_at_ms: Option<u64>,
    pub finished_at_ms: Option<u64>,
}

impl TaskRecord {
    pub fn new(id: TaskId, name: &str, kind: TaskKind, priority: Priority) -> Self {
        TaskRecord {
            id,
            name: name.to_string(),
            kind,
            priority,
            status: TaskStatus::Queued,
            progress: 0,
            error: None,
            result: None,
            created_at_ms: crate::core::runtime::now_ms(),
            started_at_ms: None,
            finished_at_ms: None,
        }
    }

    /// JSON form for `/api/task/:id` and WS `task.status` events.
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        m.insert("task_id".to_string(), crate::core::json::num_val(self.id));
        m.insert("name".to_string(), crate::core::json::str_val(&self.name));
        m.insert("status".to_string(), crate::core::json::str_val(self.status.name()));
        m.insert("kind".to_string(), crate::core::json::str_val(kind_name(self.kind)));
        m.insert(
            "priority".to_string(),
            crate::core::json::str_val(self.priority.name()),
        );
        m.insert("progress".to_string(), crate::core::json::num_val(self.progress));
        if let Some(e) = &self.error {
            m.insert("error".to_string(), crate::core::json::str_val(e));
        }
        if let Some(r) = &self.result {
            m.insert("result".to_string(), r.clone());
        }
        m.insert("created_at".to_string(), crate::core::json::num_val(self.created_at_ms));
        if let Some(s) = self.started_at_ms {
            m.insert("started_at".to_string(), crate::core::json::num_val(s));
        }
        if let Some(f) = self.finished_at_ms {
            m.insert("finished_at".to_string(), crate::core::json::num_val(f));
        }
        Value::Obj(m)
    }
}

pub fn kind_name(kind: TaskKind) -> &'static str {
    match kind {
        TaskKind::FastQuery => "fast_query",
        TaskKind::BackgroundQuery => "background_query",
        TaskKind::Interactive => "interactive",
        TaskKind::LongRunning => "long_running",
        TaskKind::Exclusive => "exclusive",
        TaskKind::Periodic => "periodic",
    }
}

/// A scheduled background job: runs every `interval`, never overlapping.
/// Stored behind `Arc` in the manager, so `last_run` is interior-mutable.
pub struct PeriodicJob {
    pub name: String,
    pub kind: TaskKind,
    pub priority: Priority,
    pub interval: std::time::Duration,
    /// Cadence used while no frontend is watching (`ActivityGate` idle):
    /// `Some(d)` keeps a slow heartbeat (temperature / registration),
    /// `None` pauses the job entirely until a frontend shows up.
    pub idle_interval: Option<std::time::Duration>,
    pub last_run: std::sync::Mutex<Option<Instant>>,
    pub running: std::sync::atomic::AtomicBool,
}

impl PeriodicJob {
    pub fn new(name: &str, interval: std::time::Duration, priority: Priority) -> Self {
        PeriodicJob {
            name: name.to_string(),
            kind: TaskKind::Periodic,
            priority,
            interval,
            idle_interval: None,
            last_run: std::sync::Mutex::new(None),
            running: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Keep running slowly while idle instead of pausing.
    pub fn with_idle_interval(mut self, idle: std::time::Duration) -> Self {
        self.idle_interval = Some(idle);
        self
    }

    /// Should this job fire now? (interval elapsed, not already running)
    pub fn due(&self, now: Instant) -> bool {
        self.due_after(now, self.interval)
    }

    /// Is the job due at `now` given an effective interval? Used by the
    /// manager to shorten/lengthen a job's cadence with frontend activity
    /// without re-registering it.
    pub fn due_after(&self, now: Instant, interval: std::time::Duration) -> bool {
        if self.running.load(std::sync::atomic::Ordering::Relaxed) {
            return false;
        }
        let last = self.last_run.lock().unwrap_or_else(|p| p.into_inner());
        match *last {
            None => true,
            Some(t) => now.duration_since(t) >= interval,
        }
    }

    pub fn mark_started(&self, now: Instant) {
        self.running.store(true, std::sync::atomic::Ordering::Relaxed);
        *self.last_run.lock().unwrap_or_else(|p| p.into_inner()) = Some(now);
    }

    pub fn mark_finished(&self) {
        self.running.store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_rank_ordering() {
        assert!(Priority::Sms.rank() > Priority::Control.rank());
        assert!(Priority::Control.rank() > Priority::UserQuery.rank());
        assert!(Priority::UserQuery.rank() > Priority::Polling.rank());
        assert!(Priority::Polling.rank() > Priority::Maintenance.rank());
        assert_eq!(Priority::Sms.name(), "P0");
        assert!(Priority::Sms.is_sms());
        assert!(!Priority::Control.is_sms());
        assert!(Priority::UserQuery.is_user_visible());
        assert!(!Priority::Polling.is_user_visible());
    }

    #[test]
    fn priority_records_use_contract_names() {
        assert_eq!(Priority::Maintenance.name(), "P4");
        assert_eq!(Priority::Maintenance.label(), "maintenance");
        assert_eq!(Priority::Polling.name(), "P3");
    }

    #[test]
    fn status_names_and_terminal() {
        assert_eq!(TaskStatus::Queued.name(), "queued");
        assert!(!TaskStatus::Running.is_terminal());
        assert!(TaskStatus::Completed.is_terminal());
        assert!(TaskStatus::Timeout.is_terminal());
    }

    #[test]
    fn record_json_shape() {
        let mut r = TaskRecord::new(7, "scan.cell", TaskKind::Exclusive, Priority::Control);
        r.status = TaskStatus::Progress;
        r.progress = 42;
        let v = r.to_json();
        assert!(v.dump().contains("\"task_id\":7"));
        assert!(v.dump().contains("\"status\":\"progress\""));
        assert!(v.dump().contains("\"progress\":42"));
    }

    #[test]
    fn periodic_job_due_behavior() {
        let j = PeriodicJob::new("signal", std::time::Duration::from_millis(10), Priority::Polling);
        let now = Instant::now();
        assert!(j.due(now)); // first run is immediately due
        j.mark_started(now);
        assert!(!j.due(now)); // running suppresses
        j.mark_finished();
        assert!(!j.due(now)); // interval not elapsed
        let later = now + std::time::Duration::from_millis(50);
        assert!(j.due(later));
    }
}
