//! Module support: the refresh context every module service uses.
//!
//! A `RefreshCtx` bundles the three things a service needs — the AT channel,
//! the state cache and the event bus — and implements the shared refresh
//! policy once, so no module re-invents backoff, stale fallback or duty-cycle
//! yielding:
//!
//! ```text
//! AT -> Parser -> Domain Model -> StateCache -> EventBus -> API -> frontends
//! ```

use crate::core::channel::{soft, AtChannel};
use crate::core::error::BackendError;
use crate::core::task::Priority;
use crate::core::json::{self, Value};
use crate::state::bus::EventBus;
use crate::state::cache::{Freshness, StateCache};
use std::sync::Arc;
use std::time::Duration;

/// Round to 1 decimal place. Raw `n * step` f64 arithmetic yields artifacts
/// like 22.000000000000004 for SINR; pages render these values directly, so
/// the precision is snapped once here for every consumer.
pub fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Seconds since the Unix epoch as f64 (event timestamps).
pub fn now_secs_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Read one sysfs counter, `None` when unreadable.
pub fn read_counter(path: &str) -> Option<u64> {
    let raw = std::fs::read_to_string(path).ok()?;
    raw.trim().parse::<u64>().ok()
}

/// First-error collector for best-effort read sequences.
///
/// Some routes read several commands and answer with whatever came back
/// (`network.dhcp`, `modem.mcs`): a modem that only supports part of them must
/// still produce a useful object, but if *nothing* could be read the route
/// should fail with the first real reason instead of answering an empty object.
/// One implementation, used by every module that needs it.
#[derive(Default)]
pub struct ReadErrors {
    first: Option<BackendError>,
}

impl ReadErrors {
    pub fn new() -> Self {
        ReadErrors { first: None }
    }

    /// `Some(value)` on success, `None` on failure while remembering the error.
    pub fn note<T>(&mut self, r: Result<T, BackendError>) -> Option<T> {
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                if self.first.is_none() {
                    self.first = Some(e);
                }
                None
            }
        }
    }

    /// The first error seen, if any.
    pub fn into_option(self) -> Option<BackendError> {
        self.first
    }
}

/// The refresh context handed to every module service.
pub struct RefreshCtx<'a> {
    pub channel: &'a dyn AtChannel,
    pub cache: &'a Arc<StateCache>,
    pub bus: &'a Arc<EventBus>,
    /// Cache provenance tag (`snapshot` for collectors, `api` for on-demand).
    pub source: &'static str,
}

impl<'a> RefreshCtx<'a> {
    pub fn new(
        channel: &'a dyn AtChannel,
        cache: &'a Arc<StateCache>,
        bus: &'a Arc<EventBus>,
    ) -> Self {
        RefreshCtx {
            channel,
            cache,
            bus,
            source: "snapshot",
        }
    }

    /// Tag the cache entries this context writes.
    pub fn with_source(mut self, source: &'static str) -> Self {
        self.source = source;
        self
    }

    /// Write a topic to the cache and publish the update event.
    pub fn store(&self, topic: &str, event: &str, value: &Value) {
        self.cache.set(topic, value.clone(), self.source);
        self.bus.publish(topic, event, value.clone());
    }

    /// On failure, fall back to the cached value: refresh its TTL and
    /// re-publish the event so a page never drops to a placeholder because one
    /// collector tick lost the modem race. Returns Null when the cache is
    /// empty.
    pub fn stale(&self, topic: &str, event: &str) -> Value {
        match self.cache.get(topic) {
            (Some(Value::Obj(old)), _) => {
                self.store(topic, event, &Value::Obj(old.clone()));
                Value::Obj(old)
            }
            _ => Value::Null,
        }
    }

    /// Slow-command query with failure backoff.
    ///
    /// Slow commands (4–12 s or plain unsupported on the MT5700M) must not use
    /// the fast query shape: retries would keep the port busy and starve the
    /// fast collectors. Failures are recorded in the cache for `backoff`, and
    /// during that window the command is not sent at all — the page keeps
    /// rendering the previous (SWR) value.
    pub fn slow(
        &self,
        backoff_key: &str,
        command: &str,
        at_timeout: Duration,
        queued_timeout: Duration,
        backoff: Duration,
    ) -> Option<String> {
        if self.channel.duty_gate() {
            return None;
        }
        let bk = format!("snapshot.backoff.{}", backoff_key);
        if matches!(self.cache.get(&bk), (_, Freshness::Fresh)) {
            return None;
        }
        match self
            .channel
            .query_prio(command, at_timeout, queued_timeout, Priority::Low)
        {
            Ok(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
            _ => {
                self.cache.set_ttl(&bk, json::num_val(0), "backoff", backoff);
                None
            }
        }
    }

    /// Background read with an explicit priority (periodic collectors).
    ///
    /// Transport errors that just mean "no answer right now" (timeout, busy
    /// channel) are softened to an empty string, so a collector records a
    /// stale value instead of failing a task the UI depends on.
    pub fn read(
        &self,
        command: &str,
        at_timeout: Duration,
        queued_timeout: Duration,
        priority: Priority,
    ) -> Result<String, BackendError> {
        soft(self
            .channel
            .query_prio(command, at_timeout, queued_timeout, priority))
    }

    /// Fast read (cache-gated through the arbiter).
    pub fn query(&self, command: &str) -> Result<String, BackendError> {
        soft(self.channel.query(command))
    }

    /// Write/action.
    pub fn action(&self, command: &str) -> Result<String, BackendError> {
        self.channel.action(command)
    }

    /// Cache lookup for a topic (used by `*.get` routes).
    pub fn cached(&self, topic: &str) -> Option<Value> {
        match self.cache.get(topic) {
            (Some(Value::Obj(v)), _) => Some(Value::Obj(v)),
            _ => None,
        }
    }
}
