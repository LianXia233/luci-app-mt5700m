//! Network domain model: operator, access technology and registration state.
//!
//! Two topics are published from this one model, exactly as before:
//! `network` (operator + system mode) and `registration` (the cell the modem
//! is camped on). The JSON field names are the ones LuCI and the WebUI have
//! always consumed, so both renderings stay wire-compatible.

use crate::core::json::{self, Value};

/// Registration state of the serving cell (`+CxxREG`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RegistrationState {
    /// 3GPP registration status (0 = not registered, 1 = registered, ...).
    pub state: u8,
    /// Tracking area code (5G/4G) — string, the modem sends hex.
    pub tac: Option<String>,
    /// Cell identity (hex string).
    pub ci: Option<String>,
    /// Access technology code, when reported.
    pub act: Option<String>,
    /// 5G NSSAI (C5GREG only).
    pub nssai: Option<String>,
    pub mcc: Option<String>,
    pub mnc: Option<String>,
    /// Location area code (2G/3G family).
    pub lac: Option<String>,
}

impl RegistrationState {
    /// True when no registration line was parsed at all.
    pub fn is_empty(&self) -> bool {
        self.state == 0
            && self.tac.is_none()
            && self.ci.is_none()
            && self.act.is_none()
            && self.nssai.is_none()
            && self.mcc.is_none()
            && self.mnc.is_none()
            && self.lac.is_none()
    }

    /// Domain JSON: `{state, tac, ci, act, nssai, mcc, mnc, lac}` with absent
    /// fields omitted (the shape the cache always held).
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        m.insert("state".to_string(), json::num_val(self.state));
        let mut put = |k: &str, v: &Option<String>| {
            if let Some(s) = v {
                if !s.is_empty() {
                    m.insert(k.to_string(), json::str_val(s));
                }
            }
        };
        put("tac", &self.tac);
        put("ci", &self.ci);
        put("act", &self.act);
        put("nssai", &self.nssai);
        put("mcc", &self.mcc);
        put("mnc", &self.mnc);
        put("lac", &self.lac);
        Value::Obj(m)
    }
}

/// One activated PDP context address (`AT+CGPADDR`), as the diagnostics panel
/// displays it.
#[derive(Debug, Clone, PartialEq)]
pub struct PdpAddress {
    pub cid: u32,
    pub address: String,
    /// `IPv4` / `IPv6` / `未知`.
    pub family: &'static str,
}

impl PdpAddress {
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        m.insert("cid".to_string(), json::num_val(self.cid as u64));
        m.insert("address".to_string(), json::str_val(&self.address));
        m.insert("family".to_string(), json::str_val(self.family));
        Value::Obj(m)
    }
}

/// One address family of the data call as the modem's DHCP client reports it
/// (`AT^DHCP?` for IPv4, `AT^DHCPV6?` for IPv6).
///
/// Field names match what the Info page has always rendered, so the frontends
/// keep their `{address, netmask, gateway, dhcpServer, ...}` shape while the
/// AT syntax and the hex decoding live here.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DhcpLease {
    pub address: Option<String>,
    pub netmask: Option<String>,
    pub gateway: Option<String>,
    pub dhcp_server: Option<String>,
    pub primary_dns: Option<String>,
    pub secondary_dns: Option<String>,
}

impl DhcpLease {
    /// True when no field was decoded at all.
    pub fn is_empty(&self) -> bool {
        self.address.is_none()
            && self.netmask.is_none()
            && self.gateway.is_none()
            && self.dhcp_server.is_none()
            && self.primary_dns.is_none()
            && self.secondary_dns.is_none()
    }

    /// Domain JSON: snake_case keys, absent fields omitted.
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        let mut put = |k: &str, v: &Option<String>| {
            if let Some(s) = v {
                if !s.is_empty() {
                    m.insert(k.to_string(), json::str_val(s));
                }
            }
        };
        put("address", &self.address);
        put("netmask", &self.netmask);
        put("gateway", &self.gateway);
        put("dhcp_server", &self.dhcp_server);
        put("primary_dns", &self.primary_dns);
        put("secondary_dns", &self.secondary_dns);
        Value::Obj(m)
    }
}

/// 5G access mode (`^C5GOPTION`).
///
/// The page renders the triple as "仅 SA / 仅 NSA / SA+NSA / 其他"; that label
/// mapping stays in the frontend, the values are the module's.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct C5gOptionState {
    pub nr_sa_support_flag: Option<u8>,
    pub nr_dc_mode: Option<u8>,
    pub gc_access_mode: Option<u8>,
}

impl C5gOptionState {
    /// True when the reply did not carry a parseable triple.
    pub fn is_empty(&self) -> bool {
        self.nr_sa_support_flag.is_none()
            && self.nr_dc_mode.is_none()
            && self.gc_access_mode.is_none()
    }

    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        let mut put = |k: &str, v: Option<u8>| {
            if let Some(v) = v {
                m.insert(k.to_string(), json::num_val(v));
            }
        };
        put("nr_sa_support_flag", self.nr_sa_support_flag);
        put("nr_dc_mode", self.nr_dc_mode);
        put("gc_access_mode", self.gc_access_mode);
        Value::Obj(m)
    }
}

/// Which radio access technology a frequency lock applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockKind {
    Lte,
    Nr,
}

impl LockKind {
    /// Short name used in CLI arguments and JSON keys.
    pub fn as_str(self) -> &'static str {
        match self {
            LockKind::Lte => "lte",
            LockKind::Nr => "nr",
        }
    }

    /// Largest PCI the manual allows (503 LTE / 1007 NR).
    pub fn pci_max(self) -> u64 {
        match self {
            LockKind::Lte => 503,
            LockKind::Nr => 1007,
        }
    }
}

/// One locked band entry (`^LTEFREQLOCK` / `^NRFREQLOCK` reply row).
///
/// `pci` is decimal here although the reply prints it in hex; `scs` only
/// exists for NR. Values the row did not carry stay `None`, and the frontends
/// render an empty input for them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LockItem {
    pub band: Option<i64>,
    pub arfcn: Option<i64>,
    pub pci: Option<i64>,
    pub scs: Option<i64>,
}

impl LockItem {
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        let mut put = |k: &str, v: Option<i64>| {
            if let Some(v) = v {
                m.insert(k.to_string(), json::num_val(v));
            }
        };
        put("band", self.band);
        put("arfcn", self.arfcn);
        put("pci", self.pci);
        put("scs", self.scs);
        Value::Obj(m)
    }
}

/// A frequency-lock reading.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LockState {
    /// 0 = unlocked, 1 = ARFCN lock, 2 = cell lock, 3 = band lock.
    pub lock_type: u8,
    /// Mobility state the firmware reports next to the type (0 for an unlock).
    pub mobility: u8,
    /// Locked band entries; empty for an unlock or an unparseable reply.
    pub items: Vec<LockItem>,
}

impl LockState {
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        m.insert("lock_type".to_string(), json::num_val(self.lock_type));
        m.insert("mobility".to_string(), json::num_val(self.mobility));
        let items: Vec<Value> = self.items.iter().map(|i| i.to_json()).collect();
        m.insert("items".to_string(), Value::Arr(items));
        Value::Obj(m)
    }
}

/// Network domain model (operator + system mode + registration).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NetworkState {
    pub operator: Option<String>,
    /// Access technology as the UI prints it (`LTE`, `NR`, `WCDMA`, ...).
    pub sysmode: Option<String>,
    /// Full system-mode string from `^SYSINFOEX` (e.g. `LTE/NR`).
    pub sysmode_detail: Option<String>,
    pub registration: RegistrationState,
}

impl NetworkState {
    /// True when nothing was learned from the modem this round.
    pub fn is_empty(&self) -> bool {
        self.operator.is_none() && self.sysmode.is_none() && self.sysmode_detail.is_none()
    }

    /// Domain JSON for the `network` topic.
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        if let Some(op) = &self.operator {
            m.insert("operator".to_string(), json::str_val(op));
        }
        if let Some(sm) = &self.sysmode {
            m.insert("sysmode".to_string(), json::str_val(sm));
        }
        if let Some(d) = &self.sysmode_detail {
            m.insert("sysmode_detail".to_string(), json::str_val(d));
        }
        Value::Obj(m)
    }

    /// Rebuild the model from cache JSON (used by the API and by the CLI
    /// text renderer, so neither re-parses AT).
    pub fn from_json(m: &std::collections::BTreeMap<String, Value>) -> Self {
        let mut st = NetworkState::default();
        if let Some(v) = m.get("operator").and_then(|v| v.as_str()) {
            st.operator = Some(v.to_string());
        }
        if let Some(v) = m.get("sysmode").and_then(|v| v.as_str()) {
            st.sysmode = Some(v.to_string());
        }
        if let Some(v) = m.get("sysmode_detail").and_then(|v| v.as_str()) {
            st.sysmode_detail = Some(v.to_string());
        }
        st
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c5goption_json_omits_missing_fields() {
        let st = C5gOptionState {
            nr_sa_support_flag: Some(1),
            nr_dc_mode: Some(1),
            gc_access_mode: None,
        };
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        assert_eq!(m.get("nr_sa_support_flag").and_then(|v| v.as_i64()), Some(1));
        assert!(!m.contains_key("gc_access_mode"));
        assert!(!st.is_empty());
        assert!(C5gOptionState::default().is_empty());
    }

    #[test]
    fn registration_json_omits_absent_fields() {
        let mut reg = RegistrationState {
            state: 1,
            ..Default::default()
        };
        reg.tac = Some("149002".into());
        let Value::Obj(m) = reg.to_json() else {
            panic!("object")
        };
        assert_eq!(m.get("state").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(m.get("tac").and_then(|v| v.as_str()), Some("149002"));
        assert!(m.get("ci").is_none());
        assert!(m.get("nssai").is_none());
    }

    #[test]
    fn empty_registration_is_all_zero() {
        let reg = RegistrationState::default();
        assert!(reg.is_empty());
        assert!(!RegistrationState {
            state: 1,
            ..Default::default()
        }
        .is_empty());
    }

    #[test]
    fn network_state_round_trips_through_json() {
        let st = NetworkState {
            operator: Some("CHN-UNICOM".into()),
            sysmode: Some("NR".into()),
            sysmode_detail: Some("LTE/NR".into()),
            registration: RegistrationState::default(),
        };
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        assert_eq!(NetworkState::from_json(&m), st);
    }
}
