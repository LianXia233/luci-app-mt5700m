//! SIM service: refresh policy for the `sim` topic plus the card operations
//! the system page performs (slot switch, hot-plug switch, PIN management).
//!
//! Five independent reads back the periodic snapshot (card status, ICCID,
//! IMSI, active slot, hot-plug switch), each with a 60 s failure backoff so a
//! missing card cannot make the collector hold the port. A card that answers
//! nothing leaves the topic empty rather than stale, which is what the UI
//! shows as "no SIM".
//!
//! The write sequences live here, not in a frontend: the slot switch brackets
//! `^SCICHG` with `^HVSST` and cycles the radio, and the PIN operations pick
//! their own command (CPIN / CLCK / CPWD) from the requested operation.

use crate::core::error::BackendError;
use crate::core::json::Value;
use crate::core::task::Priority;
use crate::modules::sim::commands::{self, CIMI, CPIN, ICCID};
use crate::modules::sim::parser;
use crate::modules::sim::state::{SimPinState, SimState};
use crate::scheduler::channel::run_in_task;
use crate::scheduler::jobs::TaskManager;
use crate::state::bus::{EVENT_SIM_UPDATED, TOPIC_SIM};
use crate::state::refresh::RefreshCtx;
use std::thread::sleep;
use std::time::Duration;

const AT_TIMEOUT: Duration = Duration::from_secs(6);
const QUEUED_TIMEOUT: Duration = Duration::from_secs(5);
const BACKOFF: Duration = Duration::from_secs(60);
const PERIOD: Duration = Duration::from_secs(60);
/// Pause between the radio-off and radio-on half of a slot switch.
const RADIO_CYCLE_PAUSE: Duration = Duration::from_millis(100);

/// Refresh the SIM topic.
pub fn refresh(ctx: &RefreshCtx) -> Result<SimState, BackendError> {
    let mut st = SimState::default();
    if let Some(text) = ctx.slow("sim_cpin", CPIN, AT_TIMEOUT, QUEUED_TIMEOUT, BACKOFF) {
        st.status = parser::parse_cpin(&text);
    }
    if let Some(text) = ctx.slow("sim_iccid", ICCID, AT_TIMEOUT, QUEUED_TIMEOUT, BACKOFF) {
        st.iccid = parser::parse_iccid(&text);
    }
    if let Some(text) = ctx.slow("sim_cimi", CIMI, AT_TIMEOUT, QUEUED_TIMEOUT, BACKOFF) {
        st.imsi = parser::parse_imsi(&text);
    }
    if let Some(text) = ctx.slow("sim_scichg", commands::SCICHG, AT_TIMEOUT, QUEUED_TIMEOUT, BACKOFF) {
        st.slot = parser::parse_scichg(&text);
    }
    if let Some(text) = ctx.slow("sim_tdsimhp", commands::TDSIMHP, AT_TIMEOUT, QUEUED_TIMEOUT, BACKOFF) {
        st.hotplug = parser::parse_tdsimhp(&text);
    }
    // Keep values read earlier that this tick could not refresh.
    if let Some(prev) = cached(ctx.cache) {
        if st.number.is_none() {
            st.number = prev.number;
        }
        if st.slot.is_none() {
            st.slot = prev.slot;
        }
        if st.hotplug.is_none() {
            st.hotplug = prev.hotplug;
        }
    }
    ctx.store(TOPIC_SIM, EVENT_SIM_UPDATED, &st.to_json());
    Ok(st)
}

/// Cache-first read for the API.
pub fn cached(cache: &std::sync::Arc<crate::state::cache::StateCache>) -> Option<SimState> {
    match cache.get(TOPIC_SIM) {
        (Some(Value::Obj(m)), _) => Some(SimState::from_json(&m)),
        _ => None,
    }
}

/// Read the PIN/card answer the system page and the PIN dialog render.
///
/// `+CPIN?` decides everything: a card that is not inserted answers a CME
/// error instead of a `+CPIN:` line, and an answer that cannot be classified
/// is a failure (the page then keeps its previous state). `^SIMSQ?` refines
/// "locked" into "locked" / "dead", and `+CLCK="SC",2` is only asked when the
/// card is ready — asking it earlier is an error on the modem side.
pub fn read_pin(ctx: &RefreshCtx) -> Result<SimPinState, BackendError> {
    let mut st = SimPinState::default();
    let text = match ctx.query(CPIN) {
        Ok(text) => text,
        // A rejected `+CPIN?` still carries the state: that is how "no card"
        // and "PIN required" are reported.
        Err(BackendError::AtRejected(text)) => text,
        Err(e) => return Err(e),
    };
    let Some(code) = parser::parse_pin_code(&text) else {
        // An empty reply is a timeout (the channel softens it); anything else
        // is a shape this decoder does not know.
        return Err(if text.trim().is_empty() {
            BackendError::AtTimeout
        } else {
            BackendError::AtRejected(format!("+CPIN? answered an unknown state: {}", text.trim()))
        });
    };
    let lock = parser::lock_of(&code);
    st.code = Some(code.clone());
    st.lock = Some(lock.to_string());
    st.blocked = parser::is_blocked(lock);
    st.needs_new_pin = parser::needs_new_pin(lock);

    if let Ok(text) = ctx.query(commands::SIMSQ) {
        if let Some(status) = parser::parse_simsq(&text) {
            let (dead, present) = parser::simsq_flags(status);
            st.card_status = Some(status);
            st.dead = dead;
            st.present = present;
        }
    }

    if lock == "ready" {
        let fac = commands::facility(false);
        if let Ok(text) = ctx.query(&commands::clck_query(fac)) {
            st.pin_enabled = parser::parse_clck(&text);
        }
    }
    Ok(st)
}

/// Switch the active SIM slot.
///
/// The vendor sequence the frontend used to send step by step: deactivate the
/// card interface, move the slot, re-activate, then cycle the radio so the
/// modem re-reads the new card. Invalidate the snapshot afterwards so the next
/// read shows the new slot.
pub fn switch_slot(ctx: &RefreshCtx, target: i64) -> Result<(), BackendError> {
    if !commands::valid_slot(target) {
        return Err(BackendError::InvalidParameter(format!(
            "SIM 卡槽只能是 0 或 1，收到 {}",
            target
        )));
    }
    ctx.action(&commands::hvsst_power(false))?;
    ctx.action(&commands::scichg_slot(target, 1 - target))?;
    ctx.action(&commands::hvsst_power(true))?;
    ctx.action(commands::CFUN_OFF)?;
    sleep(RADIO_CYCLE_PAUSE);
    ctx.action(commands::CFUN_ON)?;
    ctx.cache.invalidate(TOPIC_SIM);
    Ok(())
}

/// Read the SIM power path (`^HVSST?`).
///
/// A live read. An unparsable answer is an error here; the `sim.activation`
/// route turns that into an absent field so the page keeps the switch where the
/// user left it.
pub fn read_activation(ctx: &RefreshCtx) -> Result<(bool, Option<i64>), BackendError> {
    let text = ctx.query(commands::HVSST_QUERY)?;
    parser::parse_hvsst(&text).ok_or_else(|| {
        BackendError::AtRejected(format!(
            "^HVSST? answered an unknown shape: {}",
            text.trim()
        ))
    })
}

/// Switch the SIM power path off or on (`^HVSST=1,<0|1>`).
///
/// Deactivating removes mobile service immediately, which is what the page's
/// confirmation says; the snapshot is dropped so the next read shows reality.
pub fn set_activation(ctx: &RefreshCtx, active: bool) -> Result<(), BackendError> {
    ctx.action(&commands::hvsst_power(active))?;
    ctx.cache.invalidate(TOPIC_SIM);
    Ok(())
}

/// Enable/disable hot-plug detection (`^TDSIMHP`).
pub fn set_hotplug(ctx: &RefreshCtx, on: bool) -> Result<(), BackendError> {
    ctx.action(&commands::tdsimhp(on))?;
    ctx.cache.invalidate(TOPIC_SIM);
    Ok(())
}

/// Validate and run one PIN operation.
///
/// Manual 6.3.3/5.7.3: PIN/PUK are 4–8 digits. The rules and their wording
/// live here so the dialog does not have to know what a PUK is, and the
/// frontend shows the message the backend rejected it with.
pub fn apply_pin(
    ctx: &RefreshCtx,
    operation: &str,
    pin: &str,
    new_pin: Option<&str>,
    pin2: bool,
) -> Result<(), BackendError> {
    let pin = pin.trim();
    let new_pin = new_pin.unwrap_or("").trim();
    let fac = commands::facility(pin2);
    let command = match operation {
        "verify" => {
            check_digits(pin, "PIN 码")?;
            commands::cpin(pin)
        }
        "unblock" => {
            check_digits(pin, "PUK 码")?;
            check_digits(new_pin, "新 PIN 码")?;
            commands::cpin_unblock(pin, new_pin)
        }
        "enable" | "disable" => {
            check_digits(pin, "PIN 码")?;
            commands::clck_set(fac, operation == "enable", pin)
        }
        "change" => {
            check_digits(pin, "PIN 码")?;
            check_digits(new_pin, "新 PIN 码")?;
            if new_pin == pin {
                return Err(BackendError::InvalidParameter(
                    "新 PIN 码不能与原 PIN 码相同".to_string(),
                ));
            }
            commands::cpwd(fac, pin, new_pin)
        }
        other => {
            return Err(BackendError::InvalidParameter(format!(
                "未知的 PIN 操作: {}",
                other
            )))
        }
    };
    ctx.action(&command)?;
    ctx.cache.invalidate(TOPIC_SIM);
    Ok(())
}

/// 4–8 digits, as the manual requires for PIN and PUK.
fn check_digits(value: &str, label: &str) -> Result<(), BackendError> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(BackendError::InvalidParameter(format!(
            "{}只能是数字",
            label
        )));
    }
    if !(4..=8usize).contains(&value.len()) {
        return Err(BackendError::InvalidParameter(format!(
            "{}长度必须为 4-8 位",
            label
        )));
    }
    Ok(())
}

/// Register the periodic refresh job.
pub fn spawn(tasks: &TaskManager) {
    tasks.add_periodic(
        "sim.refresh",
        PERIOD,
        Priority::Background,
        Some(Duration::from_secs(15)),
        Box::new(|ctx| run_in_task(ctx, |r| refresh(r).map(|st| st.to_json()))),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_validation_rejects_non_digits_and_short_values() {
        let short = check_digits("12", "PIN 码").unwrap_err();
        assert!(short.message().contains("4-8"));
        let text = check_digits("12ab", "PIN 码").unwrap_err();
        assert!(text.message().contains("数字"));
        assert!(check_digits("1234", "PIN 码").is_ok());
        assert!(check_digits("12345678", "PUK 码").is_ok());
        assert!(check_digits("123456789", "PIN 码").is_err());
        assert!(check_digits("", "PIN 码").is_err());
    }
}
