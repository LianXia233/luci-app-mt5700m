//! Topic-based event bus: the backbone of the event-driven backend.
//!
//! Producers (`publish`) hand events to the bus; consumers (`subscribe`)
//! receive only the topics they opted into. High-frequency telemetry
//! (signal, temperature, traffic, PDCP) is coalesced by the emitter so a
//! burst of identical state changes collapses to one event per ~100 ms
//! window — the UI never needs to poll and the backend never floods.
//!
//! Every event envelope follows:
//!   { "topic": "signal", "event": "signal.updated", "data": {...}, "timestamp": ... }

use crate::json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

/// 事件历史上限：为 LuCI RPC `events(since)` 增量拉取保留的最近事件数。
/// 轮询间隔 1.5 s，高并发时 500 足够覆盖一个窗口，避免内存无限增长。
const HISTORY_MAX: usize = 500;

pub const EMIT_TICK_MS: u64 = 100;

/// One event delivered to subscribers.
#[derive(Debug, Clone)]
pub struct Event {
    pub topic: String,
    pub event: String,
    pub data: Value,
    pub timestamp: u64,
}

impl Event {
    /// JSON envelope `{ "type": ..., "data": ..., "timestamp": ... }`.
    /// Kept in the same `{type,data}` shape the WebUI already consumes.
    pub fn to_json(&self) -> Value {
        let mut m = std::collections::BTreeMap::new();
        m.insert("type".to_string(), crate::json::str_val(&self.event));
        m.insert("data".to_string(), self.data.clone());
        m.insert("timestamp".to_string(), crate::json::num_val(self.timestamp));
        Value::Obj(m)
    }
}

/// Common topic names (stable strings shared by producers and consumers).
pub const TOPIC_SIGNAL: &str = "signal";
pub const TOPIC_NETWORK: &str = "network";
pub const TOPIC_CELL: &str = "cell";
pub const TOPIC_TEMPERATURE: &str = "temperature";
pub const TOPIC_TRAFFIC: &str = "traffic";
pub const TOPIC_SIM: &str = "sim";
pub const TOPIC_REGISTRATION: &str = "registration";
pub const TOPIC_USB: &str = "usb";
pub const TOPIC_MODEM: &str = "modem";
pub const TOPIC_TASK: &str = "task";
pub const TOPIC_SMS: &str = "sms";
pub const TOPIC_SCAN: &str = "scan";
pub const TOPIC_BEAM: &str = "beam";
pub const TOPIC_ENDC: &str = "endc";
pub const TOPIC_TXPOWER: &str = "txpower";
pub const TOPIC_NR_TXPOWER: &str = "nr_txpower";
/// Interface byte counters + the shared traffic history (see snapshot.rs
/// `collect_netrate`). Same physical source LuCI reads via
/// `mt5700m-traffic`, so both UIs agree on one number.
pub const TOPIC_NETRATE: &str = "netrate";
/// Prefix for the read-gate's per-command raw cache (`raw:AT+CPIN?`).
///
/// 前缀式设计而不是独立 topic：读命令有40+ 条且会持续新增，用固定
/// 集合枚举必然漏；加前缀后闸门对**任何**读命令自动适用，新增读点
/// 不需要改后端。
pub const TOPIC_RAW: &str = "raw:";

pub const DEFAULT_TOPICS: [&str; 17] = [
    TOPIC_SIGNAL,
    TOPIC_NETWORK,
    TOPIC_CELL,
    TOPIC_TEMPERATURE,
    TOPIC_TRAFFIC,
    TOPIC_SIM,
    TOPIC_REGISTRATION,
    TOPIC_ENDC,
    TOPIC_TXPOWER,
    TOPIC_NR_TXPOWER,
    TOPIC_NETRATE,
    TOPIC_USB,
    TOPIC_MODEM,
    TOPIC_TASK,
    TOPIC_SMS,
    TOPIC_SCAN,
    TOPIC_BEAM,
];

/// Topics that must be delivered immediately (no coalescing delay).
pub const IMMEDIATE_TOPICS: [&str; 5] = [TOPIC_TASK, TOPIC_USB, TOPIC_MODEM, TOPIC_SMS, TOPIC_SCAN];

struct Subscriber {
    id: u64,
    topics: RwLock<HashSet<String>>,
    tx: std::sync::mpsc::SyncSender<Event>,
}

/// Handle used to unsubscribe a session.
#[derive(Clone)]
pub struct Subscription {
    id: u64,
}

struct BusInner {
    subscribers: Vec<Arc<Subscriber>>,
    /// Coalesced per-topic pending events (latest wins).
    pending: HashMap<String, Event>,
}

/// 全局事件历史（RPC 增量拉取）：每次发布分配单调 seq，LuCI 通过
/// `events(since)` 取回自 since 之后的事件。WebSocket 走订阅推送，
/// 这里只服务请求-响应模型的前端（LuCI ucode -> RPC）。
struct HistoryState {
    seq: u64,
    events: VecDeque<(u64, Value)>,
}

pub struct EventBus {
    inner: Mutex<BusInner>,
    next_sub: AtomicU64,
    stop: Arc<std::sync::atomic::AtomicBool>,
    history: Mutex<HistoryState>,
}

impl EventBus {
    pub fn new() -> Arc<Self> {
        let bus = Arc::new(EventBus {
            inner: Mutex::new(BusInner {
                subscribers: Vec::new(),
                pending: HashMap::new(),
            }),
            next_sub: AtomicU64::new(1),
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            history: Mutex::new(HistoryState {
                seq: 0,
                events: VecDeque::new(),
            }),
        });
        let ticker = bus.clone();
        crate::runtime::spawn_thread("event-bus", move || {
            let tick = Duration::from_millis(EMIT_TICK_MS);
            loop {
                if ticker.stop.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(tick);
                ticker.flush_pending();
            }
        });
        bus
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Subscribe to a set of topics. Returns a receiver plus a handle to
    /// later unsubscribe. The receiver is bounded (backpressure: the bus
    /// drops the *oldest* events for slow consumers; coalescing keeps
    /// volume low).
    pub fn subscribe(&self, topics: &[String]) -> (std::sync::mpsc::Receiver<Event>, Subscription) {
        let (tx, rx) = std::sync::mpsc::sync_channel(128);
        let id = self.next_sub.fetch_add(1, Ordering::Relaxed);
        let sub = Arc::new(Subscriber {
            id,
            topics: RwLock::new(topics.iter().cloned().collect()),
            tx,
        });
        self.inner.lock().unwrap().subscribers.push(sub);
        (rx, Subscription { id })
    }

    /// Add topics to an existing subscription.
    pub fn add_topics(&self, sub: &Subscription, topics: &[String]) {
        let subs = self.inner.lock().unwrap().subscribers.clone();
        for s in subs {
            if s.id == sub.id {
                let mut set = s.topics.write().unwrap();
                for t in topics {
                    set.insert(t.clone());
                }
            }
        }
    }

    /// Remove topics from an existing subscription (idempotent).
    pub fn remove_topics(&self, sub: &Subscription, topics: &[String]) {
        let subs = self.inner.lock().unwrap().subscribers.clone();
        for s in subs {
            if s.id == sub.id {
                let mut set = s.topics.write().unwrap();
                for t in topics {
                    set.remove(t);
                }
            }
        }
    }

    /// Remove a subscription (idempotent).
    pub fn unsubscribe(&self, sub: &Subscription) {
        let mut inner = self.inner.lock().unwrap();
        inner.subscribers.retain(|s| s.id != sub.id);
    }

    /// Publish an event. `coalesce=true` (default for telemetry) merges
    /// bursts within one emitter tick; `coalesce=false` delivers now.
    pub fn publish(&self, topic: &str, event: &str, data: Value) {
        let ev = Event {
            topic: topic.to_string(),
            event: event.to_string(),
            data,
            timestamp: crate::runtime::now_secs(),
        };
        self.record(&ev);
        let immediate = IMMEDIATE_TOPICS.contains(&topic);
        if immediate {
            self.deliver(&ev);
        } else {
            self.inner.lock().unwrap().pending.insert(topic.to_string(), ev);
        }
    }

    /// Publish with explicit coalescing choice (callers that know the
    /// event rate can opt out of the tick delay).
    pub fn publish_now(&self, topic: &str, event: &str, data: Value) {
        let ev = Event {
            topic: topic.to_string(),
            event: event.to_string(),
            data,
            timestamp: crate::runtime::now_secs(),
        };
        self.record(&ev);
        self.deliver(&ev);
    }

    /// 将事件写入全局历史（单调 seq），供 RPC `events(since)` 增量拉取。
    fn record(&self, ev: &Event) {
        let mut h = self.history.lock().unwrap();
        let seq = h.seq + 1;
        h.seq = seq;
        h.events.push_back((seq, ev.to_json()));
        while h.events.len() > HISTORY_MAX {
            h.events.pop_front();
        }
    }

    /// 返回 (最新 seq, 自 since 之后的事件列表)。LuCI 前端轮询此接口，
    /// 事件形状与原 WebSocket 推送一致：{type,data,timestamp}。
    pub fn events_since(&self, since: u64) -> (u64, Vec<Value>) {
        let h = self.history.lock().unwrap();
        let latest = h.seq;
        let events = h
            .events
            .iter()
            .filter(|(s, _)| *s > since)
            .map(|(_, v)| v.clone())
            .collect();
        (latest, events)
    }

    fn deliver(&self, ev: &Event) {
        let subs = self.inner.lock().unwrap().subscribers.clone();
        for s in &subs {
            let matched = s.topics.read().unwrap().contains(&ev.topic);
            if !matched {
                continue;
            }
            match s.tx.try_send(ev.clone()) {
                Ok(()) => {}
                Err(std::sync::mpsc::TrySendError::Full(_)) => {
                    // Slow consumer: drop the event (coalescing keeps the
                    // stream cheap; the next tick delivers the latest).
                }
                Err(std::sync::mpsc::TrySendError::Disconnected(_)) => {
                    self.inner.lock().unwrap().subscribers.retain(|x| x.id != s.id);
                }
            }
        }
    }

    fn flush_pending(&self) {
        let drained: Vec<Event> = {
            let mut inner = self.inner.lock().unwrap();
            inner.pending.drain().map(|(_, v)| v).collect()
        };
        for ev in drained {
            self.deliver(&ev);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscribe_filters_topics() {
        let bus = EventBus::new();
        let (rx, sub) = bus.subscribe(&["signal".to_string(), "network".to_string()]);
        bus.publish("signal", "signal.updated", crate::json::num_val(1));
        bus.publish("temperature", "temperature.updated", crate::json::num_val(2));
        // signal is immediate; temperature is coalesced -> wait one tick
        let ev = rx.recv_timeout(Duration::from_millis(200)).expect("signal event");
        assert_eq!(ev.topic, "signal");
        // temperature not subscribed -> must not arrive
        assert!(rx.recv_timeout(Duration::from_millis(50)).is_err());
        bus.unsubscribe(&sub);
    }

    #[test]
    fn coalescing_collapses_burst() {
        let bus = EventBus::new();
        let (rx, sub) = bus.subscribe(&["signal".to_string()]);
        // burst of identical-ish updates within one tick
        for i in 0..10 {
            bus.publish("signal", "signal.updated", crate::json::num_val(i));
        }
        std::thread::sleep(Duration::from_millis(EMIT_TICK_MS + 50));
        let mut count = 0;
        while let Ok(ev) = rx.try_recv() {
            assert_eq!(ev.topic, "signal");
            // the final payload must be the latest value (9)
            assert_eq!(ev.data.as_u64(), Some(9));
            count += 1;
        }
        assert!(count <= 1, "burst must collapse to <=1 event, got {}", count);
        bus.unsubscribe(&sub);
    }

    #[test]
    fn envelope_shape() {
        let bus = EventBus::new();
        let ev = Event {
            topic: "signal".into(),
            event: "signal.updated".into(),
            data: crate::json::num_val(-86),
            timestamp: 123,
        };
        let j = ev.to_json();
        assert!(j.dump().contains("\"type\":\"signal.updated\""));
        assert!(j.dump().contains("\"data\":-86"));
        bus.stop();
    }
}
