//! SMS service: status, list, send, delete, storage and the IMS sequence.
//!
//! Every function here is a transaction through the AT channel the caller
//! hands in (`RefreshCtx` for the API/collectors, `DaemonChannel` for the
//! CLI). The module never opens the port and never builds wire sequences: the
//! PDU codec (`pdu.rs`) produces the octets, `AtChannel::send_sms_pdu` runs the
//! multi-phase `AT+CMGS` on the single arbiter thread.
//!
//! What used to be page-side logic and now lives here, once:
//!
//! * PDU mode is a precondition of listing (`+CMGF=0`), not something every
//!   caller remembers to toggle;
//! * the destination is normalised the way the send page always did
//!   (country code 86 prepended to an 11-digit local number);
//! * the clear-all sequence reads the storage planes itself and settles after
//!   each step instead of relying on a page's `setTimeout`;
//! * the IMS enable/disable step list is the module's (`commands::ims_sequence`).
//!
//! There is no periodic SMS collector: new messages arrive as `+CMTI` URCs,
//! which the URC dispatcher already publishes on the SMS topic.

use crate::core::channel::SmsPart;
use crate::core::error::BackendError;
use crate::core::json::{self, Value};
use crate::modules::sms::commands::{
    self, CMGD_ALL, CMGF_PDU, CMGF_QUERY, CMGL_ALL, CPMS_QUERY, CSCA_QUERY, IMSSWITCH_QUERY,
    STORAGES,
};
use crate::modules::sms::parser::{parse_cmgf, parse_cpms, parse_csca, parse_cmgl, parse_imsswitch};
use crate::modules::sms::pdu;
use crate::modules::sms::state::{SmsMessage, SmsSettings, SmsStorage};
use crate::state::refresh::RefreshCtx;
use std::time::Duration;

/// Query budget (the modem is slow to answer while a PDP context wakes up).
const AT_TIMEOUT: Duration = Duration::from_secs(12);
const QUEUED_TIMEOUT: Duration = Duration::from_secs(8);
/// Settle time around a storage switch / delete-all (measured on the MT5700M:
/// `+CMGD` right after `+CPMS` is dropped otherwise).
const STORAGE_SETTLE: Duration = Duration::from_millis(500);
const DELETE_SETTLE: Duration = Duration::from_millis(1000);

/// Read `^IMSSWITCH?`, `+CSCA?` (only when IMS is on, as the page always did)
/// and `+CMGF?`, plus the storage planes.
///
/// Never fails: a missing or busy modem yields `{enabled: false}`, which the
/// pages render as "短信未开启" — the same branch their failed raw read took.
pub fn read_status(ctx: &RefreshCtx) -> SmsSettings {
    let mut st = SmsSettings::default();
    if let Ok(text) = ctx.query(CMGF_QUERY) {
        st.enabled = parse_cmgf(&text).is_some();
    }
    if let Ok(text) = ctx.query(IMSSWITCH_QUERY) {
        st.ims_on = parse_imsswitch(&text);
    }
    if st.ims_on == Some(true) {
        if let Ok(text) = ctx.query(CSCA_QUERY) {
            st.center = parse_csca(&text);
        }
    }
    st.storage = read_storage(ctx).unwrap_or_default();
    st
}

/// `+CPMS?` — the three storage planes.
pub fn read_storage(ctx: &RefreshCtx) -> Result<SmsStorage, BackendError> {
    let text = ctx.query(CPMS_QUERY)?;
    Ok(parse_cpms(&text))
}

/// Ensure PDU mode: `+CMGF?`, and `+CMGF=0` when the modem is in text mode.
///
/// Shared by list and send so neither can leave the modem in a mode that
/// generates invalid PDUs.
pub fn ensure_pdu_mode(ctx: &RefreshCtx) -> Result<(), BackendError> {
    let format = ctx.query(CMGF_QUERY).ok().and_then(|t| parse_cmgf(&t));
    if format != Some(0) {
        ctx.action(CMGF_PDU)?;
    }
    Ok(())
}

/// `+CMGL=4` — every stored message, decoded.
///
/// A list is a snapshot, not a cached read: it goes through `action` so the
/// read gate cannot answer with an older list. `+CMGL` replies with `OK` and no
/// headers when the store is empty, which is an empty list, not an error.
pub fn list(ctx: &RefreshCtx) -> Result<Vec<SmsMessage>, BackendError> {
    ensure_pdu_mode(ctx)?;
    let raw = ctx.action(CMGL_ALL)?;
    Ok(parse_cmgl(&raw))
}

/// Destination as the PDU encoder needs it: digits with a leading `+`.
///
/// The send page has always promoted an 11-digit local number to country code
/// 86; keeping that here means the CLI and both frontends format identically.
pub fn normalize_destination(target: &str) -> Option<String> {
    let mut digits: String = target.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    if !digits.starts_with("86") && digits.len() == 11 {
        digits = format!("86{}", digits);
    }
    if !(5..=20).contains(&digits.len()) {
        return None;
    }
    Some(format!("+{}", digits))
}

/// Encode `text` for `number` and send it, one `AT+CMGS` transaction per part.
///
/// The returned parts count is what the page shows as success feedback; a
/// failure carries the transport's message (which already names the failing
/// part when the message was split).
pub fn send(ctx: &RefreshCtx, number: &str, text: &str) -> Result<usize, BackendError> {
    let destination = normalize_destination(number).ok_or_else(|| {
        BackendError::InvalidParameter(format!("短信目标号码无效: {}", number))
    })?;
    if text.trim().is_empty() {
        return Err(BackendError::InvalidParameter("短信内容不能为空".into()));
    }
    // The centre the network expects in front of every TPDU: the same
    // `+CSCA?` value the send page used to read, best effort — a modem that
    // does not answer leaves the field `00`, i.e. "the SIM's own centre".
    let center = ctx.query(CSCA_QUERY).ok().and_then(|text| parse_csca(&text));
    let pdus = pdu::encode(&destination, text, center.as_deref());
    let parts: Vec<SmsPart> = pdus
        .iter()
        .map(|p| SmsPart {
            length: p.length as usize,
            hex: p.hex.clone(),
        })
        .collect();
    ctx.channel.send_sms_pdu(&parts)?;
    Ok(parts.len())
}

/// `+CMGD=<index>` — delete one stored message.
pub fn delete(ctx: &RefreshCtx, index: i64) -> Result<(), BackendError> {
    if index < 0 {
        return Err(BackendError::InvalidParameter(format!(
            "短信索引无效: {}",
            index
        )));
    }
    ctx.action(&commands::cmgd(index))?;
    Ok(())
}

/// Empty every storage plane the modem reports.
///
/// This is the settings page's "清空所有短信": read `+CPMS?`, then for each
/// distinct plane switch the selected storage to it (`+CPMS="X","X","X"`) and
/// delete everything (`+CMGD=1,4`), settling between the steps because the
/// firmware ignores a delete that arrives while the storage switch is still in
/// flight. Returns the planes that were cleared.
pub fn clear_all(ctx: &RefreshCtx) -> Result<Vec<String>, BackendError> {
    let storage = read_storage(ctx)?;
    let names = storage.names();
    if names.is_empty() {
        return Err(BackendError::AtRejected(
            "+CPMS? 未返回存储配置".to_string(),
        ));
    }
    for name in &names {
        std::thread::sleep(STORAGE_SETTLE);
        ctx.action(&commands::cpms(name, name, name))?;
        std::thread::sleep(STORAGE_SETTLE);
        ctx.action(CMGD_ALL)?;
        std::thread::sleep(DELETE_SETTLE);
    }
    Ok(names)
}

/// `+CPMS="<read>","<write>","<receive>"` — pick where messages are stored.
///
/// The names come from the settings page's two radio options, so they are
/// validated against the storage list the module knows; a wrong name would
/// otherwise be echoed back by the modem as a bare `ERROR`.
pub fn set_storage(
    ctx: &RefreshCtx,
    read: &str,
    write: &str,
    receive: &str,
) -> Result<(), BackendError> {
    for name in [read, write, receive] {
        if !commands::valid_storage(name) {
            return Err(BackendError::InvalidParameter(format!(
                "存储位置只能是 {:?}，收到 {}",
                STORAGES.join("/"),
                name
            )));
        }
    }
    ctx.action(&commands::cpms(read, write, receive))?;
    Ok(())
}

/// `+CSCA="<number>"` — set the service centre the modem uses when the PDU
/// carries no SMSC prefix.
pub fn set_center(ctx: &RefreshCtx, number: &str) -> Result<(), BackendError> {
    let number = number.trim();
    if number.is_empty() {
        return Err(BackendError::InvalidParameter("短信中心号码不能为空".into()));
    }
    if !number
        .chars()
        .all(|c| c.is_ascii_digit() || c == '+' || c == ' ' || c == '-')
    {
        return Err(BackendError::InvalidParameter(format!(
            "短信中心号码含有非法字符: {}",
            number
        )));
    }
    ctx.action(&commands::csca_set(number))?;
    Ok(())
}

/// Enable/disable SMS over IMS: the module's five-step sequence, in order,
/// each step settled (`AT+CFUN=0` → IMS PDP profile → `AT+CEUS` →
/// `^IMSSWITCH` → `AT+CFUN=1`).
///
/// Synchronous on purpose: the pages show one progress line per step while
/// they wait, and the whole sequence is ~7 s on the MT5700M.
pub fn set_ims(ctx: &RefreshCtx, enable: bool) -> Result<(), BackendError> {
    for step in commands::ims_sequence(enable) {
        ctx.action(&step.command)?;
        if step.delay_ms > 0 {
            std::thread::sleep(Duration::from_millis(step.delay_ms));
        }
    }
    Ok(())
}

/// The compose hint: `{encoding, chars, parts}` for a not-yet-sent text.
///
/// Pure arithmetic in the PDU codec — no AT traffic — but it must be *the*
/// codec's arithmetic, otherwise the hint would promise a different part count
/// than the send produces.
pub fn analyze(text: &str) -> Value {
    let stats = pdu::message_stats(text);
    let mut m = std::collections::BTreeMap::new();
    m.insert("encoding".to_string(), json::str_val(stats.encoding));
    m.insert("chars".to_string(), json::num_val(stats.chars as i64));
    m.insert("parts".to_string(), json::num_val(stats.parts as i64));
    Value::Obj(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destinations_are_promoted_the_way_the_page_did() {
        assert_eq!(
            normalize_destination("13800138000").as_deref(),
            Some("+8613800138000")
        );
        assert_eq!(
            normalize_destination("+86 138 0013 8000").as_deref(),
            Some("+8613800138000")
        );
        assert_eq!(normalize_destination("+123456").as_deref(), Some("+123456"));
        // Too short / nothing usable.
        assert_eq!(normalize_destination("12"), None);
        assert_eq!(normalize_destination("abc"), None);
        assert_eq!(normalize_destination(""), None);
    }

    #[test]
    fn analyze_matches_the_encoder() {
        let Value::Obj(m) = analyze("hello") else {
            panic!("object")
        };
        assert_eq!(m.get("encoding").and_then(|v| v.as_str()), Some("7bit"));
        assert_eq!(m.get("chars").and_then(|v| v.as_i64()), Some(5));
        assert_eq!(m.get("parts").and_then(|v| v.as_i64()), Some(1));
        let Value::Obj(m) = analyze(&"你".repeat(71)) else {
            panic!("object")
        };
        assert_eq!(m.get("encoding").and_then(|v| v.as_str()), Some("UCS2"));
        assert_eq!(m.get("chars").and_then(|v| v.as_i64()), Some(71));
        assert_eq!(m.get("parts").and_then(|v| v.as_i64()), Some(2));
        let Value::Obj(m) = analyze("") else {
            panic!("object")
        };
        assert_eq!(m.get("parts").and_then(|v| v.as_i64()), Some(0));
    }
}
