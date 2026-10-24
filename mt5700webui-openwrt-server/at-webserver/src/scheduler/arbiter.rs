//! AT request queue / arbiter — the single choke point to the modem.
//!
//! Every AT exchange in the backend (WebSocket command path, LuCI control
//! socket, background collectors, scans, band-lock scheduler, SMS) must go
//! through this arbiter. It guarantees:
//!
//!   * **one transaction at a time** — a single executor thread, and a
//!     response (or a verified safe abort) must finish before the next write;
//!   * **25 ms wire pacing** — the start of every AT write is separated from
//!     the previous one by at least [`crate::scheduler::pacer::MIN_SEND_GAP`],
//!     enforced here and again in the transport (SMS multi-step writes);
//!   * **strict P0..P4 priority** — SMS (P0), key control (P1), user query
//!     (P2), routine polling (P3), maintenance (P4);
//!   * **SMS transaction isolation** — while an SMS transaction is open, only
//!     P0 requests may start. An in-flight non-SMS transaction is never
//!     interrupted; the SMS waits for it to reach a safe boundary;
//!   * **request deduplication** (N identical reads -> 1 AT exchange),
//!   * **per-request timeout** (a slow AT can never freeze the backend),
//!   * **queue deadline / backpressure** (requests that wait too long fail
//!     fast instead of piling up),
//!   * **unified retry** (timeout/transport faults retry with backoff;
//!     anchored modem errors never do),
//!   * **exclusive access** (scan/flash/reboot own the channel; ordinary
//!     requests queue and expire cleanly),
//!   * **cooperative cancellation** (a cancel flag can inject an abort
//!     token on the wire for interruptible commands like scan),
//!   * **an audit trail** with id/priority/enqueue/send/finish/result for
//!     every request (served by `scheduler.audit`).

use crate::core::channel::SmsPart;
use crate::core::error::BackendError;
use crate::core::runtime::{next_id, now_ms};
use crate::core::task::Priority;
use crate::scheduler::pacer;
use std::cell::Cell;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

// ------------------------------------------------------------------ transport

/// The blocking AT transport the arbiter drives. Implemented by the daemon's
/// `AtClient` (serial/TCP) so the arbiter stays transport-agnostic.
pub trait AtTransport: Send + Sync {
    /// Send one command and wait for the final result.
    fn send(&self, command: &str, timeout: Duration) -> Result<String, BackendError>;
    /// Send one command, polling `cancel` each read cycle; when cancelled,
    /// optionally inject `abort_wire` (e.g. the scan-abort token) and keep
    /// draining until the modem settles.
    fn send_interruptible(
        &self,
        command: &str,
        timeout: Duration,
        cancel: &AtomicBool,
        abort_wire: Option<&[u8]>,
    ) -> Result<String, BackendError>;
    /// Whether a transport is currently attached (serial open / TCP up).
    fn connected(&self) -> bool;
    /// Multi-phase SMS send (PDU mode). Default: unsupported. Implemented by
    /// the daemon transport; executed on the single arbiter thread so the
    /// whole CMGS transaction is serialised with every other AT exchange.
    fn send_sms_pdu(&self, _parts: &[SmsPart]) -> Result<String, BackendError> {
        Err(BackendError::TransportError(
            "SMS transport not supported".into(),
        ))
    }
}

// ------------------------------------------------------------------ retry

/// Unified retry policy.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub base_backoff: Duration,
    pub retryable: fn(&BackendError) -> bool,
}

impl RetryPolicy {
    /// No retry — for write commands that must not re-fire.
    pub fn none() -> Self {
        RetryPolicy {
            max_retries: 0,
            base_backoff: Duration::from_millis(0),
            retryable: |_| false,
        }
    }

    /// Default for read/query commands: up to 2 retries with 200 ms backoff,
    /// only for transient faults (timeout / transport / busy).
    pub fn queries() -> Self {
        RetryPolicy {
            max_retries: 2,
            base_backoff: Duration::from_millis(200),
            retryable: BackendError::retryable,
        }
    }

    /// One retry for critical recovery flows.
    pub fn recovery() -> Self {
        RetryPolicy {
            max_retries: 1,
            base_backoff: Duration::from_millis(500),
            retryable: BackendError::retryable,
        }
    }
}

// ------------------------------------------------------------------ request

/// Fallback AT timeout for a user-visible read whose command is unknown.
///
/// Sized for the slow tail rather than the fast case. On a single exclusive
/// channel an over-short timeout is the expensive mistake: the request fails
/// and, if it retries, re-occupies the channel and starves the other frontend
/// as well. An over-long timeout only costs idle time on a command that has
/// already failed.
const UI_QUERY_DEFAULT_TIMEOUT: Duration = Duration::from_secs(12);

/// AT timeout for one user-visible read, matched on the uppercase command with
/// any `=payload` suffix stripped.
///
/// Latencies are device measurements, not guesses: `AT^HCSQ?` answers in
/// ~4 s, `AT^HFREQINFO?` in ~3.4 s, `AT^CHIPTEMP?` in ~0.1 s but ~6 s
/// end-to-end while the channel is loaded, `AT^NTXPOWER?` in ~11.6 s. Commands
/// the modem does not implement still burn their entire budget before giving
/// up, so they are bounded rather than left to starve a page render.
fn ui_query_timeout(upper: &str) -> Duration {
    let base = upper.split('=').next().unwrap_or(upper);
    let secs: u64 = match base {
        // Fast, well-behaved queries.
        "AT" | "AT+CSQ" | "AT+CEREG?" | "AT+CREG?" | "AT+C5GREG?" | "AT+CPIN?"
        | "AT+CIMI" | "ATI" | "AT+CGMR" | "AT+CGSN" | "AT^ICCID?" | "AT^SIMSQ?" => 6,
        // Signal: ~4 s observed.
        "AT^HCSQ?" | "AT^HCSQ" => 10,
        // Carrier / frequency: ~3.4 s observed.
        "AT^HFREQINFO?" | "AT^HFREQINFO" | "AT+COPS?" => 12,
        // Temperature measured 6.4 s end-to-end under load.
        "AT^CHIPTEMP?" | "AT^CHIPTEMP" => 8,
        // Slow / frequently unsupported: bounded so one page load cannot walk
        // away without a render.
        "AT^NTXPOWER?" | "AT^MONSC" | "AT^LENDC?" | "AT^TXPOWER?" | "AT^SYSINFOEX"
        | "AT^C5GOPTION?" | "AT^NRRCCAPQRY" | "AT^CASCELLINFO?" => 12,
        _ => UI_QUERY_DEFAULT_TIMEOUT.as_secs(),
    };
    Duration::from_secs(secs)
}

/// Payload carried with a request. `None` is a plain AT command; `Sms` is a
/// multi-phase PDU SMS send executed on the single arbiter thread.
#[derive(Debug, Clone)]
pub enum AtPayload {
    None,
    SMS { parts: Vec<SmsPart> },
}

/// A request to the AT channel. Callers build this, then `submit`.
#[derive(Debug, Clone)]
pub struct AtRequestSpec {
    pub command: String,
    pub priority: Priority,
    /// Per-attempt AT timeout.
    pub timeout: Duration,
    /// Max time allowed in the queue before failing (backpressure).
    pub queued_timeout: Duration,
    /// Exclusive: only one such request may be queued/running at a time;
    /// a second is rejected immediately.
    pub exclusive: bool,
    /// Deduplication key (usually the command itself). Requests with an
    /// existing in-flight key share the result instead of re-executing.
    pub dedup_key: Option<String>,
    pub retry: RetryPolicy,
    /// Optional cancel flag; when set the request fails (or aborts on the
    /// wire for interruptible commands).
    pub cancel: Option<Arc<AtomicBool>>,
    /// Wire bytes to inject on cancel (e.g. "abcd\r" for ^CELLSCAN).
    pub abort_wire: Option<Vec<u8>>,
    /// Payload other than a plain command (SMS send).
    pub payload: AtPayload,
    /// Human-readable purpose for logs.
    pub label: String,
    /// SMS-class request. Set automatically for `AT+CMGS` payloads and for
    /// any command issued inside an SMS transaction; never set by callers
    /// that merely want a higher priority.
    pub sms: bool,
}

impl AtRequestSpec {
    fn base(command: &str) -> Self {
        AtRequestSpec {
            command: command.to_string(),
            priority: Priority::UserQuery,
            timeout: Duration::from_secs(5),
            queued_timeout: Duration::from_secs(10),
            exclusive: false,
            dedup_key: None,
            retry: RetryPolicy::none(),
            cancel: None,
            abort_wire: None,
            payload: AtPayload::None,
            label: String::new(),
            sms: false,
        }
    }

    /// Fast read-only query (3 s AT timeout, 3 s queue budget, dedup + retry).
    ///
    /// Only safe for commands that genuinely answer inside 3 s. The MT5700M
    /// answers `AT^HCSQ?` in ~4 s and `AT^CHIPTEMP?` in ~6 s, so callers that
    /// may hit those must use [`AtRequestSpec::ui_query`] instead.
    pub fn fast_query(command: &str) -> Self {
        let mut s = Self::base(command);
        s.priority = Priority::UserQuery;
        s.timeout = Duration::from_secs(3);
        s.queued_timeout = Duration::from_secs(3);
        s.dedup_key = Some(command.to_string());
        s.retry = RetryPolicy::queries();
        s.label = format!("query {}", command);
        s
    }

    /// Read-only query issued on behalf of a *user-visible* page (LuCI or the
    /// WebUI), sized from the MT5700M's measured command latencies.
    ///
    /// Rules, all derived from device measurements:
    ///   * the AT timeout covers the command's *observed* response time with
    ///     headroom, so a healthy read succeeds on the first attempt;
    ///   * `retry: none` — retrying a slow read re-occupies the exclusive
    ///     channel and multiplies contention instead of fixing it;
    ///   * `queued_timeout` is generous: a page that waits briefly behind a
    ///     background refresh is fine, being rejected with `Busy` is not;
    ///   * dedup is keyed on the command, so N concurrent identical reads from
    ///     LuCI *and* the WebUI collapse into a single AT exchange.
    pub fn ui_query(command: &str) -> Self {
        let upper = command.trim().to_ascii_uppercase();
        let mut s = Self::base(command);
        s.priority = Priority::UserQuery;
        s.timeout = ui_query_timeout(&upper);
        s.queued_timeout = Duration::from_secs(20);
        s.dedup_key = Some(command.to_string());
        s.retry = RetryPolicy::none();
        s.label = format!("ui query {}", command);
        s
    }

    /// Interactive user action (5 s AT timeout, generous queue budget, no
    /// dedup). P1: dial/disconnect/radio/recovery and manual writes.
    ///
    /// The queue budget covers an SMS transaction ahead of it (SMS is bounded
    /// to tens of seconds) without dropping the key operation.
    pub fn interactive(command: &str) -> Self {
        let mut s = Self::base(command);
        s.priority = Priority::Control;
        s.timeout = Duration::from_secs(5);
        s.queued_timeout = Duration::from_secs(60);
        s.label = format!("control {}", command);
        s
    }

    /// Long-running exclusive operation with abort injection (scan etc.).
    pub fn long_exclusive(command: &str, timeout: Duration, abort_wire: &[u8]) -> Self {
        let mut s = Self::base(command);
        s.priority = Priority::Control;
        s.timeout = timeout;
        s.queued_timeout = timeout + Duration::from_secs(5);
        s.exclusive = true;
        s.abort_wire = Some(abort_wire.to_vec());
        s.label = format!("exclusive {}", command);
        s
    }

    /// Background refresh query (P3 by default; callers may lower it to P4).
    pub fn background(command: &str) -> Self {
        let mut s = Self::base(command);
        s.priority = Priority::Polling;
        s.timeout = Duration::from_secs(5);
        s.queued_timeout = Duration::from_secs(8);
        s.dedup_key = Some(command.to_string());
        s.retry = RetryPolicy::queries();
        s.label = format!("background {}", command);
        s
    }

    /// SMS send: a P0 multi-phase `AT+CMGS` transaction (long timeout; the
    /// CMGS transaction can take a while on a busy network). The parts are
    /// the module's encoded PDUs.
    pub fn sms_send(parts: Vec<SmsPart>) -> Self {
        let mut s = Self::base("AT+CMGS");
        s.priority = Priority::Sms;
        s.timeout = Duration::from_secs(30);
        s.queued_timeout = Duration::from_secs(15);
        s.payload = AtPayload::SMS { parts };
        s.label = "sms send".to_string();
        s.sms = true;
        s
    }
}

pub type AtResult = Result<String, BackendError>;

struct QueuedRequest {
    id: u64,
    seq: u64,
    spec: AtRequestSpec,
    tx: std::sync::mpsc::SyncSender<AtResult>,
    enqueued_at: Instant,
}

/// Heap item: highest priority first, then earliest seq.
struct HeapItem(QueuedRequest);

impl PartialEq for HeapItem {
    fn eq(&self, o: &Self) -> bool {
        self.0.seq == o.0.seq
    }
}
impl Eq for HeapItem {}
impl PartialOrd for HeapItem {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for HeapItem {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.0
            .spec
            .priority
            .rank()
            .cmp(&o.0.spec.priority.rank())
            .then_with(|| o.0.seq.cmp(&self.0.seq))
    }
}

/// How long an SMS transaction may pause ordinary traffic before the
/// dispatcher stops honouring the hold. SMS operations are bounded (each AT
/// step has a timeout; the whole flow is single-digit to tens of seconds),
/// so reaching this ceiling means the owner thread is stuck. Ordinary
/// requests must not be blocked forever: the hold is ignored (and recorded)
/// while the stuck transaction eventually times out on its own.
const SMS_HOLD_MAX: Duration = Duration::from_secs(180);

struct QueueState {
    heap: BinaryHeap<HeapItem>,
    /// dedup_key -> waiters sharing the canonical execution.
    dedup: HashMap<String, Vec<std::sync::mpsc::SyncSender<AtResult>>>,
    /// True while an exclusive request is queued or running.
    exclusive_active: bool,
    seq: u64,
    stop: bool,
    /// Tokens of open SMS transactions (one per owning thread).
    sms_tokens: HashSet<u64>,
    /// Number of open SMS transactions (nested leases share one token).
    sms_hold_count: usize,
    sms_hold_started: Option<Instant>,
    sms_hold_expired_logged: bool,
}

// The SMS transaction token is bound to the thread that opened it: the SMS
// service runs synchronously on a route/CLI thread, and marking *every*
// request submitted while a transaction is open as P0 would let unrelated
// callers hijack the guard. A thread-local makes ownership explicit without
// threading a token through every module signature.
thread_local! {
    static SMS_TOKEN: Cell<Option<u64>> = Cell::new(None);
}

fn current_sms_token() -> Option<u64> {
    SMS_TOKEN.with(|c| c.get())
}

// ------------------------------------------------------------------ audit

/// One scheduler audit row. Times are Unix milliseconds; `wait_ms` and
/// `duration_ms` are derived. Served by `scheduler.audit` and used by the
/// automated 25 ms/priority acceptance tests.
#[derive(Debug, Clone)]
pub struct AuditRecord {
    pub id: u64,
    pub label: String,
    pub priority: &'static str,
    pub priority_label: &'static str,
    pub sms: bool,
    pub enqueued_at_ms: u64,
    pub started_at_ms: Option<u64>,
    pub finished_at_ms: Option<u64>,
    pub result: Option<&'static str>,
    pub error: Option<String>,
    pub attempts: u32,
}

impl AuditRecord {
    pub fn to_json(&self) -> crate::core::json::Value {
        use crate::core::json::{num_val, str_val, Value};
        let mut m = std::collections::BTreeMap::new();
        m.insert("task_id".to_string(), num_val(self.id));
        m.insert("label".to_string(), str_val(&self.label));
        m.insert("priority".to_string(), str_val(self.priority));
        m.insert("priority_class".to_string(), str_val(self.priority_label));
        m.insert("sms".to_string(), Value::Bool(self.sms));
        m.insert("enqueued_at".to_string(), num_val(self.enqueued_at_ms));
        if let Some(t) = self.started_at_ms {
            m.insert("sent_at".to_string(), num_val(t));
            m.insert(
                "wait_ms".to_string(),
                num_val(t.saturating_sub(self.enqueued_at_ms)),
            );
        }
        if let Some(t) = self.finished_at_ms {
            m.insert("finished_at".to_string(), num_val(t));
            if let Some(s) = self.started_at_ms {
                m.insert("duration_ms".to_string(), num_val(t.saturating_sub(s)));
            }
        }
        if let Some(r) = self.result {
            m.insert("result".to_string(), str_val(r));
        }
        if let Some(e) = &self.error {
            m.insert("error".to_string(), str_val(e));
        }
        m.insert("attempts".to_string(), num_val(self.attempts as i64));
        Value::Obj(m)
    }
}

struct AuditLog {
    rows: HashMap<u64, AuditRecord>,
    order: VecDeque<u64>,
    capacity: usize,
}

impl AuditLog {
    fn new(capacity: usize) -> Self {
        AuditLog {
            rows: HashMap::new(),
            order: VecDeque::new(),
            capacity,
        }
    }

    fn insert(&mut self, record: AuditRecord) {
        if self.order.len() >= self.capacity {
            if let Some(old) = self.order.pop_front() {
                self.rows.remove(&old);
            }
        }
        self.order.push_back(record.id);
        self.rows.insert(record.id, record);
    }

    fn update<F: FnOnce(&mut AuditRecord)>(&mut self, id: u64, f: F) {
        if let Some(r) = self.rows.get_mut(&id) {
            f(r);
        }
    }

    fn snapshot(&self, limit: usize) -> crate::core::json::Value {
        use crate::core::json::Value;
        let mut out = Vec::new();
        for id in self.order.iter().rev().take(limit) {
            if let Some(r) = self.rows.get(id) {
                out.push(r.to_json());
            }
        }
        Value::Arr(out)
    }
}

pub struct AtArbiter {
    queue: Mutex<QueueState>,
    cond: Condvar,
    stop: AtomicBool,
    /// True while a request is being executed (used by the URC monitor to
    /// avoid racing the reader buffer with an in-flight command).
    busy: AtomicBool,
    transport: Arc<dyn AtTransport>,
    max_queue: usize,
    audit: Mutex<AuditLog>,
    /// Total AT transactions started, for diagnostics.
    executed: AtomicU64,
}

/// Share of a one-second window that background collectors may occupy.
const BACKGROUND_DUTY_LIMIT_PCT: u64 = 60;

/// One-second sliding window over channel occupancy, in milliseconds.
struct DutyWindow {
    busy_ms: AtomicU64,
    last_tick_ms: AtomicU64,
}

impl DutyWindow {
    const fn new() -> Self {
        DutyWindow {
            busy_ms: AtomicU64::new(0),
            last_tick_ms: AtomicU64::new(0),
        }
    }

    fn record_and_check(&self, ms: u64) -> bool {
        let now = now_ms();
        let prev = self.last_tick_ms.load(Ordering::Relaxed);
        if prev == 0 {
            self.last_tick_ms.store(now, Ordering::Relaxed);
            return false;
        }
        self.busy_ms.fetch_add(ms, Ordering::Relaxed);
        let elapsed = now.saturating_sub(prev);
        const WINDOW_MS: u64 = 1000;
        if elapsed < WINDOW_MS {
            return false;
        }
        let busy = self.busy_ms.swap(0, Ordering::Relaxed);
        self.last_tick_ms.store(now, Ordering::Relaxed);
        busy * 100 >= elapsed * BACKGROUND_DUTY_LIMIT_PCT
    }
}

/// Process-wide duty window. A single global is correct here: there is exactly
/// one AT channel per daemon, so occupancy is inherently a global quantity.
static DUTY: DutyWindow = DutyWindow::new();

pub fn channel_budget_exhausted() -> bool {
    DUTY.record_and_check(0)
}

pub fn channel_user_starved() -> bool {
    USER_STARVED.load(Ordering::Relaxed)
}

pub fn clear_channel_starvation() {
    USER_STARVED.store(false, Ordering::Relaxed);
}

static USER_STARVED: AtomicBool = AtomicBool::new(false);

impl AtArbiter {
    pub fn new(transport: Arc<dyn AtTransport>) -> Arc<Self> {
        let arbiter = Arc::new(AtArbiter {
            queue: Mutex::new(QueueState {
                heap: BinaryHeap::new(),
                dedup: HashMap::new(),
                exclusive_active: false,
                seq: 0,
                stop: false,
                sms_tokens: HashSet::new(),
                sms_hold_count: 0,
                sms_hold_started: None,
                sms_hold_expired_logged: false,
            }),
            cond: Condvar::new(),
            stop: AtomicBool::new(false),
            busy: AtomicBool::new(false),
            transport,
            max_queue: 256,
            audit: Mutex::new(AuditLog::new(1024)),
            executed: AtomicU64::new(0),
        });
        let runner = arbiter.clone();
        crate::core::runtime::spawn_thread("at-arbiter", move || runner.executor_loop());
        arbiter
    }

    /// Queue capacity (backpressure bound). Default 256.
    pub fn set_max_queue(&mut self, n: usize) {
        self.max_queue = n;
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        self.cond.notify_all();
    }

    // ---------------------------------------------------------- SMS guard

    /// Open an SMS transaction on the current thread. While it is open the
    /// dispatcher only starts P0 requests; every request issued by this
    /// thread is automatically P0/SMS-class. Dropping the returned guard
    /// closes the transaction and wakes the queue.
    pub fn sms_transaction(self: &Arc<Self>) -> crate::core::channel::SmsTransaction {
        // Nested guard: the outer guard owns the hold.
        if let Some(token) = current_sms_token() {
            let q = self.queue.lock().unwrap_or_else(|p| p.into_inner());
            if q.sms_tokens.contains(&token) {
                return crate::core::channel::SmsTransaction::inactive();
            }
        }
        let token = next_id();
        {
            let mut q = self.queue.lock().unwrap_or_else(|p| p.into_inner());
            q.sms_tokens.insert(token);
            if q.sms_hold_count == 0 {
                q.sms_hold_started = Some(Instant::now());
                q.sms_hold_expired_logged = false;
            }
            q.sms_hold_count += 1;
            let mut audit = self.audit.lock().unwrap_or_else(|p| p.into_inner());
            audit.insert(AuditRecord {
                id: token,
                label: "sms.transaction".into(),
                priority: Priority::Sms.name(),
                priority_label: Priority::Sms.label(),
                sms: true,
                enqueued_at_ms: now_ms(),
                started_at_ms: Some(now_ms()),
                finished_at_ms: None,
                result: None,
                error: None,
                attempts: 0,
            });
        }
        SMS_TOKEN.with(|c| c.set(Some(token)));
        self.cond.notify_all();
        let this = self.clone();
        crate::core::channel::SmsTransaction::new(move || this.end_sms_transaction(token))
    }

    fn end_sms_transaction(&self, token: u64) {
        {
            let mut q = self.queue.lock().unwrap_or_else(|p| p.into_inner());
            if !q.sms_tokens.remove(&token) {
                return;
            }
            q.sms_hold_count = q.sms_hold_count.saturating_sub(1);
            if q.sms_hold_count == 0 {
                q.sms_hold_started = None;
                q.sms_hold_expired_logged = false;
            }
        }
        if current_sms_token() == Some(token) {
            SMS_TOKEN.with(|c| c.set(None));
        }
        {
            let mut audit = self.audit.lock().unwrap_or_else(|p| p.into_inner());
            audit.update(token, |r| {
                r.finished_at_ms = Some(now_ms());
                r.result = Some("released");
            });
        }
        self.cond.notify_all();
    }

    /// True when at least one SMS transaction is open (diagnostics/UI).
    pub fn sms_hold_active(&self) -> bool {
        let q = self.queue.lock().unwrap_or_else(|p| p.into_inner());
        q.sms_hold_count > 0
    }

    /// Open-transaction age in milliseconds (None when idle).
    pub fn sms_hold_age_ms(&self) -> Option<u64> {
        let q = self.queue.lock().unwrap_or_else(|p| p.into_inner());
        q.sms_hold_started
            .map(|t| t.elapsed().as_millis() as u64)
    }

    // ---------------------------------------------------------- submit

    /// Submit a request; returns a channel that resolves with the result.
    /// Never blocks the caller beyond a brief mutex acquire.
    pub fn submit(&self, mut spec: AtRequestSpec) -> std::sync::mpsc::Receiver<AtResult> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);

        // SMS class: a CMGS payload is always P0; a command issued inside an
        // SMS transaction (owner thread) is upgraded to P0 as well and never
        // deduplicated against ordinary traffic.
        let sms_owned = matches!(spec.payload, AtPayload::SMS { .. })
            || current_sms_token()
                .map(|t| {
                    self.queue
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .sms_tokens
                        .contains(&t)
                })
                .unwrap_or(false);
        if sms_owned {
            spec.priority = Priority::Sms;
            spec.sms = true;
            spec.dedup_key = None;
        }

        let mut q = self.queue.lock().unwrap_or_else(|p| p.into_inner());

        // Backpressure: refuse once the queue is saturated. SMS (P0) is
        // exempt: it is bounded by its transaction and must not be dropped
        // behind a backlog of polling.
        if q.heap.len() >= self.max_queue && !spec.exclusive && !spec.sms {
            if spec.priority.is_user_visible() {
                USER_STARVED.store(true, Ordering::Relaxed);
            }
            let _ = tx.send(Err(BackendError::Busy));
            return rx;
        }

        // Exclusive arbitration: only one exclusive request at a time. A
        // long exclusive must not start while SMS is pending or a transaction
        // is open — the SMS would wait behind a scan for minutes.
        if spec.exclusive {
            let sms_waiting = q.sms_hold_count > 0 || q.heap.iter().any(|h| h.0.spec.sms);
            if q.exclusive_active || sms_waiting {
                let _ = tx.send(Err(BackendError::Busy));
                return rx;
            }
            q.exclusive_active = true;
        }

        // Deduplication: identical in-flight reads share one execution.
        if !spec.sms {
            if let Some(key) = spec.dedup_key.clone() {
                if let Some(waiters) = q.dedup.get_mut(&key) {
                    waiters.push(tx);
                    return rx;
                }
                q.dedup.insert(key, Vec::new());
            }
        }

        let seq = q.seq;
        q.seq += 1;
        let id = next_id();
        let priority = spec.priority;
        let sms = spec.sms;
        let label = spec.label.clone();
        q.heap.push(HeapItem(QueuedRequest {
            id,
            seq,
            spec,
            tx,
            enqueued_at: Instant::now(),
        }));
        drop(q);

        {
            let mut audit = self.audit.lock().unwrap_or_else(|p| p.into_inner());
            audit.insert(AuditRecord {
                id,
                label,
                priority: priority.name(),
                priority_label: priority.label(),
                sms,
                enqueued_at_ms: now_ms(),
                started_at_ms: None,
                finished_at_ms: None,
                result: None,
                error: None,
                attempts: 0,
            });
        }
        // Notify the executor; the condvar lives on the arbiter, not in the
        // queue state (the executor waits on the same pair).
        self.cond.notify_one();
        rx
    }

    /// True when the transport is attached (used to gate snapshots).
    pub fn connected(&self) -> bool {
        self.transport.connected()
    }

    /// True while a request is executing on the channel (URC monitor gate).
    pub fn busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }

    // ---------------------------------------------------------- executor

    fn executor_loop(self: Arc<Self>) {
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return;
            }
            let item: QueuedRequest = {
                let mut q = self.queue.lock().unwrap_or_else(|p| p.into_inner());
                loop {
                    if self.stop.load(Ordering::SeqCst) {
                        return;
                    }
                    let sms_hold = Self::sms_hold_effective(&mut q, &self.audit);
                    if let Some(req) = pop_ready(&mut q, sms_hold) {
                        break req;
                    }
                    let (guard, _) = self
                        .cond
                        .wait_timeout(q, Duration::from_millis(100))
                        .unwrap_or_else(|p| p.into_inner());
                    q = guard;
                }
            };
            self.execute(item);
        }
    }

    /// Whether the dispatcher must restrict itself to P0 right now.
    ///
    /// A hold older than [`SMS_HOLD_MAX`] is treated as stuck and ignored so
    /// ordinary traffic cannot be blocked forever. The owner's requests still
    /// carry their own timeouts, so the stuck transaction terminates on its
    /// own; this is not a wire abort.
    fn sms_hold_effective(q: &mut QueueState, audit: &Mutex<AuditLog>) -> bool {
        if q.sms_hold_count == 0 {
            return false;
        }
        let expired = q
            .sms_hold_started
            .map(|t| t.elapsed() > SMS_HOLD_MAX)
            .unwrap_or(false);
        if expired && !q.sms_hold_expired_logged {
            q.sms_hold_expired_logged = true;
            let mut a = audit.lock().unwrap_or_else(|p| p.into_inner());
            a.insert(AuditRecord {
                id: next_id(),
                label: "sms.hold.expired".into(),
                priority: Priority::Sms.name(),
                priority_label: Priority::Sms.label(),
                sms: true,
                enqueued_at_ms: now_ms(),
                started_at_ms: Some(now_ms()),
                finished_at_ms: Some(now_ms()),
                result: Some("expired"),
                error: Some(format!(
                    "SMS transaction held the channel for more than {}s; ordinary dispatch resumed",
                    SMS_HOLD_MAX.as_secs()
                )),
                attempts: 0,
            });
        }
        !expired
    }

    fn execute(self: &Arc<Self>, req: QueuedRequest) {
        self.busy.store(true, Ordering::SeqCst);
        self.executed.fetch_add(1, Ordering::Relaxed);

        // Fail fast when the caller cancelled while queued.
        if req_cancelled(&req.spec) {
            self.finish(req, Err(BackendError::TaskCancelled), 0);
            return;
        }

        // Queue deadline: a request that waited too long is dropped (Busy)
        // so a flood of page refreshes cannot pile up behind a scan.
        if req.enqueued_at.elapsed() > req.spec.queued_timeout {
            if req.spec.priority.is_user_visible() {
                USER_STARVED.store(true, Ordering::Relaxed);
            }
            self.finish(req, Err(BackendError::Busy), 0);
            return;
        }

        self.audit
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .update(req.id, |r| r.started_at_ms = Some(now_ms()));

        let started = Instant::now();
        let mut attempts = 0;
        let max = req.spec.retry.max_retries;
        loop {
            attempts += 1;
            // Contract: at least 25 ms between the starts of any two AT
            // writes, enforced backend-wide before the transport is called.
            pacer::wait_turn();
            let result = self.run_once(&req.spec);
            let terminal = result
                .as_ref()
                .err()
                .map(|e| !(req.spec.retry.retryable)(e))
                .unwrap_or(true);
            if terminal || attempts > max {
                DUTY.record_and_check(started.elapsed().as_millis() as u64);
                if result.is_ok() && req.spec.priority.is_user_visible() {
                    clear_channel_starvation();
                }
                let label = if result.is_ok() { "ok" } else { "error" };
                let error = result.as_ref().err().map(|e| e.message());
                self.audit
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .update(req.id, |r| {
                        r.attempts = attempts;
                        r.result = Some(label);
                        r.error = error;
                    });
                self.finish(req, result, attempts);
                return;
            }
            // Transient fault: back off, then re-run.
            std::thread::sleep(req.spec.retry.base_backoff);
            if req_cancelled(&req.spec) {
                DUTY.record_and_check(started.elapsed().as_millis() as u64);
                self.finish(req, Err(BackendError::TaskCancelled), attempts);
                return;
            }
        }
    }

    fn run_once(&self, spec: &AtRequestSpec) -> AtResult {
        match &spec.payload {
            AtPayload::SMS { parts } => {
                return self.transport.send_sms_pdu(parts);
            }
            AtPayload::None => {}
        }
        let cancel = spec
            .cancel
            .as_ref()
            .cloned()
            .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
        if spec.abort_wire.is_some() {
            self.transport.send_interruptible(
                &spec.command,
                spec.timeout,
                &cancel,
                spec.abort_wire.as_deref(),
            )
        } else {
            self.transport.send(&spec.command, spec.timeout)
        }
    }

    /// Deliver the result to the canonical caller and any dedup waiters,
    /// then clear exclusive/dedup state.
    fn finish(&self, req: QueuedRequest, result: AtResult, attempts: u32) {
        if req.spec.exclusive {
            let mut q = self.queue.lock().unwrap_or_else(|p| p.into_inner());
            q.exclusive_active = false;
            self.cond.notify_all();
        }
        if let Some(key) = &req.spec.dedup_key {
            let mut q = self.queue.lock().unwrap_or_else(|p| p.into_inner());
            if let Some(waiters) = q.dedup.get_mut(key) {
                for w in waiters.drain(..) {
                    let _ = w.send(result.clone());
                }
                q.dedup.remove(key);
            }
        }
        self.audit
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .update(req.id, |r| {
                r.finished_at_ms = Some(now_ms());
                r.attempts = attempts;
            });
        let _ = req.tx.send(result);
        self.busy.store(false, Ordering::SeqCst);
        // A held SMS request may have completed while non-SMS work waits;
        // wake the dispatcher even though no new request arrived.
        self.cond.notify_all();
    }

    // ---------------------------------------------------------- diagnostics

    /// Newest-first audit rows (bounded by `limit`).
    pub fn audit_json(&self, limit: usize) -> crate::core::json::Value {
        self.audit
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .snapshot(limit)
    }

    /// Scheduler metrics for the diagnostics page.
    pub fn status_json(&self) -> crate::core::json::Value {
        use crate::core::json::{num_val, str_val, Value};
        let q = self.queue.lock().unwrap_or_else(|p| p.into_inner());
        let mut m = std::collections::BTreeMap::new();
        m.insert("queued".to_string(), num_val(q.heap.len() as i64));
        m.insert("max_queue".to_string(), num_val(self.max_queue as i64));
        m.insert("sms_hold".to_string(), Value::Bool(q.sms_hold_count > 0));
        m.insert("sms_holds".to_string(), num_val(q.sms_hold_count as i64));
        m.insert(
            "sms_hold_age_ms".to_string(),
            match q.sms_hold_started {
                Some(t) => num_val(t.elapsed().as_millis() as u64),
                None => Value::Null,
            },
        );
        m.insert(
            "min_send_gap_ms".to_string(),
            num_val(pacer::MIN_SEND_GAP.as_millis() as u64),
        );
        m.insert(
            "since_last_send_ms".to_string(),
            match pacer::global().since_last_ms() {
                Some(v) => num_val(v),
                None => Value::Null,
            },
        );
        m.insert(
            "executed".to_string(),
            num_val(self.executed.load(Ordering::Relaxed) as i64),
        );
        m.insert(
            "exclusive_active".to_string(),
            Value::Bool(q.exclusive_active),
        );
        m.insert("busy".to_string(), Value::Bool(self.busy()));
        m.insert(
            "priority_levels".to_string(),
            Value::Arr(
                [Priority::Sms, Priority::Control, Priority::UserQuery, Priority::Polling, Priority::Maintenance]
                    .iter()
                    .map(|p| str_val(p.name()))
                    .collect(),
            ),
        );
        Value::Obj(m)
    }
}

/// Pop the next request the dispatcher may start.
///
/// While an SMS transaction is open (or a P0 request is already queued), the
/// top of the heap is the only candidate, and it must be P0. Non-SMS requests
/// are *not* removed, so they keep their queue deadline and run as soon as
/// the guard is released.
fn pop_ready(q: &mut QueueState, sms_hold: bool) -> Option<QueuedRequest> {
    let sms_pending = q.heap.iter().any(|h| h.0.spec.sms);
    if sms_hold || sms_pending {
        match q.heap.peek() {
            Some(top) if top.0.spec.sms => q.heap.pop().map(|h| h.0),
            _ => None,
        }
    } else {
        q.heap.pop().map(|h| h.0)
    }
}

fn req_cancelled(spec: &AtRequestSpec) -> bool {
    spec.cancel
        .as_ref()
        .map(|c| c.load(Ordering::SeqCst))
        .unwrap_or(false)
}

impl Drop for AtArbiter {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake transport: configurable latency + programmable responses.
    struct FakeTransport {
        delay: Mutex<Duration>,
        responses: Mutex<Vec<(String, Result<String, BackendError>)>>,
        calls: Mutex<usize>,
        connected: bool,
        /// How long an interruptible (exclusive/scan) call blocks before OK.
        interruptible_block: Mutex<Duration>,
        /// Start timestamps of every send, for pacing/ordering assertions.
        starts: Mutex<Vec<(String, Instant)>>,
    }

    impl FakeTransport {
        fn new() -> Arc<Self> {
            Arc::new(FakeTransport {
                delay: Mutex::new(Duration::from_millis(5)),
                responses: Mutex::new(vec![("AT".to_string(), Ok("OK".into()))]),
                calls: Mutex::new(0),
                connected: true,
                interruptible_block: Mutex::new(Duration::from_millis(20)),
                starts: Mutex::new(Vec::new()),
            })
        }

        fn push(&self, cmd: &str, r: Result<String, BackendError>) {
            self.responses.lock().unwrap().push((cmd.to_string(), r));
        }

        fn call_count(&self) -> usize {
            *self.calls.lock().unwrap()
        }

        fn set_delay(&self, d: Duration) {
            *self.delay.lock().unwrap() = d;
        }

        fn set_interruptible_block(&self, d: Duration) {
            *self.interruptible_block.lock().unwrap() = d;
        }

        fn start_log(&self) -> Vec<(String, Instant)> {
            self.starts.lock().unwrap().clone()
        }

        fn reset_starts(&self) {
            self.starts.lock().unwrap().clear();
        }
    }

    impl AtTransport for FakeTransport {
        fn send(&self, command: &str, timeout: Duration) -> AtResult {
            *self.calls.lock().unwrap() += 1;
            self.starts
                .lock()
                .unwrap()
                .push((command.to_string(), Instant::now()));
            let delay = *self.delay.lock().unwrap();
            std::thread::sleep(delay.min(timeout));
            if delay >= timeout {
                return Err(BackendError::AtTimeout);
            }
            let mut responses = self.responses.lock().unwrap();
            if let Some(idx) = responses.iter().position(|(c, _)| c == command) {
                let (_, r) = responses.remove(idx);
                return r;
            }
            Ok("OK".into())
        }

        fn send_interruptible(
            &self,
            command: &str,
            timeout: Duration,
            cancel: &AtomicBool,
            _abort: Option<&[u8]>,
        ) -> AtResult {
            *self.calls.lock().unwrap() += 1;
            self.starts
                .lock()
                .unwrap()
                .push((command.to_string(), Instant::now()));
            let block = *self.interruptible_block.lock().unwrap();
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
                if waited >= block {
                    return Ok("OK".into());
                }
            }
        }

        fn connected(&self) -> bool {
            self.connected
        }

        fn send_sms_pdu(&self, parts: &[SmsPart]) -> AtResult {
            *self.calls.lock().unwrap() += 1;
            self.starts.lock().unwrap().push((
                format!("SMS:{}", parts.first().map(|p| p.hex.as_str()).unwrap_or("")),
                Instant::now(),
            ));
            let delay = *self.delay.lock().unwrap();
            std::thread::sleep(delay);
            Ok("+CMGS: 1\r\nOK".into())
        }
    }

    fn spec_fast(cmd: &str) -> AtRequestSpec {
        AtRequestSpec::fast_query(cmd)
    }

    #[test]
    fn single_execution_with_dedup() {
        let t = FakeTransport::new();
        t.set_delay(Duration::from_millis(30));
        let arbiter = AtArbiter::new(t.clone());
        let mut rxs = Vec::new();
        for _ in 0..8 {
            rxs.push(arbiter.submit(spec_fast("AT+CSQ")));
        }
        let results: Vec<String> = rxs
            .into_iter()
            .map(|rx| rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap())
            .collect();
        assert_eq!(results.len(), 8);
        assert_eq!(t.call_count(), 1, "dedup must collapse to 1 AT call");
        arbiter.stop();
    }

    #[test]
    fn deliver_results_to_all_dedup_waiters() {
        let t = FakeTransport::new();
        t.set_delay(Duration::from_millis(5));
        let arbiter = AtArbiter::new(t.clone());
        let a = arbiter.submit(spec_fast("AT+CPIN?"));
        let b = arbiter.submit(spec_fast("AT+CPIN?"));
        assert_eq!(a.recv_timeout(Duration::from_secs(2)).unwrap().unwrap(), "OK");
        assert_eq!(b.recv_timeout(Duration::from_secs(2)).unwrap().unwrap(), "OK");
        arbiter.stop();
    }

    #[test]
    fn priority_preempts_background() {
        let t = FakeTransport::new();
        t.set_delay(Duration::from_millis(20));
        let arbiter = AtArbiter::new(t.clone());
        // Hold the channel with one background request so the next two queue.
        let first = arbiter.submit(AtRequestSpec::background("AT+COPS?"));
        std::thread::sleep(Duration::from_millis(5));
        let bg = arbiter.submit(AtRequestSpec::background("AT+CGMR"));
        let hi = arbiter.submit(AtRequestSpec::interactive("AT+CFUN?"));
        let _ = hi.recv_timeout(Duration::from_secs(3)).unwrap();
        let _ = bg.recv_timeout(Duration::from_secs(3)).unwrap();
        let _ = first.recv_timeout(Duration::from_secs(3)).unwrap();
        let log = t.start_log();
        let order: Vec<&str> = log.iter().map(|(c, _)| c.as_str()).collect();
        let hi_pos = order.iter().position(|c| *c == "AT+CFUN?").unwrap();
        let bg_pos = order.iter().position(|c| *c == "AT+CGMR").unwrap();
        assert!(hi_pos < bg_pos, "P1 must run before P3: {:?}", order);
        arbiter.stop();
    }

    #[test]
    fn retry_on_timeout_then_success() {
        let t = FakeTransport::new();
        t.set_delay(Duration::from_secs(5));
        let arbiter = AtArbiter::new(t.clone());
        let mut spec = spec_fast("AT+CSQ");
        spec.timeout = Duration::from_millis(300);
        let rx = arbiter.submit(spec);
        let res = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(res, Err(BackendError::AtTimeout)));
        assert_eq!(t.call_count(), 3, "2 retries after the first attempt");
        arbiter.stop();
    }

    #[test]
    fn non_retryable_rejected_immediately() {
        let t = FakeTransport::new();
        t.set_delay(Duration::from_millis(5));
        t.push("AT+CMD", Err(BackendError::AtRejected("ERROR".into())));
        let arbiter = AtArbiter::new(t.clone());
        let mut spec = spec_fast("AT+CMD");
        spec.retry = RetryPolicy::queries();
        let rx = arbiter.submit(spec);
        let res = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(res, Err(BackendError::AtRejected(_))));
        assert_eq!(t.call_count(), 1, "anchored modem error must not retry");
        arbiter.stop();
    }

    #[test]
    fn exclusive_rejects_second() {
        let t = FakeTransport::new();
        t.set_delay(Duration::from_millis(50));
        let arbiter = AtArbiter::new(t.clone());
        let a = arbiter.submit(AtRequestSpec::long_exclusive(
            "AT^CELLSCAN",
            Duration::from_secs(1),
            b"abcd\r",
        ));
        let b = arbiter.submit(AtRequestSpec::long_exclusive(
            "AT^CELLSCAN",
            Duration::from_secs(1),
            b"abcd\r",
        ));
        let res_b = b.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(res_b, Err(BackendError::Busy)));
        let _ = a.recv_timeout(Duration::from_secs(3)).unwrap();
        arbiter.stop();
    }

    #[test]
    fn queue_deadline_backpressure() {
        let t = FakeTransport::new();
        t.set_delay(Duration::from_millis(5));
        t.set_interruptible_block(Duration::from_secs(2));
        let arbiter = AtArbiter::new(t.clone());
        let _ex = arbiter.submit(AtRequestSpec::long_exclusive(
            "AT^CELLSCAN",
            Duration::from_secs(3),
            b"abcd\r",
        ));
        std::thread::sleep(Duration::from_millis(80));
        let mut spec = spec_fast("AT+CSQ");
        spec.queued_timeout = Duration::from_millis(60);
        let rx = arbiter.submit(spec);
        let res = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(res, Err(BackendError::Busy)));
        arbiter.stop();
    }

    #[test]
    fn cancel_while_queued() {
        let t = FakeTransport::new();
        t.set_delay(Duration::from_millis(5));
        let arbiter = AtArbiter::new(t.clone());
        let _ex = arbiter.submit(AtRequestSpec::long_exclusive(
            "AT^CELLSCAN",
            Duration::from_secs(1),
            b"abcd\r",
        ));
        std::thread::sleep(Duration::from_millis(80));
        let cancel = Arc::new(AtomicBool::new(true));
        let mut spec = spec_fast("AT+CSQ");
        spec.cancel = Some(cancel);
        let rx = arbiter.submit(spec);
        let res = rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(matches!(res, Err(BackendError::TaskCancelled)));
        arbiter.stop();
    }

    #[test]
    fn interruptible_aborts_on_cancel() {
        let t = FakeTransport::new();
        t.set_interruptible_block(Duration::from_millis(600));
        let arbiter = AtArbiter::new(t.clone());
        let cancel = Arc::new(AtomicBool::new(false));
        let mut spec = AtRequestSpec::long_exclusive(
            "AT^CELLSCAN",
            Duration::from_secs(5),
            b"abcd\r",
        );
        spec.cancel = Some(cancel.clone());
        let rx = arbiter.submit(spec);
        std::thread::sleep(Duration::from_millis(100));
        cancel.store(true, Ordering::SeqCst);
        let res = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(matches!(res, Err(BackendError::TaskCancelled)));
        arbiter.stop();
    }

    /// 25 ms contract: start times of consecutive wire writes are at least
    /// 25 ms apart, including retries.
    #[test]
    fn wire_pacing_enforces_25ms_between_sends() {
        let t = FakeTransport::new();
        t.set_delay(Duration::from_millis(1));
        let arbiter = AtArbiter::new(t.clone());
        std::thread::sleep(Duration::from_millis(50));
        t.reset_starts();
        let mut rxs = Vec::new();
        for i in 0..4 {
            rxs.push(arbiter.submit(spec_fast(&format!("AT+TEST{}", i))));
        }
        for rx in rxs {
            rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
        }
        let log = t.start_log();
        assert_eq!(log.len(), 4);
        for pair in log.windows(2) {
            let gap = pair[1].1.duration_since(pair[0].1);
            assert!(
                gap >= pacer::MIN_SEND_GAP - Duration::from_millis(1),
                "wire gap {:?} < 25ms between {} and {}",
                gap,
                pair[0].0,
                pair[1].0
            );
        }
        arbiter.stop();
    }

    /// While an SMS transaction is open, a queued P1 control request must not
    /// start; dropping the guard releases it. The in-flight SMS request is
    /// never interrupted.
    #[test]
    fn sms_transaction_pauses_non_sms_until_release() {
        let t = FakeTransport::new();
        t.set_delay(Duration::from_millis(10));
        let arbiter = AtArbiter::new(t.clone());
        let guard = arbiter.sms_transaction();

        // Issued from the guard's thread: upgraded to P0/SMS.
        let sms = arbiter.submit(AtRequestSpec::ui_query("AT+CMGL=4"));
        sms.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();

        // A P1 control request from another thread queues while the guard
        // is open and must not reach the wire.
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let a2 = arbiter.clone();
        let control = std::thread::spawn(move || {
            let rx = a2.submit(AtRequestSpec::interactive("AT+CFUN=1"));
            started_tx.send(()).unwrap();
            rx.recv_timeout(Duration::from_secs(5))
        });
        // Give the dispatcher time to (incorrectly) start it if it would.
        std::thread::sleep(Duration::from_millis(120));
        assert!(
            !t.start_log().iter().any(|(c, _)| c == "AT+CFUN=1"),
            "non-SMS request started inside the SMS transaction"
        );

        drop(guard);
        let res = control.join().unwrap().unwrap();
        assert!(res.is_ok());
        assert!(t.start_log().iter().any(|(c, _)| c == "AT+CFUN=1"));
        arbiter.stop();
        let _ = started_rx.try_recv();
    }

    #[test]
    fn sms_requests_are_audited_as_p0() {
        let t = FakeTransport::new();
        t.set_delay(Duration::from_millis(2));
        let arbiter = AtArbiter::new(t.clone());
        {
            let _guard = arbiter.sms_transaction();
            let rx = arbiter.submit(AtRequestSpec::ui_query("AT+CMGS=42"));
            rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
        }
        let audit = arbiter.audit_json(50);
        let dump = audit.dump();
        assert!(dump.contains("\"priority\":\"P0\""));
        assert!(dump.contains("\"sms\":true"));
        assert!(dump.contains("\"sent_at\""));
        assert!(dump.contains("\"finished_at\""));
        assert!(dump.contains("\"result\":\"ok\""));
        arbiter.stop();
    }

    #[test]
    fn audit_records_priority_enqueue_send_finish() {
        let t = FakeTransport::new();
        t.set_delay(Duration::from_millis(2));
        let arbiter = AtArbiter::new(t.clone());
        let rx = arbiter.submit(AtRequestSpec::background("AT+CGMR"));
        rx.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
        let dump = arbiter.audit_json(10).dump();
        assert!(dump.contains("\"priority\":\"P3\""));
        assert!(dump.contains("\"enqueued_at\""));
        assert!(dump.contains("\"wait_ms\""));
        assert!(dump.contains("\"duration_ms\""));
        assert!(dump.contains("\"task_id\""));
        arbiter.stop();
    }

    #[test]
    fn sms_transaction_release_on_drop_clears_hold() {
        let t = FakeTransport::new();
        let arbiter = AtArbiter::new(t.clone());
        {
            let _guard = arbiter.sms_transaction();
            assert!(arbiter.sms_hold_active());
        }
        assert!(!arbiter.sms_hold_active());
        // A normal request still runs after the release.
        let rx = arbiter.submit(spec_fast("AT+CSQ"));
        assert!(rx.recv_timeout(Duration::from_secs(2)).unwrap().is_ok());
        arbiter.stop();
    }

    #[test]
    fn nested_sms_guard_is_owned_by_outer() {
        let t = FakeTransport::new();
        let arbiter = AtArbiter::new(t.clone());
        {
            let _outer = arbiter.sms_transaction();
            {
                let _inner = arbiter.sms_transaction();
                assert!(arbiter.sms_hold_active());
            }
            assert!(
                arbiter.sms_hold_active(),
                "inner guard must not release the outer transaction"
            );
        }
        assert!(!arbiter.sms_hold_active());
        arbiter.stop();
    }

    #[test]
    fn long_exclusive_rejected_while_sms_pending() {
        let t = FakeTransport::new();
        t.set_delay(Duration::from_millis(5));
        let arbiter = AtArbiter::new(t.clone());
        let _guard = arbiter.sms_transaction();
        let rx = arbiter.submit(AtRequestSpec::long_exclusive(
            "AT^CELLSCAN",
            Duration::from_secs(1),
            b"abcd\r",
        ));
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            Err(BackendError::Busy)
        ));
        arbiter.stop();
    }
}
