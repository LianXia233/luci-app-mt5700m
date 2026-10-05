//! AT commands owned by the network module.
//!
//! Read commands are tried in order: the first one that answers with a
//! `REG:` line wins. `C5GREG` carries the 5G fields (tac/ci/AcT/NSSAI), so it
//! is preferred; `CEREG` and `CREG` are the fallbacks for 4G/2G-only firmware.

/// Registration queries, most specific first.
pub const REG_QUERIES: [&str; 3] = ["AT+C5GREG?", "AT+CEREG?", "AT+CREG?"];

/// Operator name + access technology (`+COPS: <mode>,<format>,"<name>",<act>`).
pub const COPS: &str = "AT+COPS?";

/// Detailed system mode (vendor command).
pub const SYSINFOEX: &str = "AT^SYSINFOEX";

/// True when a response actually contains a registration line.
///
/// A failed response can still carry text without any `REG:` line; treating
/// that as an answer would record `state=0` and stop the fallback chain.
pub fn has_registration_line(text: &str) -> bool {
    text.lines().any(|l| l.trim().contains("REG:"))
}

/// AT command that selects `+CFUN` (radio on/off) when a caller needs to
/// re-register the modem.
pub fn cfun(state: u8) -> String {
    format!("AT+CFUN={}", state)
}

/// AT command that brings the data call up or down.
pub fn ndisdup(up: bool) -> String {
    format!("AT^NDISDUP=1,{}", if up { 1 } else { 0 })
}

/// AT command that sets the network preference mode.
pub fn set_mode(value: u8) -> String {
    format!("AT^SETMODE={}", value)
}
