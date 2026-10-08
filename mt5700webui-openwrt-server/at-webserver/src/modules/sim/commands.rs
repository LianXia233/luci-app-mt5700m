//! AT commands owned by the SIM module.
//!
//! Manual references (MT5700M AT manual):
//!   6.3  `AT+CPIN`  — password entry, PUK unblock
//!   6.6  `AT^SIMSQ` — card status (absent / locked / dead)
//!   5.6  `AT+CLCK`  — enable/disable/query the SIM PIN1 lock
//!   5.7  `AT+CPWD`  — change the PIN
//! `^SCICHG` / `^TDSIMHP` / `^HVSST` are the vendor slot-switch/hot-plug
//! commands; the whole slot-switch sequence lives here so no frontend has to
//! know its order.

/// SIM PIN/ready status (`+CPIN: READY`).
pub const CPIN: &str = "AT+CPIN?";
/// Card status, finer than `+CPIN?` (`^SIMSQ: <n>,<status>`).
pub const SIMSQ: &str = "AT^SIMSQ?";
/// Current SIM slot (`^SCICHG: <slot>,<other>`).
pub const SCICHG: &str = "AT^SCICHG?";
/// Hot-plug switch state (`^TDSIMHP: <n>`).
pub const TDSIMHP: &str = "AT^TDSIMHP?";
/// Integrated circuit card identifier (vendor command).
pub const ICCID: &str = "AT^ICCID?";
/// International mobile subscriber identity.
pub const CIMI: &str = "AT+CIMI";
/// Subscriber number stored on the SIM.
pub const CNUM: &str = "AT+CNUM";

/// SIM power path query (`^HVSST: <…>,<active>,<slot>`).
pub const HVSST_QUERY: &str = "AT^HVSST?";

/// `AT^HVSST=1,<0|1>` — switch the SIM power path off or on.
///
/// One constructor for every caller, including the two writes that bracket a
/// slot switch, so the verb cannot drift between the deactivate/reactivate
/// sequence and the page's own control.
pub fn hvsst_power(active: bool) -> String {
    format!("AT^HVSST=1,{}", if active { 1 } else { 0 })
}
/// Radio off/on, the tail of the slot-switch sequence.
pub const CFUN_OFF: &str = "AT+CFUN=0";
pub const CFUN_ON: &str = "AT+CFUN=1";

/// The two slots the hardware exposes (0 = external, 1 = internal).
pub const SLOT_COUNT: i64 = 2;

/// True when `slot` names a real slot.
pub fn valid_slot(slot: i64) -> bool {
    (0..SLOT_COUNT).contains(&slot)
}

/// `AT^SCICHG=<slot>,<other>` — the second field is the *inactive* slot.
pub fn scichg_slot(slot: i64, other: i64) -> String {
    format!("AT^SCICHG={},{}", slot, other)
}

/// `AT^TDSIMHP=<0|1>`.
pub fn tdsimhp(on: bool) -> String {
    format!("AT^TDSIMHP={}", if on { 1 } else { 0 })
}

/// `AT+CPIN="<pin>"` — enter the PIN.
pub fn cpin(pin: &str) -> String {
    format!("AT+CPIN=\"{}\"", pin)
}

/// `AT+CPIN="<puk>","<newpin>"` — PUK unblock (manual 6.3.2).
pub fn cpin_unblock(puk: &str, new_pin: &str) -> String {
    format!("AT+CPIN=\"{}\",\"{}\"", puk, new_pin)
}

/// `AT+CLCK="<fac>",2` — query the lock state.
pub fn clck_query(fac: &str) -> String {
    format!("AT+CLCK=\"{}\",2", fac)
}

/// `AT+CLCK="<fac>",<0|1>,"<pin>"` — disable/enable the PIN lock.
pub fn clck_set(fac: &str, enable: bool, pin: &str) -> String {
    format!(
        "AT+CLCK=\"{}\",{},\"{}\"",
        fac,
        if enable { 1 } else { 0 },
        pin
    )
}

/// `AT+CPWD="<fac>","<old>","<new>"` — change the PIN (manual 5.7).
pub fn cpwd(fac: &str, old: &str, new: &str) -> String {
    format!("AT+CPWD=\"{}\",\"{}\",\"{}\"", fac, old, new)
}

/// Facility selector: SIM PIN1 (`SC`) or PIN2 (`P2`), the only two supported.
pub fn facility(pin2: bool) -> &'static str {
    if pin2 {
        "P2"
    } else {
        "SC"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slot_switch_sequence_commands() {
        assert!(valid_slot(0) && valid_slot(1) && !valid_slot(2) && !valid_slot(-1));
        assert_eq!(scichg_slot(0, 1), "AT^SCICHG=0,1");
        assert_eq!(scichg_slot(1, 0), "AT^SCICHG=1,0");
        assert_eq!(tdsimhp(true), "AT^TDSIMHP=1");
        assert_eq!(tdsimhp(false), "AT^TDSIMHP=0");
    }

    #[test]
    fn pin_commands_quote_their_arguments() {
        assert_eq!(cpin("1234"), "AT+CPIN=\"1234\"");
        assert_eq!(cpin_unblock("12345678", "4321"), "AT+CPIN=\"12345678\",\"4321\"");
        assert_eq!(clck_query("SC"), "AT+CLCK=\"SC\",2");
        assert_eq!(clck_set("SC", true, "1234"), "AT+CLCK=\"SC\",1,\"1234\"");
        assert_eq!(clck_set("P2", false, "1234"), "AT+CLCK=\"P2\",0,\"1234\"");
        assert_eq!(cpwd("SC", "1234", "5678"), "AT+CPWD=\"SC\",\"1234\",\"5678\"");
        assert_eq!(facility(false), "SC");
        assert_eq!(facility(true), "P2");
    }

    #[test]
    fn hvsst_power_matches_the_vendor_spelling() {
        // The ^HVSST write the CLI has always sent: `AT^HVSST=1,<0|1>`. The two
        // halves of the slot-switch bracket are the same command, so they come
        // from the same constructor.
        assert_eq!(hvsst_power(false), "AT^HVSST=1,0");
        assert_eq!(hvsst_power(true), "AT^HVSST=1,1");
        assert_eq!(HVSST_QUERY, "AT^HVSST?");
    }
}
