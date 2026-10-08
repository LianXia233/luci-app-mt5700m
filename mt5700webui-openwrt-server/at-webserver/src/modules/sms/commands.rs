//! AT commands owned by the SMS module.
//!
//! References (MT5700M AT manual): `AT+CMGF` message format, `AT+CMGL` list,
//! `AT+CMGD` delete, `AT+CPMS` storage, `AT+CSCA` service centre,
//! `AT^IMSSWITCH` IMS switch. `AT+CGDCONT`/`AT+CEUS`/`AT+CFUN` appear only in the
//! IMS configuration sequence, which is the modem's own IMS profile setup.

/// Message format query (`+CMGF: 0` = PDU mode, 1 = text mode).
pub const CMGF_QUERY: &str = "AT+CMGF?";
/// PDU mode: what the module's own encoder produces and the pages consume.
pub const CMGF_PDU: &str = "AT+CMGF=0";
/// List all messages in PDU mode (index, status, PDU).
pub const CMGL_ALL: &str = "AT+CMGL=4";
/// Preferred message storage query.
pub const CPMS_QUERY: &str = "AT+CPMS?";
/// Service centre address query.
pub const CSCA_QUERY: &str = "AT+CSCA?";
/// IMS switch query (`^IMSSWITCH: <on>,<a>,<b>`).
pub const IMSSWITCH_QUERY: &str = "AT^IMSSWITCH?";

/// `AT+CMGD=<index>`.
pub fn cmgd(index: i64) -> String {
    format!("AT+CMGD={}", index)
}

/// Delete every message in the currently selected storage.
pub const CMGD_ALL: &str = "AT+CMGD=1,4";

/// `AT+CPMS="<read>","<write>","<receive>"`.
pub fn cpms(read: &str, write: &str, receive: &str) -> String {
    format!("AT+CPMS=\"{}\",\"{}\",\"{}\"", read, write, receive)
}

/// `AT+CSCA="<number>"`.
pub fn csca_set(number: &str) -> String {
    format!("AT+CSCA=\"{}\"", number)
}

/// `AT^IMSSWITCH=<on>,0,0` — the trailing two fields are the vendor's reserved
/// arguments and have always been sent as zeros.
pub fn imsswitch(on: bool) -> String {
    format!("AT^IMSSWITCH={},0,0", if on { 1 } else { 0 })
}

/// Storage names the modem exposes (SIM card, module).
pub const STORAGES: [&str; 2] = ["SM", "ME"];

/// True when `name` is a storage the modem has.
pub fn valid_storage(name: &str) -> bool {
    STORAGES.contains(&name)
}

// ------------------------------------------------------------ IMS sequence
//
// Enabling/disabling SMS over IMS is not one command: the modem needs the IMS
// PDP context configured and the radio off while it happens. These are the
// exact steps (and settle times) the pages used to run, kept in order here so
// there is one copy of the sequence.

/// One step of the IMS configuration sequence.
#[derive(Debug, Clone, PartialEq)]
pub struct ImsStep {
    pub command: String,
    /// Settle time after the command, in milliseconds.
    pub delay_ms: u64,
    /// Why this step runs — used by the CLI rendering and the logs.
    pub label: &'static str,
}

/// The IMS profile PDP context (`cid` 5) with or without the `ims` APN.
fn cgdcont_ims(apn: &str) -> String {
    format!(
        "AT+CGDCONT=5,\"IPV4V6\",\"{}\",\"\",0,0,0,0,1,1,1,,,,,,0,,0,0,0,0",
        apn
    )
}

/// The full enable/disable sequence, in order.
pub fn ims_sequence(enable: bool) -> Vec<ImsStep> {
    vec![
        ImsStep {
            command: "AT+CFUN=0".to_string(),
            delay_ms: 2000,
            label: "正在开启飞行模式...",
        },
        ImsStep {
            command: cgdcont_ims(if enable { "ims" } else { "" }),
            delay_ms: 0,
            label: if enable {
                "正在配置IMS参数..."
            } else {
                "正在清除IMS参数..."
            },
        },
        ImsStep {
            command: format!("AT+CEUS={}", if enable { 0 } else { 1 }),
            delay_ms: 1000,
            label: if enable {
                "正在设置EPS服务..."
            } else {
                "正在关闭EPS服务..."
            },
        },
        ImsStep {
            command: imsswitch(enable),
            delay_ms: 1000,
            label: if enable {
                "正在开启IMS功能..."
            } else {
                "正在关闭IMS功能..."
            },
        },
        ImsStep {
            command: "AT+CFUN=1".to_string(),
            delay_ms: 2000,
            label: "正在关闭飞行模式...",
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_writes_quote_their_arguments() {
        assert_eq!(cmgd(12), "AT+CMGD=12");
        assert_eq!(cmgd(-1), "AT+CMGD=-1");
        assert_eq!(
            cpms("ME", "ME", "SM"),
            "AT+CPMS=\"ME\",\"ME\",\"SM\""
        );
        assert_eq!(csca_set("+8613800138000"), "AT+CSCA=\"+8613800138000\"");
        assert_eq!(imsswitch(true), "AT^IMSSWITCH=1,0,0");
        assert_eq!(imsswitch(false), "AT^IMSSWITCH=0,0,0");
        assert!(valid_storage("SM") && valid_storage("ME"));
        assert!(!valid_storage("MT") && !valid_storage(""));
    }

    #[test]
    fn ims_sequence_is_five_steps_and_differs_by_direction() {
        let on = ims_sequence(true);
        let off = ims_sequence(false);
        assert_eq!(on.len(), 5);
        assert_eq!(on[0].command, "AT+CFUN=0");
        assert_eq!(on[4].command, "AT+CFUN=1");
        assert!(on[1].command.contains("\"ims\""));
        assert_eq!(on[2].command, "AT+CEUS=0");
        assert_eq!(on[3].command, "AT^IMSSWITCH=1,0,0");
        assert!(off[1].command.contains("\"\""));
        assert_eq!(off[2].command, "AT+CEUS=1");
        assert_eq!(off[3].command, "AT^IMSSWITCH=0,0,0");
        // The radio is always cycled, and the profile always carries cid 5.
        for step in &on {
            if step.command.starts_with("AT+CGDCONT") {
                assert!(step.command.starts_with("AT+CGDCONT=5,\"IPV4V6\""));
            }
        }
    }
}
