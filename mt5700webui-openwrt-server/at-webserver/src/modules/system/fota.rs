//! FOTA (firmware-over-the-air) upgrade: the one implementation of the flow.
//!
//! The flow is stateful and outlives a page: after `system.fota_start` the
//! modem spends minutes downloading, the page may be closed or reloaded, and
//! the upgrade keeps running. It is therefore a module-owned task, exactly like
//! the cell scan — `system.fota_start` submits it, `system.fota` answers from
//! the task registry plus a snapshot (never touching the modem), and
//! `system.fota_abort` cancels it.
//!
//! The WebUI used to own every step of this: `ATE0`, `AT^FOTAMODE=0,1,0,1`,
//! `AT^FOTAOEMDL="…"` validated in the page, then a 1 s poller reading
//! `AT^FOTASTATE?` / `AT^FOTADLQ` and driving the state machine (resume on 31,
//! `AT^FWUP` on 40, the step/progress UI on 30). All of it lives here now; the
//! page renders `{phase, step, progress, state, stateName, total, received}`
//! and shows the same texts and the same step numbers as before.

use crate::core::error::BackendError;
use crate::core::json::{self, Value};
use crate::core::task::{Priority, TaskKind};
use crate::modules::system::commands::{
    fota_state_name, fota_url_set, FOTA_DOWNLOAD_RESUME, FOTA_ECHO_OFF, FOTA_MODE_INIT,
    FOTA_PROGRESS_QUERY, FOTA_STATE_QUERY, FOTA_UPGRADE,
};
use crate::modules::system::parser::{parse_fota_progress, parse_fota_state};
use crate::scheduler::arbiter::AtRequestSpec;
use crate::scheduler::jobs::{TaskCtx, TaskManager};
use crate::state::bus::TOPIC_FOTA;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

/// The state poller's cadence — 1 s, the interval the page used.
const POLL_INTERVAL: Duration = Duration::from_secs(1);
/// Terminal states that end the flow: 13 check failed, 14 no update,
/// 20 download failed.
const STATE_CHECK_FAILED: i64 = 13;
const STATE_NO_UPDATE: i64 = 14;
const STATE_DOWNLOAD_FAILED: i64 = 20;
const STATE_DOWNLOADING: i64 = 30;
const STATE_PAUSED: i64 = 31;
const STATE_DOWNLOADED: i64 = 40;
const STATE_INSTALLING: i64 = 50;
/// A paused download is resumed at most once every 5 s (the page's own rate
/// limit), so a modem that stays paused does not get a command per second.
const RESUME_INTERVAL: Duration = Duration::from_secs(5);
/// `^FOTASTATE?` is a fast query; the flow's own patience is what matters, so
/// each step is bounded but generous.
const STEP_TIMEOUT: Duration = Duration::from_secs(12);
const QUEUED_TIMEOUT: Duration = Duration::from_secs(20);
/// Hard cap on one upgrade run: download + install with headroom. The task
/// ends there (and says so) instead of polling forever.
const FLOW_TIMEOUT: Duration = Duration::from_secs(1800);
/// Task name registered with the manager, so state/abort address this flow and
/// never another exclusive task.
const TASK_NAME: &str = "fota.system";
/// The snapshot's TTL: long enough that a page reloaded after an upgrade still
/// sees how the flow ended.
const FOTA_SNAPSHOT_TTL: Duration = Duration::from_secs(1800);

/// Timeout for one step of the flow.
///
/// `^FOTADLQ` walks the flash partition for a downloaded-image summary and is
/// the slowest read here; the page gave it its 6 s default, which is too tight
/// while the download holds the modem. `AT^FWUP` is the one command that must
/// not be re-issued after a mere timeout — the modem reboots as soon as it
/// accepts it — so it gets a shorter budget and no retry: a re-send would
/// interrupt the flash.
fn step_timeout(command: &str) -> Duration {
    let upper = command.trim().to_ascii_uppercase();
    if upper == FOTA_PROGRESS_QUERY {
        Duration::from_secs(20)
    } else if upper == FOTA_UPGRADE {
        Duration::from_secs(8)
    } else {
        STEP_TIMEOUT
    }
}

/// What the flow is doing, as the page's `step` numbers and its own words.
#[derive(Clone)]
struct Snapshot {
    phase: &'static str,
    step: i64,
    progress: i64,
    state: Option<i64>,
    total: i64,
    received: i64,
    error: Option<String>,
}

impl Snapshot {
    fn idle() -> Self {
        Snapshot {
            phase: "idle",
            step: 0,
            progress: 0,
            state: None,
            total: 0,
            received: 0,
            error: None,
        }
    }

    fn to_json(&self, running: bool) -> Value {
        let mut m: BTreeMap<String, Value> = BTreeMap::new();
        m.insert("running".to_string(), Value::Bool(running));
        m.insert("phase".to_string(), json::str_val(self.phase));
        m.insert("step".to_string(), json::num_val(self.step));
        m.insert("progress".to_string(), json::num_val(self.progress));
        match self.state {
            Some(code) => {
                m.insert("state".to_string(), json::num_val(code));
                m.insert("stateName".to_string(), json::str_val(fota_state_name(code)));
            }
            None => {
                m.insert("state".to_string(), Value::Null);
                m.insert("stateName".to_string(), Value::Null);
            }
        }
        m.insert("total".to_string(), json::num_val(self.total));
        m.insert("received".to_string(), json::num_val(self.received));
        if let Some(e) = &self.error {
            m.insert("error".to_string(), json::str_val(e));
        }
        Value::Obj(m)
    }
}

/// The running flow's snapshot, if a flow is queued or running.
fn active(tasks: &Arc<TaskManager>) -> Option<crate::core::task::TaskRecord> {
    tasks
        .list()
        .into_iter()
        .find(|r| r.name == TASK_NAME && !r.status.is_terminal())
}

/// `system.fota` — the flow's current state.
///
/// Answered from the task registry and the snapshot cache only: the page calls
/// this on load and every poll, and it must never queue behind the download it
/// is reporting on. With no flow running it returns the last snapshot so a
/// reload right after the upgrade still shows how it ended.
pub fn state(
    tasks: &Arc<TaskManager>,
    cache: &Arc<crate::state::cache::StateCache>,
) -> Value {
    let running = active(tasks).is_some();
    match cached_snapshot(cache) {
        Some(snap) => snap.to_json(running),
        None => Snapshot::idle().to_json(running),
    }
}

/// The last published snapshot (`{state, received}` are the only fields that
/// change fast; the whole object is cached so a reload is consistent).
fn cached_snapshot(cache: &Arc<crate::state::cache::StateCache>) -> Option<Snapshot> {
    match cache.get(TOPIC_FOTA).0 {
        Some(Value::Obj(m)) => Some(Snapshot {
            phase: match m.get("phase").and_then(|v| v.as_str()) {
                Some(p) => match p {
                    "idle" => "idle",
                    "running" => "running",
                    "done" => "done",
                    "error" => "error",
                    _ => "idle",
                },
                None => "idle",
            },
            step: m.get("step").and_then(|v| v.as_i64()).unwrap_or(0),
            progress: m.get("progress").and_then(|v| v.as_i64()).unwrap_or(0),
            state: m.get("state").and_then(|v| v.as_i64()),
            total: m.get("total").and_then(|v| v.as_i64()).unwrap_or(0),
            received: m.get("received").and_then(|v| v.as_i64()).unwrap_or(0),
            error: m.get("error").and_then(|v| v.as_str()).map(str::to_string),
        }),
        _ => None,
    }
}

/// `system.fota_start` — validate the server address and start the flow.
///
/// Params: `{url: "http://…"}`. The address rules (http:// only) and the
/// trailing slash the modem expects are applied by `commands::fota_url_set`,
/// so a rejected address never reaches the modem — the same behaviour the
/// page's own check had.
pub fn start(tasks: &Arc<TaskManager>, params: &Value) -> Result<Value, BackendError> {
    let url = params
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if url.is_empty() {
        return Err(BackendError::InvalidParameter("地址不能为空".to_string()));
    }
    let command = fota_url_set(url)
        .map_err(|e| BackendError::InvalidParameter(e.to_string()))?;
    if active(tasks).is_some() {
        return Err(BackendError::Busy);
    }
    submit(tasks, command);
    let mut m: BTreeMap<String, Value> = BTreeMap::new();
    m.insert("started".to_string(), Value::Bool(true));
    Ok(Value::Obj(m))
}

/// `system.fota_abort` — cancel the flow. `{aborted: false}` when none ran, so
/// a cancel racing the flow's own completion is not an error to show the user.
pub fn abort(tasks: &Arc<TaskManager>) -> Value {
    let mut m: BTreeMap<String, Value> = BTreeMap::new();
    match active(tasks) {
        Some(record) => {
            tasks.cancel(record.id);
            m.insert("aborted".to_string(), Value::Bool(true));
        }
        None => {
            m.insert("aborted".to_string(), Value::Bool(false));
        }
    }
    Value::Obj(m)
}

/// Publish the snapshot on the `fota` topic — the one place a FOTA push is
/// produced, so the page's event and its poll answer can never disagree.
fn publish(ctx: &TaskCtx, snap: &Snapshot, running: bool) {
    let value = snap.to_json(running);
    // The state route answers from this snapshot, so it must not expire while
    // the flow is idle between the page's own polls — the registry's `running`
    // flag is what tells a caller whether it is live.
    ctx.cache
        .set_ttl(TOPIC_FOTA, value.clone(), "fota", FOTA_SNAPSHOT_TTL);
    ctx.bus.publish_now(TOPIC_FOTA, "fota.progress", value);
}

/// The flow itself: set the mode, point the modem at the server, then poll the
/// state machine until a terminal state, resuming a paused download and
/// flashing a complete one.
fn run(ctx: &TaskCtx, url_command: String) -> Result<Value, BackendError> {
    let mut snap = Snapshot {
        phase: "running",
        step: 1,
        ..Snapshot::idle()
    };
    publish(ctx, &snap, true);

    // 1) Prepare: no echo, HTTP update mode, the server address. A failure here
    //    is reported as such and the flow stops — exactly what the page did.
    let prep = [
        (FOTA_ECHO_OFF, "初始化 FOTA 失败"),
        (FOTA_MODE_INIT, "初始化 FOTA 失败"),
        (url_command.as_str(), "设置 FOTA 地址失败"),
    ];
    for (command, message) in prep {
        if ctx.at_request(step_spec(command)).is_err() {
            return fail(ctx, &mut snap, message);
        }
    }
    snap.step = 2;
    publish(ctx, &snap, true);

    // 2) Poll the state machine.
    let mut last_state: Option<i64> = None;
    let mut last_resume = std::time::Instant::now();
    loop {
        if ctx.cancelled() {
            snap.phase = "error";
            snap.error = Some("升级已取消".to_string());
            publish(ctx, &snap, false);
            return Ok(snap.to_json(false));
        }
        let state = match ctx.at_request(step_spec(FOTA_STATE_QUERY)) {
            Ok(text) => parse_fota_state(&text),
            Err(BackendError::TaskCancelled) => {
                snap.phase = "error";
                snap.error = Some("升级已取消".to_string());
                publish(ctx, &snap, false);
                return Ok(snap.to_json(false));
            }
            // A transient read failure is not a failed upgrade: the flow keeps
            // its state and tries again on the next tick. The page showed the
            // same "稍后重试" behaviour.
            Err(_) => {
                std::thread::sleep(POLL_INTERVAL);
                continue;
            }
        };
        let Some(state) = state else {
            return fail(ctx, &mut snap, "读取升级状态失败");
        };
        snap.state = Some(state);
        // `last_state` only marks the transitions: a download in progress is
        // published every tick (progress moves), everything else once.
        let changed = last_state != Some(state);
        last_state = Some(state);

        match state {
            STATE_CHECK_FAILED => return fail(ctx, &mut snap, "查询新版本失败"),
            STATE_NO_UPDATE => return fail(ctx, &mut snap, "服务器无新版本"),
            STATE_DOWNLOAD_FAILED => return fail(ctx, &mut snap, "固件下载失败"),
            STATE_DOWNLOADING => {
                snap.step = 2;
                if let Ok(text) = ctx.at_request(step_spec(FOTA_PROGRESS_QUERY)) {
                    let (total, received) = parse_fota_progress(&text);
                    snap.total = total;
                    snap.received = received;
                    if total > 0 {
                        snap.progress = ((received as f64 / total as f64) * 100.0)
                            .clamp(0.0, 100.0)
                            .floor() as i64;
                    }
                }
                publish(ctx, &snap, true);
            }
            STATE_PAUSED => {
                snap.step = 2;
                if last_resume.elapsed() >= RESUME_INTERVAL {
                    last_resume = std::time::Instant::now();
                    let _ = ctx.at_request(step_spec(FOTA_DOWNLOAD_RESUME));
                }
                publish(ctx, &snap, true);
            }
            STATE_DOWNLOADED => {
                snap.step = 3;
                snap.progress = 100;
                publish(ctx, &snap, true);
                // Flash. Whether the modem answers before it reboots or the
                // connection drops is irrelevant to the caller; a rejected
                // FWUP is the only failure reported.
                if let Err(e) = ctx.at_request(step_spec(FOTA_UPGRADE)) {
                    if matches!(e, BackendError::AtRejected(_)) {
                        return fail(ctx, &mut snap, "固件升级启动失败");
                    }
                }
                snap.step = 4;
                snap.phase = "done";
                publish(ctx, &snap, false);
                return Ok(snap.to_json(false));
            }
            STATE_INSTALLING => {
                snap.step = 4;
                if changed {
                    publish(ctx, &snap, true);
                }
            }
            _ => {
                if changed {
                    publish(ctx, &snap, true);
                }
            }
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// One step of the flow through the single arbiter: `ui_query` class budgets
/// (dedup + no retry) but with the FOTA flow's own queued allowance. A step is
/// a user-visible read issued by a task, so it must not be starved by, or
/// starve, the periodic refreshers.
fn step_spec(command: &str) -> AtRequestSpec {
    AtRequestSpec {
        command: command.to_string(),
        priority: Priority::Interactive,
        timeout: step_timeout(command),
        queued_timeout: QUEUED_TIMEOUT,
        exclusive: false,
        dedup_key: None,
        retry: crate::scheduler::arbiter::RetryPolicy::none(),
        cancel: None,
        abort_wire: None,
        payload: crate::scheduler::arbiter::AtPayload::None,
        label: format!("fota {}", command),
    }
}

fn fail(ctx: &TaskCtx, snap: &mut Snapshot, message: &str) -> Result<Value, BackendError> {
    snap.phase = "error";
    snap.error = Some(message.to_string());
    publish(ctx, snap, false);
    Err(BackendError::Internal(message.to_string()))
}

/// Submit the flow as a named `LongRunning` task, exactly like the scan.
fn submit(tasks: &Arc<TaskManager>, url_command: String) {
    tasks.submit(
        TASK_NAME,
        TaskKind::LongRunning,
        Priority::Interactive,
        Some(FLOW_TIMEOUT),
        Box::new(move |ctx: &TaskCtx| run(ctx, url_command)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::json::Value;
    use crate::scheduler::arbiter::AtTransport;

    /// No modem at all: a rejected parameter must fail before any AT access,
    /// which is exactly what these tests pin.
    struct NoTransport;

    impl AtTransport for NoTransport {
        fn send(&self, _cmd: &str, _timeout: Duration) -> Result<String, BackendError> {
            Err(BackendError::ModemUnavailable)
        }
        fn send_interruptible(
            &self,
            _cmd: &str,
            _timeout: Duration,
            _cancel: &std::sync::atomic::AtomicBool,
            _abort: Option<&[u8]>,
        ) -> Result<String, BackendError> {
            Err(BackendError::ModemUnavailable)
        }
        fn connected(&self) -> bool {
            false
        }
    }

    fn no_modem_with(cache: Arc<crate::state::cache::StateCache>) -> Arc<TaskManager> {
        let bus = crate::state::bus::EventBus::new();
        let arbiter = crate::scheduler::arbiter::AtArbiter::new(Arc::new(NoTransport));
        TaskManager::new(arbiter, cache, bus)
    }

    fn no_modem() -> Arc<TaskManager> {
        no_modem_with(Arc::new(crate::state::cache::StateCache::new()))
    }

    fn params(url: &str) -> Value {
        let mut m: BTreeMap<String, Value> = BTreeMap::new();
        m.insert("url".to_string(), json::str_val(url));
        Value::Obj(m)
    }

    #[test]
    fn the_address_rules_are_the_pages_rules() {
        // The page's own check: http:// only, and it appended the trailing
        // slash the modem's URL expects.
        let cmd = fota_url_set("http://fota.example.com/path").unwrap();
        assert_eq!(cmd, "AT^FOTAOEMDL=\"http://fota.example.com/path/\"");
        let cmd = fota_url_set("http://fota.example.com/path/").unwrap();
        assert_eq!(cmd, "AT^FOTAOEMDL=\"http://fota.example.com/path/\"");
        assert!(fota_url_set("https://fota.example.com").is_err());
        assert!(fota_url_set("ftp://x").is_err());
        // A quote inside the address would end the AT string argument.
        assert!(fota_url_set("http://x/\"y").is_err());
    }

    #[test]
    fn start_rejects_a_bad_address_before_touching_the_modem() {
        let tasks = no_modem();
        let err = start(&tasks, &params("https://nope")).unwrap_err();
        assert_eq!(err.code(), "INVALID_PARAMETER");
        assert_eq!(err.detail(), "仅支持 http 协议");
        let err = start(&tasks, &params("   ")).unwrap_err();
        assert_eq!(err.code(), "INVALID_PARAMETER");
        // Nothing was submitted for a rejected address.
        assert!(active(&tasks).is_none());
    }

    #[test]
    fn state_without_a_flow_is_an_idle_snapshot() {
        let cache = Arc::new(crate::state::cache::StateCache::new());
        let tasks = no_modem_with(cache.clone());
        let v = state(&tasks, &cache);
        let dumped = v.dump();
        assert!(dumped.contains("\"running\":false"));
        assert!(dumped.contains("\"phase\":\"idle\""));
        assert!(dumped.contains("\"step\":0"));
        assert!(dumped.contains("\"state\":null"));
        // Aborting an idle flow is not an error.
        assert!(abort(&tasks).dump().contains("\"aborted\":false"));
    }

    #[test]
    fn step_timeouts_keep_the_flash_command_short() {
        assert_eq!(step_timeout(FOTA_PROGRESS_QUERY), Duration::from_secs(20));
        assert_eq!(step_timeout(FOTA_UPGRADE), Duration::from_secs(8));
        assert_eq!(step_timeout(FOTA_STATE_QUERY), STEP_TIMEOUT);
    }
}
