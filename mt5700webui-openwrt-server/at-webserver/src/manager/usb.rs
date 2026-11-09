//! USB discovery and driver binding — Rust port of `usr/share/mt5700m/usb.sh`.
//!
//! The manager is the only caller. Semantics are intentionally identical to the
//! shell helper so a partially migrated system keeps working:
//!
//! * identity is the documented TD Tech pair `3466:3301` (normal mode);
//! * the NCM control interface is class `02`, subclass `0d`;
//! * a dynamic `option` id can claim the CDC pair first, so binding always
//!   releases non-`cdc_ncm` drivers from class 02/0a interfaces first;
//! * the PCUI port is found by its `ff:06:12` descriptor (or a `PCUI` label),
//!   never by a fixed `ttyUSBn` number.
//!
//! Every path is overridable through the same environment variables the shell
//! used (`MT5700M_SYSFS_ROOT`, `MT5700M_DEV_ROOT`), which is what the tests use.

use std::fs;
use std::path::{Path, PathBuf};

use crate::core::sys;

const VENDOR: &str = "3466";

fn sysfs_root() -> PathBuf {
    PathBuf::from(std::env::var("MT5700M_SYSFS_ROOT").unwrap_or_else(|_| "/sys".to_string()))
}

fn dev_root() -> PathBuf {
    PathBuf::from(std::env::var("MT5700M_DEV_ROOT").unwrap_or_else(|_| "/dev".to_string()))
}

fn read_trim(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn lower(path: &Path) -> Option<String> {
    read_trim(path).map(|s| s.to_ascii_lowercase())
}

/// `(state, product, slot)` for the first matching module, preferring normal mode.
pub fn usb_info() -> Option<(String, String, String)> {
    let root = sysfs_root().join("bus/usb/devices");
    let entries = fs::read_dir(&root).ok()?;
    let mut first: Option<(String, String, String)> = None;

    for entry in entries.flatten() {
        let dir = entry.path();
        let Some(vendor) = lower(&dir.join("idVendor")) else {
            continue;
        };
        if vendor != VENDOR {
            continue;
        }
        let Some(product) = lower(&dir.join("idProduct")) else {
            continue;
        };
        let slot = dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        let state = match product.as_str() {
            "3301" => "normal",
            "3302" => "upgrade",
            "3303" => "dump",
            _ => "unknown",
        };
        let item = (state.to_string(), product.clone(), slot);
        if state == "normal" {
            return Some(item);
        }
        if first.is_none() {
            first = Some(item);
        }
    }
    first
}

pub fn normal_slot() -> Option<String> {
    let (state, _, slot) = usb_info()?;
    (state == "normal").then_some(slot)
}

/// First netdev under the module's USB slot.
pub fn netdev() -> Option<String> {
    let slot = normal_slot()?;
    let base = sysfs_root().join("bus/usb/devices");
    let entries = fs::read_dir(&base).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name != slot && !name.starts_with(&format!("{}:", slot)) {
            continue;
        }
        let net = entry.path().join("net");
        let Ok(nets) = fs::read_dir(&net) else { continue };
        for dev in nets.flatten() {
            let dev_name = dev.file_name().to_string_lossy().to_string();
            if sysfs_root().join("class/net").join(&dev_name).is_dir() {
                return Some(dev_name);
            }
        }
    }
    None
}

pub fn carrier(dev: &str) -> bool {
    if dev.is_empty() {
        return false;
    }
    read_trim(&sysfs_root().join("class/net").join(dev).join("carrier"))
        .map(|v| v == "1")
        .unwrap_or(false)
}

fn modprobe(module: &str) {
    let _ = sys::run("modprobe", &[module], std::time::Duration::from_secs(10));
}

fn interface_dirs(slot: &str) -> Vec<PathBuf> {
    let base = sysfs_root().join("bus/usb/devices");
    let mut out = Vec::new();
    if let Ok(entries) = fs::read_dir(&base) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with(&format!("{}:", slot)) {
                out.push(entry.path());
            }
        }
    }
    out
}

fn driver_name(interface: &Path) -> Option<String> {
    let target = fs::read_link(interface.join("driver")).ok()?;
    target
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
}

fn unbind(interface: &Path) {
    let Some(driver) = driver_name(interface) else {
        return;
    };
    if driver == "cdc_ncm" {
        return;
    }
    let unbind_path = interface.join("driver/unbind");
    if !unbind_path.exists() {
        return;
    }
    let name = interface
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let _ = fs::write(&unbind_path, format!("{}\n", name));
}

fn bind(interface: &Path, driver: &str) -> bool {
    let bind_path = sysfs_root()
        .join("bus/usb/drivers")
        .join(driver)
        .join("bind");
    if !bind_path.exists() {
        return false;
    }
    let name = interface
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    fs::write(&bind_path, format!("{}\n", name)).is_ok()
}

/// Bind the CDC-NCM control interface, releasing a wrongly claimed option driver.
pub fn bind_network_driver() -> bool {
    let Some(slot) = normal_slot() else {
        return false;
    };
    for module in ["usbnet", "cdc_ether", "cdc_ncm"] {
        modprobe(module);
    }

    let mut control: Option<PathBuf> = None;
    for interface in interface_dirs(&slot) {
        let class = lower(&interface.join("bInterfaceClass")).unwrap_or_default();
        let subclass = lower(&interface.join("bInterfaceSubClass")).unwrap_or_default();
        if class != "02" && class != "0a" {
            continue;
        }
        unbind(&interface);
        if class == "02" && subclass == "0d" {
            control = Some(interface);
        }
    }

    let Some(control) = control else {
        return false;
    };
    if driver_name(&control).as_deref() == Some("cdc_ncm") {
        return true;
    }
    bind(&control, "cdc_ncm")
}

fn usb_device_dir_for(path: &Path) -> Option<PathBuf> {
    let mut current = fs::canonicalize(path).ok()?;
    loop {
        if current.join("idVendor").exists() && current.join("idProduct").exists() {
            return Some(current);
        }
        current = current.parent()?.to_path_buf();
    }
}

fn tty_interface_dir(tty: &str) -> Option<PathBuf> {
    let name = Path::new(tty).file_name()?.to_string_lossy().to_string();
    let device = sysfs_root().join("class/tty").join(name).join("device");
    let mut current = fs::canonicalize(device).ok()?;
    loop {
        if current.join("bInterfaceClass").exists() || current.join("interface").exists() {
            return Some(current);
        }
        current = current.parent()?.to_path_buf();
    }
}

fn port_belongs_to_normal(tty: &str) -> bool {
    let name = Path::new(tty).file_name().map(|n| n.to_string_lossy().to_string());
    let Some(name) = name else { return false };
    let device = sysfs_root().join("class/tty").join(name).join("device");
    let Some(usb_dir) = usb_device_dir_for(&device) else {
        return false;
    };
    let vendor = lower(&usb_dir.join("idVendor")).unwrap_or_default();
    let product = lower(&usb_dir.join("idProduct")).unwrap_or_default();
    format!("{}:{}", vendor, product) == "3466:3301"
}

fn port_is_pcui(tty: &str) -> bool {
    if !port_belongs_to_normal(tty) {
        return false;
    }
    let Some(interface) = tty_interface_dir(tty) else {
        return false;
    };
    let class = lower(&interface.join("bInterfaceClass")).unwrap_or_default();
    let subclass = lower(&interface.join("bInterfaceSubClass")).unwrap_or_default();
    let protocol = lower(&interface.join("bInterfaceProtocol")).unwrap_or_default();
    if format!("{}:{}:{}", class, subclass, protocol) == "ff:06:12" {
        return true;
    }
    let description = read_trim(&interface.join("interface")).unwrap_or_default();
    let lower_desc = description.to_ascii_lowercase();
    lower_desc.contains("pcui") || lower_desc.contains("pc- ui")
}

pub fn pcui_port() -> Option<String> {
    if normal_slot().is_none() {
        return None;
    }
    let entries = fs::read_dir(dev_root()).ok()?;
    let mut found: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with("ttyUSB") {
            found.push(format!("/dev/{}", name));
        }
    }
    found.sort();
    found.into_iter().find(|tty| port_is_pcui(tty))
}

/// Ensure the AT ports exist; binds NCM first so the option id cannot steal it.
pub fn bind_serial_driver() -> bool {
    let _ = bind_network_driver();
    if pcui_port().is_some() {
        return true;
    }
    modprobe("usbserial");
    modprobe("option");

    for candidate in ["option1", "option"] {
        let new_id = sysfs_root()
            .join("bus/usb-serial/drivers")
            .join(candidate)
            .join("new_id");
        if !new_id.exists() {
            continue;
        }
        if fs::write(&new_id, "3466 3301\n").is_ok() {
            let _ = bind_network_driver();
            return true;
        }
    }
    false
}

pub fn wait_netdev(seconds: u64) -> Option<String> {
    for _ in 0..seconds.max(1) {
        if let Some(dev) = netdev() {
            return Some(dev);
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    None
}

/// Force a rebind of the NCM control/data pair.
pub fn force_ncm_rebind() -> bool {
    let Some(slot) = normal_slot() else {
        return false;
    };
    let mut control: Option<PathBuf> = None;
    let mut data: Option<PathBuf> = None;
    for interface in interface_dirs(&slot) {
        let class = lower(&interface.join("bInterfaceClass")).unwrap_or_default();
        let subclass = lower(&interface.join("bInterfaceSubClass")).unwrap_or_default();
        if class != "02" && class != "0a" {
            continue;
        }
        unbind(&interface);
        match format!("{}:{}", class, subclass).as_str() {
            "02:0d" => control = Some(interface),
            "0a:00" | "02:06" => data = Some(interface),
            _ => {}
        }
    }
    let Some(control) = control else {
        return false;
    };
    if driver_name(&control).as_deref() != Some("cdc_ncm") && !bind(&control, "cdc_ncm") {
        return false;
    }
    if let Some(data) = data {
        let _ = bind(&data, "cdc_ncm");
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_info_prefers_normal_mode() {
        let dir = std::env::temp_dir().join(format!("mt5700m-usb-{}", std::process::id()));
        let devices = dir.join("bus/usb/devices");
        for (slot, product) in [("2-1", "3302"), ("2-2", "3301")] {
            let dev = devices.join(slot);
            fs::create_dir_all(&dev).unwrap();
            fs::write(dev.join("idVendor"), "3466\n").unwrap();
            fs::write(dev.join("idProduct"), product).unwrap();
        }
        std::env::set_var("MT5700M_SYSFS_ROOT", &dir);
        let info = usb_info().unwrap();
        std::env::remove_var("MT5700M_SYSFS_ROOT");
        assert_eq!(info.0, "normal");
        assert_eq!(info.2, "2-2");
    }
}
