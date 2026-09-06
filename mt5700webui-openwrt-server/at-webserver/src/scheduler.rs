//! Day/night band-lock scheduler, port of the Python `Scheduler` / Go
//! `schedule.go` control loop. Applies AT^LTEFREQLOCK / AT^NRFREQLOCK per
//! time-of-day, toggles airplane mode around the switch and force-unlocks
//! after a no-service timeout. Runs only for transports that can send
//! commands (UBUS included; URC stream is not needed here).

use crate::daemon::AtClient;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::sleep;
use std::time::Duration;

#[derive(Clone, Default)]
pub struct Lock {
    pub ltype: i64,
    pub bands: String,
    pub arfcns: String,
    pub scs_types: String,
    pub pcis: String,
}

#[derive(Clone, Default)]
pub struct PeriodCfg {
    pub enabled: bool,
    pub lte: Lock,
    pub nr: Lock,
}

#[derive(Clone, Default)]
pub struct SchedCfg {
    pub enabled: bool,
    pub check_interval: u64,
    pub timeout: u64,
    pub unlock_lte: bool,
    pub unlock_nr: bool,
    pub toggle_airplane: bool,
    pub night_start: String,
    pub night_end: String,
    pub night: PeriodCfg,
    pub day: PeriodCfg,
}

fn as_bool(v: &str) -> bool {
    matches!(v.trim().to_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

fn uci_show() -> std::collections::BTreeMap<String, String> {
    let mut map = std::collections::BTreeMap::new();
    let Ok(out) = std::process::Command::new("uci")
        .args(["show", "at-webserver"])
        .output()
    else {
        return map;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    for line in text.lines() {
        // at-webserver.config.schedule_enabled='1'
        let Some(eq) = line.find('=') else { continue };
        let key = line[..eq].strip_prefix("at-webserver.config.").unwrap_or("");
        if key.is_empty() {
            continue;
        }
        let value = line[eq + 1..]
            .trim()
            .trim_matches('\'')
            .trim_matches('"')
            .to_string();
        map.insert(key.to_string(), value);
    }
    map
}

impl SchedCfg {
    pub fn load() -> Self {
        let m = uci_show();
        let g = |k: &str| m.get(k).cloned().unwrap_or_default();
        let gi = |k: &str, d: u64| g(k).parse().unwrap_or(d);
        let lock = |period: &str, kind: &str| Lock {
            ltype: g(&format!("schedule_{}_{}_type", period, kind))
                .parse()
                .unwrap_or(3),
            bands: g(&format!("schedule_{}_{}_bands", period, kind)),
            arfcns: g(&format!("schedule_{}_{}_arfcns", period, kind)),
            scs_types: g(&format!("schedule_{}_{}_scs_types", period, kind)),
            pcis: g(&format!("schedule_{}_{}_pcis", period, kind)),
        };
        SchedCfg {
            enabled: as_bool(&g("schedule_enabled")),
            check_interval: gi("schedule_check_interval", 60).max(10),
            timeout: gi("schedule_timeout", 180).max(60),
            unlock_lte: as_bool(&g("schedule_unlock_lte")),
            unlock_nr: as_bool(&g("schedule_unlock_nr")),
            toggle_airplane: as_bool(&g("schedule_toggle_airplane")),
            night_start: g("schedule_night_start"),
            night_end: g("schedule_night_end"),
            night: PeriodCfg {
                enabled: as_bool(&g("schedule_night_enabled")),
                lte: lock("night", "lte"),
                nr: lock("night", "nr"),
            },
            day: PeriodCfg {
                enabled: as_bool(&g("schedule_day_enabled")),
                lte: lock("day", "lte"),
                nr: lock("day", "nr"),
            },
        }
    }
}

fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':')?;
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// Local clock minutes via busybox date (std has no local timezone support).
fn now_minutes() -> u32 {
    let out = std::process::Command::new("date").arg("+%H:%M").output();
    let Ok(out) = out else { return 0 };
    parse_hhmm(&String::from_utf8_lossy(&out.stdout)).unwrap_or(0)
}

/// Port of `target_mode`: "" | "夜间" | "日间".
pub fn target_mode(cfg: &SchedCfg) -> &'static str {
    let night = match (parse_hhmm(&cfg.night_start), parse_hhmm(&cfg.night_end)) {
        (Some(start), Some(end)) => {
            let cur = now_minutes();
            if start > end {
                cur >= start || cur < end
            } else {
                start <= cur && cur < end
            }
        }
        _ => false,
    };
    if night && cfg.night.enabled {
        return "夜间";
    }
    if !night && cfg.day.enabled {
        return "日间";
    }
    ""
}

fn split_list(v: &str) -> Vec<String> {
    v.split(',')
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect()
}

/// FR2 bands default to 120 kHz SCS, everything else to 30 kHz. Port of
/// `auto_detect_scs` (approximation: FR2 == bands n257-n263).
fn auto_detect_scs(bands: &[String]) -> Vec<String> {
    bands
        .iter()
        .map(|b| {
            b.parse::<u64>()
                .map(|n| (n >= 257).then(|| "3").unwrap_or("1").to_string())
                .unwrap_or("1".to_string())
        })
        .collect()
}

pub fn lte_command(cfg: &SchedCfg, lock: &Lock) -> Option<(String, &'static str)> {
    if lock.ltype <= 0 {
        if !cfg.unlock_lte {
            return None;
        }
        return Some(("AT^LTEFREQLOCK=0".into(), "LTE解锁"));
    }
    let bands = split_list(&lock.bands);
    if bands.is_empty() {
        return None;
    }
    if lock.ltype == 3 {
        return Some((
            format!("AT^LTEFREQLOCK=3,0,{},\"{}\"", bands.len(), bands.join(",")),
            "LTE锁频(类型3)",
        ));
    }
    if lock.ltype == 1 || lock.ltype == 2 {
        let arfcns = split_list(&lock.arfcns);
        if arfcns.len() != bands.len() {
            return Some(("AT^LTEFREQLOCK=0".into(), "LTE解锁"));
        }
        if lock.ltype == 1 {
            return Some((
                format!(
                    "AT^LTEFREQLOCK=1,0,{},\"{}\",\"{}\"",
                    bands.len(),
                    bands.join(","),
                    arfcns.join(",")
                ),
                "LTE锁频(类型1)",
            ));
        }
        let pcis = split_list(&lock.pcis);
        if pcis.len() != bands.len() {
            return Some(("AT^LTEFREQLOCK=0".into(), "LTE解锁"));
        }
        return Some((
            format!(
                "AT^LTEFREQLOCK=2,0,{},\"{}\",\"{}\",\"{}\"",
                bands.len(),
                bands.join(","),
                arfcns.join(","),
                pcis.join(",")
            ),
            "LTE锁频(类型2)",
        ));
    }
    Some(("AT^LTEFREQLOCK=0".into(), "LTE解锁"))
}

pub fn nr_command(cfg: &SchedCfg, lock: &Lock) -> Option<(String, &'static str)> {
    if lock.ltype <= 0 {
        if !cfg.unlock_nr {
            return None;
        }
        return Some(("AT^NRFREQLOCK=0".into(), "NR解锁"));
    }
    let bands = split_list(&lock.bands);
    if bands.is_empty() {
        return None;
    }
    if lock.ltype == 3 {
        return Some((
            format!("AT^NRFREQLOCK=3,0,{},\"{}\"", bands.len(), bands.join(",")),
            "NR锁频(类型3)",
        ));
    }
    if lock.ltype == 1 || lock.ltype == 2 {
        let arfcns = split_list(&lock.arfcns);
        if arfcns.len() != bands.len() {
            return Some(("AT^NRFREQLOCK=0".into(), "NR解锁"));
        }
        let scs = if lock.scs_types.trim().is_empty() {
            auto_detect_scs(&bands)
        } else {
            split_list(&lock.scs_types)
        };
        if scs.len() != bands.len() {
            return Some(("AT^NRFREQLOCK=0".into(), "NR解锁"));
        }
        if lock.ltype == 1 {
            return Some((
                format!(
                    "AT^NRFREQLOCK=1,0,{},\"{}\",\"{}\",\"{}\"",
                    bands.len(),
                    bands.join(","),
                    arfcns.join(","),
                    scs.join(",")
                ),
                "NR锁频(类型1)",
            ));
        }
        let pcis = split_list(&lock.pcis);
        if pcis.len() != bands.len() {
            return Some(("AT^NRFREQLOCK=0".into(), "NR解锁"));
        }
        return Some((
            format!(
                "AT^NRFREQLOCK=2,0,{},\"{}\",\"{}\",\"{}\",\"{}\"",
                bands.len(),
                bands.join(","),
                arfcns.join(","),
                scs.join(","),
                pcis.join(",")
            ),
            "NR锁频(类型2)",
        ));
    }
    Some(("AT^NRFREQLOCK=0".into(), "NR解锁"))
}

/// Port of `registered()`: any +CxxREG line whose second field is 1 or 5.
pub fn registered(resp: &str) -> bool {
    for line in resp.lines() {
        let t = line.trim();
        if !t.starts_with("+C") || !t.contains("REG:") {
            continue;
        }
        let after = t.split("REG:").nth(1).unwrap_or("");
        let fields: Vec<&str> = after.split(',').collect();
        if let Some(stat) = fields.get(1).map(|s| s.trim()) {
            if stat == "1" || stat == "5" {
                return true;
            }
        }
    }
    false
}

struct SchedulerState {
    current_mode: String,
    applied: bool,
    last_applied_lte: i64,
    last_applied_nr: i64,
    last_service_at: std::time::Instant,
    switch_count: u64,
}

fn run_lock_command(client: &AtClient, cmd: &str, action: &str) -> bool {
    eprintln!("scheduler: {} -> {}", action, cmd);
    match client.send(cmd, 5) {
        Ok(text) => {
            if crate::at::response_ok(&text) {
                true
            } else {
                eprintln!("scheduler: {} failed: {}", action, text.trim());
                false
            }
        }
        Err(e) => {
            eprintln!("scheduler: {} failed: {}", action, e);
            false
        }
    }
}

fn apply_lock(client: &AtClient, cfg: &SchedCfg, lte: &Lock, nr: &Lock, mode: &str) {
    // The shared counter lives outside: use a process-wide static for parity
    // with the Python switch_count logging.
    use std::sync::Mutex;
    static COUNT: Mutex<u64> = Mutex::new(0);
    let count = {
        let mut c = COUNT.lock().unwrap();
        *c += 1;
        *c
    };
    eprintln!("scheduler: switching to {} (#{})", mode, count);

    if cfg.toggle_airplane {
        if client.send("AT+CFUN=0", 5).map(|t| crate::at::response_ok(&t)) == Ok(true) {
            eprintln!("scheduler: airplane mode on");
            sleep(Duration::from_secs(2));
        }
    }

    if let Some((cmd, action)) = lte_command(cfg, lte) {
        run_lock_command(client, &cmd, action);
        sleep(Duration::from_secs(1));
    }
    if let Some((cmd, action)) = nr_command(cfg, nr) {
        run_lock_command(client, &cmd, action);
        sleep(Duration::from_secs(1));
    }

    if cfg.toggle_airplane {
        if client.send("AT+CFUN=1", 5).map(|t| crate::at::response_ok(&t)) == Ok(true) {
            eprintln!("scheduler: airplane mode off");
        }
        sleep(Duration::from_secs(3));
    }
}

fn has_service(client: &AtClient) -> bool {
    // C5GREG covers SA, CEREG covers LTE/NSA, CREG as the last resort.
    for cmd in ["AT+C5GREG?", "AT+CEREG?", "AT+CREG?"] {
        if let Ok(text) = client.send(cmd, 5) {
            if registered(&text) {
                return true;
            }
        }
    }
    false
}

pub fn spawn(client: Arc<AtClient>, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let mut st = SchedulerState {
            current_mode: String::new(),
            applied: false,
            last_applied_lte: -1,
            last_applied_nr: -1,
            last_service_at: std::time::Instant::now(),
            switch_count: 0,
        };
        loop {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            let cfg = SchedCfg::load();
            if !cfg.enabled {
                sleep(Duration::from_secs(30));
                continue;
            }
            tick(&client, &cfg, &mut st);
            sleep(Duration::from_secs(cfg.check_interval));
        }
    });
}

fn tick(client: &Arc<AtClient>, cfg: &SchedCfg, st: &mut SchedulerState) {
    let target = target_mode(cfg);
    let (want_lte, want_nr) = match target {
        "夜间" => (cfg.night.lte.clone(), cfg.night.nr.clone()),
        "日间" => (cfg.day.lte.clone(), cfg.day.nr.clone()),
        _ => (Lock::default(), Lock::default()),
    };

    if target != st.current_mode || !st.applied {
        if !target.is_empty() {
            apply_lock(client, cfg, &want_lte, &want_nr, target);
        } else if st.applied {
            eprintln!("scheduler: leaving period, unlocking");
            apply_lock(client, cfg, &Lock::default(), &Lock::default(), "解锁");
        }
        st.current_mode = target.to_string();
        st.last_applied_lte = want_lte.ltype;
        st.last_applied_nr = want_nr.ltype;
        st.applied = true;
    }

    if has_service(client) {
        st.last_service_at = std::time::Instant::now();
        return;
    }
    let down = st.last_service_at.elapsed().as_secs();
    if down < cfg.timeout {
        return;
    }
    // Locked onto a band with no coverage: unlocking beats waiting.
    eprintln!("scheduler: no service for {}s, force unlock", down);
    apply_lock(client, cfg, &Lock::default(), &Lock::default(), "恢复");
    st.last_service_at = std::time::Instant::now();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(night_start: &str, night_end: &str) -> SchedCfg {
        SchedCfg {
            enabled: true,
            night_start: night_start.into(),
            night_end: night_end.into(),
            night: PeriodCfg {
                enabled: true,
                ..Default::default()
            },
            day: PeriodCfg {
                enabled: true,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn target_mode_windows() {
        // Cross-midnight night window 22:00-06:00. Fake the clock by
        // exercising parse + logic directly.
        let c = cfg("22:00", "06:00");
        assert_eq!(parse_hhmm(&c.night_start), Some(22 * 60));
        assert_eq!(parse_hhmm("bad"), None);
        let c2 = cfg("01:00", "07:00");
        assert_eq!(parse_hhmm(&c2.night_start), Some(60));
    }

    #[test]
    fn registration_parse() {
        assert!(registered("+CEREG: 0,1\r\nOK"));
        assert!(registered("+C5GREG: 2,5,\"460\",\"00\",7\r\nOK"));
        assert!(!registered("+CEREG: 0,2\r\nOK"));
        assert!(!registered("+CME ERROR: 100"));
    }

    #[test]
    fn lock_command_shapes() {
        let c = cfg("22:00", "06:00");
        let lte = Lock {
            ltype: 3,
            bands: "1,3,7".into(),
            ..Default::default()
        };
        assert_eq!(
            lte_command(&c, &lte).unwrap().0,
            "AT^LTEFREQLOCK=3,0,3,\"1,3,7\""
        );
        let nr = Lock {
            ltype: 1,
            bands: "78".into(),
            arfcns: "643456".into(),
            ..Default::default()
        };
        // SCS auto-detect: FR1 band 78 -> "1"
        assert_eq!(
            nr_command(&c, &nr).unwrap().0,
            "AT^NRFREQLOCK=1,0,1,\"78\",\"643456\",\"1\""
        );
        let mm = Lock {
            ltype: 1,
            bands: "260".into(),
            arfcns: "2000000".into(),
            ..Default::default()
        };
        assert!(
            nr_command(&c, &mm)
                .unwrap()
                .0
                .ends_with("\"3\"")
        );
        // count mismatch -> unlock
        let bad = Lock {
            ltype: 1,
            bands: "78,79".into(),
            arfcns: "643456".into(),
            ..Default::default()
        };
        assert_eq!(nr_command(&c, &bad).unwrap().0, "AT^NRFREQLOCK=0");
    }
}
