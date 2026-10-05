//! `+CPIN?` / `^ICCID?` / `+CIMI` / `+CNUM` decoders.

/// `+CPIN: READY` -> `READY`.
pub fn parse_cpin(raw: &str) -> Option<String> {
    raw.lines()
        .find_map(|l| l.trim().strip_prefix("+CPIN:"))
        .map(|rest| rest.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
}

/// `^ICCID: 8986...` -> the digits, quotes stripped.
pub fn parse_iccid(raw: &str) -> Option<String> {
    raw.lines()
        .find_map(|l| l.trim().strip_prefix("^ICCID:"))
        .map(|rest| rest.trim().trim_matches('"').to_string())
        .filter(|s| !s.is_empty())
}

/// `AT+CIMI` answers with the bare 15-digit IMSI (accept >= 10 digits).
pub fn parse_imsi(raw: &str) -> Option<String> {
    raw.lines()
        .map(|l| l.trim())
        .find(|t| t.len() >= 10 && t.bytes().all(|b| b.is_ascii_digit()))
        .map(|t| t.to_string())
}

/// `+CNUM: "","+8613800138000",145` -> the second quoted field.
pub fn parse_cnum(raw: &str) -> Option<String> {
    let line = raw.lines().find(|l| l.trim().starts_with("+CNUM:"))?;
    let body = line.trim().strip_prefix("+CNUM:")?.trim();
    let mut quoted = body.split('"').filter(|s| !s.trim().is_empty());
    let _name = quoted.next();
    quoted.next().map(|s| s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpin_and_iccid() {
        assert_eq!(parse_cpin("+CPIN: READY\nOK").as_deref(), Some("READY"));
        assert_eq!(parse_cpin("+CPIN: SIM PIN").as_deref(), Some("SIM PIN"));
        assert_eq!(parse_cpin("ERROR"), None);
        assert_eq!(
            parse_iccid("^ICCID: \"89860012345678901234\"").as_deref(),
            Some("89860012345678901234")
        );
    }

    #[test]
    fn imsi_requires_digits() {
        assert_eq!(
            parse_imsi("460001234567890\nOK").as_deref(),
            Some("460001234567890")
        );
        assert_eq!(parse_imsi("+CME ERROR: 10"), None);
    }

    #[test]
    fn cnum_skips_empty_name() {
        assert_eq!(
            parse_cnum("+CNUM: \"\",\"+8613800138000\",145").as_deref(),
            Some("+8613800138000")
        );
        assert_eq!(parse_cnum("OK"), None);
    }
}
