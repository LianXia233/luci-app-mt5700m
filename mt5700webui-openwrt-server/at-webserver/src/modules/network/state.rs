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

/// Access-technology configuration (`^SYSCFGEX`), the "网络系统配置" card.
///
/// Field names are the page's (`acqorder`, `band`, `roam`, `srvdomain`,
/// `lteband`); absent means the reply did not carry it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SysCfgState {
    pub acqorder: Option<String>,
    pub band: Option<String>,
    pub roam: Option<i64>,
    pub srvdomain: Option<i64>,
    pub lteband: Option<String>,
}

impl SysCfgState {
    pub fn is_empty(&self) -> bool {
        self.acqorder.is_none()
    }

    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        let mut put = |k: &str, v: Value| {
            m.insert(k.to_string(), v);
        };
        for (key, value) in [("acqorder", &self.acqorder), ("band", &self.band), ("lteband", &self.lteband)] {
            if let Some(v) = value {
                if !v.is_empty() {
                    put(key, json::str_val(v));
                }
            }
        }
        if let Some(v) = self.roam {
            put("roam", json::num_val(v));
        }
        if let Some(v) = self.srvdomain {
            put("srvdomain", json::num_val(v));
        }
        Value::Obj(m)
    }
}


/// Built-in autodial configuration (`^SETAUTODIAL?`) as the dial page shows it.
///
/// The page renders every field, so they stay string-typed exactly like the AT
/// text: a password or APN that is not a number must not be coerced.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AutodialState {
    pub enable: Option<i64>,
    pub dial_mode: Option<i64>,
    pub protocol: String,
    pub apn: String,
    pub username: String,
    pub password: String,
    pub auth_type: Option<i64>,
    /// `^NDISSTATQRY?` said the modem's own data session is up. The page used
    /// this to fill in a missing dial mode; that decision is the module's now.
    pub ndis_active: bool,
}

impl AutodialState {
    pub fn is_empty(&self) -> bool {
        self.enable.is_none()
            && self.dial_mode.is_none()
            && self.protocol.is_empty()
            && self.apn.is_empty()
            && self.username.is_empty()
            && self.password.is_empty()
            && self.auth_type.is_none()
    }

    /// `{enable, dialMode, protocol, apn, username, password, authType}` —
    /// `dialMode` includes the NDIS fallback, which is what the page displayed.
    ///
    /// An unread state answers `{}` rather than empty strings: the page merges
    /// the object it gets, so absent fields mean "keep the value you show".
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        if self.is_empty() {
            return Value::Obj(m);
        }
        if let Some(v) = self.enable {
            m.insert("enable".to_string(), json::num_val(v));
        }
        let dial_mode = match self.dial_mode {
            Some(mode) => Some(mode),
            None if self.ndis_active => Some(1),
            None => None,
        };
        if let Some(v) = dial_mode {
            m.insert("dialMode".to_string(), json::num_val(v));
        }
        m.insert("protocol".to_string(), json::str_val(&self.protocol));
        m.insert("apn".to_string(), json::str_val(&self.apn));
        m.insert("username".to_string(), json::str_val(&self.username));
        m.insert("password".to_string(), json::str_val(&self.password));
        if let Some(v) = self.auth_type {
            m.insert("authType".to_string(), json::num_val(v));
        }
        Value::Obj(m)
    }
}

/// Connection/RRC state (`^RRCSTAT?`).
///
/// `state` is the code the manual numbers 0..3 (the page labels them
/// Idle/Connected/Inactive/Invalid) and `camped` is the 98/99 flag some
/// firmware versions append. Both are optional: a reply that only carries the
/// state (or only the flag) publishes exactly what it carried.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RrcState {
    pub state: Option<i64>,
    pub camped: Option<i64>,
}

impl RrcState {
    pub fn is_empty(&self) -> bool {
        self.state.is_none() && self.camped.is_none()
    }

    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        if let Some(v) = self.state {
            m.insert("state".to_string(), json::num_val(v));
        }
        if let Some(v) = self.camped {
            m.insert("camped".to_string(), json::num_val(v));
        }
        Value::Obj(m)
    }
}

/// USB port mode (`^SETMODE?`), the dial page's "USB 端口模式" card.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsbModeState {
    pub mode: Option<i64>,
}

impl UsbModeState {
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        if let Some(v) = self.mode {
            m.insert("mode".to_string(), json::num_val(v));
        }
        Value::Obj(m)
    }
}

/// Interface configuration (`^TDCFG?`): the NIC mode, the mutually exclusive
/// post-route flag and the DMZ host. One read answers both cards on the page.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InterfaceCfgState {
    pub mode: Option<i64>,
    pub post_route: Option<i64>,
    pub dmz_enabled: bool,
    pub dmz_host: String,
}

impl InterfaceCfgState {
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        if let Some(v) = self.mode {
            m.insert("mode".to_string(), json::num_val(v));
        }
        if let Some(v) = self.post_route {
            m.insert("postRoute".to_string(), json::num_val(v));
        }
        let mut dmz = std::collections::BTreeMap::new();
        dmz.insert("enabled".to_string(), Value::Bool(self.dmz_enabled));
        dmz.insert("host".to_string(), json::str_val(&self.dmz_host));
        m.insert("dmz".to_string(), Value::Obj(dmz));
        Value::Obj(m)
    }
}

/// One PDP context definition + its activation state (`+CGDCONT?`/`+CGACT?`).
#[derive(Debug, Clone, PartialEq)]
pub struct PdpContext {
    pub cid: u32,
    pub apn_type: String,
    pub apn: String,
    pub pdp_addr: String,
    pub active: bool,
}

impl PdpContext {
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        m.insert("cid".to_string(), json::num_val(self.cid as u64));
        m.insert("type".to_string(), json::str_val(&self.apn_type));
        m.insert("apn".to_string(), json::str_val(&self.apn));
        m.insert("pdp_addr".to_string(), json::str_val(&self.pdp_addr));
        m.insert("active".to_string(), Value::Bool(self.active));
        Value::Obj(m)
    }
}
/// IMS registration (`+CIREG`), the wireless page's "IMS registration" row.
///
/// `enabled` is the report flag the modem echoes back (`<n>`), `registered` is
/// `<reg_info>`: 1 = the UE is IMS-registered. Both are omitted when the modem
/// did not answer, which the page renders as no data instead of guessing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImsState {
    pub enabled: Option<i64>,
    pub registered: Option<i64>,
}

impl ImsState {
    pub fn is_empty(&self) -> bool {
        self.enabled.is_none() && self.registered.is_none()
    }

    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        if let Some(v) = self.enabled {
            m.insert("enabled".to_string(), json::num_val(v));
        }
        if let Some(v) = self.registered {
            m.insert("registered".to_string(), json::num_val(v));
        }
        Value::Obj(m)
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

    #[test]
    fn syscfg_json_uses_the_page_fields() {
        let st = SysCfgState {
            acqorder: Some("08030201".into()),
            band: Some("3FFFFFFF".into()),
            roam: Some(1),
            srvdomain: Some(2),
            lteband: Some("7FFFFFFFFFFFFFFF".into()),
        };
        let Value::Obj(m) = st.to_json() else {
            panic!("object")
        };
        assert_eq!(m.get("acqorder").and_then(|v| v.as_str()), Some("08030201"));
        assert_eq!(m.get("band").and_then(|v| v.as_str()), Some("3FFFFFFF"));
        assert_eq!(m.get("roam").and_then(|v| v.as_i64()), Some(1));
        assert_eq!(m.get("srvdomain").and_then(|v| v.as_i64()), Some(2));
        assert_eq!(m.get("lteband").and_then(|v| v.as_str()), Some("7FFFFFFFFFFFFFFF"));
        assert!(SysCfgState::default().is_empty());
        assert!(!st.is_empty());
    }
}
