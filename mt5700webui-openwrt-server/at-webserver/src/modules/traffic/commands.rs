//! AT commands owned by the traffic module.

/// PDCP statistics. The query form appends two cumulative byte counters after
/// the 14 traffic fields.
pub const PDCP: &str = "AT^PDCPDATAINFO?";

/// Reset the modem's data-flow counters (`AT^DSFLOWCLR`).
///
/// A write: it clears the firmware's own accounting, so it is only ever issued
/// by an explicit user action through the API.
pub const DSFLOWCLR: &str = "AT^DSFLOWCLR";

/// `AT^PDCPDATAINFO=<0|1>[,<interval_ms>]` — the PDCP data-report URC switch.
///
/// The URC stream itself is already decoded and published on
/// `TOPIC_TRAFFIC` (`pdcp_data` events); this only drives the on/off switch,
/// so the WebUI no longer speaks raw AT. `interval` is the report period in
/// milliseconds; the WebUI's picker allows 200–65535 (default 500).
pub fn pdcp_report(enabled: bool, interval: Option<i64>) -> Option<String> {
    if !enabled {
        // The off form carries no interval: `AT^PDCPDATAINFO=0`.
        return Some("AT^PDCPDATAINFO=0".to_string());
    }
    match interval {
        Some(ms) if (200..=65535).contains(&ms) => {
            Some(format!("AT^PDCPDATAINFO=1,{ms}"))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::pdcp_report;

    #[test]
    fn pdcp_report_shape() {
        assert_eq!(pdcp_report(false, None).as_deref(), Some("AT^PDCPDATAINFO=0"));
        // Off ignores any interval.
        assert_eq!(pdcp_report(false, Some(500)).as_deref(), Some("AT^PDCPDATAINFO=0"));
        assert_eq!(pdcp_report(true, Some(500)).as_deref(), Some("AT^PDCPDATAINFO=1,500"));
        // Bounds from the WebUI picker (200–65535).
        assert_eq!(pdcp_report(true, Some(200)).as_deref(), Some("AT^PDCPDATAINFO=1,200"));
        assert_eq!(pdcp_report(true, Some(65535)).as_deref(), Some("AT^PDCPDATAINFO=1,65535"));
        assert_eq!(pdcp_report(true, Some(199)), None);
        assert_eq!(pdcp_report(true, Some(65536)), None);
        // On without an interval is not a valid report request.
        assert_eq!(pdcp_report(true, None), None);
    }
}
