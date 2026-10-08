//! Device monitor — USB hotplug supervision for the MT5700M backend.
//!
//! The modem is a USB-attached serial device, so its presence and operating
//! mode (normal / upgrade / dump) can change at any time. Rather than
//! blocking the daemon on a dead port (which would stall every AT request),
//! a lightweight poller watches the serial port set and reacts to changes:
//!
//!   * **attach**      — cache is invalidated so the snapshot collectors
//!                       refetch, and `usb.attached` / `modem.available`
//!                       events are pushed to subscribers,
//!   * **detach**      — modem-dependent tasks (scan / beam / network ops /
//!                       sms) are cancelled, the cache is invalidated, and
//!                       `usb.detached` / `modem.unavailable` events fire,
//!   * **mode change** — normal <-> upgrade / dump transitions emit
//!                       `modem.mode_changed` and invalidate the cache.
//!
//! The poll loop is intentionally cheap (one `/sys` walk every 2 s, no AT
//! traffic) so it is safe on OpenWrt's constrained hardware, and it uses no
//! hotplug daemon or inotify dependency — plain `std::fs` works on every
//! platform the crate builds for.

use crate::state::bus::{EventBus, TOPIC_MODEM, TOPIC_TASK, TOPIC_USB};
use crate::core::json::{self, Value};
use crate::serial::manager::{scan_serial_ports, QUECTEL_VID};
use crate::state::cache::StateCache;
use crate::core::task::TaskKind;
use crate::scheduler::jobs::TaskManager;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Poll cadence. Fast enough to notice a USB re-plug within ~2 s, slow
/// enough to cost ~nothing while the modem is stable.
const SCAN_INTERVAL: Duration = Duration::from_secs(2);

/// Topic that are invalidated whenever the device set changes.
const STATE_TOPICS: [&str; 9] = [
    crate::state::bus::TOPIC_SIGNAL,
    crate::state::bus::TOPIC_NETWORK,
    crate::state::bus::TOPIC_CELL,
    crate::state::bus::TOPIC_TEMPERATURE,
    crate::state::bus::TOPIC_TRAFFIC,
    crate::state::bus::TOPIC_SIM,
    crate::state::bus::TOPIC_REGISTRATION,
    crate::state::bus::TOPIC_MODEM,
    crate::state::bus::TOPIC_USB,
];

/// Task kinds that cannot outlive the modem they talk to.
const MODEM_TASK_KINDS: [TaskKind; 4] = [
    TaskKind::LongRunning,
    TaskKind::Exclusive,
    TaskKind::Interactive,
    TaskKind::Periodic,
];

/// The modem's current presence, derived from a serial-port scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModemPresence {
    /// Any MT5700M USB device visible on the bus.
    pub present: bool,
    /// Operating mode of the best-matching device:
    /// `normal` / `upgrade` / `dump` / `unknown`.
    pub state: String,
    /// Preferred AT port path (PCUI in normal mode), if any.
    pub path: Option<String>,
    /// Whether the preferred port is the sysfs-classified PCUI.
    pub is_pcui: bool,
}

impl Default for ModemPresence {
    fn default() -> Self {
        ModemPresence {
            present: false,
            state: "unknown".into(),
            path: None,
            is_pcui: false,
        }
    }
}

impl ModemPresence {
    /// True when the modem answers AT (present + normal mode + a port).
    pub fn available(&self) -> bool {
        self.present && self.state == "normal" && self.path.is_some()
    }

    /// Scan the serial ports once and build the presence summary.
    pub fn detect() -> ModemPresence {
        let ports = scan_serial_ports();
        let mut best: Option<ModemPresence> = None;
        // Rank: PCUI+normal > answers-AT > any QUECTEL node (normal first,
        // then upgrade/dump so mode transitions are still observable).
        for p in &ports {
            if p.vendor != QUECTEL_VID {
                continue;
            }
            let candidate = ModemPresence {
                present: true,
                state: p.state.clone(),
                path: Some(p.path.clone()),
                is_pcui: p.is_pcui,
            };
            let replace = match &best {
                None => true,
                Some(b) => {
                    let score = |c: &ModemPresence| {
                        (c.is_pcui && c.state == "normal") as u8 * 4
                            + (c.state == "normal") as u8 * 2
                            + c.is_pcui as u8
                    };
                    score(&candidate) > score(b)
                }
            };
            if replace {
                best = Some(candidate);
            }
        }
        best.unwrap_or_default()
    }
}

/// Shared device monitor handle.
pub struct DeviceMonitor {
    cache: Arc<StateCache>,
    bus: Arc<EventBus>,
    tasks: Arc<TaskManager>,
    last: Mutex<Option<ModemPresence>>,
    stop: AtomicBool,
}

impl DeviceMonitor {
    /// Create the monitor (does not start polling yet).
    pub fn new(
        cache: Arc<StateCache>,
        bus: Arc<EventBus>,
        tasks: Arc<TaskManager>,
    ) -> Arc<Self> {
        Arc::new(DeviceMonitor {
            cache,
            bus,
            tasks,
            last: Mutex::new(None),
            stop: AtomicBool::new(false),
        })
    }

    /// Start the background poll loop.
    pub fn start(self: &Arc<Self>) {
        let mon = self.clone();
        crate::core::runtime::spawn_thread("device-monitor", move || mon.poll_loop());
        // Seed the baseline immediately so the first transition is detected.
        let now = ModemPresence::detect();
        *self.last.lock().unwrap() = Some(now.clone());
        self.emit_state(&now);
    }

    /// Stop polling (idempotent).
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Last observed presence (cheap read for the daemon/state API).
    pub fn current(&self) -> Option<ModemPresence> {
        self.last.lock().unwrap().clone()
    }

    fn poll_loop(self: Arc<Self>) {
        loop {
            if self.stop.load(Ordering::Relaxed) {
                return;
            }
            let next = ModemPresence::detect();
            let changed = {
                let last = self.last.lock().unwrap();
                last.as_ref().map(|l| l != &next).unwrap_or(true)
            };
            if changed {
                let prev = self.last.lock().unwrap().replace(next.clone());
                self.handle_transition(prev.as_ref(), &next);
            }
            std::thread::sleep(SCAN_INTERVAL);
        }
    }

    /// React to a presence transition (attach / detach / mode change).
    fn handle_transition(&self, prev: Option<&ModemPresence>, next: &ModemPresence) {
        let was = prev.map(|p| p.available()).unwrap_or(false);
        let is = next.available();
        if !was && is {
            // Attach: refresh everything — the old cached values belong to a
            // different (possibly dead) modem instance.
            self.cache.invalidate_many(&STATE_TOPICS);
            self.emit_event(TOPIC_USB, "usb.attached", &next);
            self.emit_event(TOPIC_MODEM, "modem.available", &next);
            return;
        }
        if was && !is {
            // Detach / mode loss: stop anything that needs the modem, drop
            // the stale state, and tell subscribers the modem is gone.
            let cancelled = self.tasks.cancel_active(&MODEM_TASK_KINDS);
            self.cache.invalidate_many(&STATE_TOPICS);
            self.emit_event(TOPIC_USB, "usb.detached", &next);
            self.emit_event(TOPIC_MODEM, "modem.unavailable", &next);
            if cancelled > 0 {
                let mut m = std::collections::BTreeMap::new();
                m.insert("cancelled".to_string(), json::num_val(cancelled));
                m.insert("reason".to_string(), json::str_val("usb.detached"));
                self.bus.publish(TOPIC_TASK, "task.batch_cancelled", Value::Obj(m));
            }
            return;
        }
        if prev.map(|p| p.state != next.state).unwrap_or(false) {
            // Mode flip while still attached (e.g. normal -> upgrade).
            self.cache.invalidate_many(&STATE_TOPICS);
            self.emit_event(TOPIC_MODEM, "modem.mode_changed", &next);
        }
    }

    /// Push the canonical `{present,state,path,available}` payload once.
    fn emit_state(&self, p: &ModemPresence) {
        let mut m = std::collections::BTreeMap::new();
        m.insert("present".to_string(), json::bool_val(p.present));
        m.insert("state".to_string(), json::str_val(&p.state));
        m.insert("available".to_string(), json::bool_val(p.available()));
        if let Some(path) = &p.path {
            m.insert("path".to_string(), json::str_val(path));
        }
        m.insert("is_pcui".to_string(), json::bool_val(p.is_pcui));
        self.bus.publish(TOPIC_USB, "usb.state", Value::Obj(m));
    }

    fn emit_event(&self, topic: &str, event: &str, p: &ModemPresence) {
        let mut m = std::collections::BTreeMap::new();
        m.insert("present".to_string(), json::bool_val(p.present));
        m.insert("state".to_string(), json::str_val(&p.state));
        m.insert("available".to_string(), json::bool_val(p.available()));
        if let Some(path) = &p.path {
            m.insert("path".to_string(), json::str_val(path));
        }
        self.bus.publish(topic, event, Value::Obj(m));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_on_host_is_quiet_and_absent() {
        // On the host there is usually no MT5700M attached; the scan must
        // not panic and must report absent.
        let p = ModemPresence::detect();
        assert!(!p.available());
        // present may be true if a QUECTEL device happens to be plugged in,
        // but the state machine must still produce a consistent value.
        assert_eq!(p.state.is_empty(), false);
    }

    #[test]
    fn availability_requires_normal_mode_and_port() {
        let ok = ModemPresence {
            present: true,
            state: "normal".into(),
            path: Some("/dev/ttyUSB1".into()),
            is_pcui: true,
        };
        assert!(ok.available());
        let upgrade = ModemPresence {
            state: "upgrade".into(),
            ..ok.clone()
        };
        assert!(!upgrade.available());
        let no_path = ModemPresence {
            path: None,
            ..ok.clone()
        };
        assert!(!no_path.available());
        let absent = ModemPresence {
            present: false,
            ..ok
        };
        assert!(!absent.available());
    }
}
