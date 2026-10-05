//! Background state collectors (modem snapshot) for the async backend.
//!
//! Every state topic (signal / network / registration / temperature /
//! traffic / cell / sim / modem_info) is refreshed by a **periodic task**
//! registered on the shared `TaskManager`. The tasks run at background/low
//! priority through the single AT arbiter, so they can never stall a page
//! load, a WebSocket command or a user action — and a slow modem only delays
//! the *next* refresh, never the UI.
//!
//! Collectors write into the `StateCache` (fresh value + TTL) and publish a
//! `*.updated` event on the `EventBus`; the event bridge fans them out to the
//! WebSocket subscribers. Readers never block on the modem:
//!
//!   * cache fresh        -> return immediately
//!   * cache stale        -> return the stale value + background refresh
//!   * modem unavailable  -> collector skips, cache ages out fast
//!
//! No sensitive data (IMEI/IMSI/SMS content) is ever logged; `modem_info`
//! and `sim` values are stored in the cache (read by authorized frontends)
//! but only non-secret summary fields are echoed in events.

use crate::scheduler::arbiter::{AtPayload, AtRequestSpec, AtResult, RetryPolicy};
use crate::state::bus::{
    EventBus, TOPIC_CELL, TOPIC_ENDC, TOPIC_MODEM, TOPIC_NETRATE, TOPIC_NETWORK, TOPIC_NR_TXPOWER,
    TOPIC_REGISTRATION, TOPIC_SIGNAL, TOPIC_SIM, TOPIC_TEMPERATURE, TOPIC_TRAFFIC, TOPIC_TXPOWER,
};
use crate::core::json::{self, Value};
use crate::state::cache::{Freshness, StateCache};
use crate::core::task::Priority;
use crate::scheduler::jobs::{TaskCtx, TaskManager};
use std::time::Duration;

/// Register every background collector as a periodic task on the manager.
/// Cadence mirrors the cache TTLs so values stay fresh between refreshes.
///
/// 采集频率按 MT5700 模组实测 AT 响应耗时设定（2026-10-04 设备实测）：
///
///   * AT^HCSQ?           ~4.0 s 成功（NR 信号）
///   * AT^CHIPTEMP?       ~0.1 s 成功
///   * AT+CEREG?          ~0.1 s 成功
///   * AT+C5GREG?         可用（后端通道实测正常，带 tac/ci/act/nssai）
///   * AT^HFREQINFO?      ~3.4 s 成功（频点/band）
///   * AT^NTXPOWER?       ~11.6 s 成功（NR 发射功率）
///   * AT^MONSC / AT^TXPOWER? / AT^LENDC? / AT^SIMSTATE? / AT^COPS?(偶发)
///                        失败也要占 8~12 s（模组对不支持命令响应极慢）
///
/// 设备 AT 串口是独占通道：慢命令若按 fast_query（3 s 超时 + 2 次重试）
/// 执行，会反复超时并长时间独占串口，把 signal/network 等快采集器全部
/// 饿死（任务超时 -> 无 `.updated` 事件 -> 页面数据不更新，实测全 "—"）。
/// 因此慢命令一律走 slow_query：宽松 AT 超时、不重试、Low 优先级，失败
/// 后进入退避期（退避期内直接跳过，不占通道）。WebUI 走事件推送 + SWR
/// 快照，10~300 s 的数据新鲜度完全够用。
pub fn spawn_all(tasks: &TaskManager) {
    // temperature: CHIPTEMP ~0.1 s 快命令；60 s 周期，TTL 与
    // mt5700m-manager 的温度缓存刷新保持匹配。
    tasks.add_periodic(
        "snapshot.temperature",
        Duration::from_secs(60),
        Priority::Low,
        Some(Duration::from_secs(15)),
        Box::new(|ctx| collect_temperature(ctx)),
    );
    tasks.add_periodic(
        "snapshot.traffic",
        Duration::from_secs(30),
        Priority::Low,
        Some(Duration::from_secs(15)),
        Box::new(|ctx| collect_traffic(ctx)),
    );
    // cell: MONSC 实测不支持（失败 8 s+，600 s 退避）；HFREQINFO ~3.4 s
    // 提供频点/band；COPS 补充运营商。120 s 周期。
    tasks.add_periodic(
        "snapshot.cell",
        Duration::from_secs(120),
        Priority::Low,
        Some(Duration::from_secs(20)),
        Box::new(|ctx| collect_cell(ctx)),
    );
    // endc / txpower：实测 LENDC?/TXPOWER? 在 NR 组网不支持（失败 8~12 s），
    // 300 s 低频退避重试，避免周期性占死通道。
    tasks.add_periodic(
        "snapshot.endc",
        Duration::from_secs(300),
        Priority::Low,
        Some(Duration::from_secs(20)),
        Box::new(|ctx| collect_endc(ctx)),
    );
    tasks.add_periodic(
        "snapshot.txpower",
        Duration::from_secs(300),
        Priority::Low,
        Some(Duration::from_secs(20)),
        Box::new(|ctx| collect_txpower(ctx)),
    );
    // nr_txpower: NTXPOWER ~11.6 s 才响应，180 s 周期（占通道约 6%），
    // 不重试。
    tasks.add_periodic(
        "snapshot.nr_txpower",
        Duration::from_secs(180),
        Priority::Low,
        Some(Duration::from_secs(20)),
        Box::new(|ctx| collect_nr_txpower(ctx)),
    );
    tasks.add_periodic(
        "snapshot.sim",
        Duration::from_secs(60),
        Priority::Background,
        Some(Duration::from_secs(15)),
        Box::new(|ctx| collect_sim(ctx)),
    );
    tasks.add_periodic(
        "snapshot.modem_info",
        Duration::from_secs(60),
        Priority::Background,
        Some(Duration::from_secs(15)),
        Box::new(|ctx| collect_modem_info(ctx)),
    );
    // netrate：网卡字节计数 + 与 LuCI 共享的 mt5700m-traffic 历史。
    // 纯本地 sysfs/文件读取，全程零 AT 流量，5 s 一��也不会占独占串口。
    tasks.add_periodic(
        "snapshot.netrate",
        Duration::from_secs(5),
        Priority::Background,
        Some(Duration::from_secs(5)),
        Box::new(|ctx| collect_netrate(ctx)),
    );
}

/// Read one sysfs counter, returning `None` when unreadable.
fn read_counter(path: &str) -> Option<u64> {
    let raw = std::fs::read_to_string(path).ok()?;
    raw.trim().parse::<u64>().ok()
}

/// Candidate modem-facing interfaces, most specific first.
///
/// Mirrors `mt5700m-traffic`'s default (`eth2`) but probes a couple of
/// fallbacks so the numbers still show up when the kernel renames the netdev
/// across a USB re-enumeration.
const NETRATE_IFACES: [&str; 4] = ["eth2", "usb0", "wwan0", "eth1"];

/// Detect which interface carries the modem's traffic.
fn detect_netrate_iface() -> Option<&'static str> {
    NETRATE_IFACES
        .iter()
        .copied()
        .find(|d| std::path::Path::new(&format!("/sys/class/net/{}/statistics", d)).is_dir())
}

/// Collect interface byte counters plus the shared traffic history.
///
/// 单一数据源（2026-10-04）：累计/日/月流量来自
/// `/usr/sbin/mt5700m-traffic json` —— 与 LuCI 概览页「IP 流量统计」
/// 走的是**同一个二进制、同一份 /etc/mt5700m/traffic-history**。
/// 此前 WebUI 读 AT^DSFLOWQRY（模组 PDCP 计数），LuCI 读网卡计数，
/// 两者物理量不同，同一台设备会显示两套对不上的流量。统一到网卡口径后
/// 两侧数字必然一致。
///
/// 写入方始终只有 mt5700m-traffic 的 daemon 一个；这里和两个前端都只读，
/// 不存在双写。执行外部命令读 json 是为了和 LuCI 走**同一条代码路径**
/// （只有它知道 history 的格式），而不是各自解析文件。
///
/// 实时速率由前端按两次采样差计算，这里只给原始累计计数。
fn collect_netrate(ctx: &TaskCtx) -> Result<Value, crate::core::error::BackendError> {
    let dev = match detect_netrate_iface() {
        Some(d) => d,
        None => {
            // 接口还没 up：写一份明确标注的 unavailable，前端据此显示
            // 「等待接口」而不是把上一轮的旧值继续显示成当前值。
            let mut m = std::collections::BTreeMap::new();
            m.insert("available".to_string(), json::bool_val(false));
            m.insert(
                "reason".to_string(),
                json::str_val("/sys/class/net 下未找到可用接口（eth2/usb0/wwan0/eth1）"),
            );
            let v = Value::Obj(m);
            store(ctx, TOPIC_NETRATE, "netrate.updated", &v);
            return Ok(v);
        }
    };

    let base = format!("/sys/class/net/{}/statistics", dev);
    let rx = read_counter(&format!("{}/rx_bytes", base));
    let tx = read_counter(&format!("{}/tx_bytes", base));

    if rx.is_none() && tx.is_none() {
        let mut m = std::collections::BTreeMap::new();
        m.insert("available".to_string(), json::bool_val(false));
        m.insert("device".to_string(), json::str_val(dev));
        m.insert("reason".to_string(), json::str_val("接口计数器不可读"));
        let v = Value::Obj(m);
        store(ctx, TOPIC_NETRATE, "netrate.updated", &v);
        return Ok(v);
    }

    let mut m = std::collections::BTreeMap::new();
    m.insert("available".to_string(), json::bool_val(true));
    m.insert("device".to_string(), json::str_val(dev));
    m.insert(
        "rx_bytes".to_string(),
        rx.map(|v| json::num_val(v as f64)).unwrap_or(Value::Null),
    );
    m.insert(
        "tx_bytes".to_string(),
        tx.map(|v| json::num_val(v as f64)).unwrap_or(Value::Null),
    );
    m.insert("timestamp".to_string(), json::num_val(now_secs_f64()));

    // 累计/日/月：与 LuCI 同源的那一份。
    match run_traffic_json() {
        Some(report) => {
            m.insert("source".to_string(), json::str_val("mt5700m-traffic"));
            m.insert("traffic".to_string(), report);
        }
        None => {
            // 采集器本身不可用不影响实时速率（上面已拿到计数器），
            // 只标明累计部分缺失。
            m.insert("source".to_string(), json::str_val("unavailable"));
        }
    }

    let v = Value::Obj(m);
    store(ctx, TOPIC_NETRATE, "netrate.updated", &v);
    Ok(v)
}

/// Shell out to `mt5700m-traffic json` and parse its output as JSON.
///
/// Returns the raw `Value` (an object with an `interfaces` array) so the
/// frontends can read total/day/month without this backend having to
/// re-define that file format.
fn run_traffic_json() -> Option<Value> {
    use std::process::{Command, Stdio};

    let out = Command::new("/bin/sh")
        .arg("-c")
        .arg("/usr/sbin/mt5700m-traffic json")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    // json::parse 失败时返回 None（不是 Result），无需再 .ok()
    crate::core::json::parse(text)
}

/// Seconds since the Unix epoch, as f64. Used for event timestamps only.
fn now_secs_f64() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Write a topic to the cache and publish the update event.
fn store(ctx: &TaskCtx, topic: &str, event: &str, value: &Value) {
    ctx.cache.set(topic, value.clone(), "snapshot");
    ctx.bus.publish(topic, event, value.clone());
}

/// On failure, fall back to the cached value: refresh its TTL and re-publish
/// the event so the page never drops to a placeholder because one collector
/// tick lost the modem race. Silently returns Null when the cache is empty.
fn stale_refresh(ctx: &TaskCtx, topic: &str, event: &str) -> Value {
    match ctx.cache.get(topic) {
        (Some(Value::Obj(old)), _) => {
            store(ctx, topic, event, &Value::Obj(old.clone()));
            Value::Obj(old)
        }
        _ => Value::Null,
    }
}

/// Slow-command query with failure backoff.
///
/// 慢命令（模组实测 4~12 s 响应或直接失败）不能走 `ctx.query()` 的
/// fast_query（3 s AT 超时 + 2 次重试）：重试会让慢命令反复超时并长时间
/// 独占串口，把快采集器全部饿死。这里用宽松的 AT 超时 + 不重试 + Low
/// 优先级；失败（超时/无数据）后用 `backoff` 时长记入缓存，退避期内
/// 直接返回 `None` 不占通道，到期后重试一次。
fn slow_query(
    ctx: &TaskCtx,
    backoff_key: &str,
    command: &str,
    at_timeout: Duration,
    queued_timeout: Duration,
    backoff: Duration,
) -> Option<String> {
    // Duty-cycle gate: while background work is overrunning the channel — or a
    // user page has just been turned away for lack of capacity — stay off it.
    // Skipping costs nothing observable (the pages render the cached snapshot)
    // and is what stops the two frontends from starving each other.
    if crate::scheduler::arbiter::channel_budget_exhausted() || crate::scheduler::arbiter::channel_user_starved() {
        return None;
    }
    let bk = format!("snapshot.backoff.{}", backoff_key);
    if matches!(ctx.cache.get(&bk), (_, Freshness::Fresh)) {
        return None; // 退避期内跳过
    }
    let spec = AtRequestSpec {
        command: command.to_string(),
        priority: Priority::Low,
        timeout: at_timeout,
        queued_timeout,
        exclusive: false,
        dedup_key: Some(command.to_string()),
        retry: RetryPolicy::none(),
        cancel: None,
        abort_wire: None,
        payload: AtPayload::None,
        label: format!("snapshot {}", command),
    };
    match ctx.at_request(spec) {
        Ok(text) if !text.trim().is_empty() => Some(text.trim().to_string()),
        _ => {
            ctx.cache
                .set_ttl(&bk, json::num_val(0), "backoff", backoff);
            None
        }
    }
}

/// Round to 1 decimal place. Raw `n * step` arithmetic on f64 yields
/// artifacts like 22.000000000000004 for SINR; pages render these values
/// directly, so snap the precision here once for every consumer.
fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Non-fatal collector wrapper: modem/transport errors are expected (absent
/// modem, USB in upgrade mode, ...) and must never poison the scheduler.
/// Returns the trimmed raw response so callers can feed it to their parser.
fn soft(r: AtResult) -> Result<String, crate::core::error::BackendError> {
    r.map(|text| text.trim().to_string())
}


/// Parse `^HCSQ: "LTE",rssi,rsrp,sinr,rsrq,...` into the same field names the
/// dashboard already consumes (sysmode/rsrp/rsrq/sinr/rssi).


/// Parse a `+CxxREG:` line: registration state + family-specific fields.
/// `+CxxREG: <mode>,<stat>[,...]` — the first field is the unsolicited-report
/// mode, the actual registration state is the second; some older replies
/// carry only the stat (single field).
///
/// Field layout differs per family (3GPP 27.007):
///   * C5GREG: `<n>,<stat>[,<tac>,<ci>,<AcT>,<len>,<NSSAI>]`
///   * CEREG:  `<n>,<stat>[,<tac>,<ci>[,<AcT>]]`
///   * CREG:   `<n>,<stat>[,<lac>,<ci>]` or `<n>,<stat>[,<mcc>,<mnc>,<lac>,<ci>]`
/// The WebUI Diagnostics panel renders tac/ci/nssai from the 5G family, so we
/// expose a unified field set (tac/ci/act/nssai/mcc/mnc/lac) and let each
/// consumer pick what it understands.


/// Quoted or numeric MCC-MNC operator name from `+COPS:`.




fn collect_temperature(ctx: &TaskCtx) -> Result<Value, crate::core::error::BackendError> {
    // CHIPTEMP 本身是快命令（0.1 s），但 fast_query（3 s 超时 + 2 次重试）
    // 在 AT 通道被慢命令（LENDC/TXPOWER/MONSC 等 8~15 s）占用时排队等不及，
    // 整体失败且不发事件，页面温度就掉回"—"。改用与 HCSQ 相同的宽松 spec
    // 排队等待；失败时用旧缓存续命（刷新 TTL 并重推事件），保证页面始终有值。
    let spec = AtRequestSpec {
        command: "AT^CHIPTEMP?".to_string(),
        priority: Priority::Low,
        timeout: Duration::from_secs(12),
        queued_timeout: Duration::from_secs(8),
        exclusive: false,
        dedup_key: Some("AT^CHIPTEMP?".to_string()),
        retry: RetryPolicy::none(),
        cancel: None,
        abort_wire: None,
        payload: AtPayload::None,
        label: "snapshot AT^CHIPTEMP?".to_string(),
    };
    let value = match ctx.at_request(spec) {
        Ok(text) if !text.trim().is_empty() => parse_chiptemp(text.trim()),
        _ => stale_refresh(ctx, TOPIC_TEMPERATURE, "temperature.updated"),
    };
    if !matches!(value, Value::Null) {
        store(ctx, TOPIC_TEMPERATURE, "temperature.updated", &value);
    }
    Ok(value)
}

/// Parse `^CHIPTEMP: t0..t11` (tenths of degrees) into named fields plus a
/// rounded average. Mirrors the WebUI `parseCHIPTEMP`.
pub fn parse_chiptemp(raw: &str) -> Value {
    let names = [
        "sub3GPA", "sub6GPA", "mimoPa", "tcxo", "peri1", "peri2", "ap1", "ap2", "modem1",
        "modem2", "bbp1", "bbp2",
    ];
    let mut m = std::collections::BTreeMap::new();
    let Some(body) = raw
        .lines()
        .find_map(|l| l.trim().strip_prefix("^CHIPTEMP:"))
    else {
        return Value::Obj(m);
    };
    let fields: Vec<&str> = body.split(',').map(|f| f.trim()).collect();
    let mut sum: f64 = 0.0;
    let mut count: usize = 0;
    for (i, name) in names.iter().enumerate() {
        let raw_v = fields.get(i).and_then(|f| f.parse::<u64>().ok()).unwrap_or(0);
        let v = if raw_v >= 65535 || raw_v > 1500 {
            0.0
        } else {
            raw_v as f64 / 10.0
        };
        m.insert((*name).to_string(), json::num_val(v));
        if v > 0.0 {
            sum += v;
            count += 1;
        }
    }
    if count > 0 {
        let avg = round1(sum / count as f64);
        m.insert("average".to_string(), json::num_val(avg));
    }
    Value::Obj(m)
}

fn collect_traffic(ctx: &TaskCtx) -> Result<Value, crate::core::error::BackendError> {
    let text = soft(ctx.query("AT^PDCPDATAINFO?"))?;
    let value = text
        .lines()
        .find_map(|l| crate::transport::urc::handle_pdcp(l))
        .unwrap_or(Value::Null);
    store(ctx, TOPIC_TRAFFIC, "traffic.updated", &value);
    Ok(value)
}

fn collect_cell(ctx: &TaskCtx) -> Result<Value, crate::core::error::BackendError> {
    // Serving cell: best-effort raw snapshot. Full per-carrier analysis stays
    // in the interactive WebUI flows (MONSC/HFREQINFO/MONSSC); the cache only
    // guarantees the dashboard has *something* instantly available. Besides
    // the raw line we mirror the WebUI `parseMONSC` field mapping (mcc/mnc/
    // channel/cid/pci/lac) so the dashboard's cell params never sit on "--".
    //
    // MT5700 实测：AT^MONSC 不支持（失败也要占 8 s+），只能低频退避尝试；
    // AT^HFREQINFO? 快（~3.4 s）且带频点/band，作为 cell 卡片「频点」的
    // 数据源；AT+COPS? 补充运营商名。
    let mut m = std::collections::BTreeMap::new();
    if let Some(text) = slow_query(ctx, "cell_hfreq", "AT^HFREQINFO?", Duration::from_secs(15), Duration::from_secs(10), Duration::from_secs(120)) {
        // ^HFREQINFO: <grp>,<flags>,<band>,<dl_arfcn>,<dl_freq_khz>,<dl_bw_khz>,
        //             <ul_arfcn>,<ul_freq_khz>,<ul_bw_khz>（实测
        //             ^HFREQINFO: 0,7,41,513000,2565000,100000,513000,2565000,100000）
        if let Some(body) = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("^HFREQINFO:"))
        {
            let fields: Vec<&str> = body.split(',').map(|f| f.trim()).collect();
            if let Some(band) = fields.get(2).filter(|s| !s.is_empty()) {
                m.insert("band".to_string(), json::str_val(band));
            }
            if let Some(arfcn) = fields.get(3).filter(|s| !s.is_empty()) {
                m.insert("channel".to_string(), json::str_val(arfcn));
            }
            if let Some(bw) = fields.get(5).and_then(|s| s.parse::<u64>().ok()) {
                m.insert("dlBandwidth".to_string(), json::num_val(bw as i64 / 1000));
            }
        }
    }
    if let Some(cops) = slow_query(ctx, "cell_cops", "AT+COPS?", Duration::from_secs(8), Duration::from_secs(6), Duration::from_secs(60)) {
        // TRANSITIONAL: the operator parser now lives in the network module;
        // this collector is deleted when the cell module takes over.
        if let Some(op) = crate::modules::network::parser::parse_cops_operator(&cops) {
            m.insert("operator".to_string(), json::str_val(&op));
        }
    }
    if let Some(text) = slow_query(ctx, "cell_monsc", "AT^MONSC", Duration::from_secs(12), Duration::from_secs(10), Duration::from_secs(600)) {
        m.insert("raw".to_string(), json::str_val(text.trim()));
        if let Some(body) = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("^MONSC:"))
        {
            let fields: Vec<&str> = body.split(',').map(|f| f.trim()).collect();
            if let Some(arfcn) = fields.first() {
                m.insert("arfcn".to_string(), json::str_val(arfcn));
            }
            if fields.len() >= 4 {
                let sysmode = fields[0].trim_matches('"');
                if !sysmode.is_empty() {
                    m.insert("sysmode".to_string(), json::str_val(sysmode));
                }
                let mut put = |key: &str, i: usize| {
                    if let Some(v) = fields.get(i) {
                        let s = v.trim().trim_matches('"');
                        if !s.is_empty() {
                            m.insert(key.to_string(), json::str_val(s));
                        }
                    }
                };
                put("mcc", 1);
                put("mnc", 2);
                put("channel", 3);
                // ^MONSC 的小区参数按模式错位：LTE 的 cid/pci/lac 在
                // 4/5/6，NR 在 5/6/7，WCDMA 的 pci 是十进制、cid/lac 十六进制。
                match sysmode {
                    "LTE" => {
                        if let Some(v) = fields.get(4).and_then(|s| hex_dec(s)) {
                            m.insert("cid".to_string(), json::str_val(&v.to_string()));
                        }
                        if let Some(v) = fields.get(5).and_then(|s| hex_dec(s)) {
                            m.insert("pci".to_string(), json::num_val(v as i64));
                        }
                        if let Some(v) = fields.get(6).and_then(|s| hex_dec(s)) {
                            m.insert("lac".to_string(), json::str_val(&v.to_string()));
                        }
                    }
                    "NR" => {
                        if let Some(v) = fields.get(5).and_then(|s| hex_dec(s)) {
                            m.insert("cid".to_string(), json::str_val(&v.to_string()));
                        }
                        if let Some(v) = fields.get(6).and_then(|s| hex_dec(s)) {
                            m.insert("pci".to_string(), json::num_val(v as i64));
                        }
                        if let Some(v) = fields.get(7).and_then(|s| hex_dec(s)) {
                            m.insert("lac".to_string(), json::str_val(&v.to_string()));
                        }
                    }
                    "WCDMA" => {
                        if let Some(v) = fields.get(4).and_then(|s| s.parse::<u64>().ok()) {
                            m.insert("pci".to_string(), json::num_val(v as i64));
                        }
                        if let Some(v) = fields.get(5).and_then(|s| hex_dec(s)) {
                            m.insert("cid".to_string(), json::str_val(&v.to_string()));
                        }
                        if let Some(v) = fields.get(6).and_then(|s| hex_dec(s)) {
                            m.insert("lac".to_string(), json::str_val(&v.to_string()));
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    // 小区卡片与注册共用数据源：C5GREG 已稳定拿到 tac/ci（Tracking Area 与
    // 小区标识），MONSC 在 NR 组网不可用时用它们填 TAC/LAC 与小区 ID，别让
    // 小区参数区掉成"—"。MONSC 本身给到的 mcc/mnc/pci 仍优先保留。
    if let (Some(Value::Obj(reg)), _) = ctx.cache.get(TOPIC_REGISTRATION) {
        if let Some(v) = reg.get("tac") {
            m.entry("lac".to_string()).or_insert_with(|| v.clone());
        }
        if let Some(v) = reg.get("ci") {
            m.entry("cid".to_string()).or_insert_with(|| v.clone());
        }
    }
    let value = Value::Obj(m);
    store(ctx, TOPIC_CELL, "cell.updated", &value);
    Ok(value)
}

/// Parse a hex string to decimal; invalid/empty input yields None.
fn hex_dec(s: &str) -> Option<u64> {
    u64::from_str_radix(s.trim().trim_matches('"'), 16).ok()
}

fn collect_endc(ctx: &TaskCtx) -> Result<Value, crate::core::error::BackendError> {
    // AT^LENDC? 实测在 NR 组网不支持（失败也要占 8 s+），走 600 s 退避，
    // 避免周期性占死通道；查询应答带 <enable> 前缀，URC 不带；按字段数
    // 区分（同前端 parseLendc）。
    let mut m = std::collections::BTreeMap::new();
    if let Some(text) = slow_query(ctx, "endc_lendc", "AT^LENDC?", Duration::from_secs(15), Duration::from_secs(12), Duration::from_secs(600)) {
        let fields: Vec<u8> = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("^LENDC:"))
            .map(|body| {
                body.split(',')
                    .map(|f| f.trim().parse::<u8>().unwrap_or(0))
                    .filter(|v| *v == 0 || *v == 1)
                    .collect()
            })
            .unwrap_or_default();
        let v: Vec<u8> = if fields.len() >= 5 { fields[1..].to_vec() } else { fields };
        if v.len() >= 4 {
            m.insert("available".to_string(), json::num_val(v[0] as i64));
            m.insert("plmnAvailable".to_string(), json::num_val(v[1] as i64));
            m.insert("restricted".to_string(), json::num_val((v[2] == 0) as i64));
            m.insert("established".to_string(), json::num_val(v[3] as i64));
        }
    }
    let value = Value::Obj(m);
    store(ctx, TOPIC_ENDC, "endc.updated", &value);
    Ok(value)
}

fn collect_txpower(ctx: &TaskCtx) -> Result<Value, crate::core::error::BackendError> {
    // AT^TXPOWER? 实测仅 GUL 组网有效，NR 下失败（占 8 s+），600 s 退避；
    // 无效功率（999）按空处理（同前端 parseTxPower）。
    let mut m = std::collections::BTreeMap::new();
    if let Some(text) = slow_query(ctx, "txpower", "AT^TXPOWER?", Duration::from_secs(15), Duration::from_secs(12), Duration::from_secs(600)) {
        if let Some(body) = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("^TXPOWER:"))
        {
            let fields: Vec<&str> = body.split(',').map(|f| f.trim()).collect();
            if fields.len() >= 5 {
                let power = |f: &str| -> Option<i64> {
                    let n = f.parse::<i64>().ok()?;
                    if n == 999 {
                        None
                    } else {
                        Some(n)
                    }
                };
                if let Some(total) = power(fields[0]) {
                    // <stxpwr> 单位 0.1dBm
                    m.insert("total".to_string(), json::num_val(round1(total as f64 / 10.0)));
                }
                for (key, idx) in [("pusch", 1), ("pucch", 2), ("srs", 3), ("prach", 4)] {
                    if let Some(v) = power(fields[idx]) {
                        m.insert(key.to_string(), json::num_val(v));
                    }
                }
            }
        }
    }
    let value = Value::Obj(m);
    store(ctx, TOPIC_TXPOWER, "txpower.updated", &value);
    Ok(value)
}

fn collect_nr_txpower(ctx: &TaskCtx) -> Result<Value, crate::core::error::BackendError> {
    // AT^NTXPOWER? 实测 ~11.6 s 才响应（NR 组网有效），15 s 宽松超时 + 不
    // 重试 + 600 s 失败退避；每 5 个字段一个载波，最多 4 个。
    let mut carriers: Vec<Value> = Vec::new();
    if let Some(text) = slow_query(ctx, "nr_txpower", "AT^NTXPOWER?", Duration::from_secs(15), Duration::from_secs(12), Duration::from_secs(600)) {
        if let Some(body) = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("^NTXPOWER:"))
        {
            let fields: Vec<&str> = body.split(',').map(|f| f.trim()).collect();
            let mut i = 0;
            while i + 4 < fields.len() && carriers.len() < 4 {
                let mut c = std::collections::BTreeMap::new();
                let power = |f: &str| -> Option<i64> {
                    let n = f.parse::<i64>().ok()?;
                    if n == 999 {
                        None
                    } else {
                        Some(n)
                    }
                };
                for (key, off) in [("pusch", 0), ("pucch", 1), ("srs", 2), ("prach", 3)] {
                    if let Some(v) = power(fields[i + off]) {
                        c.insert(key.to_string(), json::num_val(v));
                    }
                }
                if let Ok(freq) = fields[i + 4].parse::<i64>() {
                    if freq > 0 {
                        c.insert("freq".to_string(), json::num_val(freq));
                    }
                }
                if !c.is_empty() {
                    carriers.push(Value::Obj(c));
                }
                i += 5;
            }
        }
    }
    let mut m = std::collections::BTreeMap::new();
    m.insert("carriers".to_string(), Value::Arr(carriers));
    let value = Value::Obj(m);
    store(ctx, TOPIC_NR_TXPOWER, "nr_txpower.updated", &value);
    Ok(value)
}

fn collect_sim(ctx: &TaskCtx) -> Result<Value, crate::core::error::BackendError> {
    let mut m = std::collections::BTreeMap::new();
    if let Some(text) = slow_query(ctx, "sim_cpin", "AT+CPIN?", Duration::from_secs(6), Duration::from_secs(5), Duration::from_secs(60)) {
        for line in text.lines() {
            if let Some(rest) = line.trim().strip_prefix("+CPIN:") {
                m.insert(
                    "status".to_string(),
                    json::str_val(rest.trim().trim_matches('"')),
                );
                break;
            }
        }
    }
    if let Some(text) = slow_query(ctx, "sim_iccid", "AT^ICCID?", Duration::from_secs(6), Duration::from_secs(5), Duration::from_secs(60)) {
        for line in text.lines() {
            if let Some(rest) = line.trim().strip_prefix("^ICCID:") {
                m.insert(
                    "iccid".to_string(),
                    json::str_val(rest.trim().trim_matches('"')),
                );
                break;
            }
        }
    }
    if let Some(text) = slow_query(ctx, "sim_cimi", "AT+CIMI", Duration::from_secs(6), Duration::from_secs(5), Duration::from_secs(60)) {
        for line in text.lines() {
            let t = line.trim();
            if t.len() >= 10 && t.bytes().all(|b| b.is_ascii_digit()) {
                m.insert("imsi".to_string(), json::str_val(t));
                break;
            }
        }
    }
    let value = Value::Obj(m);
    store(ctx, TOPIC_SIM, "sim.updated", &value);
    Ok(value)
}

fn collect_modem_info(ctx: &TaskCtx) -> Result<Value, crate::core::error::BackendError> {
    let mut m = std::collections::BTreeMap::new();
    if let Some(text) = slow_query(ctx, "modem_ati", "ATI", Duration::from_secs(6), Duration::from_secs(5), Duration::from_secs(60)) {
        for (prefix, key) in [
            ("Manufacturer:", "manufacturer"),
            ("Model:", "model"),
            ("Revision:", "revision"),
        ] {
            if let Some(rest) = text.lines().find_map(|l| l.trim().strip_prefix(prefix)) {
                m.insert(
                    key.to_string(),
                    json::str_val(rest.trim().trim_matches('"')),
                );
            }
        }
    }
    if let Some(text) = slow_query(ctx, "modem_cgsn", "AT+CGSN", Duration::from_secs(6), Duration::from_secs(5), Duration::from_secs(60)) {
        for line in text.lines() {
            let t = line.trim();
            if t.len() == 15 && t.bytes().all(|b| b.is_ascii_digit()) {
                m.insert("imei".to_string(), json::str_val(t));
                break;
            }
        }
    }
    let value = Value::Obj(m);
    // modem_info is long-lived; give the cache a longer TTL.
    ctx.cache
        .set_ttl(TOPIC_MODEM, value.clone(), "snapshot", Duration::from_secs(60));
    ctx.bus.publish(TOPIC_MODEM, "modem.info", value.clone());
    Ok(value)
}

/// Total number of AT exchanges the periodic snapshot may produce per minute
/// (kept tiny after the 2026-10-04 collector rework: the slow commands
/// endc/txpower/nr_txpower/cell run at 120~300 s with failure backoff, and
/// every exchange is deduplicated + low/background priority).
#[allow(dead_code)]
pub fn snapshot_rate_per_minute() -> usize {
    4 + 3 + 2 + 2 + 1 + 1 + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chiptemp_parse() {
        let v = parse_chiptemp("^CHIPTEMP: 432,445,451,398,401,407,460,455,470,468,410,415\r\nOK");
        assert_eq!(v.get("tcxo").and_then(|x| x.as_f64()), Some(39.8));
        assert_eq!(v.get("ap1").and_then(|x| x.as_f64()), Some(46.0));
        assert!(v.get("average").and_then(|x| x.as_f64()).unwrap_or(0.0) > 0.0);
    }
}
