//! Task manager — the heart of the async MT5700M backend.
//!
//! Every unit of work (a background snapshot refresh, an interactive SMS
//! send, a long cell scan, a band-lock transition) is registered here,
//! tracked through its lifecycle and observed via `task.*` events:
//!
//!   Queued -> Running -> (Completed | Failed | Cancelled | Timeout)
//!
//! Responsibilities:
//!
//!   * **registry** — task records (id/kind/priority/status/progress/result),
//!   * **lifecycle events** — `task.queued/started/progress/completed/failed/
//!     cancelled/timeout` published on the event bus,
//!   * **cancellation** — `cancel(id)` flips a per-task flag which is also
//!     wired into in-flight AT requests (so scans abort on the wire),
//!   * **timeouts** — a watchdog marks stuck tasks `Timeout` instead of
//!     letting them run forever,
//!   * **periodic jobs** — background refreshers (signal/network/temperature/
//!     traffic/sim/modem-info) run here, never overlapping, at fixed
//!     intervals, with background priority so user actions are not starved,
//!   * **backpressure / bounds** — records are pruned, at most one instance
//!     of each periodic job runs at a time, and the AT channel itself is
//!     bounded by the arbiter.
//!
//! The manager is deliberately thread-light: task work is spawned per task,
//! the AT channel is a single arbiter (see `at_queue`), and no UI path ever
//! blocks waiting for the modem — callers either read the cache or receive a
//! `task_id` and observe events.

use crate::scheduler::arbiter::{AtArbiter, AtRequestSpec, AtResult};
use crate::core::error::BackendError;
use crate::state::bus::{EventBus, TOPIC_TASK};
use crate::core::json::{self, Value};
use crate::core::runtime::{next_id, now_ms, spawn_thread};
use crate::state::cache::StateCache;
use crate::core::task::{PeriodicJob, Priority, TaskId, TaskKind, TaskRecord, TaskStatus};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Upper bound on retained task records (old terminal records are pruned).
const MAX_RECORDS: usize = 512;
/// Terminal records are dropped after this age.
const RECORD_TTL: Duration = Duration::from_secs(300);
/// Watchdog cadence for task timeouts.
const WATCHDOG_TICK: Duration = Duration::from_millis(500);
/// Periodic job polling cadence.
const JOB_TICK: Duration = Duration::from_secs(1);

/// Outcome of a task closure.
pub type TaskResult = Result<Value, BackendError>;
/// One-shot task body: runs once with a context that gives access to the
/// AT channel, the state cache and the event bus.
pub type TaskFn = Box<dyn FnOnce(&TaskCtx) -> TaskResult + Send + 'static>;
/// Periodic job body: re-invoked every interval (must be shareable).
pub type PeriodicFn = Box<dyn Fn(&TaskCtx) -> TaskResult + Send + Sync + 'static>;

/// Per-task execution context handed to task bodies.
pub struct TaskCtx {
    pub arbiter: Arc<AtArbiter>,
    pub cache: Arc<StateCache>,
    pub bus: Arc<EventBus>,
    pub task_id: TaskId,
    cancel: Arc<AtomicBool>,
    progress_slot: Arc<Mutex<(u8, String)>>,
    /// Overall task deadline (None = no watchdog timeout).
    deadline: Option<Instant>,
}

impl TaskCtx {
    /// True when `cancel(task_id)` was called (or the watchdog timed out).
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// Report progress. Emits a live `task.progress` event and stores the
    /// latest value on the task record.
    pub fn progress(&self, pct: u8, note: &str) {
        *self.progress_slot.lock().unwrap() = (pct.min(100), note.to_string());
        let mut m = std::collections::BTreeMap::new();
        m.insert("task_id".to_string(), json::num_val(self.task_id));
        m.insert("status".to_string(), json::str_val("progress"));
        m.insert("progress".to_string(), json::num_val(pct.min(100)));
        m.insert("note".to_string(), json::str_val(note));
        self.bus
            .publish_now(TOPIC_TASK, "task.progress", Value::Obj(m));
    }

    /// Submit an AT request through the single arbiter, wired to this task's
    /// cancel flag, and wait (bounded) for the result. Never blocks other
    /// tasks: it only waits on the shared channel like every other consumer.
    pub fn at_request(&self, mut spec: AtRequestSpec) -> AtResult {
        spec.cancel = Some(self.cancel.clone());
        let rx = self.arbiter.submit(spec.clone());
        let budget = spec.queued_timeout + spec.timeout + Duration::from_secs(2);
        match rx.recv_timeout(budget) {
            Ok(r) => r,
            Err(_) => Err(BackendError::TaskTimeout),
        }
    }

    /// Fast read-only query (dedup + retry through the arbiter).
    pub fn query(&self, command: &str) -> AtResult {
        self.at_request(AtRequestSpec::fast_query(command))
    }

    /// Interactive write action (no dedup, no retry).
    pub fn action(&self, command: &str) -> AtResult {
        self.at_request(AtRequestSpec::interactive(command))
    }
}

struct Inner {
    records: HashMap<TaskId, TaskRecord>,
    cancels: HashMap<TaskId, Arc<AtomicBool>>,
    deadlines: HashMap<TaskId, Instant>,
    jobs: Vec<Arc<PeriodicTask>>,
    stop: bool,
}

/// A registered periodic job (interval + shareable body).
struct PeriodicTask {
    job: PeriodicJob,
    f: PeriodicFn,
    timeout: Option<Duration>,
}

/// Shared task manager (clone = shared handle).
pub struct TaskManager {
    arbiter: Arc<AtArbiter>,
    cache: Arc<StateCache>,
    bus: Arc<EventBus>,
    inner: Mutex<Inner>,
}

impl TaskManager {
    /// Create the manager and start the periodic-job runner + watchdog.
    pub fn new(arbiter: Arc<AtArbiter>, cache: Arc<StateCache>, bus: Arc<EventBus>) -> Arc<Self> {
        let m = Arc::new(TaskManager {
            arbiter,
            cache,
            bus,
            inner: Mutex::new(Inner {
                records: HashMap::new(),
                cancels: HashMap::new(),
                deadlines: HashMap::new(),
                jobs: Vec::new(),
                stop: false,
            }),
        });
        let jobs = m.clone();
        spawn_thread("task-jobs", move || jobs.job_loop());
        let wd = m.clone();
        spawn_thread("task-watchdog", move || wd.watchdog_loop());
        m
    }

    pub fn stop(&self) {
        self.inner.lock().unwrap().stop = true;
    }

    /// Register a one-shot task and run it on a worker thread.
    /// Takes `self: &Arc<Self>` so the worker can hold its own Arc handle.
    pub fn submit(
        self: &Arc<Self>,
        name: &str,
        kind: TaskKind,
        priority: Priority,
        timeout: Option<Duration>,
        f: TaskFn,
    ) -> TaskId {
        let id = next_id();
        {
            let mut inner = self.inner.lock().unwrap();
            inner.records.insert(id, TaskRecord::new(id, name, kind, priority));
            inner.cancels.insert(id, Arc::new(AtomicBool::new(false)));
            if let Some(t) = timeout {
                inner.deadlines.insert(id, Instant::now() + t);
            }
        }
        self.emit(id, "task.queued", None);
        let this = self.clone();
        spawn_thread("task-worker", move || this.run(id, f, None));
        id
    }

    /// The event bus this manager publishes task events on. Exposed so the
    /// control-socket API path (which has no bus handle of its own) can serve
    /// the same registry as the WebSocket and RPC transports.
    pub fn bus(&self) -> &Arc<EventBus> {
        &self.bus
    }

    /// Register a periodic job. Runs immediately on the next job tick, then
    /// every `interval`. Never overlaps (a long run delays the next run).
    pub fn add_periodic(
        &self,
        name: &str,
        interval: Duration,
        priority: Priority,
        timeout: Option<Duration>,
        f: PeriodicFn,
    ) {
        let job = PeriodicJob::new(name, interval, priority);
        let pt = Arc::new(PeriodicTask { job, f, timeout });
        self.inner.lock().unwrap().jobs.push(pt);
    }

    /// Request cancellation. Also flips the flag wired into in-flight AT
    /// requests so interruptible commands (scan) abort on the wire.
    pub fn cancel(&self, id: TaskId) -> bool {
        let flag = self.inner.lock().unwrap().cancels.get(&id).cloned();
        let Some(flag) = flag else { return false };
        flag.store(true, Ordering::SeqCst);
        {
            let mut inner = self.inner.lock().unwrap();
            if let Some(r) = inner.records.get_mut(&id) {
                if !r.status.is_terminal() {
                    r.status = TaskStatus::Cancelled;
                    r.finished_at_ms = Some(now_ms());
                    inner.deadlines.remove(&id);
                }
            }
        }
        self.emit(id, "task.cancelled", None);
        true
    }

    /// Snapshot of one task record.
    pub fn get(&self, id: TaskId) -> Option<TaskRecord> {
        self.inner.lock().unwrap().records.get(&id).cloned()
    }

    /// All records, newest first (bounded, pruned).
    pub fn list(&self) -> Vec<TaskRecord> {
        let mut v: Vec<TaskRecord> = self
            .inner
            .lock()
            .unwrap()
            .records
            .values()
            .cloned()
            .collect();
        v.sort_by_key(|r| std::cmp::Reverse(r.id));
        v
    }

    /// True when any non-terminal task of the given kinds exists (e.g. scan).
    pub fn has_active(&self, kinds: &[TaskKind]) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.records.values().any(|r| {
            !r.status.is_terminal() && kinds.iter().any(|k| *k == r.kind)
        })
    }

    /// Cancel every non-terminal task of the given kinds (e.g. when the USB
    /// modem vanishes). Returns how many tasks were marked cancelled.
    pub fn cancel_active(&self, kinds: &[TaskKind]) -> usize {
        let ids: Vec<TaskId> = {
            let inner = self.inner.lock().unwrap();
            inner
                .records
                .iter()
                .filter_map(|(id, r)| {
                    (!r.status.is_terminal() && kinds.iter().any(|k| *k == r.kind))
                        .then_some(*id)
                })
                .collect()
        };
        for id in &ids {
            self.cancel(*id);
        }
        ids.len()
    }

    /// Block (bounded) until a task reaches a terminal state; used by the
    /// LuCI control socket which still expects request -> result semantics.
    pub fn await_task(&self, id: TaskId, timeout: Duration) -> TaskResult {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(r) = self.get(id) {
                if r.status.is_terminal() {
                    return match r.status {
                        TaskStatus::Completed => Ok(r.result.unwrap_or(Value::Null)),
                        TaskStatus::Cancelled => Err(BackendError::TaskCancelled),
                        TaskStatus::Timeout => Err(BackendError::TaskTimeout),
                        _ => Err(BackendError::Internal(r.error.clone().unwrap_or_default())),
                    };
                }
            }
            if Instant::now() >= deadline {
                return Err(BackendError::TaskTimeout);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    // ------------------------------------------------------------ internals

    fn run(
        self: Arc<Self>,
        id: TaskId,
        f: TaskFn,
        periodic: Option<Arc<PeriodicTask>>,
    ) {
        let cancel = self.inner.lock().unwrap().cancels.get(&id).cloned();
        let Some(cancel) = cancel else { return };
        {
            let mut inner = self.inner.lock().unwrap();
            if let Some(r) = inner.records.get_mut(&id) {
                r.status = TaskStatus::Running;
                r.started_at_ms = Some(now_ms());
            }
        }
        self.emit(id, "task.started", None);

        let progress_slot = Arc::new(Mutex::new((0u8, String::new())));
        let deadline = self.inner.lock().unwrap().deadlines.get(&id).copied();
        let ctx = TaskCtx {
            arbiter: self.arbiter.clone(),
            cache: self.cache.clone(),
            bus: self.bus.clone(),
            task_id: id,
            cancel: cancel.clone(),
            progress_slot: progress_slot.clone(),
            deadline,
        };
        let outcome = f(&ctx);
        let (pct, note) = progress_slot.lock().unwrap().clone();

        // Watchdog may have already marked the record Timeout — honour it.
        let existing = self.inner.lock().unwrap().records.get(&id).map(|r| r.status);
        let status = match existing {
            Some(TaskStatus::Timeout) => TaskStatus::Timeout,
            Some(TaskStatus::Cancelled) => TaskStatus::Cancelled,
            _ => {
                if ctx.cancelled() {
                    TaskStatus::Cancelled
                } else {
                    match &outcome {
                        Ok(_) => TaskStatus::Completed,
                        Err(BackendError::TaskCancelled) => TaskStatus::Cancelled,
                        Err(BackendError::TaskTimeout) => TaskStatus::Timeout,
                        Err(_) => TaskStatus::Failed,
                    }
                }
            }
        };

        {
            let mut inner = self.inner.lock().unwrap();
            if let Some(r) = inner.records.get_mut(&id) {
                r.status = status;
                r.progress = pct;
                r.finished_at_ms = Some(now_ms());
                match &outcome {
                    Ok(v) => r.result = Some(v.clone()),
                    Err(e) => r.error = Some(e.message()),
                }
                inner.deadlines.remove(&id);
            }
        }
        let event = match status {
            TaskStatus::Completed => "task.completed",
            TaskStatus::Failed => "task.failed",
            TaskStatus::Cancelled => "task.cancelled",
            TaskStatus::Timeout => "task.timeout",
            _ => "task.started",
        };
        self.emit(id, event, Some(note));
        if let Some(pt) = periodic {
            pt.job.mark_finished();
        }
        self.prune();
    }

    fn emit(&self, id: TaskId, event: &str, note: Option<String>) {
        let rec = self.get(id);
        let Some(rec) = rec else { return };
        let Value::Obj(mut m) = rec.to_json() else { return };
        if let Some(n) = note {
            m.insert("note".to_string(), json::str_val(&n));
        }
        self.bus.publish_now(TOPIC_TASK, event, Value::Obj(m));
    }

    fn prune(&self) {
        let now = crate::core::runtime::now_ms();
        let mut inner = self.inner.lock().unwrap();
        let stale: Vec<TaskId> = inner
            .records
            .iter()
            .filter_map(|(id, r)| {
                let old = r
                    .finished_at_ms
                    .map(|f| now.saturating_sub(f) >= RECORD_TTL.as_millis() as u64)
                    .unwrap_or(false);
                (r.status.is_terminal() && old).then_some(*id)
            })
            .collect();
        let overflow: Vec<TaskId> = if inner.records.len() > MAX_RECORDS {
            let mut ids: Vec<TaskId> = inner.records.keys().copied().collect();
            ids.sort();
            ids.truncate(inner.records.len() - MAX_RECORDS);
            ids
        } else {
            Vec::new()
        };
        for id in stale.into_iter().chain(overflow) {
            inner.records.remove(&id);
            inner.cancels.remove(&id);
            inner.deadlines.remove(&id);
        }
    }

    fn job_loop(self: Arc<Self>) {
        loop {
            if self.inner.lock().unwrap().stop {
                return;
            }
            let jobs: Vec<Arc<PeriodicTask>> = self.inner.lock().unwrap().jobs.clone();
            let now = Instant::now();
            for pt in &jobs {
                if pt.job.due(now) {
                    pt.job.mark_started(now);
                    let id = next_id();
                    {
                        let mut inner = self.inner.lock().unwrap();
                        inner.records.insert(
                            id,
                            TaskRecord::new(id, &pt.job.name, TaskKind::Periodic, pt.job.priority),
                        );
                        inner.cancels.insert(id, Arc::new(AtomicBool::new(false)));
                        if let Some(t) = pt.timeout {
                            inner.deadlines.insert(id, Instant::now() + t);
                        }
                    }
                    self.emit(id, "task.queued", None);
                    let this = self.clone();
                    let pt = pt.clone();
                    // Clone the Arc for the body closure; `pt.f` itself cannot
                    // be cloned (dyn Fn), but an Arc<PeriodicTask> clone keeps
                    // the shared body alive for the worker.
                    let body_pt = pt.clone();
                    let body = Box::new(move |ctx: &TaskCtx| (body_pt.f)(ctx));
                    spawn_thread("periodic", move || this.run(id, body, Some(pt)));
                }
            }
            std::thread::sleep(JOB_TICK);
        }
    }

    fn watchdog_loop(self: Arc<Self>) {
        loop {
            if self.inner.lock().unwrap().stop {
                return;
            }
            let now = Instant::now();
            let overdue: Vec<TaskId> = {
                let inner = self.inner.lock().unwrap();
                inner
                    .records
                    .iter()
                    .filter_map(|(id, r)| {
                        if r.status == TaskStatus::Running || r.status == TaskStatus::Queued {
                            if let Some(d) = inner.deadlines.get(id) {
                                if now >= *d {
                                    return Some(*id);
                                }
                            }
                        }
                        None
                    })
                    .collect()
            };
            for id in overdue {
                let flag = self.inner.lock().unwrap().cancels.get(&id).cloned();
                if let Some(flag) = flag {
                    flag.store(true, Ordering::SeqCst);
                }
                {
                    let mut inner = self.inner.lock().unwrap();
                    if let Some(r) = inner.records.get_mut(&id) {
                        r.status = TaskStatus::Timeout;
                        r.error = Some("任务超时".into());
                        r.finished_at_ms = Some(now_ms());
                        inner.deadlines.remove(&id);
                    }
                }
                self.emit(id, "task.timeout", None);
            }
            std::thread::sleep(WATCHDOG_TICK);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::arbiter::AtTransport;

    /// Minimal fake transport for task-manager tests (single OK response).
    struct FakeT {
        connected: bool,
    }
    impl FakeT {
        fn new() -> Arc<Self> {
            Arc::new(FakeT { connected: true })
        }
    }
    impl AtTransport for FakeT {
        fn send(&self, _command: &str, timeout: Duration) -> AtResult {
            std::thread::sleep(timeout.min(Duration::from_millis(5)));
            Ok("OK".into())
        }
        fn send_interruptible(
            &self,
            _command: &str,
            timeout: Duration,
            cancel: &AtomicBool,
            _abort: Option<&[u8]>,
        ) -> AtResult {
            let mut waited = Duration::ZERO;
            let step = Duration::from_millis(5);
            loop {
                if cancel.load(Ordering::SeqCst) {
                    return Err(BackendError::TaskCancelled);
                }
                waited += step;
                std::thread::sleep(step);
                if waited >= timeout {
                    return Err(BackendError::AtTimeout);
                }
                if waited >= Duration::from_millis(10) {
                    return Ok("OK".into());
                }
            }
        }
        fn connected(&self) -> bool {
            self.connected
        }
    }

    fn harness() -> (Arc<TaskManager>, Arc<FakeT>) {
        let t = FakeT::new();
        let arbiter = AtArbiter::new(t.clone());
        let cache = Arc::new(StateCache::new());
        let bus = EventBus::new();
        (TaskManager::new(arbiter, cache, bus), t)
    }

    #[test]
    fn task_lifecycle_completes() {
        let (m, _t) = harness();
        let id = m.submit(
            "test",
            TaskKind::BackgroundQuery,
            Priority::Background,
            None,
            Box::new(|_ctx| Ok(json::num_val(42))),
        );
        let rec = m.await_task(id, Duration::from_secs(2)).expect("ok");
        assert_eq!(rec.as_u64(), Some(42));
        let r = m.get(id).unwrap();
        assert_eq!(r.status, TaskStatus::Completed);
    }

    #[test]
    fn cancel_marks_cancelled() {
        let (m, _t) = harness();
        let id = m.submit(
            "test",
            TaskKind::LongRunning,
            Priority::Normal,
            None,
            Box::new(|ctx| {
                for _ in 0..50 {
                    if ctx.cancelled() {
                        return Err(BackendError::TaskCancelled);
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(json::num_val(1))
            }),
        );
        std::thread::sleep(Duration::from_millis(30));
        assert!(m.cancel(id));
        let _ = m.await_task(id, Duration::from_secs(2));
        assert_eq!(m.get(id).unwrap().status, TaskStatus::Cancelled);
    }

    #[test]
    fn watchdog_times_out_stuck_task() {
        let (m, _t) = harness();
        let id = m.submit(
            "stuck",
            TaskKind::LongRunning,
            Priority::Normal,
            Some(Duration::from_millis(200)),
            Box::new(|_ctx| {
                std::thread::sleep(Duration::from_secs(5));
                Ok(json::num_val(1))
            }),
        );
        let _ = m.await_task(id, Duration::from_secs(3));
        assert_eq!(m.get(id).unwrap().status, TaskStatus::Timeout);
    }

    #[test]
    fn failed_task_records_error() {
        let (m, _t) = harness();
        let id = m.submit(
            "fail",
            TaskKind::Interactive,
            Priority::Interactive,
            None,
            Box::new(|_ctx| Err(BackendError::AtTimeout)),
        );
        let _ = m.await_task(id, Duration::from_secs(2));
        let r = m.get(id).unwrap();
        assert_eq!(r.status, TaskStatus::Failed);
        assert!(r.error.as_deref().unwrap_or("").contains("超时"));
    }

    #[test]
    fn periodic_runs_multiple_times() {
        let (m, _t) = harness();
        let counter = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let c = counter.clone();
        m.add_periodic(
            "tick",
            Duration::from_millis(80),
            Priority::Background,
            None,
            Box::new(move |_ctx| {
                c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(json::num_val(1))
            }),
        );
        // The job loop ticks once per second: wait past two full ticks,
        // leaving slack for worker-thread scheduling under parallel tests.
        std::thread::sleep(Duration::from_millis(2200));
        assert!(counter.load(std::sync::atomic::Ordering::Relaxed) >= 2);
        m.stop();
    }
}
