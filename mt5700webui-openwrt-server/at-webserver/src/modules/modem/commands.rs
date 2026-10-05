//! AT commands owned by the modem module.

/// Identity block: `Manufacturer:` / `Model:` / `Revision:` lines.
pub const ATI: &str = "ATI";
/// IMEI.
pub const CGSN: &str = "AT+CGSN";
/// Firmware/version block used by the system page.
pub const CGMR: &str = "AT+CGMR";
/// Transmit power (GUL firmware only; NR answers with an error).
pub const TXPOWER: &str = "AT^TXPOWER?";
/// Per-carrier NR transmit power (~11.6 s response).
pub const NTXPOWER: &str = "AT^NTXPOWER?";
/// EN-DC status (5 field reply when queried, 4 when unsolicited).
pub const LENDC: &str = "AT^LENDC?";

/// Radio on/off (`0` = minimum functionality).
pub fn cfun(state: u8) -> String {
    format!("AT+CFUN={}", state)
}

/// IMEI write (vendor `^PHYNUM`; the CLI's `set-imei` verb uses the same form).
pub fn phynum_imei(imei: &str) -> String {
    format!("AT^PHYNUM=IMEI,{}", imei)
}

/// Modem restart (vendor command).
pub const RESET: &str = "AT^RESET";

/// Downlink MCS table of the serving carriers. A vendor *query* that takes the
/// direction as its argument, so the scheduler classifies it as a write.
pub const MCS_DL: &str = "AT^MCS=1";

/// Uplink MCS table, same command with the other direction.
pub const MCS_UL: &str = "AT^MCS=0";
