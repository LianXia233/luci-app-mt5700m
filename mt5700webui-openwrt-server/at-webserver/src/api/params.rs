//! Request-parameter access shared by every module API.
//!
//! Route handlers must not reach into `Value` by hand: extracting the same
//! number/string/bool from a JSON param object was starting to appear in each
//! module's `api.rs`, and the rejection text has to stay uniform because the
//! frontends display it. Missing or mistyped parameters are always a client
//! error (`InvalidParameter`), never an internal one.

use crate::core::error::BackendError;
use crate::core::json::Value;

/// Integer parameter, or `None` when absent/mistyped.
pub fn num(params: &Value, key: &str) -> Option<i64> {
    params.get(key).and_then(|v| v.as_i64())
}

/// Boolean parameter, or `None` when absent/mistyped.
pub fn boolean(params: &Value, key: &str) -> Option<bool> {
    params.get(key).and_then(|v| v.as_bool())
}

/// String parameter, or `None` when absent/mistyped.
pub fn text<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(|v| v.as_str())
}

/// Integer parameter, or a client error naming the missing field.
pub fn required_num(params: &Value, key: &str) -> Result<i64, BackendError> {
    num(params, key).ok_or_else(|| missing(key))
}

/// Integer list parameter, or `None` when absent, not an array, or containing
/// anything that is not a number.
///
/// Used by the threshold-table write, which takes nine values the modem itself
/// orders — a partially-parsed list would silently shift the remaining rungs of
/// the thermal ladder, so anything but a list of integers is rejected.
pub fn num_list(params: &Value, key: &str) -> Option<Vec<i64>> {
    let arr = params.get(key).and_then(|v| v.as_arr())?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        out.push(item.as_i64()?);
    }
    Some(out)
}

/// Integer list parameter, or a client error naming the missing field.
pub fn required_num_list(params: &Value, key: &str) -> Result<Vec<i64>, BackendError> {
    num_list(params, key).ok_or_else(|| missing(key))
}

/// Boolean parameter, or a client error naming the missing field.
pub fn required_bool(params: &Value, key: &str) -> Result<bool, BackendError> {
    boolean(params, key).ok_or_else(|| missing(key))
}

/// Non-empty string parameter, or a client error naming the missing field.
pub fn required_text<'a>(params: &'a Value, key: &str) -> Result<&'a str, BackendError> {
    text(params, key)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| missing(key))
}

fn missing(key: &str) -> BackendError {
    BackendError::InvalidParameter(format!("missing or invalid parameter: {}", key))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::json;

    fn obj(pairs: &[(&str, Value)]) -> Value {
        let mut m = std::collections::BTreeMap::new();
        for (k, v) in pairs {
            m.insert(k.to_string(), v.clone());
        }
        Value::Obj(m)
    }

    #[test]
    fn optional_reads_do_not_error_on_absent_or_mistyped() {
        let p = obj(&[("n", json::num_val(3)), ("s", json::str_val("x")), ("b", json::bool_val(true))]);
        assert_eq!(num(&p, "n"), Some(3));
        assert_eq!(num(&p, "s"), None);
        assert_eq!(boolean(&p, "b"), Some(true));
        assert_eq!(boolean(&p, "n"), None);
        assert_eq!(text(&p, "s"), Some("x"));
        assert_eq!(text(&p, "missing"), None);
    }

    #[test]
    fn required_reads_reject_with_the_field_name() {
        let p = obj(&[("n", json::num_val(-1))]);
        assert_eq!(required_num(&p, "n").ok(), Some(-1));
        let err = required_num(&p, "slot").unwrap_err();
        assert_eq!(err.code(), "INVALID_PARAMETER");
        assert!(err.message().contains("slot"));
        assert!(required_text(&p, "operation").is_err());
        assert!(required_bool(&p, "hotplug").is_err());
        // Whitespace is not a value.
        let blank = obj(&[("operation", json::str_val("  "))]);
        assert!(required_text(&blank, "operation").is_err());
    }

    #[test]
    fn num_list_reads_every_item_or_none() {
        let nine: Vec<Value> = [60, 70, 65, 80, 75, 90, 85, 100, 95]
            .iter()
            .map(|n| json::num_val(*n))
            .collect();
        let p = obj(&[("thresholds", Value::Arr(nine.clone()))]);
        assert_eq!(
            num_list(&p, "thresholds"),
            Some(vec![60, 70, 65, 80, 75, 90, 85, 100, 95])
        );
        assert_eq!(num_list(&p, "missing"), None);

        // A single non-numeric entry must not yield a short list: the modem's
        // threshold ladder is positional, so a shifted list is worse than an
        // outright rejection.
        let mixed = obj(&[("thresholds",
            Value::Arr(vec![json::num_val(60), json::str_val("70")]))]);
        assert_eq!(num_list(&mixed, "thresholds"), None);
        let not_array = obj(&[("thresholds", json::str_val("60,70"))]);
        assert_eq!(num_list(&not_array, "thresholds"), None);
        assert!(required_num_list(&not_array, "thresholds").is_err());
    }
}
