//! DNS service: configuration, application and verification.
//!
//! The actual interface mutation lives in one shell script
//! (`/usr/share/mt5700m/dns.sh`) because it must also run from the manager's
//! boot/up paths, where the Rust daemon may not be running yet. This service:
//!
//! * validates and persists the config (UCI);
//! * starts `dns.sh apply` as an async task so no RPC thread blocks on
//!   `network reload`;
//! * reads back what netifd actually uses and reports saved / applied /
//!   verified separately.
//!
//! No AT command is involved anywhere here. Applying DNS therefore cannot
//! bypass, delay or interleave with the AT scheduler; an SMS transaction is
//! unaffected by a DNS apply.

use crate::core::error::BackendError;
use crate::core::json::{self, Value};
use crate::core::sys;
use crate::core::task::{Priority, TaskKind, TaskId};
use crate::modules::dns::config::{DnsConfig, DnsFamily};
use crate::modules::dns::state::{
    interface_runtime, runtime_matches, ApplyState, InterfaceRuntime,
};
use crate::modules::dns::{APPLY_SCRIPT, IFACE4, IFACE6};
use crate::scheduler::jobs::TaskManager;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// One DNS apply at a time. The script itself is idempotent, but two
/// concurrent `network reload`s would still be pointless churn on a router.
static APPLYING: AtomicBool = AtomicBool::new(false);

const APPLY_TIMEOUT: Duration = Duration::from_secs(30);
const TASK_TIMEOUT: Duration = Duration::from_secs(45);
const TASK_NAME: &str = "dns.apply";

/// `{saved, defaults, applied, runtime, verified, pending, warnings}`.
pub fn status_json() -> Value {
    let config = DnsConfig::load();
    let state = ApplyState::load();
    let (rt4, rt6) = (interface_runtime(IFACE4), interface_runtime(IFACE6));
    status_from(config, state, rt4, rt6)
}

fn status_from(
    config: DnsConfig,
    state: ApplyState,
    rt4: InterfaceRuntime,
    rt6: InterfaceRuntime,
) -> Value {
    let mut warnings: Vec<String> = Vec::new();
    if let Err(e) = config.validate() {
        warnings.push(format!("保存的 DNS 配置无效: {}", e));
    }

    let v4 = family_status(DnsFamily::V4, &config, &state, &rt4, &mut warnings);
    let v6 = family_status(DnsFamily::V6, &config, &state, &rt6, &mut warnings);

    if state.result == "error" {
        warnings.push(format!(
            "上次应用失败: {}",
            state.error.clone().unwrap_or_else(|| "未知错误".to_string())
        ));
    } else if state.result == "partial" {
        warnings.push(format!(
            "上次仅部分应用: {}",
            state.error.clone().unwrap_or_else(|| "请查看运行状态".to_string())
        ));
    } else if state.result == "never" {
        warnings.push("DNS 配置尚未应用到接口".to_string());
    }

    let pending_any = v4.pending || v6.pending;
    let verified_any = v4.verified == Some(false) || v6.verified == Some(false);
    if pending_any {
        warnings.push("配置已保存，但接口尚未使用新值；请点击“保存并应用”".to_string());
    }
    if verified_any {
        warnings.push("接口运行时 DNS 与已应用配置不一致".to_string());
    }

    let mut m = BTreeMap::new();
    m.insert("saved".to_string(), config.to_json());
    m.insert("defaults".to_string(), DnsConfig::defaults_json());
    m.insert("state".to_string(), state.to_json());
    m.insert("applying".to_string(), Value::Bool(is_applying()));
    m.insert("runtime".to_string(), {
        let mut r = BTreeMap::new();
        r.insert("ipv4".to_string(), runtime_json(&rt4, &config.effective_servers(DnsFamily::V4)));
        r.insert("ipv6".to_string(), runtime_json(&rt6, &config.effective_servers(DnsFamily::V6)));
        Value::Obj(r)
    });
    m.insert("verified".to_string(), {
        let mut v = BTreeMap::new();
        v.insert("ipv4".to_string(), option_bool(v4.verified));
        v.insert("ipv6".to_string(), option_bool(v6.verified));
        v.insert("ipv4Reason".to_string(), optional_str(&v4.reason));
        v.insert("ipv6Reason".to_string(), optional_str(&v6.reason));
        Value::Obj(v)
    });
    m.insert("pending".to_string(), {
        let mut p = BTreeMap::new();
        p.insert("ipv4".to_string(), Value::Bool(v4.pending));
        p.insert("ipv6".to_string(), Value::Bool(v6.pending));
        p.insert("any".to_string(), Value::Bool(pending_any));
        Value::Obj(p)
    });
    m.insert("interfaces".to_string(), {
        let mut i = BTreeMap::new();
        i.insert("ipv4".to_string(), json::str_val(&rt4.name));
        i.insert("ipv6".to_string(), json::str_val(&rt6.name));
        Value::Obj(i)
    });
    m.insert(
        "warnings".to_string(),
        Value::Arr(warnings.iter().map(|w| json::str_val(w)).collect()),
    );
    Value::Obj(m)
}

struct FamilyStatus {
    pending: bool,
    /// `None` = not verifiable (family disabled, interface absent or down);
    /// the reason explains which.
    verified: Option<bool>,
    reason: Option<String>,
}

fn family_status(
    family: DnsFamily,
    config: &DnsConfig,
    state: &ApplyState,
    rt: &InterfaceRuntime,
    warnings: &mut Vec<String>,
) -> FamilyStatus {
    let (enabled, expected) = match family {
        DnsFamily::V4 => (config.ipv4.enabled, config.effective_servers(DnsFamily::V4)),
        DnsFamily::V6 => (config.ipv6.enabled, config.effective_servers(DnsFamily::V6)),
    };
    let (applied_enabled, applied) = match family {
        DnsFamily::V4 => (state.v4_enabled, state.v4_servers.clone()),
        DnsFamily::V6 => (state.v6_enabled, state.v6_servers.clone()),
    };

    let pending = enabled != applied_enabled || expected != applied;
    let mut reason = None;

    if !enabled {
        // Disabled means "do not manage this family". There is nothing to
        // verify; old applied values are reported as pending until removed.
        return FamilyStatus {
            pending,
            verified: None,
            reason: Some("disabled".to_string()),
        };
    }
    if expected.is_empty() {
        return FamilyStatus {
            pending,
            verified: None,
            reason: Some("no-servers".to_string()),
        };
    }
    if !rt.configured {
        if family == DnsFamily::V6 && state.v6_skipped {
            // Expected on an IPv4-only dial profile; not an error.
            reason = Some("ipv6-interface-absent".to_string());
        } else {
            warnings.push(format!(
                "{} 接口 {} 未配置，无法校验 DNS",
                family.label(),
                rt.name
            ));
            reason = Some("interface-absent".to_string());
        }
        return FamilyStatus {
            pending,
            verified: None,
            reason,
        };
    }
    if !rt.up {
        reason = Some("interface-down".to_string());
        return FamilyStatus {
            pending,
            verified: None,
            reason,
        };
    }
    // netifd may add resolvers from a delegated prefix; requiring every
    // configured server to be present is the useful check, and unexpected
    // extras are surfaced through the raw runtime list instead of silently
    // declaring failure.
    let verified = runtime_matches(&expected, &rt.dns, false);
    if !verified {
        reason = Some("runtime-mismatch".to_string());
    }
    FamilyStatus {
        pending,
        verified: Some(verified),
        reason,
    }
}

fn runtime_json(rt: &InterfaceRuntime, expected: &[String]) -> Value {
    let mut m = BTreeMap::new();
    m.insert("configured".to_string(), Value::Bool(rt.configured));
    m.insert("up".to_string(), Value::Bool(rt.up));
    m.insert(
        "dns".to_string(),
        Value::Arr(rt.dns.iter().map(|s| json::str_val(s)).collect()),
    );
    m.insert(
        "l3Device".to_string(),
        optional_str(&rt.l3_device),
    );
    m.insert(
        "matchesSaved".to_string(),
        Value::Bool(runtime_matches(expected, &rt.dns, false)),
    );
    Value::Obj(m)
}

fn option_bool(v: Option<bool>) -> Value {
    match v {
        Some(b) => Value::Bool(b),
        None => Value::Null,
    }
}

fn optional_str(v: &Option<String>) -> Value {
    match v {
        Some(s) => json::str_val(s),
        None => Value::Null,
    }
}

pub fn is_applying() -> bool {
    APPLYING.load(Ordering::SeqCst)
}

/// Validate and persist a partial JSON update. Does **not** touch interfaces;
/// the caller applies explicitly (`dns.apply`) so "saved" and "applied" stay
/// distinguishable.
pub fn save(params: &Value) -> Result<Value, BackendError> {
    let mut config = DnsConfig::load();
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
    let mut m = BTreeMap::new();
    m.insert("saved".to_string(), Value::Bool(true));
    m.insert("config".to_string(), config.to_json());
    m.insert("applied".to_string(), Value::Bool(false));
    Ok(Value::Obj(m))
}

/// Restore the documented defaults (saved only; call `apply` afterwards).
pub fn reset_defaults() -> Result<Value, BackendError> {
    let config = DnsConfig::default();
    config.save().map_err(|e| {
        if sys::uci_available() {
            BackendError::Internal(e)
        } else {
            BackendError::Unavailable("uci".to_string())
        }
    })?;
    let mut m = BTreeMap::new();
    m.insert("saved".to_string(), Value::Bool(true));
    m.insert("config".to_string(), config.to_json());
    m.insert("defaults".to_string(), Value::Bool(true));
    Ok(Value::Obj(m))
}

/// Start `dns.sh apply` as a task and return its id.
///
/// The HTTP/WS request returns immediately; the page observes
/// `task.completed`/`task.failed` and then re-reads `dns.status` to show the
/// verified runtime values. Rejecting a second concurrent apply is deliberate:
/// two netifd reloads racing would make the verification meaningless.
pub fn start_apply(tasks: &Arc<TaskManager>) -> Result<TaskId, BackendError> {
    let config = DnsConfig::load();
    config
        .validate()
        .map_err(BackendError::InvalidParameter)?;
    if !std::path::Path::new(APPLY_SCRIPT).exists() {
        return Err(BackendError::Unavailable(format!(
            "缺少 DNS 应用脚本 {}",
            APPLY_SCRIPT
        )));
    }
    if APPLYING.swap(true, Ordering::SeqCst) {
        return Err(BackendError::Busy);
    }
    let id = tasks.submit(
        TASK_NAME,
        TaskKind::LongRunning,
        Priority::UserQuery,
        Some(TASK_TIMEOUT),
        Box::new(|ctx| {
            let _guard = ApplyGuard;
            ctx.progress(10, "正在应用 DNS 配置");
            let out = sys::run(APPLY_SCRIPT, &["apply"], APPLY_TIMEOUT);
            if !out.ok {
                return Err(BackendError::Internal(format!(
                    "DNS 应用失败: {}",
                    out.error()
                )));
            }
            ctx.progress(80, "正在校验接口运行时 DNS");
            // Give netifd a bounded moment to publish the new resolvers; this
            // is not a sleep-in-place for the UI (the task already owns its
            // own thread) and the verification below is authoritative.
            std::thread::sleep(Duration::from_millis(250));
            let status = status_json();
            let mut m = BTreeMap::new();
            m.insert("applied".to_string(), Value::Bool(true));
            m.insert(
                "verified".to_string(),
                status.get("verified").cloned().unwrap_or(Value::Null),
            );
            m.insert(
                "warnings".to_string(),
                status.get("warnings").cloned().unwrap_or(Value::Null),
            );
            m.insert(
                "state".to_string(),
                status.get("state").cloned().unwrap_or(Value::Null),
            );
            Ok(Value::Obj(m))
        }),
    );
    Ok(id)
}

struct ApplyGuard;

impl Drop for ApplyGuard {
    fn drop(&mut self) {
        APPLYING.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::dns::config::{FamilyConfig, DEFAULT_V4_PRIMARY, DEFAULT_V4_SECONDARY};

    fn rt(name: &str, configured: bool, up: bool, dns: &[&str]) -> InterfaceRuntime {
        InterfaceRuntime {
            name: name.to_string(),
            configured,
            up,
            dns: dns.iter().map(|s| s.to_string()).collect(),
            l3_device: None,
        }
    }

    fn applied_ok() -> ApplyState {
        ApplyState {
            result: "ok".to_string(),
            applied_at: Some(1_700_000_000),
            error: None,
            v4_enabled: true,
            v4_servers: vec![DEFAULT_V4_PRIMARY.to_string(), DEFAULT_V4_SECONDARY.to_string()],
            v6_enabled: true,
            v6_servers: vec!["2402:4e00::".to_string(), "2400:3200::1".to_string()],
            v6_skipped: false,
        }
    }

    #[test]
    fn verified_when_runtime_has_the_saved_servers() {
        let cfg = DnsConfig::default();
        let value = status_from(
            cfg,
            applied_ok(),
            rt(IFACE4, true, true, &[DEFAULT_V4_PRIMARY, DEFAULT_V4_SECONDARY]),
            rt(IFACE6, true, true, &["2402:4e00::", "2400:3200::1"]),
        );
        let verified = value.get("verified").unwrap();
        assert_eq!(verified.get("ipv4").unwrap(), &Value::Bool(true));
        assert_eq!(
            value
                .get("pending")
                .and_then(|p| p.get("ipv4"))
                .and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn down_interface_is_not_reported_as_verified() {
        let value = status_from(
            DnsConfig::default(),
            applied_ok(),
            rt(IFACE4, true, false, &[]),
            rt(IFACE6, false, false, &[]),
        );
        let verified = value.get("verified").unwrap();
        assert_eq!(verified.get("ipv4").unwrap(), &Value::Null);
        assert_eq!(
            verified.get("ipv4Reason").unwrap().as_str(),
            Some("interface-down")
        );
    }

    #[test]
    fn saved_but_not_applied_is_pending() {
        let mut state = applied_ok();
        state.v4_servers = vec!["1.1.1.1".to_string()];
        let value = status_from(
            DnsConfig::default(),
            state,
            rt(IFACE4, true, true, &[DEFAULT_V4_PRIMARY, DEFAULT_V4_SECONDARY]),
            rt(IFACE6, true, true, &["2402:4e00::"]),
        );
        let text = value.dump();
        assert!(text.contains("配置已保存"));
        assert_eq!(
            value
                .get("pending")
                .and_then(|p| p.get("ipv4"))
                .and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn missing_ipv6_interface_is_a_skip_not_a_failure() {
        let mut state = applied_ok();
        state.v6_skipped = true;
        state.v6_servers.clear();
        state.v6_enabled = false;
        let value = status_from(
            DnsConfig::default(),
            state,
            rt(IFACE4, true, true, &[DEFAULT_V4_PRIMARY, DEFAULT_V4_SECONDARY]),
            rt(IFACE6, false, false, &[]),
        );
        let text = value.dump();
        assert!(text.contains("ipv6-interface-absent"));
        // IPv4 is still verified.
        assert!(text.contains("\"ipv4\":true"));
    }

    #[test]
    fn disabled_family_is_not_verified_and_not_pending_when_removed() {
        let mut cfg = DnsConfig::default();
        cfg.ipv6.enabled = false;
        let mut state = applied_ok();
        state.v6_enabled = false;
        state.v6_servers.clear();
        let value = status_from(
            cfg,
            state,
            rt(IFACE4, true, true, &[DEFAULT_V4_PRIMARY]),
            rt(IFACE6, true, true, &[]),
        );
        let verified = value.get("verified").unwrap();
        assert_eq!(verified.get("ipv6").unwrap(), &Value::Null);
        assert_eq!(
            verified.get("ipv6Reason").unwrap().as_str(),
            Some("disabled")
        );
        assert_eq!(
            value
                .get("pending")
                .and_then(|p| p.get("ipv6"))
                .and_then(|v| v.as_bool()),
            Some(false)
        );
        assert_eq!(
            value
                .get("pending")
                .and_then(|p| p.get("ipv4"))
                .and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn invalid_saved_config_is_surfaced_as_a_warning() {
        let mut cfg = DnsConfig::default();
        cfg.ipv4 = FamilyConfig {
            enabled: true,
            primary: "bad".to_string(),
            secondary: String::new(),
        };
        let value = status_from(
            cfg,
            applied_ok(),
            rt(IFACE4, true, true, &[]),
            rt(IFACE6, true, true, &[]),
        );
        assert!(value.dump().contains("保存的 DNS 配置无效"));
    }
}
