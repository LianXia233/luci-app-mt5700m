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

// ------------------------------------------------- NR capabilities
//
// `^NRRCCAPQRY=<kind>` is both a query and its argument (like `^MCS`), so the
// kinds are named here and the write of the same ability uses the matching
// `^NRRCCAPCFG` kind.

/// NR carrier aggregation.
pub const NRRCCAP_CA: i64 = 3;
/// VoNR (5G voice) mode, 0..=3.
pub const NRRCCAP_VONR: i64 = 2;
/// DSS (dynamic spectrum sharing): LTE-CRS rate matching, additional DMRS.
pub const NRRCCAP_DSS: i64 = 5;
/// Highest VoNR mode the modem exposes.
pub const VONR_MAX: i64 = 3;

/// `AT^NRRCCAPQRY=<kind>`.
pub fn nrrccapqry(kind: i64) -> String {
    format!("AT^NRRCCAPQRY={}", kind)
}

/// `AT^NRRCCAPCFG=<kind>,<values…>`.
pub fn nrrccapcfg(kind: i64, values: &[i64]) -> String {
    let csv = values
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(",");
    format!("AT^NRRCCAPCFG={},{}", kind, csv)
}


/// Modem restart (vendor command).
pub const RESET: &str = "AT^RESET";

/// Downlink MCS table of the serving carriers. A vendor *query* that takes the
/// direction as its argument, so the scheduler classifies it as a write.
pub const MCS_DL: &str = "AT^MCS=1";

/// Uplink MCS table, same command with the other direction.
pub const MCS_UL: &str = "AT^MCS=0";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nr_capability_forms() {
        assert_eq!(nrrccapqry(NRRCCAP_CA), "AT^NRRCCAPQRY=3");
        assert_eq!(nrrccapqry(NRRCCAP_VONR), "AT^NRRCCAPQRY=2");
        assert_eq!(nrrccapqry(NRRCCAP_DSS), "AT^NRRCCAPQRY=5");
        assert_eq!(nrrccapcfg(NRRCCAP_CA, &[1]), "AT^NRRCCAPCFG=3,1");
        assert_eq!(nrrccapcfg(NRRCCAP_VONR, &[0]), "AT^NRRCCAPCFG=2,0");
        assert_eq!(nrrccapcfg(NRRCCAP_DSS, &[1, 1]), "AT^NRRCCAPCFG=5,1,1");
    }
}
