//! NTP service: configuration, application, observation and manual sync.
//!
//! The plugin does not implement NTP and never starts a second clock
//! controller. It writes the configuration of the system provider
//! (`sysntpd`/`chrony`), restarts that provider, and reads its status back.
//! Manual sync triggers that same provider (`chronyc` for chrony, a restart
//! of `sysntpd` for the stock daemon) and polls its state. A one-shot
//! `ntpd -nq` query is used only when no system service exists at all, so it
//! can never race a running clock controller.
//!
//! Nothing here touches the AT channel. NTP work therefore cannot bypass the
//! AT scheduler, delay SMS, or be delayed by a modem operation.
//!
//! Rules that the implementation keeps:
//!
//! * saved / applied / verified are separate answers;
//! * `synced` is only `Some(true)` with positive evidence, never guessed;
//! * a failed sync is recorded with its category (DNS / network / timeout /
//!   server) and retried with backoff; it never fails plugin startup or dial;
//! * one apply and one manual sync at a time (double-click safe).

use crate::core::error::BackendError;
use crate::core::json::{self, Value};
use crate::core::sys;
use crate::core::task::{Priority, TaskId, TaskKind};
use crate::core::time;
use crate::modules::timesync::config::{self, NtpConfig};
use crate::modules::timesync::provider::{self, Provider};
use crate::modules::timesync::state::NtpState;
use crate::scheduler::jobs::{TaskCtx, TaskManager};
use std::collections::BTreeMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const MONITOR_PERIOD: Duration = Duration::from_secs(60);
const MONITOR_TIMEOUT: Duration = Duration::from_secs(120);
const APPLY_TASK_TIMEOUT: Duration = Duration::from_secs(75);
const SYNC_TASK_TIMEOUT: Duration = Duration::from_secs(120);
/// Overall budget for one manual sync. The task timeout leaves headroom so a
/// slow server cannot make the task fail before the state is written.
const SYNC_BUDGET: Duration = Duration::from_secs(90);
const NTP_QUERY_TIMEOUT: Duration = Duration::from_secs(8);
const DNS_TIMEOUT: Duration = Duration::from_secs(4);
const APPLY_TASK_NAME: &str = "ntp.apply";
const SYNC_TASK_NAME: &str = "ntp.sync";
const MONITOR_NAME: &str = "ntp.monitor";
/// Automatic retries stop after this many failures. The page still allows an
/// explicit retry; this only stops a permanently broken setup from running a
/// query every few minutes forever.
const MAX_AUTO_RETRIES: u32 = 6;

/// One apply at a time.
static APPLYING: AtomicBool = AtomicBool::new(false);
/// One manual sync at a time.
static SYNCING: AtomicBool = AtomicBool::new(false);

/// Detection spawns `pidof`/`ubus`; the page polls status, so cache the
/// answer briefly instead of paying for four commands per request.
static PROVIDER_CACHE: Mutex<Option<(Instant, Provider, bool)>> = Mutex::new(None);
const PROVIDER_TTL: Duration = Duration::from_secs(15);

pub fn is_applying() -> bool {
    APPLYING.load(Ordering::SeqCst)
}

pub fn is_syncing() -> bool {
    SYNCING.load(Ordering::SeqCst)
}

fn invalidate_provider_cache() {
    *PROVIDER_CACHE.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

fn provider_info_fresh() -> (Provider, bool) {
    let provider = provider::detect();
    let running = provider::running(provider);
    (provider, running)
}

fn provider_info() -> (Provider, bool) {
    {
        let cache = PROVIDER_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, provider, running)) = *cache {
            if at.elapsed() < PROVIDER_TTL {
                return (provider, running);
            }
        }
    }
    let (provider, running) = provider_info_fresh();
    *PROVIDER_CACHE.lock().unwrap_or_else(|e| e.into_inner()) =
        Some((Instant::now(), provider, running));
    (provider, running)
}

/// `{saved, defaults, state, provider, time, applying, syncing, pending, verified, warnings}`.
///
/// Reads UCI, the runtime state file, cached provider info and the clock; no
/// NTP packet is sent and no long command is run from the request thread.
pub fn status_json() -> Value {
    let now = time::now_unix();
    let config = NtpConfig::load();
    let state = NtpState::load();
    let (provider, running) = provider_info();
    let config_matches = config.validate().is_ok()
        && config.enabled
        && provider.configurable()
        && provider::config_matches(provider, &config.effective_servers());
    status_from(config, state, provider, running, config_matches, now)
}

fn status_from(
    config: NtpConfig,
    state: NtpState,
    provider: Provider,
    running: bool,
    config_matches: bool,
    now: u64,
) -> Value {
    let effective = config.effective_servers();
    let mut warnings: Vec<String> = Vec::new();
    if let Err(e) = config.validate() {
        warnings.push(format!("保存的 NTP 配置无效: {}", e));
    }
    match provider {
        Provider::None => warnings.push(
            "系统未检测到 NTP 服务（sysntpd/chrony）；配置可以保存，但无法应用".to_string(),
        ),
        Provider::ExternalNtpd => warnings.push(
            "检测到外部 ntpd 进程；插件只读取其状态，不修改它的配置".to_string(),
        ),
        _ => {}
    }
    if config.enabled && provider.configurable() && !running {
        warnings.push("NTP 服务当前未运行；应用配置后会启动".to_string());
    }

    let applied_matches = state.apply.result == "ok"
        && state.apply.provider.as_deref() == Some(provider.name())
        && state.apply.servers == effective;

    if config.enabled {
        match state.apply.result.as_str() {
            "never" => warnings.push("NTP 配置尚未应用到系统服务".to_string()),
            "error" | "partial" => warnings.push(format!(
                "上次应用失败: {}",
                state
                    .apply
                    .error
                    .clone()
                    .unwrap_or_else(|| "未知错误".to_string())
            )),
            _ => {}
        }
        if state.apply.result == "ok" && !applied_matches {
            warnings.push("配置已保存，但尚未应用到系统 NTP 服务".to_string());
        }
    } else if !state.apply.servers.is_empty() {
        warnings.push("NTP 管理已关闭，但系统服务中仍有插件写入的服务器；请点击“应用”恢复".to_string());
    }

    match state.last_sync_result.as_deref() {
        Some("failed") => warnings.push(format!(
            "上次手动同步失败: {}",
            state.last_error.clone().unwrap_or_else(|| "未知错误".to_string())
        )),
        Some("unverified") => {
            warnings.push("上次手动同步已执行，但无法确认同步结果".to_string())
        }
        _ => {}
    }
    if state.synced == Some(true) {
        let stale = state
            .observed_at
            .map(|at| now.saturating_sub(at) > crate::modules::timesync::state::STALE_AFTER_SECS)
            .unwrap_or(true);
        if stale {
            warnings.push("同步状态已超过 3 分钟未更新，时间暂不可信".to_string());
        }
    } else if config.enabled {
        warnings.push("系统时间尚未由 NTP 确认同步，当前时间不可信".to_string());
    }
    if state.failures > 0 {
        if let Some(at) = state.next_retry_at {
            if state.failures < MAX_AUTO_RETRIES {
                warnings.push(format!(
                    "同步失败 {} 次，将在 {} 自动重试",
                    state.failures,
                    time::format_unix_local(at)
                ));
            } else {
                warnings.push(format!(
                    "同步已失败 {} 次，自动重试已停止，请手动重试",
                    state.failures
                ));
            }
        }
    }

    let pending = if config.enabled {
        !applied_matches
    } else {
        !state.apply.servers.is_empty()
    };
    let verified = config.enabled
        && applied_matches
        && config_matches
        && running
        && state.apply.provider.as_deref() == Some(provider.name());

    let mut provider_json = BTreeMap::new();
    provider_json.insert("name".to_string(), json::str_val(provider.name()));
    provider_json.insert("running".to_string(), Value::Bool(running));
    provider_json.insert(
        "configurable".to_string(),
        Value::Bool(provider.configurable()),
    );

    let mut m = BTreeMap::new();
    m.insert("saved".to_string(), config.to_json());
    m.insert("defaults".to_string(), NtpConfig::defaults_json());
    m.insert("state".to_string(), state.to_json_at(now));
    m.insert("provider".to_string(), Value::Obj(provider_json));
    m.insert("time".to_string(), time_json());
    m.insert("applying".to_string(), Value::Bool(is_applying()));
    m.insert("syncing".to_string(), Value::Bool(is_syncing()));
    m.insert("pending".to_string(), Value::Bool(pending));
    m.insert("verified".to_string(), Value::Bool(verified));
    m.insert("trusted".to_string(), Value::Bool(state.trusted_at(now)));
    m.insert(
        "effectiveServers".to_string(),
        Value::Arr(effective.iter().map(|s| json::str_val(s)).collect()),
    );
    m.insert(
        "warnings".to_string(),
        Value::Arr(warnings.iter().map(|w| json::str_val(w)).collect()),
    );
    Value::Obj(m)
}

/// The unified time interface as JSON. `unix` is the authoritative value;
/// `local` and `timezone` are display-only and come from the device config.
pub fn time_json() -> Value {
    let mut m = BTreeMap::new();
    m.insert("unix".to_string(), json::num_val(time::now_unix() as i64));
    m.insert("unixMs".to_string(), json::num_val(time::now_unix_ms() as i64));
    m.insert("local".to_string(), json::str_val(&time::local_time_string()));
    m.insert(
        "timezone".to_string(),
        json::str_val(&time::device_timezone()),
    );
    m.insert(
        "plausible".to_string(),
        Value::Bool(time::clock_plausible()),
    );
    Value::Obj(m)
}

/// `{config, defaults, provider, applying, syncing}` — no runtime probing.
pub fn config_get() -> Value {
    let config = NtpConfig::load();
    let (provider, running) = provider_info();
    let mut provider_json = BTreeMap::new();
    provider_json.insert("name".to_string(), json::str_val(provider.name()));
    provider_json.insert("running".to_string(), Value::Bool(running));
    provider_json.insert(
        "configurable".to_string(),
        Value::Bool(provider.configurable()),
    );
    let mut m = BTreeMap::new();
    m.insert("config".to_string(), config.to_json());
    m.insert("defaults".to_string(), NtpConfig::defaults_json());
    m.insert("provider".to_string(), Value::Obj(provider_json));
    m.insert("applying".to_string(), Value::Bool(is_applying()));
    m.insert("syncing".to_string(), Value::Bool(is_syncing()));
    Value::Obj(m)
}

/// Validate and persist. Application is a separate step so "saved" never
/// implies "the daemon is using it".
pub fn save(params: &Value) -> Result<Value, BackendError> {
    let mut config = NtpConfig::load();
    config
        .merge_json(params)
        .map_err(BackendError::InvalidParameter)?;
    config
        .validate()
        .map_err(BackendError::InvalidParameter)?;
    config.save().map_err(|e| {
        if sys::uci_available() {
            BackendError::Internal(e)
        } else {
            BackendError::Unavailable("uci".to_string())
        }
    })?;
    log(&format!(
        "NTP 配置已保存: enabled={} servers={:?}",
        config.enabled,
        config.effective_servers()
    ));
    let mut m = BTreeMap::new();
    m.insert("saved".to_string(), Value::Bool(true));
    m.insert("applied".to_string(), Value::Bool(false));
    m.insert("config".to_string(), config.to_json());
    Ok(Value::Obj(m))
}

pub fn reset_defaults() -> Result<Value, BackendError> {
    let config = NtpConfig::default();
    config.save().map_err(|e| {
        if sys::uci_available() {
            BackendError::Internal(e)
        } else {
            BackendError::Unavailable("uci".to_string())
        }
    })?;
    log("NTP 配置已恢复默认值（尚未应用）");
    let mut m = BTreeMap::new();
    m.insert("saved".to_string(), Value::Bool(true));
    m.insert("defaults".to_string(), Value::Bool(true));
    m.insert("applied".to_string(), Value::Bool(false));
    m.insert("config".to_string(), config.to_json());
    Ok(Value::Obj(m))
}

/// Apply the saved configuration asynchronously.
///
/// The request returns a task id immediately. A second apply while one is
/// running is rejected with `BUSY` instead of racing two daemon restarts.
pub fn start_apply(tasks: &Arc<TaskManager>) -> Result<TaskId, BackendError> {
    if !sys::uci_available() {
        return Err(BackendError::Unavailable("uci".to_string()));
    }
    let config = NtpConfig::load();
    config
        .validate()
        .map_err(BackendError::InvalidParameter)?;
    if config.enabled {
        let (provider, _) = provider_info_fresh();
        if !provider.configurable() {
            return Err(match provider {
                Provider::ExternalNtpd => BackendError::Unavailable(
                    "检测到外部 ntpd，插件不会修改其配置".to_string(),
                ),
                _ => BackendError::Unavailable(
                    "系统未安装 sysntpd/chrony，无法应用 NTP 配置".to_string(),
                ),
            });
        }
    }
    if APPLYING.swap(true, Ordering::SeqCst) {
        return Err(BackendError::Busy);
    }
    let id = tasks.submit(
        APPLY_TASK_NAME,
        TaskKind::LongRunning,
        Priority::UserQuery,
        Some(APPLY_TASK_TIMEOUT),
        Box::new(|ctx| {
            let _guard = ApplyGuard;
            ctx.progress(10, "正在读取 NTP 配置");
            let config = NtpConfig::load();
            if let Err(e) = config.validate() {
                return Err(BackendError::InvalidParameter(e));
            }
            let (provider, _) = provider_info_fresh();
            ctx.progress(40, &format!("正在配置 {} 服务", provider.name()));
            match apply_config(&config, provider) {
                Ok(value) => Ok(value),
                Err(e) => {
                    record_apply_failure(&config, provider, &e.detail());
                    Err(e)
                }
            }
        }),
    );
    Ok(id)
}

/// Trigger an immediate manual sync asynchronously.
pub fn start_sync(tasks: &Arc<TaskManager>) -> Result<TaskId, BackendError> {
    if !sys::uci_available() {
        return Err(BackendError::Unavailable("uci".to_string()));
    }
    let config = NtpConfig::load();
    config
        .validate()
        .map_err(BackendError::InvalidParameter)?;
    if config.effective_servers().is_empty() {
        return Err(BackendError::InvalidParameter(
            "没有启用的 NTP 服务器".to_string(),
        ));
    }
    if SYNCING.swap(true, Ordering::SeqCst) {
        return Err(BackendError::Busy);
    }
    let id = tasks.submit(
        SYNC_TASK_NAME,
        TaskKind::LongRunning,
        Priority::UserQuery,
        Some(SYNC_TASK_TIMEOUT),
        Box::new(|ctx| {
            let _guard = SyncGuard;
            run_manual_sync(ctx)
        }),
    );
    Ok(id)
}

// --------------------------------------------------------------- apply ----

/// Write the provider config and verify it. Caller holds `APPLYING`.
fn apply_config(config: &NtpConfig, provider: Provider) -> Result<Value, BackendError> {
    if !sys::uci_available() {
        return Err(BackendError::Unavailable("uci".to_string()));
    }
    let now = time::now_unix();

    if !config.enabled {
        // Turning the plugin's NTP management off restores what the system
        // had before the plugin touched it. Nothing is invented, and a system
        // that was never managed is left alone.
        let restored = if config::is_backed_up() {
            config::restore_system().map_err(|e| {
                BackendError::Internal(format!("恢复系统 NTP 配置失败: {}", e))
            })?;
            true
        } else {
            false
        };
        let removed_block = if provider::chrony_managed() {
            provider::remove_chrony_block().map_err(|e| {
                BackendError::Internal(format!("移除 chrony 插件配置失败: {}", e))
            })?;
            true
        } else {
            false
        };
        if restored && provider == Provider::Sysntpd {
            provider::restart_sysntpd().map_err(|e| {
                BackendError::Internal(format!("重启 sysntpd 失败: {}", e))
            })?;
        }
        invalidate_provider_cache();
        record_apply_success(provider, Vec::new(), now);
        log(&format!(
            "NTP 管理已关闭（restored={} chrony_block_removed={}）",
            restored, removed_block
        ));
        return Ok(apply_json(provider, &[], false, true));
    }

    let servers = config.effective_servers();
    match provider {
        Provider::Sysntpd => {
            config::backup_system_once().map_err(|e| {
                BackendError::Internal(format!("备份系统 NTP 配置失败: {}", e))
            })?;
            provider::apply_sysntpd(&servers).map_err(|e| {
                BackendError::Internal(format!("应用 sysntpd 配置失败: {}", e))
            })?;
            verify_with_retry(provider, &servers)?;
        }
        Provider::Chrony => {
            provider::apply_chrony(&servers).map_err(|e| {
                BackendError::Internal(format!("应用 chrony 配置失败: {}", e))
            })?;
            verify_with_retry(provider, &servers)?;
        }
        Provider::ExternalNtpd => {
            return Err(BackendError::Unavailable(
                "检测到外部 ntpd，插件不会修改其配置".to_string(),
            ));
        }
        Provider::None => {
            return Err(BackendError::Unavailable(
                "系统未安装 sysntpd/chrony".to_string(),
            ));
        }
    }
    invalidate_provider_cache();
    record_apply_success(provider, servers.clone(), now);
    log(&format!(
        "NTP 配置已应用: provider={} servers={:?}",
        provider.name(),
        servers
    ));
    Ok(apply_json(provider, &servers, true, false))
}

fn verify_with_retry(provider: Provider, servers: &[String]) -> Result<(), BackendError> {
    let mut last = String::new();
    for _ in 0..15 {
        let check = match provider {
            Provider::Sysntpd => provider::verify_sysntpd(servers),
            Provider::Chrony => provider::verify_chrony(servers).and_then(|_| {
                if provider::running(Provider::Chrony) {
                    Ok(())
                } else {
                    Err("chronyd 未在运行".to_string())
                }
            }),
            _ => Err("provider cannot be configured".to_string()),
        };
        match check {
            Ok(()) => return Ok(()),
            Err(e) => last = e,
        }
        // procd restarts asynchronously; a bounded settle time keeps the
        // verification honest without blocking any other task.
        std::thread::sleep(Duration::from_millis(200));
    }
    Err(BackendError::Internal(format!(
        "NTP 配置校验失败: {}",
        last
    )))
}

fn apply_json(
    provider: Provider,
    servers: &[String],
    enabled: bool,
    disabled: bool,
) -> Value {
    let mut m = BTreeMap::new();
    m.insert("applied".to_string(), Value::Bool(true));
    m.insert("provider".to_string(), json::str_val(provider.name()));
    m.insert(
        "servers".to_string(),
        Value::Arr(servers.iter().map(|s| json::str_val(s)).collect()),
    );
    m.insert("enabled".to_string(), Value::Bool(enabled));
    m.insert("disabled".to_string(), Value::Bool(disabled));
    Value::Obj(m)
}

fn record_apply_success(provider: Provider, servers: Vec<String>, now: u64) {
    let _ = NtpState::update(|st| {
        st.apply.result = "ok".to_string();
        st.apply.at = Some(now);
        st.apply.error = None;
        st.apply.provider = Some(provider.name().to_string());
        st.apply.servers = servers;
        st.apply.failures = 0;
        st.apply.retry_at = None;
    });
}

fn record_apply_failure(config: &NtpConfig, provider: Provider, message: &str) {
    let now = time::now_unix();
    let servers = config.effective_servers();
    let _ = NtpState::update(|st| {
        let failures = st.apply.failures.saturating_add(1);
        st.apply.result = "error".to_string();
        st.apply.at = Some(now);
        st.apply.error = Some(message.to_string());
        st.apply.provider = Some(provider.name().to_string());
        st.apply.servers = servers;
        st.apply.failures = failures;
        st.apply.retry_at = Some(now.saturating_add(backoff(failures)));
    });
    log(&format!("NTP 配置应用失败: {}", message));
}

// ---------------------------------------------------------------- sync ----

struct Attempt {
    host: String,
    outcome: &'static str,
    detail: Option<String>,
}

impl Attempt {
    fn json(&self) -> Value {
        let mut m = BTreeMap::new();
        m.insert("host".to_string(), json::str_val(&self.host));
        m.insert("outcome".to_string(), json::str_val(self.outcome));
        m.insert(
            "detail".to_string(),
            self.detail
                .as_deref()
                .map(json::str_val)
                .unwrap_or(Value::Null),
        );
        Value::Obj(m)
    }
}

struct SyncOutcome {
    synced: bool,
    server: Option<String>,
    offset_ms: Option<i64>,
    warning: Option<String>,
}

struct SyncFailure {
    kind: &'static str,
    message: String,
}

impl SyncFailure {
    fn new(kind: &'static str, message: impl Into<String>) -> Self {
        SyncFailure {
            kind,
            message: message.into(),
        }
    }
}

/// How a manual sync is allowed to reach the clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SyncRoute {
    /// `chronyc` talks to the running chronyd.
    Chrony,
    /// Restart the procd-managed stock service; never start `ntpd -nq`.
    Sysntpd,
    /// No system service exists; a one-shot query cannot race anything.
    OneShot,
    /// An external daemon owns the clock; observe only, never trigger.
    ReadOnly,
}

fn sync_route(provider: Provider) -> SyncRoute {
    match provider {
        Provider::Chrony => SyncRoute::Chrony,
        Provider::Sysntpd => SyncRoute::Sysntpd,
        Provider::ExternalNtpd => SyncRoute::ReadOnly,
        Provider::None => SyncRoute::OneShot,
    }
}

fn run_manual_sync(ctx: &TaskCtx) -> Result<Value, BackendError> {
    let started = Instant::now();
    let started_unix = time::now_unix();
    let _ = NtpState::update(|st| st.last_attempt_at = Some(started_unix));

    let config = NtpConfig::load();
    config
        .validate()
        .map_err(BackendError::InvalidParameter)?;
    let servers = config.effective_servers();
    if servers.is_empty() {
        return Err(BackendError::InvalidParameter(
            "没有启用的 NTP 服务器".to_string(),
        ));
    }
    let (provider, running) = provider_info_fresh();
    let mut attempts: Vec<Attempt> = Vec::new();

    // A system clock controller owns the clock. Never spawn `ntpd -nq` next
    // to it: trigger that controller and read its state instead.
    match sync_route(provider) {
        SyncRoute::Chrony => {
            require_applied(provider, &servers)?;
            if !running {
                ctx.progress(12, "正在启动系统 chrony 服务");
                provider::restart_chrony().map_err(|e| {
                    BackendError::Internal(format!("启动 chronyd 失败: {}", e))
                })?;
                if !provider::running(Provider::Chrony) {
                    return Err(BackendError::Unavailable(
                        "chronyd 未在运行，且系统内没有可用的 chrony 服务；请先应用 NTP 配置"
                            .to_string(),
                    ));
                }
            }
            ctx.progress(20, "正在触发 chrony 立即校时");
            match sync_with_chrony(ctx) {
                Ok(outcome) => return finish_sync(outcome, attempts, started),
                Err(failure) => {
                    record_sync_failure(failure.kind, &failure.message);
                    return Err(BackendError::Internal(failure.message));
                }
            }
        }
        SyncRoute::Sysntpd => {
            require_applied(provider, &servers)?;
            match sync_with_sysntpd(ctx) {
                Ok(outcome) => return finish_sync(outcome, attempts, started),
                Err(failure) => {
                    record_sync_failure(failure.kind, &failure.message);
                    return Err(BackendError::Internal(failure.message));
                }
            }
        }
        SyncRoute::ReadOnly => {
            return Err(BackendError::Unavailable(
                "检测到外部 ntpd 进程；插件只读取其状态，不会启动第二个 NTP 进程。请通过该服务自行校时"
                    .to_string(),
            ));
        }
        // No system service at all: a one-shot query is the only process and
        // exits before returning, so there is nothing to compete with.
        SyncRoute::OneShot => {}
    }

    ctx.progress(15, "正在检查 DNS 与 NTP 服务器");
    for (index, host) in servers.iter().enumerate() {
        if ctx.cancelled() {
            return Err(BackendError::TaskCancelled);
        }
        if started.elapsed() >= SYNC_BUDGET {
            attempts.push(Attempt {
                host: host.clone(),
                outcome: "timeout",
                detail: Some("总时间预算已用尽".to_string()),
            });
            break;
        }
        match resolve_host(host) {
            Resolve::Failed(reason) => {
                attempts.push(Attempt {
                    host: host.clone(),
                    outcome: "dns",
                    detail: Some(reason),
                });
                continue;
            }
            Resolve::Unknown(reason) => {
                log(&format!("DNS 预检未完成: {}", reason));
            }
            Resolve::Ok => {}
        }
        let pct = 20 + ((index * 60) / servers.len()).min(60) as u8;
        ctx.progress(pct, &format!("正在与 {} 校时", host));
        let out = sys::run("ntpd", &["-nq", "-p", host], NTP_QUERY_TIMEOUT);
        if out.timed_out {
            attempts.push(Attempt {
                host: host.clone(),
                outcome: "timeout",
                detail: Some(format!("{} 秒内无响应", NTP_QUERY_TIMEOUT.as_secs())),
            });
            continue;
        }
        if !out.ok {
            let text = format!("{}\n{}", out.stdout, out.stderr);
            let kind = classify_ntp_text(&text);
            attempts.push(Attempt {
                host: host.clone(),
                outcome: kind,
                detail: Some(first_line(&text)),
            });
            continue;
        }
        // Give the kernel/daemon a moment to publish the new state, then ask
        // for positive evidence. Exit 0 alone is "executed", not "synced".
        std::thread::sleep(Duration::from_millis(250));
        let observed = provider::observe(provider);
        if observed.synced == Some(true) {
            let outcome = SyncOutcome {
                synced: true,
                server: observed.server.clone().or_else(|| Some(host.clone())),
                offset_ms: observed.offset_ms,
                warning: None,
            };
            attempts.push(Attempt {
                host: host.clone(),
                outcome: "ok",
                detail: None,
            });
            return finish_sync(outcome, attempts, started);
        }
        attempts.push(Attempt {
            host: host.clone(),
            outcome: "ok-unverified",
            detail: Some("校时命令已执行，系统未报告可确认的同步状态".to_string()),
        });
        let outcome = SyncOutcome {
            synced: false,
            server: Some(host.clone()),
            offset_ms: observed.offset_ms,
            warning: Some(
                "校时命令已执行，但无法确认系统时间已同步；请查看系统 NTP 服务状态"
                    .to_string(),
            ),
        };
        return finish_sync(outcome, attempts, started);
    }

    let kind = aggregate_kind(&attempts);
    let message = format!("NTP 校时失败: {}", summarise_attempts(&attempts));
    record_sync_failure(kind, &message);
    Err(BackendError::Internal(message))
}

/// Manual sync must use the same servers the system service is configured
/// with. If the saved config was never applied, say so instead of silently
/// querying a different set of servers.
fn require_applied(provider: Provider, servers: &[String]) -> Result<(), BackendError> {
    if provider::config_matches(provider, servers) {
        return Ok(());
    }
    let message = format!(
        "NTP 配置尚未应用到 {}；请先点击“应用到系统”再立即同步",
        provider.name()
    );
    log(&message);
    Err(BackendError::InvalidParameter(message))
}

/// Trigger the stock OpenWrt service.
///
/// Busybox ntpd has no "query now" command, so restarting the procd-managed
/// instance is the trigger. This is still the single system controller: no
/// second `ntpd` is spawned. Success is reported only when `ntpq`/`adjtimex`
/// yields positive evidence; otherwise the result is explicitly unverified.
fn sync_with_sysntpd(ctx: &TaskCtx) -> Result<SyncOutcome, SyncFailure> {
    ctx.progress(20, "正在重启系统 NTP 服务以立即校时");
    provider::restart_sysntpd()
        .map_err(|e| SyncFailure::new("internal", format!("重启 sysntpd 失败: {}", e)))?;

    // procd reports success before the instance is scheduled; wait for the
    // daemon itself so "restarted" cannot be mistaken for "running".
    let run_deadline = Instant::now() + Duration::from_secs(8);
    while !provider::running(Provider::Sysntpd) {
        if ctx.cancelled() {
            return Err(SyncFailure::new("internal", "任务已取消"));
        }
        if Instant::now() >= run_deadline {
            return Err(SyncFailure::new("internal", "重启后 sysntpd 未在运行"));
        }
        std::thread::sleep(Duration::from_millis(200));
    }

    let deadline = Instant::now() + Duration::from_secs(25);
    let mut unknown_rounds = 0u32;
    let mut last_detail: Option<String> = None;
    loop {
        if ctx.cancelled() {
            return Err(SyncFailure::new("internal", "任务已取消"));
        }
        let observed = provider::observe(Provider::Sysntpd);
        match observed.synced {
            Some(true) => {
                return Ok(SyncOutcome {
                    synced: true,
                    server: observed.server,
                    offset_ms: observed.offset_ms,
                    warning: None,
                });
            }
            Some(false) => unknown_rounds = 0,
            None => unknown_rounds = unknown_rounds.saturating_add(1),
        }
        if observed.detail.is_some() {
            last_detail = observed.detail;
        }
        // Three unknown rounds in a row mean the system offers no readable
        // NTP state (e.g. no ntpq and no adjtimex); polling longer would only
        // spawn commands without evidence.
        if unknown_rounds >= 3 || Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }

    let detail = last_detail.unwrap_or_else(|| "系统未提供可读取的同步状态".to_string());
    Ok(SyncOutcome {
        synced: false,
        server: None,
        offset_ms: None,
        warning: Some(format!(
            "已重启系统 NTP 服务触发立即校时，但无法确认同步结果: {}",
            detail
        )),
    })
}

fn sync_with_chrony(ctx: &TaskCtx) -> Result<SyncOutcome, SyncFailure> {
    let out = sys::run(
        "chronyc",
        &["-a", "burst 4/4"],
        Duration::from_secs(8),
    );
    if out.timed_out {
        return Err(SyncFailure::new("timeout", "chronyc 在 8 秒内无响应"));
    }
    if !out.ok {
        let text = format!("{}\n{}", out.stdout, out.stderr);
        return Err(SyncFailure::new(
            classify_ntp_text(&text),
            format!("触发 chrony 失败: {}", first_line(&text)),
        ));
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut observed = provider::observe(Provider::Chrony);
    while Instant::now() < deadline && observed.synced != Some(true) {
        if ctx.cancelled() {
            return Err(SyncFailure::new("internal", "任务已取消"));
        }
        std::thread::sleep(Duration::from_millis(500));
        observed = provider::observe(Provider::Chrony);
    }
    if observed.synced != Some(true) {
        return Err(SyncFailure::new(
            "timeout",
            observed
                .detail
                .clone()
                .unwrap_or_else(|| "chrony 在限定时间内未达到同步状态".to_string()),
        ));
    }
    // Apply the measured correction now; chrony normally slews, a manual sync
    // is the place where a step is expected.
    let step = sys::run("chronyc", &["-a", "makestep"], Duration::from_secs(5));
    let after = provider::observe(Provider::Chrony);
    let warning = if step.ok {
        None
    } else {
        Some("chrony 已同步，但立即校时（makestep）未成功；请查看 chronyc 输出".to_string())
    };
    Ok(SyncOutcome {
        synced: after.synced == Some(true),
        server: after.server.clone().or(observed.server),
        offset_ms: after.offset_ms.or(observed.offset_ms),
        warning,
    })
}

fn finish_sync(
    outcome: SyncOutcome,
    attempts: Vec<Attempt>,
    started: Instant,
) -> Result<Value, BackendError> {
    let now = time::now_unix();
    let _ = NtpState::update(|st| {
        st.last_attempt_at = Some(now);
        st.observed_at = Some(now);
        st.source = Some("manual".to_string());
        st.last_error = None;
        st.last_error_kind = None;
        st.failures = 0;
        st.next_retry_at = None;
        if outcome.synced {
            st.synced = Some(true);
            st.last_success_at = Some(now);
            st.last_sync_result = Some("synced".to_string());
            st.detail = None;
            if outcome.server.is_some() {
                st.server = outcome.server.clone();
            }
            if outcome.offset_ms.is_some() {
                st.offset_ms = outcome.offset_ms;
            }
        } else {
            st.synced = None;
            st.last_sync_result = Some("unverified".to_string());
            st.detail = outcome.warning.clone();
            if outcome.server.is_some() {
                st.server = outcome.server.clone();
            }
        }
    });
    log(&format!(
        "手动同步结果: {} server={:?}",
        if outcome.synced { "synced" } else { "unverified" },
        outcome.server
    ));

    let mut m = BTreeMap::new();
    m.insert("ok".to_string(), Value::Bool(true));
    m.insert(
        "result".to_string(),
        json::str_val(if outcome.synced { "synced" } else { "unverified" }),
    );
    m.insert(
        "synced".to_string(),
        if outcome.synced {
            Value::Bool(true)
        } else {
            Value::Null
        },
    );
    m.insert(
        "server".to_string(),
        outcome
            .server
            .as_deref()
            .map(json::str_val)
            .unwrap_or(Value::Null),
    );
    m.insert(
        "offsetMs".to_string(),
        outcome.offset_ms.map(json::num_val).unwrap_or(Value::Null),
    );
    m.insert(
        "attempts".to_string(),
        Value::Arr(attempts.iter().map(Attempt::json).collect()),
    );
    m.insert(
        "warnings".to_string(),
        Value::Arr(
            outcome
                .warning
                .iter()
                .map(|w| json::str_val(w))
                .collect::<Vec<_>>(),
        ),
    );
    m.insert(
        "durationMs".to_string(),
        json::num_val(started.elapsed().as_millis() as i64),
    );
    m.insert("time".to_string(), time_json());
    Ok(Value::Obj(m))
}

fn record_sync_failure(kind: &str, message: &str) {
    let now = time::now_unix();
    let _ = NtpState::update(|st| {
        st.last_attempt_at = Some(now);
        st.last_error = Some(message.to_string());
        st.last_error_kind = Some(
            match kind {
                "dns" | "network" | "timeout" | "server" | "clock" | "config" | "internal" => kind,
                _ => "internal",
            }
            .to_string(),
        );
        st.last_sync_result = Some("failed".to_string());
        st.failures = st.failures.saturating_add(1);
        st.next_retry_at = Some(now.saturating_add(backoff(st.failures)));
    });
    log(&format!("手动同步失败 ({}): {}", kind, message));
}

/// 30s, 60s, 120s, 300s, then 10min. Capped so retries still happen after a
/// long outage, slow enough not to fill the log.
fn backoff(failures: u32) -> u64 {
    match failures {
        0 | 1 => 30,
        2 => 60,
        3 => 120,
        4 => 300,
        _ => 600,
    }
}

// ------------------------------------------------------------ resolving ----

enum Resolve {
    Ok,
    /// The helper is missing or its output is inconclusive: let `ntpd`
    /// resolve the name itself instead of blaming DNS.
    Unknown(String),
    Failed(String),
}

fn resolve_host(host: &str) -> Resolve {
    if Ipv4Addr::from_str(host).is_ok() || Ipv6Addr::from_str(host).is_ok() {
        return Resolve::Ok;
    }
    let out = sys::run("nslookup", &[host], DNS_TIMEOUT);
    if out.timed_out {
        return Resolve::Failed(format!("DNS 查询超时: {}", host));
    }
    let text = format!("{}\n{}", out.stdout, out.stderr);
    let lower = text.to_ascii_lowercase();
    if dns_error_text(&lower) {
        return Resolve::Failed(format!(
            "DNS 解析失败: {} ({})",
            host,
            first_line(&text)
        ));
    }
    if lower.contains("no such file") || lower.contains("nslookup: not found") {
        return resolve_getent(host);
    }
    if !out.ok {
        if lower.contains("not found") {
            return resolve_getent(host);
        }
        return Resolve::Unknown(format!("nslookup 失败: {}", first_line(&text)));
    }
    if lower.contains("name:") {
        return Resolve::Ok;
    }
    Resolve::Unknown(format!("无法确认 DNS 解析结果: {}", first_line(&text)))
}

fn resolve_getent(host: &str) -> Resolve {
    let out = sys::run("getent", &["hosts", host], DNS_TIMEOUT);
    if out.timed_out {
        return Resolve::Failed(format!("DNS 查询超时: {}", host));
    }
    if out.ok && !out.stdout.trim().is_empty() {
        Resolve::Ok
    } else if out.stderr.to_ascii_lowercase().contains("no such file") {
        Resolve::Unknown("系统缺少 nslookup/getent".to_string())
    } else {
        Resolve::Failed(format!("DNS 解析失败: {}", host))
    }
}

fn dns_error_text(lower: &str) -> bool {
    [
        "can't find",
        "nxdomain",
        "servfail",
        "no answer",
        "name or service not known",
        "temporary failure in name resolution",
        "unknown host",
        "bad address",
        "query timed out",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn classify_ntp_text(text: &str) -> &'static str {
    let lower = text.to_ascii_lowercase();
    if dns_error_text(&lower) {
        return "dns";
    }
    if lower.contains("unreachable")
        || lower.contains("no route")
        || lower.contains("network is down")
        || lower.contains("host is down")
    {
        return "network";
    }
    if lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("no response")
        || lower.contains("no reply")
    {
        return "timeout";
    }
    if lower.contains("permission denied")
        || lower.contains("not permitted")
        || lower.contains("operation not permitted")
        || lower.contains("no such file")
    {
        return "internal";
    }
    "server"
}

fn aggregate_kind(attempts: &[Attempt]) -> &'static str {
    let kinds: Vec<&str> = attempts
        .iter()
        .filter(|a| !a.outcome.starts_with("ok"))
        .map(|a| a.outcome)
        .collect();
    if kinds.is_empty() {
        return "server";
    }
    if kinds.iter().all(|k| *k == "dns") {
        return "dns";
    }
    if kinds.iter().any(|k| *k == "network") {
        return "network";
    }
    if kinds.iter().all(|k| *k == "timeout") {
        return "timeout";
    }
    if kinds.iter().any(|k| *k == "timeout") {
        return "timeout";
    }
    if kinds.iter().any(|k| *k == "internal") {
        return "internal";
    }
    "server"
}

fn summarise_attempts(attempts: &[Attempt]) -> String {
    if attempts.is_empty() {
        return "没有可用的服务器".to_string();
    }
    attempts
        .iter()
        .map(|a| {
            format!(
                "{}: {}{}",
                a.host,
                a.outcome,
                a.detail
                    .as_deref()
                    .map(|d| format!(" ({})", d))
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("无输出");
    let mut s: String = line.chars().take(160).collect();
    if line.chars().count() > 160 {
        s.push('…');
    }
    s
}

fn log(message: &str) {
    // procd captures stderr into syslog. One line, no control characters.
    let message = message.replace(['\n', '\r'], " ");
    eprintln!("mt5700m-ntp: {}", message);
}

// ------------------------------------------------------------- monitor ----

/// Periodic observation and self-healing. Runs regardless of whether a LuCI
/// page is open: the system clock is a system concern, not a page feature.
pub fn spawn(tasks: &TaskManager) {
    tasks.add_periodic_always(
        MONITOR_NAME,
        MONITOR_PERIOD,
        Priority::Maintenance,
        Some(MONITOR_TIMEOUT),
        Box::new(monitor),
    );
}

fn monitor(ctx: &TaskCtx) -> Result<Value, BackendError> {
    let config = NtpConfig::load();
    let (provider, running) = provider_info_fresh();
    let now = time::now_unix();
    let mut warnings: Vec<String> = Vec::new();

    if provider != Provider::None {
        let observed = provider::observe(provider);
        let _ = NtpState::update(|st| {
            st.synced = observed.synced;
            st.observed_at = Some(now);
            if observed.server.is_some() {
                st.server = observed.server.clone();
            }
            if observed.offset_ms.is_some() {
                st.offset_ms = observed.offset_ms;
            }
            if st.source.is_none() {
                st.source = Some("system".to_string());
            }
            st.detail = observed.detail.clone();
        });
    }

    let mut state = NtpState::load();
    // The daemon recovered on its own: stop the manual-sync retry loop.
    if state.synced == Some(true) && state.failures > 0 {
        let _ = NtpState::update(|st| {
            st.failures = 0;
            st.next_retry_at = None;
            st.last_error = None;
            st.last_error_kind = None;
        });
        state = NtpState::load();
    }

    ctx.progress(35, "正在检查 NTP 配置");
    if config.validate().is_ok() {
        let desired = config.effective_servers();
        let mismatch = if !config.enabled {
            !state.apply.servers.is_empty() || provider::chrony_managed()
        } else if provider.configurable() {
            !provider::config_matches(provider, &desired) || !running
        } else {
            false
        };
        let due = state.apply.retry_at.map(|at| at <= now).unwrap_or(true);
        if mismatch && due {
            if APPLYING
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                let _guard = ApplyGuard;
                ctx.progress(55, "正在自动应用 NTP 配置");
                match apply_config(&config, provider) {
                    Ok(_) => {}
                    Err(e) => {
                        record_apply_failure(&config, provider, &e.detail());
                        warnings.push(e.detail());
                    }
                }
            }
        } else if config.enabled
            && provider.configurable()
            && running
            && provider::config_matches(provider, &desired)
            && (state.apply.result != "ok"
                || state.apply.provider.as_deref() != Some(provider.name())
                || state.apply.servers != desired)
        {
            // The provider already has exactly these servers (e.g. after a
            // reboot where the config survived but the runtime state did
            // not). Adopt it without a pointless restart.
            let _ = NtpState::update(|st| {
                st.apply.result = "ok".to_string();
                st.apply.at = Some(now);
                st.apply.error = None;
                st.apply.provider = Some(provider.name().to_string());
                st.apply.servers = desired.clone();
                st.apply.failures = 0;
                st.apply.retry_at = None;
            });
        }
    }

    state = NtpState::load();
    let sync_due = config.enabled
        && provider != Provider::None
        && state.failures > 0
        && state.failures < MAX_AUTO_RETRIES
        && state.last_sync_result.as_deref() == Some("failed")
        && state
            .next_retry_at
            .map(|at| at <= now)
            .unwrap_or(false)
        && !config.effective_servers().is_empty();
    if sync_due
        && SYNCING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    {
        let _guard = SyncGuard;
        ctx.progress(80, "正在按退避计划重试 NTP 校时");
        if let Err(e) = run_manual_sync(ctx) {
            warnings.push(e.detail());
        }
    }

    let state = NtpState::load();
    let mut m = BTreeMap::new();
    m.insert("checkedAt".to_string(), json::num_val(now as i64));
    m.insert("provider".to_string(), json::str_val(provider.name()));
    m.insert("running".to_string(), Value::Bool(running));
    m.insert(
        "synced".to_string(),
        state.synced.map(Value::Bool).unwrap_or(Value::Null),
    );
    m.insert("trusted".to_string(), Value::Bool(state.trusted_at(now)));
    m.insert(
        "warnings".to_string(),
        Value::Arr(warnings.iter().map(|w| json::str_val(w)).collect()),
    );
    Ok(Value::Obj(m))
}

// --------------------------------------------------------------- guards ----

struct ApplyGuard;

impl Drop for ApplyGuard {
    fn drop(&mut self) {
        APPLYING.store(false, Ordering::SeqCst);
    }
}

struct SyncGuard;

impl Drop for SyncGuard {
    fn drop(&mut self) {
        SYNCING.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::timesync::config::NtpServer;
    use crate::modules::timesync::state::ApplyRecord;

    fn synced_state() -> NtpState {
        NtpState {
            apply: ApplyRecord {
                result: "ok".to_string(),
                at: Some(1_700_000_000),
                error: None,
                provider: Some("sysntpd".to_string()),
                servers: vec![
                    "ntp.tencent.com".to_string(),
                    "time.cloudflare.com".to_string(),
                    "ntp.aliyun.com".to_string(),
                ],
                failures: 0,
                retry_at: None,
            },
            synced: Some(true),
            observed_at: Some(1_700_000_100),
            source: Some("system".to_string()),
            server: Some("ntp.tencent.com".to_string()),
            offset_ms: Some(2),
            last_attempt_at: None,
            last_success_at: Some(1_700_000_100),
            last_error: None,
            last_error_kind: None,
            last_sync_result: None,
            next_retry_at: None,
            failures: 0,
            detail: None,
        }
    }

    #[test]
    fn status_separates_saved_applied_and_verified() {
        let value = status_from(
            NtpConfig::default(),
            synced_state(),
            Provider::Sysntpd,
            true,
            true,
            1_700_000_120,
        );
        assert_eq!(value.get("verified").unwrap(), &Value::Bool(true));
        assert_eq!(value.get("pending").unwrap(), &Value::Bool(false));
        assert_eq!(value.get("trusted").unwrap(), &Value::Bool(true));
        assert!(value.get("time").is_some());
        assert!(value
            .get("saved")
            .and_then(|v| v.get("effective"))
            .is_some());
    }

    #[test]
    fn stale_observation_is_not_trusted_and_says_so() {
        let value = status_from(
            NtpConfig::default(),
            synced_state(),
            Provider::Sysntpd,
            true,
            true,
            1_700_010_000,
        );
        assert_eq!(value.get("trusted").unwrap(), &Value::Bool(false));
        assert!(value.dump().contains("同步状态已超过"));
    }

    #[test]
    fn saved_but_not_applied_is_pending_with_a_warning() {
        let mut state = synced_state();
        state.apply.servers = vec!["pool.ntp.org".to_string()];
        let value = status_from(
            NtpConfig::default(),
            state,
            Provider::Sysntpd,
            true,
            false,
            1_700_000_120,
        );
        assert_eq!(value.get("pending").unwrap(), &Value::Bool(true));
        assert_eq!(value.get("verified").unwrap(), &Value::Bool(false));
        assert!(value.dump().contains("尚未应用"));
    }

    #[test]
    fn no_provider_is_reported_not_verified() {
        let value = status_from(
            NtpConfig::default(),
            NtpState::default(),
            Provider::None,
            false,
            false,
            1_700_000_120,
        );
        assert_eq!(value.get("verified").unwrap(), &Value::Bool(false));
        assert_eq!(value.get("trusted").unwrap(), &Value::Bool(false));
        assert!(value.dump().contains("未检测到 NTP 服务"));
    }

    #[test]
    fn disabled_config_with_leftover_servers_is_pending() {
        let mut config = NtpConfig::default();
        config.enabled = false;
        let value = status_from(
            config,
            synced_state(),
            Provider::Sysntpd,
            true,
            false,
            1_700_000_120,
        );
        assert_eq!(value.get("pending").unwrap(), &Value::Bool(true));
        assert!(value.dump().contains("NTP 管理已关闭"));
    }

    #[test]
    fn enabled_config_with_no_servers_is_invalid_not_silently_ok() {
        let mut config = NtpConfig::default();
        config.servers = vec![NtpServer {
            host: "time.example.com".to_string(),
            enabled: false,
        }];
        assert!(config.validate().is_err());
        let value = status_from(
            config,
            synced_state(),
            Provider::Sysntpd,
            true,
            false,
            1_700_000_120,
        );
        assert!(value.dump().contains("保存的 NTP 配置无效"));
    }

    #[test]
    fn failure_categories_are_classified() {
        assert_eq!(classify_ntp_text("Temporary failure in name resolution"), "dns");
        assert_eq!(classify_ntp_text("Network is unreachable"), "network");
        assert_eq!(classify_ntp_text("Recvfrom: Connection timed out"), "timeout");
        assert_eq!(classify_ntp_text("ntpd: no server suitable for synch"), "server");
        assert_eq!(classify_ntp_text("Operation not permitted"), "internal");
    }

    #[test]
    fn backoff_grows_and_is_capped() {
        assert_eq!(backoff(1), 30);
        assert_eq!(backoff(2), 60);
        assert_eq!(backoff(3), 120);
        assert_eq!(backoff(4), 300);
        assert_eq!(backoff(5), 600);
        assert_eq!(backoff(99), 600);
    }

    #[test]
    fn aggregate_prefers_the_specific_causes() {
        let all_dns = vec![
            Attempt {
                host: "a".into(),
                outcome: "dns",
                detail: None,
            },
            Attempt {
                host: "b".into(),
                outcome: "dns",
                detail: None,
            },
        ];
        assert_eq!(aggregate_kind(&all_dns), "dns");
        let mixed = vec![
            Attempt {
                host: "a".into(),
                outcome: "timeout",
                detail: None,
            },
            Attempt {
                host: "b".into(),
                outcome: "network",
                detail: None,
            },
        ];
        assert_eq!(aggregate_kind(&mixed), "network");
        let timeouts = vec![Attempt {
            host: "a".into(),
            outcome: "timeout",
            detail: None,
        }];
        assert_eq!(aggregate_kind(&timeouts), "timeout");
    }

    #[test]
    fn dns_error_detection_does_not_match_generic_not_found() {
        assert!(dns_error_text("** server can't find ntp.example.com: NXDOMAIN"));
        assert!(dns_error_text("temporary failure in name resolution"));
        assert!(!dns_error_text("ntpd: no such file or directory"));
    }

    #[test]
    fn manual_sync_never_spawns_a_second_daemon() {
        // Only a device with no NTP service at all may use the one-shot
        // `ntpd -nq` path. Every provider that owns the clock is triggered
        // through its own service, and an external daemon is read-only.
        assert_eq!(sync_route(Provider::None), SyncRoute::OneShot);
        assert_eq!(sync_route(Provider::Sysntpd), SyncRoute::Sysntpd);
        assert_eq!(sync_route(Provider::Chrony), SyncRoute::Chrony);
        assert_eq!(sync_route(Provider::ExternalNtpd), SyncRoute::ReadOnly);
    }

    #[test]
    fn first_line_is_bounded_and_never_empty() {
        assert_eq!(first_line("\n  hello  \nrest"), "hello");
        assert_eq!(first_line("   \n"), "无输出");
        let long = "x".repeat(400);
        assert!(first_line(&long).chars().count() <= 161);
    }
}
