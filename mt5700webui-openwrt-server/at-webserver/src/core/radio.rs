//! Radio tables: plain 3GPP data, no state and no AT.
//!
//! The ARFCN→band table is needed by more than one module — `modules::cell`
//! decodes `^MONNC`/`^HFREQINFO` neighbours and serving cells with it, and
//! `modules::beam` labels the neighbours of an `^NRSSBID` report with it — and
//! it was previously copied into the WebUI's `modem/parse.ts` and again into the
//! LuCI parser. It is knowledge, not module behaviour (no cache, no transport,
//! no policy), so it lives here once and both modules read the same table.

/// Band number for a downlink ARFCN, per the 3GPP band tables the MT5700M
/// reports (`LTE` E-UTRA bands, `NR` FR1/FR2 ranges).
///
/// `None` means the ARFCN falls outside every listed band — the UI shows `—`
/// rather than guessing.
pub fn arfcn_to_band(rat: &str, arfcn: i64) -> Option<i64> {
    const LTE: [(i64, i64, i64); 9] = [
        (0, 599, 1),
        (1200, 1949, 3),
        (2400, 2649, 5),
        (3450, 3799, 8),
        (36200, 36349, 34),
        (37750, 38249, 38),
        (38250, 38649, 39),
        (38650, 39649, 40),
        (39650, 41589, 41),
    ];
    const NR: [(i64, i64, i64); 9] = [
        (422000, 434000, 1),
        (361000, 376000, 3),
        (173800, 178800, 5),
        (185000, 192000, 8),
        (151600, 160600, 28),
        (499200, 537999, 41),
        (620000, 653333, 78),
        (653334, 680000, 77),
        (693334, 733333, 79),
    ];
    let table: &[(i64, i64, i64)] = match rat {
        "LTE" => &LTE,
        "NR" => &NR,
        _ => return None,
    };
    table
        .iter()
        .find(|(lo, hi, _)| arfcn >= *lo && arfcn <= *hi)
        .map(|(_, _, band)| *band)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arfcn_band_table_matches_manual_ranges() {
        assert_eq!(arfcn_to_band("LTE", 1850), Some(3));
        assert_eq!(arfcn_to_band("LTE", 100), Some(1));
        assert_eq!(arfcn_to_band("LTE", 41589), Some(41));
        assert_eq!(arfcn_to_band("LTE", 999999), None);
        assert_eq!(arfcn_to_band("NR", 643456), Some(78));
        assert_eq!(arfcn_to_band("NR", 653334), Some(77));
        assert_eq!(arfcn_to_band("NR", 504990), Some(41));
        assert_eq!(arfcn_to_band("NR", 1), None);
        assert_eq!(arfcn_to_band("WCDMA", 100), None);
    }
}
