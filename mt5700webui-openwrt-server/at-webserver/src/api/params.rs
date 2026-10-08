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
}
