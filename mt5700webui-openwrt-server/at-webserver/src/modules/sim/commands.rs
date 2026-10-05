//! AT commands owned by the SIM module.

/// SIM PIN/ready status (`+CPIN: READY`).
pub const CPIN: &str = "AT+CPIN?";
/// Integrated circuit card identifier (vendor command).
pub const ICCID: &str = "AT^ICCID?";
/// International mobile subscriber identity.
pub const CIMI: &str = "AT+CIMI";
/// Subscriber number stored on the SIM.
pub const CNUM: &str = "AT+CNUM";

/// SIM PIN entry / disable (writes).
pub fn cpin(pin: &str) -> String {
    format!("AT+CPIN={}", pin)
}

/// PUK unblock with a new PIN.
pub fn cpuk(puk: &str, new_pin: &str) -> String {
    format!("AT+CPUK={},\"{}\"", puk, new_pin)
}
