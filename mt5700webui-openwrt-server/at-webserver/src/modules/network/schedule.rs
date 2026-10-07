//! Day/night band-lock schedule: the config DTO and its UCI mapping.
//!
//! This is the one place that knows the `at-webserver.config.schedule_*` keys.
//! The WebUI's schedule panel reads it as `network.schedule_get` and saves it as
//! `network.schedule_set`; the day/night applier (`scheduler::plan`) loads the
//! same config through `SchedCfg::load`, so the page and the applier can never
//! disagree about a field's meaning.
//!
//! The DTO is the one the panel always received over the (now removed) pseudo-AT
//! command `AT+SCHED?` — nested periods with their own lock lists, typed numbers
//! and booleans, plus the live `status` block. The daemon's old port answered
//! with the flat UCI map instead (every value a string, no `night`/`day`
//! nesting), which no frontend could read; the raw command still answers
//! `+SCHED: {json}` but now with this object.
//!
//! The master switch `schedule_enabled` is LuCI's: a save never touches it, so
//! the WebUI cannot switch the feature off for good (it would have no way back).

use crate::core::error::BackendError;
use crate::core::json::{self, Value};
use crate::scheduler::plan::{Lock, PeriodCfg, SchedCfg};
use std::collections::BTreeMap;

/// Lock types (`LOCK_TYPES` in the panel): 0 off, 1 arfcn, 2 cell, 3 band.
const LOCK_TYPES: std::ops::RangeInclusive<i64> = 0..=3;
const MIN_CHECK_INTERVAL: u64 = 10;
const MIN_TIMEOUT: u64 = 30;
pub const BAD_TIME_COPY: &str = "夜间时段请填写 HH:MM 格式，例如 22:00";

fn obj(items: Vec<(&str, Value)>) -> Value {
    let mut m = BTreeMap::new();
    for (k, v) in items {
        m.insert(k.to_string(), v);
    }
    Value::Obj(m)
}

/// `{type, bands, arfcns, scs_types, pcis}` — the panel's `LockLists`.
fn lock_json(lock: &Lock) -> Value {
    obj(vec![
        ("type", json::num_val(lock.ltype)),
        ("bands", json::str_val(&lock.bands)),
        ("arfcns", json::str_val(&lock.arfcns)),
        ("scs_types", json::str_val(&lock.scs_types)),
        ("pcis", json::str_val(&lock.pcis)),
    ])
}

/// `{enabled, start?, end?, lte, nr}` — `start`/`end` only exist on the night
/// window; the day window is everything outside it.
fn period_json(period: &PeriodCfg, window: Option<(&str, &str)>) -> Value {
    let mut items: Vec<(&str, Value)> = vec![("enabled", Value::Bool(period.enabled))];
    if let Some((start, end)) = window {
        items.push(("start", json::str_val(start)));
        items.push(("end", json::str_val(end)));
    }
    items.push(("lte", lock_json(&period.lte)));
    items.push(("nr", lock_json(&period.nr)));
    obj(items)
}

/// The page-load snapshot: the whole config plus the applier's live status.
pub fn read() -> Value {
    let cfg = SchedCfg::load();
    obj(vec![
        ("enabled", Value::Bool(cfg.enabled)),
        ("check_interval", json::num_val(cfg.check_interval as i64)),
        ("timeout", json::num_val(cfg.timeout as i64)),
        ("unlock_lte", Value::Bool(cfg.unlock_lte)),
        ("unlock_nr", Value::Bool(cfg.unlock_nr)),
        ("toggle_airplane", Value::Bool(cfg.toggle_airplane)),
        (
            "night",
            period_json(&cfg.night, Some((&cfg.night_start, &cfg.night_end))),
        ),
        ("day", period_json(&cfg.day, None)),
        ("status", crate::scheduler::plan::status(&cfg)),
    ])
}

fn time_ok(s: &str) -> bool {
    let Some((h, m)) = s.split_once(':') else {
        return false;
    };
    if h.len() != 2 || m.len() != 2 {
        return false;
    }
    matches!((h.parse::<u32>(), m.parse::<u32>()), (Ok(h), Ok(m)) if h < 24 && m < 60)
}

/// UCI booleans are `1`/`0`.
fn flag(b: bool) -> String {
    if b { "1".to_string() } else { "0".to_string() }
}

fn text_of(v: &Value) -> Option<String> {
    match v {
        Value::Str(s) => Some(s.clone()),
        Value::Num(n) => Some(n.clone()),
        _ => None,
    }
}

/// One `uci set` per known key; the applier re-reads the file every 15 s, so a
/// save takes effect on the next tick without a restart.
fn write_uci(sets: Vec<(String, String)>) {
    for (key, value) in sets {
        let _ = std::process::Command::new("uci")
            .args(["set", &format!("at-webserver.config.{}", key), &value])
            .status();
    }
    let _ = std::process::Command::new("uci")
        .args(["commit", "at-webserver"])
        .status();
}

/// Validate and persist a save (`network.schedule_set`, `AT+SCHED={json}`).
///
/// `enabled` and `status` are read-only here: the master switch is LuCI's and
/// the status is the applier's. Every other field is optional — an omitted one
/// keeps its stored value, which is what a partial caller (the CLI) wants.
pub fn write(params: &Value) -> Result<(), BackendError> {
    let bad = |msg: &str| -> Result<(), BackendError> {
        Err(BackendError::InvalidParameter(msg.to_string()))
    };
    let mut sets: Vec<(String, String)> = Vec::new();

    if let Some(v) = params.get("check_interval") {
        let Some(n) = v.as_u64() else {
            return bad("检测间隔必须是整数秒");
        };
        if n < MIN_CHECK_INTERVAL {
            return bad(&format!("检测间隔不能小于 {} 秒", MIN_CHECK_INTERVAL));
        }
        sets.push(("schedule_check_interval".to_string(), n.to_string()));
    }
    if let Some(v) = params.get("timeout") {
        let Some(n) = v.as_u64() else {
            return bad("无服务超时必须是整数秒");
        };
        if n < MIN_TIMEOUT {
            return bad(&format!("无服务超时不能小于 {} 秒", MIN_TIMEOUT));
        }
        sets.push(("schedule_timeout".to_string(), n.to_string()));
    }
    for (key, uci) in [
        ("unlock_lte", "schedule_unlock_lte"),
        ("unlock_nr", "schedule_unlock_nr"),
        ("toggle_airplane", "schedule_toggle_airplane"),
    ] {
        if let Some(v) = params.get(key) {
            let Some(b) = v.as_bool() else {
                return bad(&format!("{} 必须是布尔值", key));
            };
            sets.push((uci.to_string(), flag(b)));
        }
    }

    for (period, prefix) in [("night", "schedule_night"), ("day", "schedule_day")] {
        let Some(p) = params.get(period) else { continue };
        if let Some(v) = p.get("enabled") {
            let Some(b) = v.as_bool() else {
                return bad(&format!("{} 的启用开关必须是布尔值", period));
            };
            sets.push((format!("{}_enabled", prefix), flag(b)));
        }
        // Only the night window has explicit boundaries.
        if period == "night" {
            for (key, suffix) in [("start", "_start"), ("end", "_end")] {
                if let Some(v) = p.get(key) {
                    let Some(s) = text_of(v) else {
                        return bad(BAD_TIME_COPY);
                    };
                    if !time_ok(&s) {
                        return bad(BAD_TIME_COPY);
                    }
                    sets.push((format!("{}{}", prefix, suffix), s));
                }
            }
        }
        for (kind, lock_prefix) in [("lte", "lte"), ("nr", "nr")] {
            let Some(lock) = p.get(kind) else { continue };
            let lock_key = |field: &str| format!("{}_{}_{}", prefix, lock_prefix, field);
            if let Some(v) = lock.get("type") {
                let Some(t) = v.as_i64() else {
                    return bad("锁频类型必须是 0-3");
                };
                if !LOCK_TYPES.contains(&t) {
                    return bad("锁频类型只能是 0-3（0 关闭、1 频点、2 小区、3 Band）");
                }
                sets.push((lock_key("type"), t.to_string()));
            }
            for field in ["bands", "arfcns", "scs_types", "pcis"] {
                if let Some(v) = lock.get(field) {
                    let Some(s) = text_of(v) else {
                        return bad("锁频参数必须是逗号分隔的字符串");
                    };
                    sets.push((lock_key(field), s));
                }
            }
        }
    }

    write_uci(sets);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::json::parse;

    #[test]
    fn time_window_validation_matches_the_panel() {
        assert!(time_ok("22:00") && time_ok("00:00") && time_ok("23:59"));
        assert!(!time_ok("24:00") && !time_ok("8:00") && !time_ok("22:0"));
        assert!(!time_ok("") && !time_ok("晚上"));
        let err = write(&parse(r#"{"night":{"start":"25:00"}}"#).unwrap()).unwrap_err();
        assert_eq!(err.detail(), BAD_TIME_COPY);
    }

    #[test]
    fn bounds_reject_what_the_panel_clamps() {
        let err = write(&parse(r#"{"check_interval":5}"#).unwrap()).unwrap_err();
        assert_eq!(err.detail(), "检测间隔不能小于 10 秒");
        let err = write(&parse(r#"{"timeout":10}"#).unwrap()).unwrap_err();
        assert_eq!(err.detail(), "无服务超时不能小于 30 秒");
        let err = write(&parse(r#"{"night":{"lte":{"type":9}}}"#).unwrap()).unwrap_err();
        assert!(err.detail().starts_with("锁频类型只能是"));
        // A well-formed save is accepted (the uci calls are best-effort here).
        assert!(write(&parse(r#"{"check_interval":30,"timeout":60}"#).unwrap()).is_ok());
    }

    #[test]
    fn read_answers_the_dto_the_panel_renders() {
        let v = read();
        // Nested periods with lock lists + the status block, never the flat UCI map.
        for key in [
            "enabled",
            "check_interval",
            "timeout",
            "unlock_lte",
            "unlock_nr",
            "toggle_airplane",
            "night",
            "day",
            "status",
        ] {
            assert!(v.get(key).is_some(), "missing {}", key);
        }
        let night = v.get("night").unwrap();
        assert!(night.get("start").is_some() && night.get("end").is_some());
        assert!(night.get("lte").unwrap().get("type").is_some());
        assert!(v.get("day").unwrap().get("start").is_none());
        assert!(v.get("status").unwrap().get("current_mode").is_some());
        assert!(v.get("status").unwrap().get("switch_count").is_some());
        // The master switch is a boolean, not the UCI string "1".
        assert!(matches!(v.get("enabled"), Some(Value::Bool(_))));
    }
}
