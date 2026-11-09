//! Traffic accounting — Rust/Tokio port of `usr/sbin/mt5700m-traffic`.
//!
//! One writer, one format, one JSON shape:
//!
//! ```text
//! version 1
//! updated eth2 2026-11-09 12:34
//! total eth2 123456 7890
//! day eth2 2026-11-09 1000 200
//! month eth2 2026-11 50000 9000
//! ```
//!
//! * the runtime copy lives on tmpfs (`/tmp/mt5700m-traffic/history`) and is
//!   flushed to `/etc/mt5700m/traffic-history` every N cycles, so a power cut
//!   loses at most one flush window instead of wearing the flash on every
//!   sample;
//! * counters come from `/sys/class/net/<dev>/statistics` (zero AT traffic);
//! * `json` is the contract the Rust netrate collector and both frontends read,
//!   so its shape must stay byte-compatible with the shell implementation.
//!
//! CLI (same verbs the init script / rpcd plugin use):
//!   `mt5700m-traffic daemon`  — sample every `MT5700M_TRAFFIC_INTERVAL`s
//!   `mt5700m-traffic json`    — one accounting report on stdout
//!   `mt5700m-traffic flush`   — force the runtime copy to persistent storage

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::core::sys;

/* ---------------------------------------------------------------- config */

#[derive(Debug, Clone)]
pub struct Config {
    pub interfaces: Vec<String>,
    pub sys_class_net: PathBuf,
    pub runtime_dir: PathBuf,
    pub history_file: PathBuf,
    pub interval: u64,
    pub flush_cycles: u32,
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| default.to_string())
}

/// Same precedence as the shell script: explicit env, manager-published
/// netdev, legacy `eth2`.
fn default_interfaces() -> String {
    if let Ok(raw) = std::env::var("MT5700M_TRAFFIC_INTERFACES") {
        if !raw.trim().is_empty() {
            return raw;
        }
    }
    fs::read_to_string("/var/run/mt5700m/netdev")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "eth2".to_string())
}

impl Config {
    pub fn from_env() -> Self {
        let interval = env_or("MT5700M_TRAFFIC_INTERVAL", "30")
            .parse::<u64>()
            .unwrap_or(30)
            .max(1);
        let flush_cycles = env_or("MT5700M_TRAFFIC_FLUSH_CYCLES", "20")
            .parse::<u32>()
            .unwrap_or(20)
            .max(1);
        Self {
            interfaces: default_interfaces()
                .split_whitespace()
                .map(|s| s.to_string())
                .collect(),
            sys_class_net: PathBuf::from(env_or("MT5700M_TRAFFIC_SYS_CLASS_NET", "/sys/class/net")),
            runtime_dir: PathBuf::from(env_or(
                "MT5700M_TRAFFIC_RUNTIME_DIR",
                "/tmp/mt5700m-traffic",
            )),
            history_file: PathBuf::from(env_or(
                "MT5700M_TRAFFIC_HISTORY_FILE",
                "/etc/mt5700m/traffic-history",
            )),
            interval,
            flush_cycles,
        }
    }

    fn runtime_file(&self) -> PathBuf {
        self.runtime_dir.join("history")
    }

    fn last_file(&self, dev: &str) -> PathBuf {
        self.runtime_dir.join(format!("last-{}", dev))
    }
}

/* --------------------------------------------------------------- history */

#[derive(Debug, Clone, PartialEq)]
enum Line {
    Version(String),
    Updated {
        dev: String,
        date: String,
        time: String,
    },
    Total {
        dev: String,
        rx: u64,
        tx: u64,
    },
    Day {
        dev: String,
        date: String,
        rx: u64,
        tx: u64,
    },
    Month {
        dev: String,
        month: String,
        rx: u64,
        tx: u64,
    },
    Other(String),
}

#[derive(Debug, Clone, Default)]
pub struct History {
    lines: Vec<Line>,
}

impl History {
    fn parse(text: &str) -> Self {
        let mut lines = Vec::new();
        for raw in text.lines() {
            let mut parts = raw.split_whitespace();
            let line = match parts.next() {
                Some("version") => Line::Version(parts.next().unwrap_or("1").to_string()),
                Some("updated") => match (parts.next(), parts.next(), parts.next()) {
                    (Some(dev), Some(date), Some(time)) => Line::Updated {
                        dev: dev.to_string(),
                        date: date.to_string(),
                        time: time.to_string(),
                    },
                    _ => Line::Other(raw.to_string()),
                },
                Some("total") => match (parts.next(), parts.next(), parts.next()) {
                    (Some(dev), Some(rx), Some(tx)) => match (rx.parse(), tx.parse()) {
                        (Ok(rx), Ok(tx)) => Line::Total {
                            dev: dev.to_string(),
                            rx,
                            tx,
                        },
                        _ => Line::Other(raw.to_string()),
                    },
                    _ => Line::Other(raw.to_string()),
                },
                Some("day") => match (parts.next(), parts.next(), parts.next(), parts.next()) {
                    (Some(dev), Some(date), Some(rx), Some(tx)) => match (rx.parse(), tx.parse()) {
                        (Ok(rx), Ok(tx)) => Line::Day {
                            dev: dev.to_string(),
                            date: date.to_string(),
                            rx,
                            tx,
                        },
                        _ => Line::Other(raw.to_string()),
                    },
                    _ => Line::Other(raw.to_string()),
                },
                Some("month") => match (parts.next(), parts.next(), parts.next(), parts.next()) {
                    (Some(dev), Some(month), Some(rx), Some(tx)) => match (rx.parse(), tx.parse()) {
                        (Ok(rx), Ok(tx)) => Line::Month {
                            dev: dev.to_string(),
                            month: month.to_string(),
                            rx,
                            tx,
                        },
                        _ => Line::Other(raw.to_string()),
                    },
                    _ => Line::Other(raw.to_string()),
                },
                _ => Line::Other(raw.to_string()),
            };
            lines.push(line);
        }
        Self { lines }
    }

    fn serialize(&self) -> String {
        let mut out = String::new();
        for line in &self.lines {
            match line {
                Line::Version(v) => out.push_str(&format!("version {}\n", v)),
                Line::Updated { dev, date, time } => {
                    out.push_str(&format!("updated {} {} {}\n", dev, date, time))
                }
                Line::Total { dev, rx, tx } => {
                    out.push_str(&format!("total {} {} {}\n", dev, rx, tx))
                }
                Line::Day { dev, date, rx, tx } => {
                    out.push_str(&format!("day {} {} {} {}\n", dev, date, rx, tx))
                }
                Line::Month { dev, month, rx, tx } => {
                    out.push_str(&format!("month {} {} {} {}\n", dev, month, rx, tx))
                }
                Line::Other(raw) => {
                    out.push_str(raw);
                    out.push('\n');
                }
            }
        }
        out
    }

    /// Same pass as the shell's `awk`: update the device's rows in place and
    /// append whatever was missing, so a hand-edited file keeps its shape.
    fn apply_sample(&mut self, dev: &str, rx: u64, tx: u64, date: &str, time: &str) {
        let month = date.get(0..7).unwrap_or(date).to_string();
        let mut have_updated = false;
        let mut have_total = false;
        let mut have_day = false;
        let mut have_month = false;

        for line in self.lines.iter_mut() {
            match line {
                Line::Updated {
                    dev: d,
                    date: dd,
                    time: tt,
                } if d == dev => {
                    *dd = date.to_string();
                    *tt = time.to_string();
                    have_updated = true;
                }
                Line::Total { dev: d, rx: r, tx: t } if d == dev => {
                    *r += rx;
                    *t += tx;
                    have_total = true;
                }
                Line::Day {
                    dev: d,
                    date: dd,
                    rx: r,
                    tx: t,
                } if d == dev && dd == date => {
                    *r += rx;
                    *t += tx;
                    have_day = true;
                }
                Line::Month {
                    dev: d,
                    month: mm,
                    rx: r,
                    tx: t,
                } if d == dev && mm == &month => {
                    *r += rx;
                    *t += tx;
                    have_month = true;
                }
                _ => {}
            }
        }

        if !have_updated {
            self.lines.push(Line::Updated {
                dev: dev.to_string(),
                date: date.to_string(),
                time: time.to_string(),
            });
        }
        if !have_total {
            self.lines.push(Line::Total {
                dev: dev.to_string(),
                rx,
                tx,
            });
        }
        if !have_day {
            self.lines.push(Line::Day {
                dev: dev.to_string(),
                date: date.to_string(),
                rx,
                tx,
            });
        }
        if !have_month {
            self.lines.push(Line::Month {
                dev: dev.to_string(),
                month,
                rx,
                tx,
            });
        }
    }
}

/* ------------------------------------------------------------ file layer */

fn read_text(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

fn write_atomic(path: &Path, body: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(body.as_bytes())?;
        f.sync_all().ok();
    }
    fs::rename(&tmp, path)
}

fn ensure_history(cfg: &Config) -> History {
    let runtime = cfg.runtime_file();
    if let Some(text) = read_text(&runtime) {
        return History::parse(&text);
    }
    let mut history = if let Some(text) = read_text(&cfg.history_file) {
        if !text.trim().is_empty() {
            History::parse(&text)
        } else {
            History::parse("")
        }
    } else {
        History::parse("")
    };
    if history.lines.is_empty() {
        history.lines.push(Line::Version("1".to_string()));
        for dev in &cfg.interfaces {
            history.lines.push(Line::Updated {
                dev: dev.clone(),
                date: "1970-01-01".to_string(),
                time: "00:00".to_string(),
            });
            history.lines.push(Line::Total {
                dev: dev.clone(),
                rx: 0,
                tx: 0,
            });
        }
    }
    if let Some(parent) = runtime.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = write_atomic(&runtime, &history.serialize());
    history
}

fn flush_history(cfg: &Config) {
    let history = ensure_history(cfg);
    let _ = write_atomic(&cfg.history_file, &history.serialize());
}

/* ---------------------------------------------------------------- report */

fn json_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn date_parts(date: &str) -> (i64, i64, i64) {
    let mut it = date.split('-');
    let y = it.next().and_then(|v| v.parse().ok()).unwrap_or(1970);
    let m = it.next().and_then(|v| v.parse().ok()).unwrap_or(1);
    let d = it.next().and_then(|v| v.parse().ok()).unwrap_or(1);
    (y, m, d)
}

fn time_parts(time: &str) -> (i64, i64) {
    let mut it = time.split(':');
    let h = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    let m = it.next().and_then(|v| v.parse().ok()).unwrap_or(0);
    (h, m)
}

fn interface_json(history: &History, dev: &str) -> String {
    let mut date = String::from("1970-01-01");
    let mut time = String::from("00:00");
    let mut rx_total = 0u64;
    let mut tx_total = 0u64;
    let mut days: Vec<(String, u64, u64)> = Vec::new();
    let mut months: Vec<(String, u64, u64)> = Vec::new();

    for line in &history.lines {
        match line {
            Line::Updated {
                dev: d,
                date: dd,
                time: tt,
            } if d == dev => {
                date = dd.clone();
                time = tt.clone();
            }
            Line::Total { dev: d, rx, tx } if d == dev => {
                rx_total = *rx;
                tx_total = *tx;
            }
            Line::Day {
                dev: d,
                date: dd,
                rx,
                tx,
            } if d == dev => days.push((dd.clone(), *rx, *tx)),
            Line::Month {
                dev: d,
                month,
                rx,
                tx,
            } if d == dev => months.push((month.clone(), *rx, *tx)),
            _ => {}
        }
    }

    let (y, m, d) = date_parts(&date);
    let (hh, mm) = time_parts(&time);
    let mut out = String::new();
    out.push_str(&format!(
        "{{\"name\":\"{}\",\"updated\":{{\"date\":{{\"year\":{},\"month\":{},\"day\":{}}},\"time\":{{\"hour\":{},\"minute\":{}}}}},\"traffic\":{{\"total\":{{\"rx\":{},\"tx\":{}}},\"day\":[",
        json_escape(dev), y, m, d, hh, mm, rx_total, tx_total
    ));
    for (index, (date, rx, tx)) in days.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let (y, m, d) = date_parts(date);
        out.push_str(&format!(
            "{{\"date\":{{\"year\":{},\"month\":{},\"day\":{}}},\"rx\":{},\"tx\":{}}}",
            y, m, d, rx, tx
        ));
    }
    out.push_str("],\"month\":[");
    for (index, (month, rx, tx)) in months.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let (y, m, _) = date_parts(&format!("{}-01", month));
        out.push_str(&format!(
            "{{\"date\":{{\"year\":{},\"month\":{}}},\"rx\":{},\"tx\":{}}}",
            y, m, rx, tx
        ));
    }
    out.push_str("]}}");
    out
}

pub fn export_json(cfg: &Config) -> String {
    let history = ensure_history(cfg);
    let mut out = String::from("{\"interfaces\":[");
    for (index, dev) in cfg.interfaces.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str(&interface_json(&history, dev));
    }
    out.push_str("]}\n");
    out
}

/* ------------------------------------------------------------- collector */

fn read_counter(path: &Path) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse::<u64>().ok()
}

fn read_last(path: &Path) -> Option<(u64, u64)> {
    let text = fs::read_to_string(path).ok()?;
    let mut it = text.split_whitespace();
    let rx = it.next()?.parse::<u64>().ok()?;
    let tx = it.next()?.parse::<u64>().ok()?;
    Some((rx, tx))
}

fn local_now() -> (String, String) {
    let out = sys::run("date", &["+%Y-%m-%d %H:%M"], Duration::from_secs(2));
    if out.ok {
        let text = out.stdout.trim();
        if let Some((date, time)) = text.split_once(' ') {
            if !date.is_empty() && !time.is_empty() {
                return (date.to_string(), time.to_string());
            }
        }
    }
    (String::from("1970-01-01"), String::from("00:00"))
}

/// Returns true when the history changed (the caller then flushes).
fn collect_interface(cfg: &Config, dev: &str, history: &mut History) -> bool {
    let base = cfg.sys_class_net.join(dev).join("statistics");
    let Some(rx_now) = read_counter(&base.join("rx_bytes")) else {
        let _ = fs::remove_file(cfg.last_file(dev));
        return false;
    };
    let Some(tx_now) = read_counter(&base.join("tx_bytes")) else {
        let _ = fs::remove_file(cfg.last_file(dev));
        return false;
    };

    let last = read_last(&cfg.last_file(dev));
    let mut dirty = false;
    if let Some((last_rx, last_tx)) = last {
        let rx_delta = if rx_now >= last_rx { rx_now - last_rx } else { rx_now };
        let tx_delta = if tx_now >= last_tx { tx_now - last_tx } else { tx_now };
        let (date, time) = local_now();
        let year = date
            .get(0..4)
            .and_then(|v| v.parse::<i64>().ok())
            .unwrap_or(0);
        if year >= 2024 && (rx_delta > 0 || tx_delta > 0) {
            history.apply_sample(dev, rx_delta, tx_delta, &date, &time);
            dirty = true;
        }
    }
    let _ = fs::create_dir_all(&cfg.runtime_dir);
    let _ = write_atomic(
        &cfg.last_file(dev),
        &format!("{} {}\n", rx_now, tx_now),
    );
    dirty
}

/* ------------------------------------------------------------------- cli */

fn daemon(cfg: Config) -> i32 {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("traffic: cannot start tokio runtime: {}", err);
            return 1;
        }
    };

    rt.block_on(async move {
        let mut history = ensure_history(&cfg);
        let mut cycles: u32 = 0;
        let mut dirty = false;

        loop {
            for dev in cfg.interfaces.clone() {
                if collect_interface(&cfg, &dev, &mut history) {
                    dirty = true;
                }
            }
            cycles = cycles.saturating_add(1);
            if cycles >= cfg.flush_cycles {
                if dirty {
                    flush_history(&cfg);
                    dirty = false;
                }
                cycles = 0;
            }

            // procd stops us with SIGTERM and then runs `traffic flush`; the
            // loop itself only needs the async timer, no signal driver.
            tokio::time::sleep(Duration::from_secs(cfg.interval)).await;
        }
    })
}

pub fn run(args: &[String]) -> i32 {
    let cfg = Config::from_env();
    match args.first().map(|s| s.as_str()) {
        Some("daemon") => daemon(cfg),
        Some("json") => {
            print!("{}", export_json(&cfg));
            0
        }
        Some("flush") => {
            flush_history(&cfg);
            0
        }
        _ => {
            eprintln!("Usage: at-webserver traffic {{daemon|json|flush}}");
            64
        }
    }
}

/* ----------------------------------------------------------------- tests */

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_history() -> History {
        History::parse(
            "version 1\nupdated eth2 2026-11-09 10:00\ntotal eth2 100 200\nday eth2 2026-11-09 10 20\nmonth eth2 2026-11 100 200\n",
        )
    }

    #[test]
    fn sample_is_applied_in_place() {
        let mut history = sample_history();
        history.apply_sample("eth2", 5, 7, "2026-11-09", "10:30");
        let text = history.serialize();
        assert!(text.contains("updated eth2 2026-11-09 10:30"));
        assert!(text.contains("total eth2 105 207"));
        assert!(text.contains("day eth2 2026-11-09 15 27"));
        assert!(text.contains("month eth2 2026-11 105 207"));
    }

    #[test]
    fn new_day_and_month_are_appended() {
        let mut history = sample_history();
        history.apply_sample("eth2", 1, 2, "2026-11-10", "00:05");
        let text = history.serialize();
        assert!(text.contains("day eth2 2026-11-10 1 2"));
        assert!(text.contains("month eth2 2026-11 101 202"));
        // still one month row for 2026-11
        assert_eq!(text.matches("month eth2 2026-11 ").count(), 1);
    }

    #[test]
    fn unknown_lines_survive() {
        let mut history = History::parse("version 1\n# keep me\n");
        history.apply_sample("eth2", 1, 2, "2026-11-09", "10:00");
        assert!(history.serialize().contains("# keep me"));
    }

    #[test]
    fn json_shape_matches_the_shell_contract() {
        let history = sample_history();
        let json = interface_json(&history, "eth2");
        assert!(json.starts_with("{\"name\":\"eth2\",\"updated\":{\"date\":{\"year\":2026,\"month\":11,\"day\":9},\"time\":{\"hour\":10,\"minute\":0}},\"traffic\":{"));
        assert!(json.contains("\"total\":{\"rx\":100,\"tx\":200}"));
        assert!(json.contains("\"day\":[{\"date\":{\"year\":2026,\"month\":11,\"day\":9},\"rx\":10,\"tx\":20}]"));
        assert!(json.contains("\"month\":[{\"date\":{\"year\":2026,\"month\":11},\"rx\":100,\"tx\":200}]"));
    }
}
