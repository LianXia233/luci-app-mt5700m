//! AT commands owned by the system module: board-level switches and the
//! factory reset. (Modem identity/restart live in `modules/modem`.)

/// Chip temperature array (12 sensors, tenths of a degree). Fast command
/// (~0.1 s) whose *queue* time is the problem: it shares the port with slow
/// commands, so it is read with a generous timeout and no retry.
pub const CHIPTEMP: &str = "AT^CHIPTEMP?";

/// NIC speed setting, query (`^TDPCIELANCFG: <1|2>`) and write.
pub const TDPCIELANCFG_QUERY: &str = "AT^TDPCIELANCFG?";
/// PCIe controller / power management, query (`^TDPMCFG: <0|1>`) and write.
pub const TDPMCFG_QUERY: &str = "AT^TDPMCFG?";

/// The NIC rate values the page offers: 1 = RTL8111 (1G), 2 = RTL8125 (2.5G).
pub const NIC_RATES: [i64; 2] = [1, 2];

/// `AT^TDPCIELANCFG=<1|2>`.
pub fn tdpcpcielancfg(rate: i64) -> String {
    format!("AT^TDPCIELANCFG={}", rate)
}

/// `AT^TDPMCFG=<0|1>`.
///
/// The CLI's older form appends the three unused sub-fields
/// (`AT^TDPMCFG=1,0,0,0`); the modem accepts the short form, and this is the
/// one the page has always used.
pub fn tdpmcfg(on: bool) -> String {
    format!("AT^TDPMCFG={}", if on { 1 } else { 0 })
}

/// Set the AT configuration back to factory defaults (`AT&F`).
pub const FACTORY_RESET: &str = "AT&F";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_forms() {
        assert_eq!(tdpcpcielancfg(1), "AT^TDPCIELANCFG=1");
        assert_eq!(tdpcpcielancfg(2), "AT^TDPCIELANCFG=2");
        assert_eq!(tdpmcfg(true), "AT^TDPMCFG=1");
        assert_eq!(tdpmcfg(false), "AT^TDPMCFG=0");
        assert!(NIC_RATES.contains(&1) && NIC_RATES.contains(&2));
    }
}
