//! `+CGACT?` / `^DSAMBR` / `+CGEQOSRDP` decoders.

/// True when a field looks like a number the modem meant to send: digits,
/// optionally with decimal parts — never empty pieces (`1..2`, `.5`, `abc`).
///
/// The historical CLI and the WebUI disagreed here (`parseInt` accepts `2abc`),
/// which is exactly why the check lives in one place now.
pub fn is_number(v: &str) -> bool {
    let v = v.trim().trim_matches('"');
    !v.is_empty()
        && v.split('.')
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

/// Strip the whitespace/CR/quotes the modem pads fields with.
fn clean(v: &str) -> String {
    v.chars()
        .filter(|c| !matches!(c, ' ' | '\r' | '\n' | '"'))
        .collect()
}

/// Lowest PDP context with `state == 1` in a `+CGACT?` answer.
///
/// The WebUI used the lowest activated context as "the" context when asking for
/// AMBR/QCI; keeping that rule here means all three questions use the same cid.
pub fn parse_cgact_active_cid(raw: &str) -> Option<u32> {
    let mut lowest: Option<u32> = None;
    for line in raw.lines() {
        let Some(rest) = line.trim().strip_prefix("+CGACT:") else {
            continue;
        };
        let fields: Vec<&str> = rest.split(',').collect();
        if fields.len() < 2 {
            continue;
        }
        let Ok(cid) = clean(fields[0]).parse::<u32>() else {
            continue;
        };
        if cid == 0 || clean(fields[1]) != "1" {
            continue;
        }
        lowest = Some(match lowest {
            Some(current) => current.min(cid),
            None => cid,
        });
    }
    lowest
}

/// Subscribed rate + APN from an `^DSAMBR` answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Ambr {
    /// kbps, exactly as the modem reports them (the UI divides by 1000).
    pub down_kbps: f64,
    pub up_kbps: f64,
    /// APN the modem echoed, when the firmware reports it.
    pub apn: Option<String>,
}

/// First `^DSAMBR:` line with two numeric fields wins; anything else is "no
/// answer" (`None`), not a zero reading.
pub fn parse_dsambr(raw: &str) -> Option<Ambr> {
    for line in raw.lines() {
        let Some(rest) = line.trim().strip_prefix("^DSAMBR:") else {
            continue;
        };
        let fields: Vec<&str> = rest.split(',').map(|f| f.trim().trim_end_matches('\r')).collect();
        if fields.len() < 3 {
            continue;
        }
        let (down, up) = (clean(fields[1]), clean(fields[2]));
        if !is_number(&down) || !is_number(&up) {
            return None;
        }
        let apn = fields.get(3).map(|f| f.trim().trim_matches('"').trim().to_string());
        return Some(Ambr {
            down_kbps: down.parse().unwrap_or(0.0),
            up_kbps: up.parse().unwrap_or(0.0),
            apn: apn.filter(|a| !a.is_empty()),
        });
    }
    None
}

/// QoS class of the requested context, or of the first reported bearer when the
/// context is unknown.
pub fn parse_cgeqosrdp(raw: &str, cid: Option<u32>) -> Option<String> {
    let mut first: Option<String> = None;
    for line in raw.lines() {
        let Some(rest) = line.trim().strip_prefix("+CGEQOSRDP:") else {
            continue;
        };
        let fields: Vec<&str> = rest.split(',').collect();
        if fields.len() < 2 {
            continue;
        }
        let qci = clean(fields[1]);
        if qci.is_empty() {
            continue;
        }
        let row_cid = clean(fields[0]).parse::<u32>().ok();
        if cid.is_some() && row_cid == cid {
            return Some(qci);
        }
        if first.is_none() {
            first = Some(qci);
        }
    }
    first
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cgact_picks_lowest_active_context() {
        let raw = "+CGACT: 1,1\r\n+CGACT: 2,0\r\n+CGACT: 3,1\r\nOK";
        assert_eq!(parse_cgact_active_cid(raw), Some(1));
        assert_eq!(parse_cgact_active_cid("+CGACT: 2,0\r\nOK"), None);
        // A context numbered 0 is the modem's "no context" marker.
        assert_eq!(parse_cgact_active_cid("+CGACT: 0,1\r\nOK"), None);
    }

    #[test]
    fn dsambr_parses_kbps_and_apn() {
        let a = parse_dsambr("^DSAMBR: 1,20000,10000,\"cmnet\"\r\nOK").unwrap();
        assert_eq!(a.down_kbps, 20000.0);
        assert_eq!(a.up_kbps, 10000.0);
        assert_eq!(a.apn.as_deref(), Some("cmnet"));
    }

    #[test]
    fn dsambr_rejects_malformed_numbers() {
        assert!(parse_dsambr("^DSAMBR: 1,2abc,10\r\nOK").is_none());
        assert!(parse_dsambr("^DSAMBR: 1,,\r\nOK").is_none());
        assert!(parse_dsambr("OK\r\n").is_none());
    }

    #[test]
    fn cgeqosrdp_prefers_the_requested_context() {
        let raw = "+CGEQOSRDP: 1,9,0,0,0,0\r\n+CGEQOSRDP: 2,6,0,0,0,0\r\nOK";
        assert_eq!(parse_cgeqosrdp(raw, Some(2)).as_deref(), Some("6"));
        assert_eq!(parse_cgeqosrdp(raw, Some(7)).as_deref(), Some("9"));
        assert_eq!(parse_cgeqosrdp("OK", None), None);
    }
}
