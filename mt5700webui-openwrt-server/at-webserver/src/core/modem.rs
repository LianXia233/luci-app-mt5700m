//! How this process reaches the modem — a process-lifetime fact.
//!
//! The transport layer records it once when the modem client is constructed:
//! `serial` means the daemon owns a tty, `network` means it talks to a modem's
//! TCP AT server. The API reports it so a page can stop probing the modem with
//! a raw `AT+CONNECT?` just to learn how it is attached.
//!
//! This is not a cache: it is fixed at startup and never changes, so it lives in
//! a `OnceLock` instead of the state cache (which is TTL-driven and would expire
//! it out from under the route).

use std::sync::OnceLock;

static LINK: OnceLock<&'static str> = OnceLock::new();

/// Map a daemon `connection_type` (`SERIAL` / `NETWORK`) to the API vocabulary.
pub fn link_of(connection_type: &str) -> &'static str {
    if connection_type.eq_ignore_ascii_case("NETWORK") {
        "network"
    } else {
        "serial"
    }
}

/// Record the link kind. The first call wins — the transport is fixed at
/// startup, and the daemon builds exactly one modem client.
pub fn record(connection_type: &str) {
    let _ = LINK.set(link_of(connection_type));
}

/// The recorded link kind (`serial` when nothing was recorded, e.g. in tests).
pub fn link() -> &'static str {
    LINK.get().copied().unwrap_or("serial")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_names_are_stable() {
        assert_eq!(link_of("NETWORK"), "network");
        assert_eq!(link_of("network"), "network");
        assert_eq!(link_of("SERIAL"), "serial");
        assert_eq!(link_of("serial"), "serial");
        assert_eq!(link_of(""), "serial");
    }
}
