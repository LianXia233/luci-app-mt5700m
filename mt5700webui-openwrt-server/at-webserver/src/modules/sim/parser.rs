//! `+CPIN?` / `^SIMSQ?` / `^SCICHG?` / `^TDSIMHP?` / `+CLCK` / `+CME ERROR`
//! decoders.
//!
//! Everything the frontends used to regex out of a raw reply — including the
//! CME-error classification (CMEE=1 numbers *and* CMEE=2 descriptions) — is
//! decoded here, once.

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

/// `^SIMSQ: <n>,<status>` -> `<status>` (manual 6.6.3).
pub fn parse_simsq(raw: &str) -> Option<i64> {
    let body = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("^SIMSQ:"))?
        .trim();
    body.split(',').nth(1)?.trim().parse::<i64>().ok()
}

/// `^SCICHG: <slot>,<other>` -> the active slot.
pub fn parse_scichg(raw: &str) -> Option<i64> {
    let body = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("^SCICHG:"))?
        .trim();
    body.split(',').next()?.trim().parse::<i64>().ok()
}

/// `^TDSIMHP: <n>` -> whether hot-plug detection is on.
pub fn parse_tdsimhp(raw: &str) -> Option<bool> {
    let body = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("^TDSIMHP:"))?
        .trim();
    body.split(',').next()?.trim().parse::<i64>().ok().map(|n| n == 1)
}

/// `+CLCK: <status>` -> whether the lock is enabled.
pub fn parse_clck(raw: &str) -> Option<bool> {
    let body = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("+CLCK:"))?
        .trim();
    body.split(',').next()?.trim().parse::<i64>().ok().map(|n| n == 1)
}

/// CMEE=2 descriptions that carry the same meaning as a CMEE=1 number.
const CME_TEXTS: [(&str, i64); 10] = [
    ("operation not allowed", 3),
    ("sim not inserted", 10),
    ("sim pin required", 11),
    ("sim puk required", 12),
    ("sim failure", 13),
    ("sim busy", 14),
    ("sim wrong", 15),
    ("incorrect password", 16),
    ("sim pin2 required", 17),
    ("sim puk2 required", 18),
];

/// CME error number from a reply: `+CME ERROR: 10` or its CMEE=2 description.
pub fn parse_cme_error(raw: &str) -> Option<i64> {
    for line in raw.lines() {
        let Some(rest) = line.trim().strip_prefix("+CME ERROR:") else {
            continue;
        };
        let body = rest.trim().lines().next().unwrap_or("").trim();
        if let Ok(n) = body.parse::<i64>() {
            return Some(n);
        }
        let lowered = body.to_ascii_lowercase();
        for (text, code) in CME_TEXTS.iter() {
            if *text == lowered.as_str() {
                return Some(*code);
            }
        }
    }
    None
}

/// The `+CPIN?` state, accepting the CME errors that mean the same thing.
///
/// A card that is not inserted answers `+CME ERROR: 10` rather than a
/// `+CPIN:` line (manual 20.2), and some firmware answers 11/12/17/18 when
/// queried, so those are classified here instead of being read as "query
/// failed". `ABSENT` is the module's own code for "no card in the slot".
pub fn parse_pin_code(raw: &str) -> Option<String> {
    if let Some(code) = parse_cpin(raw) {
        return Some(code.trim().to_uppercase());
    }
    match parse_cme_error(raw)? {
        10 => Some("ABSENT".to_string()),
        11 => Some("SIM PIN".to_string()),
        12 => Some("SIM PUK".to_string()),
        17 => Some("SIM PIN2".to_string()),
        18 => Some("SIM PUK2".to_string()),
        _ => None,
    }
}

/// Lock class of a `+CPIN`/`ABSENT` code — the semantics the UI branches on.
pub fn lock_of(code: &str) -> &'static str {
    match code.trim().to_uppercase().as_str() {
        "READY" => "ready",
        "ABSENT" => "absent",
        "SIM PIN" => "pin",
        "SIM PUK" => "puk",
        "SIM PIN2" => "pin2",
        "SIM PUK2" => "puk2",
        other if other.starts_with("PH-") => "network",
        _ => "unknown",
    }
}

/// Whether the card needs a password before it can be used.
pub fn is_blocked(lock: &str) -> bool {
    matches!(lock, "pin" | "puk" | "pin2" | "puk2" | "network")
}

/// Whether the card is PUK-locked (the UI asks for PUK *and* a new PIN).
pub fn needs_new_pin(lock: &str) -> bool {
    matches!(lock, "puk" | "puk2")
}

/// `^SIMSQ` status -> (dead, present).
///
/// 98 means the card is permanently unusable (PUK attempts used up or damaged),
/// 0/99 mean it is not in the slot; everything else is a card that is there.
pub fn simsq_flags(status: i64) -> (bool, bool) {
    (status == 98, status != 0 && status != 99)
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

    #[test]
    fn slot_and_hotplug_lines() {
        assert_eq!(parse_scichg("^SCICHG: 1,0\nOK"), Some(1));
        assert_eq!(parse_scichg("^SCICHG: 0,1\nOK"), Some(0));
        assert_eq!(parse_scichg("OK"), None);
        assert_eq!(parse_tdsimhp("^TDSIMHP: 1\nOK"), Some(true));
        assert_eq!(parse_tdsimhp("^TDSIMHP: 0\nOK"), Some(false));
        assert_eq!(parse_tdsimhp("+CME ERROR: 3"), None);
    }

    #[test]
    fn simsq_status_extraction_and_flags() {
        assert_eq!(parse_simsq("^SIMSQ: 1,11\nOK"), Some(11));
        assert_eq!(parse_simsq("^SIMSQ: 1,2\nOK"), Some(2));
        assert_eq!(parse_simsq("OK"), None);
        assert_eq!(simsq_flags(11), (false, true));
        assert_eq!(simsq_flags(2), (false, true));
        assert_eq!(simsq_flags(98), (true, true));
        assert_eq!(simsq_flags(0), (false, false));
        assert_eq!(simsq_flags(99), (false, false));
    }

    #[test]
    fn clck_status() {
        assert_eq!(parse_clck("+CLCK: 1\nOK"), Some(true));
        assert_eq!(parse_clck("+CLCK: 0\nOK"), Some(false));
        assert_eq!(parse_clck("ERROR"), None);
    }

    #[test]
    fn cme_error_numbers_and_descriptions() {
        assert_eq!(parse_cme_error("+CME ERROR: 16"), Some(16));
        assert_eq!(parse_cme_error("+CME ERROR: incorrect password"), Some(16));
        assert_eq!(parse_cme_error("+CME ERROR: SIM not inserted"), Some(10));
        assert_eq!(parse_cme_error("ERROR"), None);
        assert_eq!(parse_cme_error("+CMS ERROR: 500"), None);
    }

    #[test]
    fn pin_code_covers_the_error_branch() {
        assert_eq!(parse_pin_code("+CPIN: Ready\nOK").as_deref(), Some("READY"));
        assert_eq!(parse_pin_code("+CME ERROR: 10").as_deref(), Some("ABSENT"));
        assert_eq!(parse_pin_code("+CME ERROR: sim pin required").as_deref(), Some("SIM PIN"));
        assert_eq!(parse_pin_code("+CME ERROR: 18").as_deref(), Some("SIM PUK2"));
        assert_eq!(parse_pin_code("+CME ERROR: 3"), None);
    }

    #[test]
    fn lock_classification_matches_the_ui_branches() {
        assert_eq!(lock_of("READY"), "ready");
        assert_eq!(lock_of("absent"), "absent");
        assert_eq!(lock_of("sim pin"), "pin");
        assert_eq!(lock_of("SIM PUK"), "puk");
        assert_eq!(lock_of("SIM PIN2"), "pin2");
        assert_eq!(lock_of("SIM PUK2"), "puk2");
        assert_eq!(lock_of("PH-NET PIN"), "network");
        assert_eq!(lock_of("PH-NETSUB PUK"), "network");
        assert_eq!(lock_of("something else"), "unknown");
        assert!(!is_blocked("ready") && !is_blocked("absent") && !is_blocked("unknown"));
        assert!(is_blocked("pin") && is_blocked("puk") && is_blocked("network"));
        assert!(needs_new_pin("puk") && needs_new_pin("puk2"));
        assert!(!needs_new_pin("pin") && !needs_new_pin("pin2"));
    }
}
