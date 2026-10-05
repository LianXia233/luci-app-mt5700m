//! State cache with TTL + Stale-While-Revalidate (SWR).
//!
//! Every state topic (signal, network, cell, sim, temperature, ...) is
//! cached here. Readers never block on the modem:
//!
//!   * fresh entry  -> return immediately,
//!   * expired entry -> return the stale value immediately AND hand back a
//!     "refresh" hint so the caller spawns a background refresh,
//!   * missing entry -> return `Unavailable` fast (modem absent / never
//!     collected) so pages still open instantly.
//!
//! Writers (`set`) replace entries; destructive operations (`invalidate`)
//! drop them so the next read triggers a refresh.

use crate::core::json::Value;
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Fresh,
    Stale,
    Updating,
    Unavailable,
}

impl Freshness {
    pub fn name(&self) -> &'static str {
        match self {
            Freshness::Fresh => "fresh",
            Freshness::Stale => "stale",
            Freshness::Updating => "updating",
            Freshness::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheEntry {
    pub value: Value,
    pub source: String,
    pub timestamp: Instant,
    pub ttl: Duration,
}

impl CacheEntry {
    pub fn fresh(value: Value, source: &str, ttl: Duration) -> Self {
        CacheEntry {
            value,
            source: source.to_string(),
            timestamp: Instant::now(),
            ttl,
        }
    }
}

pub struct StateCache {
    entries: RwLock<HashMap<String, CacheEntry>>,
}

/// Default per-topic TTLs (matched to the periodic refresh cadence).
///
/// TTL 必须 ≥ 采集周期，否则缓存永远 stale。周期按 2026-10-04 设备实测
/// 慢命令耗时放宽（见 snapshot.rs）：signal 15 s / registration 20 s /
/// network 30 s / cell 120 s / endc·txpower 300 s / nr_txpower 180 s。
/// temperature 特殊：mt5700m-manager 每 15 s 通过 `mt5700m-at temperature`
/// （缓存优先）刷新温度缓存，TTL 必须大到两次采集之间始终命中缓存，
/// 否则它会回退实时 AT^CHIPTEMP?（modem 上往返约 4 s）把独占串口占死，
/// 反过来饿死采集器，形成死循环。
pub fn default_ttl(topic: &str) -> Duration {
    match topic {
        "signal" => Duration::from_secs(18),
        "network" => Duration::from_secs(35),
        "registration" => Duration::from_secs(25),
        "temperature" => Duration::from_secs(600),
        "traffic" => Duration::from_secs(35),
        "cell" => Duration::from_secs(130),
        // 载波聚合按需刷新（三条慢命令），TTL 要覆盖两次点击之间。
        "ca" => Duration::from_secs(240),
        "endc" => Duration::from_secs(320),
        "txpower" => Duration::from_secs(320),
        "nr_txpower" => Duration::from_secs(200),
        "sim" => Duration::from_secs(70),
        "modem_info" => Duration::from_secs(75),
        "usb" => Duration::from_secs(8),
        // 采集器 5 s 一轮（纯 sysfs/文件读，零 AT 流量），TTL 略大于周期，
        // 保证前端读到的永远是最近一次采样而不 miss。
        "netrate" => Duration::from_secs(8),
        _ => Duration::from_secs(10),
    }
}

impl StateCache {
    pub fn new() -> Self {
        StateCache {
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Read an entry. Returns `(value, freshness)` — never blocks, never
    /// touches the modem. A `Stale`/`Unavailable` result means the caller
    /// should trigger a background refresh (SWR).
    pub fn get(&self, topic: &str) -> (Option<Value>, Freshness) {
        let entries = self.entries.read().unwrap();
        match entries.get(topic) {
            None => (None, Freshness::Unavailable),
            Some(e) if e.timestamp.elapsed() <= e.ttl => (Some(e.value.clone()), Freshness::Fresh),
            Some(e) => (Some(e.value.clone()), Freshness::Stale),
        }
    }

    /// Read the raw entry (value + age) without freshness computation.
    pub fn entry(&self, topic: &str) -> Option<CacheEntry> {
        self.entries.read().unwrap().get(topic).cloned()
    }

    /// Write a fresh value.
    pub fn set(&self, topic: &str, value: Value, source: &str) {
        let ttl = default_ttl(topic);
        self.entries
            .write()
            .unwrap()
            .insert(topic.to_string(), CacheEntry::fresh(value, source, ttl));
    }

    /// Write with an explicit TTL (e.g. long-lived modem info).
    pub fn set_ttl(&self, topic: &str, value: Value, source: &str, ttl: Duration) {
        self.entries.write().unwrap().insert(
            topic.to_string(),
            CacheEntry {
                value,
                source: source.to_string(),
                timestamp: Instant::now(),
                ttl,
            },
        );
    }

    /// Mark a topic as "currently being refreshed" without data (used by
    /// the SWR path so concurrent readers coalesce refreshes).
    pub fn mark_updating(&self, topic: &str) {
        let mut entries = self.entries.write().unwrap();
        let prev = entries.remove(topic);
        if let Some(mut e) = prev {
            e.ttl = Duration::from_millis(50); // very short: refresh lands soon
            entries.insert(topic.to_string(), e);
        }
    }

    /// Drop one topic so the next read triggers a fresh collection.
    pub fn invalidate(&self, topic: &str) {
        self.entries.write().unwrap().remove(topic);
    }

    /// Drop several topics (write operations like APN/PDP changes).
    pub fn invalidate_many(&self, topics: &[&str]) {
        let mut entries = self.entries.write().unwrap();
        for t in topics {
            entries.remove(*t);
        }
    }

    /// Drop everything (USB reset / modem re-appear).
    pub fn invalidate_all(&self) {
        self.entries.write().unwrap().clear();
    }

    /// All topic names currently held. Used by the read gate to invalidate
    /// raw entries after a write without having to enumerate command names.
    pub fn entries_snapshot(&self) -> Option<Vec<String>> {
        Some(self.entries.read().unwrap().keys().cloned().collect())
    }

    /// Full dump for the WS `snapshot` action / HTTP `/api/state`.
    /// Each topic: { "value": ..., "fresh": bool, "age_ms": ..., "source": ... }.
    pub fn snapshot(&self) -> Value {
        let entries = self.entries.read().unwrap();
        let mut map = std::collections::BTreeMap::new();
        for (topic, e) in entries.iter() {
            let fresh = e.timestamp.elapsed() <= e.ttl;
            let mut m = std::collections::BTreeMap::new();
            m.insert("value".to_string(), e.value.clone());
            m.insert("fresh".to_string(), Value::Bool(fresh));
            m.insert("age_ms".to_string(), crate::core::json::num_val(e.timestamp.elapsed().as_millis() as u64));
            m.insert("source".to_string(), crate::core::json::str_val(&e.source));
            map.insert(topic.clone(), Value::Obj(m));
        }
        Value::Obj(map)
    }
}

impl Default for StateCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(n: i64) -> Value {
        crate::core::json::num_val(n)
    }

    #[test]
    fn missing_is_unavailable() {
        let c = StateCache::new();
        let (v, f) = c.get("signal");
        assert!(v.is_none());
        assert_eq!(f, Freshness::Unavailable);
    }

    #[test]
    fn fresh_then_stale_after_ttl() {
        let c = StateCache::new();
        c.set_ttl("signal", num(-86), "test", Duration::from_millis(30));
        assert_eq!(c.get("signal").1, Freshness::Fresh);
        std::thread::sleep(Duration::from_millis(60));
        let (v, f) = c.get("signal");
        assert_eq!(f, Freshness::Stale);
        assert_eq!(v.and_then(|x| x.as_i64()), Some(-86)); // SWR: stale data still returned
    }

    #[test]
    fn invalidate_drops_entry() {
        let c = StateCache::new();
        c.set("sim", num(1), "test");
        c.invalidate("sim");
        assert_eq!(c.get("sim").1, Freshness::Unavailable);
    }

    #[test]
    fn invalidate_many_and_all() {
        let c = StateCache::new();
        c.set("signal", num(1), "t");
        c.set("network", num(2), "t");
        c.invalidate_many(&["signal", "network"]);
        assert_eq!(c.get("signal").1, Freshness::Unavailable);
        c.set("signal", num(1), "t");
        c.invalidate_all();
        assert_eq!(c.get("signal").1, Freshness::Unavailable);
    }

    #[test]
    fn snapshot_shape() {
        let c = StateCache::new();
        c.set("signal", num(-86), "collector");
        let dump = c.snapshot().dump();
        assert!(dump.contains("\"signal\""));
        assert!(dump.contains("\"fresh\":true"));
        assert!(dump.contains("\"source\":\"collector\""));
    }
}
