//! AT commands owned by the network module.
//!
//! Read commands are tried in order: the first one that answers with a
//! `REG:` line wins. `C5GREG` carries the 5G fields (tac/ci/AcT/NSSAI), so it
//! is preferred; `CEREG` and `CREG` are the fallbacks for 4G/2G-only firmware.

/// Registration queries, most specific first.
pub const REG_QUERIES: [&str; 3] = ["AT+C5GREG?", "AT+CEREG?", "AT+CREG?"];

/// Operator name + access technology (`+COPS: <mode>,<format>,"<name>",<act>`).
pub const COPS: &str = "AT+COPS?";

/// Activated PDP context addresses (`+CGPADDR: <cid>,"<address>"`). On-demand
/// only: the diagnostics panel asks for it, nothing polls it.
pub const CGPADDR: &str = "AT+CGPADDR";

/// Detailed system mode (vendor command).
pub const SYSINFOEX: &str = "AT^SYSINFOEX";

/// IPv4 parameters of the data call as the firmware's DHCP client sees them.
/// Hex-encoded little-endian bytes, six fields (address, mask, gateway, DHCP
/// server, primary/secondary DNS).
pub const DHCP_V4: &str = "AT^DHCP?";

/// Same six fields for IPv6, already in colon form.
pub const DHCP_V6: &str = "AT^DHCPV6?";

/// IPv6 capability code (`^IPV6CAP`: 1 = IPv4 only, 2 = IPv6 only,
/// 7 = dual stack sharing one APN, 11 = dual stack with separate APNs).
pub const IPV6CAP: &str = "AT^IPV6CAP?";

/// Ask the modem for detailed PS registration reports (`AT+CGREG=2`).
///
/// The WebUI has always issued this once per page load so the registration
/// URCs carry the location/cell fields. It is modem configuration, not a
/// frontend concern, so the network module owns the verb and the frontend
/// calls this route.
pub const CGREG_DETAILED: &str = "AT+CGREG=2";

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
