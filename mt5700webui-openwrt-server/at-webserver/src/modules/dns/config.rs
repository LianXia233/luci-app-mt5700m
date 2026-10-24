//! DNS configuration for the plugin-managed cellular interfaces.
//!
//! The settings live in UCI (`mt5700m.dns`) so they survive reboots and are
//! readable by the manager shell script without the daemon running. The
//! backend validates them before writing; the apply script also performs a
//! defensive check because it is called on boot paths where a corrupt config
//! must not be able to inject garbage into `network.*`.
//!
//! Only the plugin's own interfaces may be touched (`MT5700M`,
//! `MT5700Mv6`). This module never edits the global resolver, LAN/WAN, or any
//! other interface.

use crate::core::json::Value;
use crate::core::sys;
use std::collections::BTreeMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

pub const PACKAGE: &str = "mt5700m";
pub const SECTION: &str = "dns";

pub const DEFAULT_V4_PRIMARY: &str = "119.29.29.29";
pub const DEFAULT_V4_SECONDARY: &str = "223.5.5.5";
pub const DEFAULT_V6_PRIMARY: &str = "2402:4e00::";
pub const DEFAULT_V6_SECONDARY: &str = "2400:3200::1";

/// One address family's DNS block.
#[derive(Debug, Clone, PartialEq)]
pub struct FamilyConfig {
    pub enabled: bool,
    pub primary: String,
    pub secondary: String,
}

impl FamilyConfig {
    /// Ordered, de-duplicated, non-empty servers.
    pub fn servers(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(2);
        for value in [&self.primary, &self.secondary] {
            let v = value.trim();
            if !v.is_empty() && !out.iter().any(|x| x == v) {
                out.push(v.to_string());
            }
        }
        out
    }
}

/// The complete DNS settings block.
#[derive(Debug, Clone, PartialEq)]
pub struct DnsConfig {
    pub ipv4: FamilyConfig,
    pub ipv6: FamilyConfig,
}

impl Default for DnsConfig {
    fn default() -> Self {
        DnsConfig {
            ipv4: FamilyConfig {
                enabled: true,
                primary: DEFAULT_V4_PRIMARY.to_string(),
                secondary: DEFAULT_V4_SECONDARY.to_string(),
            },
            ipv6: FamilyConfig {
                enabled: true,
                primary: DEFAULT_V6_PRIMARY.to_string(),
                secondary: DEFAULT_V6_SECONDARY.to_string(),
            },
        }
    }
}

fn uci_key(option: &str) -> String {
    format!("{}.{}.{}", PACKAGE, SECTION, option)
}

fn read_bool(values: &BTreeMap<String, String>, option: &str, default: bool) -> bool {
    match values
        .get(option)
        .map(|v| v.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("1") | Some("true") | Some("on") | Some("yes") | Some("enabled") => true,
        Some("0") | Some("false") | Some("off") | Some("no") | Some("disabled") => false,
        _ => default,
    }
}

fn read_text(values: &BTreeMap<String, String>, option: &str, default: &str) -> String {
    values
        .get(option)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_string())
}

/// All options of `mt5700m.dns` from one `uci show` call.
fn section_values() -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(text) = sys::uci_show(&format!("{}.{}", PACKAGE, SECTION)) else {
        return out;
    };
    let prefix = format!("{}.{}.", PACKAGE, SECTION);
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some(name) = key.trim().strip_prefix(&prefix) else {
            continue;
        };
        let mut value = value.trim().to_string();
        if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
            value = value[1..value.len() - 1].replace("'\\''", "'");
        }
        out.insert(name.to_string(), value);
    }
    out
}

impl DnsConfig {
    /// Load from UCI, falling back to defaults for missing/invalid options.
    ///
    /// A missing `mt5700m.dns` section therefore behaves exactly like a fresh
    /// install: the four documented defaults are active.
    pub fn load() -> Self {
        let values = section_values();
        DnsConfig {
            ipv4: FamilyConfig {
                enabled: read_bool(&values, "ipv4_enabled", true),
                primary: read_text(&values, "ipv4_primary", DEFAULT_V4_PRIMARY),
                secondary: read_text(&values, "ipv4_secondary", DEFAULT_V4_SECONDARY),
            },
            ipv6: FamilyConfig {
                enabled: read_bool(&values, "ipv6_enabled", true),
                primary: read_text(&values, "ipv6_primary", DEFAULT_V6_PRIMARY),
                secondary: read_text(&values, "ipv6_secondary", DEFAULT_V6_SECONDARY),
            },
        }
    }

    /// Merge a JSON request into this config.
    ///
    /// Accepts the nested shape the page uses:
    /// `{ipv4:{enabled,primary,secondary}, ipv6:{...}}`.
    /// Absent fields keep their current value, so a partial update never
    /// silently resets the other family.
    pub fn merge_json(&mut self, params: &Value) -> Result<(), String> {
        for (name, fam) in [("ipv4", &mut self.ipv4), ("ipv6", &mut self.ipv6)] {
            let Some(block) = params.get(name) else { continue };
            if !matches!(block, Value::Obj(_)) {
                return Err(format!("{} 配置必须是对象", name));
            }
            if let Some(v) = block.get("enabled") {
                fam.enabled = v
                    .as_bool()
                    .ok_or_else(|| format!("{} 启用状态必须是布尔值", name))?;
            }
            if let Some(v) = block.get("primary") {
                fam.primary = v
                    .as_str()
                    .ok_or_else(|| format!("{} 主 DNS 必须是字符串", name))?
                    .trim()
                    .to_string();
            }
            if let Some(v) = block.get("secondary") {
                fam.secondary = v
                    .as_str()
                    .ok_or_else(|| format!("{} 备用 DNS 必须是字符串", name))?
                    .trim()
                    .to_string();
            }
        }
        Ok(())
    }

    /// Validate both families.
    ///
    /// Rules:
    /// * an enabled family needs at least one valid address (primary);
    /// * the secondary may be empty;
    /// * any non-empty value must parse as an address of that family;
    /// * unspecified (`0.0.0.0`, `::`) and multicast addresses are rejected.
    ///
    /// A disabled family may keep values (they are simply not applied), but a
    /// non-empty value must still be a valid address — the config file is not
    /// a place to park typo'd input that reappears when re-enabled.
    pub fn validate(&self) -> Result<(), String> {
        validate_family("IPv4", &self.ipv4, Family::V4)?;
        validate_family("IPv6", &self.ipv6, Family::V6)?;
        Ok(())
    }

    /// Servers that must be applied for one family (empty when disabled).
    pub fn effective_servers(&self, family: DnsFamily) -> Vec<String> {
        match family {
            DnsFamily::V4 => {
                if self.ipv4.enabled {
                    self.ipv4.servers()
                } else {
                    Vec::new()
                }
            }
            DnsFamily::V6 => {
                if self.ipv6.enabled {
                    self.ipv6.servers()
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// Persist this config to UCI and commit the package.
    pub fn save(&self) -> Result<(), String> {
        if !sys::uci_available() {
            return Err("UCI 不可用，无法保存 DNS 配置".to_string());
        }
        // Save only after validation: a rejected config must never reach the
        // file the boot script reads.
        self.validate()?;
        sys::uci_set(&format!("{}.{}", PACKAGE, SECTION), SECTION)?;
        for (option, value) in [
            ("ipv4_enabled", bool_text(self.ipv4.enabled)),
            ("ipv4_primary", self.ipv4.primary.trim()),
            ("ipv4_secondary", self.ipv4.secondary.trim()),
            ("ipv6_enabled", bool_text(self.ipv6.enabled)),
            ("ipv6_primary", self.ipv6.primary.trim()),
            ("ipv6_secondary", self.ipv6.secondary.trim()),
        ] {
            sys::uci_set(&uci_key(option), value)?;
        }
        sys::uci_commit(PACKAGE)
    }

    /// The nested JSON shape the page consumes.
    pub fn to_json(&self) -> Value {
        let mut m = BTreeMap::new();
        m.insert("ipv4".to_string(), family_json(&self.ipv4, Family::V4));
        m.insert("ipv6".to_string(), family_json(&self.ipv6, Family::V6));
        Value::Obj(m)
    }

    /// Defaults, for the "restore defaults" button and first-run display.
    pub fn defaults_json() -> Value {
        DnsConfig::default().to_json()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DnsFamily {
    V4,
    V6,
}

impl DnsFamily {
    pub fn label(self) -> &'static str {
        match self {
            DnsFamily::V4 => "ipv4",
            DnsFamily::V6 => "ipv6",
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum Family {
    V4,
    V6,
}

fn validate_family(label: &str, cfg: &FamilyConfig, family: Family) -> Result<(), String> {
    let servers = cfg.servers();
    if cfg.enabled && servers.is_empty() {
        return Err(format!("{} 已启用，但至少需要一个 DNS 地址", label));
    }
    for value in servers {
        let ok = match family {
            Family::V4 => valid_v4(&value),
            Family::V6 => valid_v6(&value),
        };
        if !ok {
            return Err(format!("{} DNS 地址无效: {}", label, value));
        }
    }
    Ok(())
}

fn valid_v4(value: &str) -> bool {
    match Ipv4Addr::from_str(value) {
        Ok(addr) => !addr.is_unspecified() && !addr.is_multicast() && addr != Ipv4Addr::BROADCAST,
        Err(_) => false,
    }
}

fn valid_v6(value: &str) -> bool {
    match Ipv6Addr::from_str(value) {
        Ok(addr) => !addr.is_unspecified() && !addr.is_multicast(),
        Err(_) => false,
    }
}

fn bool_text(v: bool) -> &'static str {
    if v {
        "1"
    } else {
        "0"
    }
}

fn family_json(cfg: &FamilyConfig, family: Family) -> Value {
    let mut m = BTreeMap::new();
    m.insert("enabled".to_string(), Value::Bool(cfg.enabled));
    m.insert("primary".to_string(), crate::core::json::str_val(&cfg.primary));
    m.insert(
        "secondary".to_string(),
        crate::core::json::str_val(&cfg.secondary),
    );
    m.insert(
        "servers".to_string(),
        Value::Arr(cfg.servers().iter().map(|s| crate::core::json::str_val(s)).collect()),
    );
    m.insert(
        "valid".to_string(),
        Value::Bool(validate_family("", cfg, family).is_ok()),
    );
    Value::Obj(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_documented_servers() {
        let d = DnsConfig::default();
        assert_eq!(d.ipv4.servers(), vec!["119.29.29.29", "223.5.5.5"]);
        assert_eq!(d.ipv6.servers(), vec!["2402:4e00::", "2400:3200::1"]);
        assert!(d.validate().is_ok());
    }

    #[test]
    fn enabled_family_requires_a_valid_address() {
        let mut d = DnsConfig::default();
        d.ipv4.primary.clear();
        d.ipv4.secondary.clear();
        assert!(d.validate().is_err());

        d.ipv4.primary = "999.1.1.1".into();
        assert!(d.validate().unwrap_err().contains("无效"));
    }

    #[test]
    fn disabled_family_may_be_empty_but_not_invalid() {
        let mut d = DnsConfig::default();
        d.ipv6.enabled = false;
        d.ipv6.primary.clear();
        d.ipv6.secondary.clear();
        assert!(d.validate().is_ok());

        d.ipv6.primary = "not-an-address".into();
        assert!(d.validate().is_err());
    }

    #[test]
    fn wrong_family_address_is_rejected() {
        let mut d = DnsConfig::default();
        d.ipv4.primary = "2402:4e00::".into();
        assert!(d.validate().is_err());
        d.ipv4.primary = DEFAULT_V4_PRIMARY.into();
        d.ipv6.primary = "223.5.5.5".into();
        assert!(d.validate().is_err());
    }

    #[test]
    fn secondary_is_optional_and_duplicates_collapse() {
        let mut d = DnsConfig::default();
        d.ipv4.secondary.clear();
        assert_eq!(d.ipv4.servers(), vec!["119.29.29.29"]);
        d.ipv4.secondary = "119.29.29.29".into();
        assert_eq!(d.ipv4.servers(), vec!["119.29.29.29"]);
    }

    #[test]
    fn merge_json_keeps_omitted_fields() {
        let mut d = DnsConfig::default();
        let p = crate::core::json::parse(r#"{"ipv4":{"primary":"1.1.1.1"}}"#).unwrap();
        d.merge_json(&p).unwrap();
        assert_eq!(d.ipv4.primary, "1.1.1.1");
        assert_eq!(d.ipv4.secondary, "223.5.5.5");
        assert_eq!(d.ipv6.primary, DEFAULT_V6_PRIMARY);
    }

    #[test]
    fn unspecified_and_multicast_are_rejected() {
        let mut d = DnsConfig::default();
        d.ipv4.primary = "0.0.0.0".into();
        assert!(d.validate().is_err());
        d.ipv4.primary = "224.0.0.1".into();
        assert!(d.validate().is_err());
        d.ipv4.primary = DEFAULT_V4_PRIMARY.into();
        d.ipv6.primary = "::".into();
        assert!(d.validate().is_err());
        d.ipv6.primary = "ff02::1".into();
        assert!(d.validate().is_err());
    }

    #[test]
    fn effective_servers_respects_the_switches() {
        let mut d = DnsConfig::default();
        d.ipv4.enabled = false;
        assert!(d.effective_servers(DnsFamily::V4).is_empty());
        assert_eq!(d.effective_servers(DnsFamily::V6).len(), 2);
    }
}
