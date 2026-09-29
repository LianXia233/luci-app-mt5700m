//! Unified error model for the async MT5700M backend.
//!
//! Every failure surfaced by the task/AT layers maps to one `BackendError`.
//! Errors are both machine readable (`code`) and human readable (`message`),
//! and carry a `retryable` hint so schedulers do not blindly retry faults
//! that retrying cannot fix (e.g. an anchored modem ERROR).

use crate::at::AtError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendError {
    /// The modem is not reachable at all (no serial node / transport down).
    ModemUnavailable,
    /// The transport (serial or TCP) failed mid-exchange.
    TransportError(String),
    /// The AT command did not answer before its timeout.
    AtTimeout,
    /// The modem answered with an anchored ERROR / CME / CMS error.
    AtRejected(String),
    /// The AT channel is saturated or a deadline in the queue elapsed.
    Busy,
    /// The task was cancelled by the caller.
    TaskCancelled,
    /// The task exceeded its overall deadline.
    TaskTimeout,
    /// A parameter was rejected before hitting the modem.
    InvalidParameter(String),
    /// The WebSocket/control client is not allowed to do this.
    PermissionDenied,
    /// The USB device vanished mid-operation.
    UsbDisconnected,
    /// Anything else (bug, unexpected shape).
    Internal(String),
}

impl BackendError {
    /// Stable machine-readable code, e.g. `AT_TIMEOUT`.
    pub fn code(&self) -> &'static str {
        match self {
            BackendError::ModemUnavailable => "MODEM_UNAVAILABLE",
            BackendError::TransportError(_) => "TRANSPORT_ERROR",
            BackendError::AtTimeout => "AT_TIMEOUT",
            BackendError::AtRejected(_) => "AT_REJECTED",
            BackendError::Busy => "BUSY",
            BackendError::TaskCancelled => "TASK_CANCELLED",
            BackendError::TaskTimeout => "TASK_TIMEOUT",
            BackendError::InvalidParameter(_) => "INVALID_PARAMETER",
            BackendError::PermissionDenied => "PERMISSION_DENIED",
            BackendError::UsbDisconnected => "USB_DISCONNECTED",
            BackendError::Internal(_) => "INTERNAL_ERROR",
        }
    }

    pub fn message(&self) -> String {
        match self {
            BackendError::ModemUnavailable => "模组不可用（未连接或传输通道断开）".into(),
            BackendError::TransportError(e) => format!("AT 传输错误: {}", e),
            BackendError::AtTimeout => "AT 指令响应超时".into(),
            BackendError::AtRejected(t) => format!("模组拒绝指令: {}", t),
            BackendError::Busy => "AT 通道繁忙，请稍后重试".into(),
            BackendError::TaskCancelled => "任务已取消".into(),
            BackendError::TaskTimeout => "任务超时".into(),
            BackendError::InvalidParameter(p) => format!("参数无效: {}", p),
            BackendError::PermissionDenied => "权限不足".into(),
            BackendError::UsbDisconnected => "USB 设备已断开".into(),
            BackendError::Internal(e) => format!("内部错误: {}", e),
        }
    }

    /// Whether a scheduler may retry this failure.
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            BackendError::ModemUnavailable
                | BackendError::TransportError(_)
                | BackendError::AtTimeout
                | BackendError::Busy
        )
    }
}

impl From<AtError> for BackendError {
    fn from(e: AtError) -> Self {
        match e {
            AtError::Disabled | AtError::NoSerialPort | AtError::NetworkFailed => {
                BackendError::ModemUnavailable
            }
            AtError::Empty => BackendError::InvalidParameter("empty command".into()),
            AtError::DaemonFailed(msg) => BackendError::TransportError(msg),
            AtError::SerialTimeout(_) | AtError::ModemError(_) => BackendError::AtTimeout,
        }
    }
}

/// JSON payload for an error (used by HTTP/WS error envelopes).
pub fn error_json(err: &BackendError) -> crate::json::Value {
    let mut m = std::collections::BTreeMap::new();
    m.insert("code".to_string(), crate::json::str_val(err.code()));
    m.insert(
        "message".to_string(),
        crate::json::str_val(&err.message()),
    );
    m.insert(
        "retryable".to_string(),
        crate::json::Value::Bool(err.retryable()),
    );
    crate::json::Value::Obj(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable() {
        assert_eq!(BackendError::AtTimeout.code(), "AT_TIMEOUT");
        assert_eq!(BackendError::TaskCancelled.code(), "TASK_CANCELLED");
        assert_eq!(BackendError::ModemUnavailable.code(), "MODEM_UNAVAILABLE");
    }

    #[test]
    fn retryable_classification() {
        assert!(BackendError::AtTimeout.retryable());
        assert!(BackendError::Busy.retryable());
        assert!(!BackendError::AtRejected("ERROR".into()).retryable());
        assert!(!BackendError::InvalidParameter("x".into()).retryable());
    }

    #[test]
    fn json_envelope() {
        let v = error_json(&BackendError::AtTimeout);
        assert!(v.dump().contains("\"code\":\"AT_TIMEOUT\""));
    }
}
