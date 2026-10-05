//! AT commands owned by the QoS module.

/// Activated PDP contexts: `+CGACT: <cid>,<state>` per line.
pub const CGACT: &str = "AT+CGACT?";

/// Subscribed AMBR, all contexts (`^DSAMBR: <cid>,<down>,<up>,<apn>`).
pub fn dsambr(cid: u32) -> String {
    format!("AT^DSAMBR={}", cid)
}

/// QoS parameters of every bearer.
pub const CGEQOSRDP_ALL: &str = "AT+CGEQOSRDP";

/// QoS parameters of one bearer.
pub fn cgeqosrdp(cid: u32) -> String {
    format!("AT+CGEQOSRDP={}", cid)
}
