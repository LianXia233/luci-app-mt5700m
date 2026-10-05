//! Read-command cache gate (方案 B).
//!
//! # 为什么需要闸门
//!
//! 「同一份缓存」只保证 LuCI 与 WebUI 看到的数据一致，**不保证两者互不干扰**。
//! 实测三条干扰通道里最隐蔽的一条是独占串口：所有 AT 指令进同一个
//! `AtArbiter`，而 MT5700 的慢命令单条就要占 8~12 s（实测 `AT^NTXPOWER?`
//! 11.6 s、`AT^LENDC?` 不支持也要占 8 s）。只要前端把「渲染要读的
//! 状态」当成AT 查询下发，两侧就会在同一条队列里互相排队。
//!
//! # 本模块的契约
//!
//! **前端渲染/轮询路径上一个AT 都不能下发。**
//!
//! - 读命令（`AT^X?` / `AT+X?` / `AT^X` 无参查询）→ 先查 raw 缓存：
//!   - 命中且未过期：直接返回，零 AT、零阻塞（µs 级）。
//!   - 命中但已过期：**立即返回旧值**（SWR），同时异步触发一次刷新。
//!     页面永远不等AT。
//!   - 从未采集过：提交一个 **单飞（single-flight）** 后台采集，立刻返回
//!     「采集中」占位。前端拿到占位继续渲染，采集完成后由 EventBus 推事件。
//! - 写命令（`AT^X=...` / 拨号 / 短信 / 清流量 / 升级 / SIM PIN / 终端）
//!   → 原样直通 AT 通道。
//!
//! # 为什么是「惰性」而不是给每条命令配周期采集器
//!
//! 40+ 条读命令若各配一个周期采集器，AT 流量会暴涨十几倍，反而把独占
//! 通道占死 —— 这正是历史上 LuCI/WebUI 互相拖垮的根因。惰性采集的 AT
//! 流量等于「实际被访问过的页面数 × 各命令的最小重采集间隔」，与前端
//! 刷新率**完全无关**：前端刷 1 次/秒 还是 1 次/毫秒，AT 增量都是 0。
//!
//! # 采集器不与前端抢通道
//!
//! 这里的采集任务用 `Priority::Low` + 长 `queued_timeout`，且**永远不阻塞
//! 调用方**：闸门在提交后立即返回，采集在后台worker 线程上跑完再写缓存。

use crate::scheduler::arbiter::AtRequestSpec;
use crate::state::bus::TOPIC_RAW;
use crate::core::json::Value;
use crate::state::cache::StateCache;
use crate::core::task::{Priority, TaskKind};
use crate::scheduler::jobs::{TaskCtx, TaskManager};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// 每条读命令的最小重采集间隔。
///
/// 太小→ 白白占AT 通道；太大 → 页面数据发霉。取值按 MT5700 实测响应
/// 耗时分层：快命令（CREG/CPIN/CMGF 类，实测 0.1 s）可以勤一点，慢命令
/// （LENDC/NTXPOWER/SYSCFGEX 类，实测 8~12 s）必须给足退避。
pub fn raw_ttl(command: &str) -> Duration {
    let c = command.to_ascii_uppercase();
    // 实测 8~12 s 甚至直接失败的慢命令：给足 5 分钟，避免反复占死通道。
    const SLOW: &[&str] = &[
        "AT^LENDC?",
        "AT^TXPOWER?",
        "AT^NTXPOWER?",
        "AT^MONNC",
        "AT^MONSC",
        "AT^MONSSC",
        "AT^HFREQINFO?",
        "AT^SYSINFOEX",
        "AT+COPS?",
        "AT^SYSCFGEX?",
        "AT^TDPCIELANCFG?",
        "AT^TDPMCFG?",
        "AT^SCICHG?",
        "AT^TDSIMHP?",
        "AT^NRRCCAPQRY",
        "AT^THERMLDAUTOPARA?",
        "AT^THERM",
    ];
    if SLOW.iter().any(|s| c.starts_with(s)) {
        return Duration::from_secs(300);
    }
    // 中等耗时（2~5 s）：1分钟。
    const MED: &[&str] = &[
        "AT^DHCP?",
        "AT^DHCPV6?",
        "AT^HFREQINFO",
        "AT^EONS",
        "AT+CGEQOSRDP",
        "AT+CGPADDR",
        "AT^IPV6CAP?",
        "AT^FOTASTATE?",
        "AT^NRSSBID?",
        "AT^LTEFREQLOCK?",
        "AT^NRFREQLOCK?",
        "AT^C5GOPTION?",
        "AT^DSFLOWQRY",
    ];
    if MED.iter().any(|s| c.starts_with(s)) {
        return Duration::from_secs(60);
    }
    // 快命令（实测 0.1 s 级）：10 s，够页面轮询又不占通道。
    Duration::from_secs(10)
}

/// raw 采集超时：慢命令要留足时间，又不能无限期占住 worker。
fn raw_timeout(command: &str) -> Duration {
    let c = command.to_ascii_uppercase();
    const SLOW: &[&str] = &[
        "AT^LENDC?",
        "AT^TXPOWER?",
        "AT^NTXPOWER?",
        "AT^MONNC",
        "AT^MONSC",
        "AT^MONSSC",
        "AT^HFREQINFO?",
        "AT^SYSINFOEX",
        "AT+COPS?",
        "AT^SYSCFGEX?",
    ];
    if SLOW.iter().any(|s| c.starts_with(s)) {
        Duration::from_secs(20)
    } else {
        Duration::from_secs(8)
    }
}

/// 带 `=` 但**语义上是查询**的命令前缀（`=` 后只跟一个 cid / index）。
///
/// 这类命令不能按"含 `=` 即写"一刀切，否则 LuCI 概览页每次刷新都会
/// 下发 4 条 AT —— 实测就是 `AT+CGEQOSRDP=1`（QCI）和
/// `AT^DSAMBR=1`（签约速率）。它们与对应的无参查询等价，只是把 cid
/// 作为选择参数带上。
///
/// **判定必须同时满足**：
///   1. 前缀命中本表；
///   2. `=` 后**恰好一个纯数字**（cid），没有第二个参数。
///
/// 条件 2 不可省：`AT^DSAMBR=1,2` 是**真赋值**（设置 cid1 的 AMR 为
/// 2），只看前缀会把它误判成读 → 闸门拦下→ 用户改不了 AMR。
/// 这个区分由`query_with_cid_param_counts_as_read` 与
/// `write_commands_rejected` 两个测试互为反向约束。
///
/// **加进这个表前必须确认该命令不会改变模组状态**。判断依据是手册的
/// 「测试形式」列：`<cid>` 出现在测试形式里说明是选对象，不是赋值。
const QUERY_WITH_PARAM: &[&str] = &[
    "AT+CGEQOSRDP=", // cid 的 QCI 查询（LuCI 概览 QCI 行）
    "AT^DSAMBR=",    // cid 的 AMR 下/上行速率查询（LuCI 概览带宽行）
    "AT+CGPADDR=",   // cid 的地址查询（AT+CGPADDR=1 等价于无参形式）
    "AT^NRRCCAPQRY=", // band 查询：=2 VoNR / =3 NR CA / =5 DSS（都是查能力）
];

/// `=` 后是否**恰好**跟一个纯数字（cid 选择参数）。
fn has_single_numeric_arg(c: &str) -> bool {
    match c.split_once('=') {
        None => false,
        Some((_, rest)) => {
            let arg = rest.trim();
            !arg.is_empty() && arg.bytes().all(|b| b.is_ascii_digit())
        }
    }
}

/// 判定一条 AT 命令是否为「读」。
///
/// 读 = 纯查询，不改变模组状态：
///   * `AT^X?` / `AT+X?`：标准查询后缀
///   * `AT^X` / `AT+X` / `ATI` / `AT+CGSN`：无参查询
///   * `AT^MONNC` / `AT+CGPADDR` / `AT^CGEQOSRDP`：厂商无参查询
///   * `QUERY_WITH_PARAM` 表内命令：带 cid/index 选择参数的查询
///
/// 写 = 任何其它带 `=` 或 `>` 的赋值、以及动作类命令（拨号/挂断/复位）。
///
/// **误判方向的选择**：宁可把读判成写（走老路，行为与今天一致），
/// 也不要���写判成读（那会让拨号变成查缓存，直接坏功能）。
pub fn is_read_command(command: &str) -> bool {
    let c = command.trim().to_ascii_uppercase();
    if !c.starts_with("AT") {
        return false;
    }
    // 查询型赋值先判定（例外规则），再落到"含 = 即写"。
    // 注意 has_single_numeric_arg：`AT^DSAMBR=1` 读，`AT^DSAMBR=1,2` 写。
    if QUERY_WITH_PARAM.iter().any(|q| c.starts_with(q)) && has_single_numeric_arg(&c) {
        return true;
    }
    // 赋值 / 动作：直接判写。
    if c.contains('=') || c.contains('>') {
        return false;
    }
    // 明确的动作类命令（无= 但有副作用）。
    const ACTION: &[&str] = &[
        "ATD", "ATA", "ATH", "AT+CHUP", "AT+RESET", "AT^RESET", "AT&F", "ATE0", "AT&W",
        "AT+CFUN", "AT^HVSST", "AT^FWUP", "AT^DSFLOWCLR", "AT^MCS", "AT^MONNC=",
    ];
    for a in ACTION {
        if c == *a || c.starts_with(&format!("{}=", a)) {
            return false;
        }
    }
    // 查询后缀。
    if c.ends_with('?') {
        return true;
    }
    // 无参查询白名单（动词型/信息型）。
    //
    // 这张表是「既不带? 也不带 =」的查询命令唯一入口。MT5700 手册里
    // 这类命令不少，且**都没有 ? 后缀**（厂商自定义风格），漏一条就
    // 会让页面每次刷新都真发 AT —— 实测 `AT+CNUM` 就是这么被漏掉的：
    // cmd_status 的 4 条 extras 查询里，它是唯一没进 raw 缓存的。
    //
    // 注意匹配顺序：含 `=` / `>` 的在上游已判写，所以这里用
    // `starts_with` 不会把 `AT^MONSC=xxx` 之类的赋值误收。
    const READONLY_VERB: &[&str] = &[
        "ATI",
        "AT+CGSN",
        "AT+CGMR",
        "AT+CIMI",   // IMSI
        "AT+CNUM",   // 本机号码（MSISDN）
        "AT+CGPADDR",
        "AT+CGEQOSRDP",
        "AT^MONNC",  // 邻区列表
        "AT^MONSC",  // 服务小区
        "AT^MONSSC", // NSA 辅服务小区
        "AT^SYSINFOEX", // 系统信息扩展（无参查询，代价高）
        "AT^DSFLOWQRY", // 数据流统计
        "AT^FOTADLQ",   // FOTA 下载进度
        // 信号强度。3GPP 标准读命令，**没有 ? 后缀**。at_queue.rs 里的
        // 网速测量（spec_fast("AT+CSQ")）和 LuCI 信号格都依赖它。
        // 实测漏判时每次刷新都真发一条 AT。
        "AT+CSQ",
        "AT+CPMS?",
        "AT+CSMS?",
        "AT+CSMP?",
        "AT+CMGF?",
        "AT+CSCA?",
        "AT+CLCC",
        "AT^SIMSQ?",
        "AT^CPIN?",
        "AT+CLCK?",
    ];
    READONLY_VERB.iter().any(|v| c.starts_with(v))
}

/// 单飞登记表：哪些 raw 命令当前正在采集。
///
/// 作用是「同一时刻同一条命令只下发一次 AT」。前端 1 秒刷 10 次时，
/// 第 2~10 次都会在这里命中，直接复用第 1 次的结果。
fn inflight() -> &'static Mutex<HashSet<String>> {
    static INFLIGHT: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    INFLIGHT.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 全局闸门开关。默认开启；`mt5700m-at config` 可关（回滚到旧行为）。
fn gate_enabled() -> bool {
    static ENABLED: AtomicBool = AtomicBool::new(true);
    // 允许通过环境变量紧急关闭（排障用），默认始终开启。
    if std::env::var("MT5700M_READ_GATE").as_deref() == Ok("0") {
        return false;
    }
    ENABLED.load(Ordering::Relaxed)
}

/// 闸门处理结果。
pub enum Gate {
    /// 命中缓存，直接返回这段文本（零 AT、零阻塞）。
    Cached(String),
    /// 无缓存可返：已提交后台采集，前端应显示「采集中」占位。
    Pending,
    /// 不是读命令，或闸门关闭 → 调用方按老路直通 AT。
    Passthrough,
}

/// 闸门入口。
///
/// * `force` —— 用户显式点「刷新」时置 true：忽略缓存新鲜度，但**仍然
///   不阻塞**（提交后台采集后返回 Pending 或旧值）。绝不因为 force 就
///   同步等 AT —— 那正是「前端拖慢后端」的最后一条漏路。
pub fn gate(
    command: &str,
    cache: &Arc<StateCache>,
    tasks: &Arc<TaskManager>,
) -> Gate {
    if !gate_enabled() {
        return Gate::Passthrough;
    }
    let cmd = command.trim().to_string();
    if !is_read_command(&cmd) {
        return Gate::Passthrough;
    }

    // 快路径：命中且新鲜 → 零成本返回。
    if let Some(entry) = cache.entry(&raw_topic(&cmd)) {
        let age = entry.timestamp.elapsed();
        if age < raw_ttl(&cmd) {
            if let Value::Str(s) = &entry.value {
                return Gate::Cached(s.clone());
            }
        }
    }

    // SWR：有过期数据就先返回旧值（页面永远有东西显示），再后台刷新。
    let stale = cache
        .entry(&raw_topic(&cmd))
        .and_then(|e| match &e.value {
            Value::Str(s) => Some(s.clone()),
            _ => None,
        });

    // 单飞：已在采集就不再提交第二条 AT。
    let key = raw_topic(&cmd);
    {
        let mut set = inflight().lock().unwrap();
        if set.contains(&key) {
            return match stale {
                Some(s) => Gate::Cached(s),
                None => Gate::Pending,
            };
        }
        set.insert(key.clone());
    }

    // 提交后台采集。失败路径必须把 key 还回来，否则这条命令会被永久
    // 标记为「飞行中」，之后永远返回 Pending。
    let cmd_for_task = cmd.clone();
    let key_for_task = key.clone();
    let cache_for_task = cache.clone();
    tasks.submit(
        &format!("raw.gate{}", key_for_task),
        TaskKind::BackgroundQuery,
        Priority::Low,
        Some(raw_timeout(&cmd_for_task) + Duration::from_secs(4)),
        Box::new(move |ctx: &TaskCtx| {
            let out = collect_raw(ctx, &cmd_for_task);
            // 无论成败都要清 inflight，否则失败一次就永久卡住。
            inflight().lock().unwrap().remove(&key_for_task);
            if let Some(text) = out {
                cache_for_task.set_ttl(
                    &key_for_task,
                    Value::Str(text.clone()),
                    "raw-gate",
                    raw_ttl(&cmd_for_task),
                );
            }
            Ok(Value::Null)
        }),
    );

    match stale {
        Some(s) => Gate::Cached(s),
        None => Gate::Pending,
    }
}

/// raw 缓存的 topic 键。加前缀避免与业务 topic 撞名。
pub fn raw_topic(command: &str) -> String {
    format!("{}{}", TOPIC_RAW, command.trim().to_ascii_uppercase())
}

/// 真正下发的那一次 AT。
///
/// 以 `ui_query` 打底（它已经带正确的 retry/timeout/dedup 约定），只把
/// 优先级降到 `Low`：**闸门采集永远排在用户写操作之后**，即便用户正在
/// 拨号，采集器也不会插队。这保证「前端刷新再快也不可能拖慢用户操作」。
fn collect_raw(ctx: &TaskCtx, command: &str) -> Option<String> {
    let mut spec = AtRequestSpec::ui_query(command);
    spec.priority = Priority::Low;
    spec.label = format!("raw gate {}", command);
    spec.dedup_key = Some(format!("raw:{}", command.trim().to_ascii_uppercase()));
    match ctx
        .arbiter
        .submit(spec)
        .recv_timeout(raw_timeout(command) + Duration::from_secs(12))
    {
        Ok(Ok(text)) => Some(text.trim().to_string()),
        _ => None,
    }
}

/// 写操作后调用：把受影响的 raw 条目作废，下一次读会重新采集。
///
/// 例如用户改了 APN → `AT^CGPADDR` / `AT^DHCP?` 的缓存必须立刻失效，
/// 否则页面会继续显示旧地址。
pub fn invalidate_related(cache: &Arc<StateCache>, written_command: &str) {
    let c = written_command.trim().to_ascii_uppercase();
    // 写命令 → 受影响的读命令。保守做法：任何写都作废全部 raw。
    // raw 条目数量有限（< 100），且写操作是低频用户行为，全量作废的
    // 代价（下次读重新采集一次）远小于漏作废导致的「页面显示旧值」。
    let _ = c;
    if let Some(entries) = cache.entries_snapshot() {
        for topic in entries {
            if topic.starts_with(TOPIC_RAW) {
                cache.invalidate(&topic);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_commands_detected() {
        for c in [
            "AT^DHCP?",
            "AT^DHCPV6?",
            "AT^IPV6CAP?",
            "AT+CGACT?",
            "AT^LTEFREQLOCK?",
            "AT^NRFREQLOCK?",
            "AT^C5GOPTION?",
            "AT^NRSSBID?",
            "AT^THERMAUTOFUN?",
            "AT^SYSCFGEX?",
            "AT^SCICHG?",
            "AT^TDSIMHP?",
            "AT^TDPCIELANCFG?",
            "AT^TDPMCFG?",
            "ATI",
            "AT+CGSN",
            "AT+CGMR",
            "AT+CPIN?",
            "AT^SIMSQ?",
            "AT+CSCA?",
            "AT+CMGF?",
            "AT+CPMS?",
            "AT^FOTASTATE?",
            "AT+C5GREG?",
            "AT^MONNC",
            "AT+CGPADDR",
            "AT+CLCC",
            "AT^LENDC?",
        ] {
            assert!(is_read_command(c), "应判为读: {c}");
        }
    }

    #[test]
    fn write_commands_rejected() {
        for c in [
            "AT^EONS=2",
            "AT^DSAMBR=1,2", // 赋值形式（非 cid 选择）→ 写
            "AT^MCS=1",
            "AT^C5GOPTION=1,3,2",
            "AT+CSCA=\"+8613800138000\"",
            "AT+CMGF=0",
            "AT+CMGS=42",
            "AT+CPMS=\"SM\",\"SM\",\"SM\"",
            "AT+CGDCONT=1,\"IP\",\"cmnet\"",
            "AT+CFUN=0",
            "AT^RESET",
            "AT&F",
            "ATD10086;",
            "ATA",
            "ATH",
            "AT+CHUP",
            "AT^DSFLOWCLR",
            "AT^TDPMCFG=1",
        ] {
            assert!(!is_read_command(c), "应判为写: {c}");
        }
    }

    /// 带 `=` 的查询型命令：`=` 后只跟 cid/index，必须判读。
    ///
    /// 这条是实机踩出来的：LuCI 概览页每次刷新都下发
    /// `AT+CGEQOSRDP=1`（QCI 行）和 `AT^DSAMBR=1`（签约带宽行），
    /// 按"含 = 即写"判会让它们绕过闸门直压 AT 通道。
    #[test]
    fn query_with_cid_param_counts_as_read() {
        for c in [
            "AT+CGEQOSRDP=1",
            "AT+CGEQOSRDP=8",
            "AT^DSAMBR=1",
            "AT^DSAMBR=5",
            "AT+CGPADDR=1",
            "AT^NRRCCAPQRY=2",
            "AT^NRRCCAPQRY=3",
            "AT^NRRCCAPQRY=5",
        ] {
            assert!(is_read_command(c), "查询型赋值应判为读: {c}");
        }
    }

    /// 反向约束：同族命令只要不是"单个纯 cid"，一律判写。
    ///
    /// `AT^DSAMBR=1,2` 是**设置** cid1 的 AMR=2（真写操作），
    /// `AT^DSAMBR=` / `AT^DSAMBR=abc` 是畸形参数。两者都必须直通AT，
    /// 否则用户改不了 AMR，或畸形参数被缓存成一个假值。
    #[test]
    fn non_cid_param_variants_stay_write() {
        for c in [
            "AT^DSAMBR=1,2",
            "AT^DSAMBR=",
            "AT^DSAMBR=abc",
            "AT^DSAMBR=1,2,3",
            "AT+CGEQOSRDP=1,2",
            "AT+CGEQOSRDP=",
            "AT^NRRCCAPQRY=",
            "AT^NRRCCAPQRY=2,1",
            "AT^NRRCCAPQRY=x",
            // QRY 是查询、CFG 是配置 —— 两者只差三个字母，最容易误判。
            "AT^NRRCCAPCFG=2,1", // 真赋值（配置 NR CA 能力）
            "AT^NRRCCAPCFG=5,1,0", // 真赋值（三参数）
        ] {
            assert!(!is_read_command(c), "非单cid 参数应判为写: {c}");
        }
    }

    #[test]
    fn ttl_tiers() {
        // 慢命令必须给足退避。
        assert!(raw_ttl("AT^LENDC?") >= Duration::from_secs(300));
        assert!(raw_ttl("AT^NTXPOWER?") >= Duration::from_secs(300));
        // 快命令要勤。
        assert!(raw_ttl("AT+CPIN?") <= Duration::from_secs(10));
    }

    /// 无参查询（既无 `?` 也无 `=`）必须判读。
    ///
    /// 实机踩出来的漏洞：`AT+CNUM` 属于这类——没有 `?` 后缀，也没在
    /// 动词白名单里，于是被判成写命令，每次LuCI 概览刷新都真发一条 AT。
    /// `cmd_status` 的 4 条 extras 查询里它是唯一没进 raw 缓存的，
    /// 实测边际成本 368 ms/次就是它贡献的。
    #[test]
    fn paramless_queries_count_as_read() {
        for c in [
            "AT+CIMI",     // IMSI
            "AT+CNUM",     // 本机号码 MSISDN
            "AT^MONSC",    // 服务小区
            "AT^MONSSC",   // NSA 辅服务小区
            "AT^DSFLOWQRY", // 数据流统计
            "AT^FOTADLQ",  // FOTA 下载进度
        ] {
            assert!(is_read_command(c), "无参查询应判为读: {c}");
        }
    }

    /// 反向约束：无参白名单不能把同族的动作/赋值命令误收。
    ///
    /// 白名单用 `starts_with` 匹配，所以必须确认没有哪条动作命令是某条
    /// 查询命令的前缀。`AT^CELLSCAN`（启动扫描）和 `AT&F0`（恢复出厂）
    /// 都不带 `=` 但有副作用，绝不能被判读。
    #[test]
    fn paramless_actions_stay_write() {
        for c in [
            "AT^CELLSCAN", // 启动小区扫描 → 动作
            "AT&F0",       // 恢复默认配置 → 动作
            "AT^MONSC=1",  // 带参数 → 上游已判写
            "AT^MONSSC=1",
            "AT^DSFLOWQRY=1",
            "AT^FOTADLQ=0",
        ] {
            assert!(!is_read_command(c), "动作/赋值应判为写: {c}");
        }
    }

    /// 全量反查：后端 cli.rs / snapshot.rs 里真实下发的每一条读命令，
    /// 都必须被判读。
    ///
    /// 这条测试是「漏一条就回归」的最后一道防线。上面几条是点状的
    /// 已知用例；这条是集合级的——新增读命令若忘了加白名单或加 `?`，
    /// 立刻在这里失败，而不是等到实机表现为「页面偶尔不显示数据」。
    ///
    /// 清单来源：`grep -ohE '"AT[+^&][^"]*"' src/cli.rs src/snapshot.rs`
    /// （只收录纯查询形式，剔除所有 `=` 赋值 / 动作命令）。
    /// 改动AT 调用点后请重新 grep 同步本清单。
    #[test]
    fn all_backend_read_commands_are_classified_as_read() {
        for c in [
            // ---- 3GPP 标准读 ----
            "AT+C5GREG?", "AT+CEREG?", "AT+CEUS?", "AT+CFUN?", "AT+CGACT?",
            "AT+CGDCONT?", "AT+CGMR", "AT+CIMI", "AT+CIREG?", "AT+CLCC?",
            "AT+CMGF?", "AT+CNUM", "AT+COPS?", "AT+CPIN?", "AT+CPMS?",
            "AT+CREG?", "AT+CSCA?", "AT+CSQ",
            // cid 选择型查询（`=` 后恰好一个纯数字）
            "AT+CGEQOSRDP=1", "AT+CGPADDR=1", "AT^DSAMBR=1", "AT^DSAMBR=8",
            // ---- 厂商读：无 ? 后缀，靠动词白名单 ----
            "AT^MONNC", "AT^MONSC", "AT^MONSSC", "AT^DSFLOWQRY", "AT^FOTADLQ",
            // ---- 厂商读：? 结尾，标准查询形式 ----
            "AT^C5GOPTION?", "AT^CASCELLINFO?", "AT^CHIPTEMP?", "AT^DCONNSTAT?",
            "AT^DHCP?", "AT^DHCPV6?", "AT^FOTAMODE?", "AT^HCSQ?", "AT^HFREQINFO?",
            "AT^HVSST?", "AT^ICCID?", "AT^IMSSWITCH?", "AT^IPV6CAP?", "AT^LEDSWITCH?",
            "AT^LENDC?", "AT^LTEFREQLOCK?", "AT^NDISSTATQRY?", "AT^NRFREQLOCK?",
            "AT^NRSSBID?", "AT^NTXPOWER?", "AT^NWTIME?", "AT^PDCPDATAINFO?",
            "AT^RRCSTAT?", "AT^SCICHG?", "AT^SETAUTODIAL?", "AT^SETDIRECTIP?",
            "AT^SETMODE?", "AT^SYSCFGEX?", "AT^SYSINFOEX", "AT^TDCFG?",
            "AT^TDPCIELANCFG?", "AT^TDPMCFG?", "AT^TDSIMHP?", "AT^THERMAUTOFUN?",
            "AT^THERMLDAUTOPARA?", "AT^THERMLDAUTOSTATUS?", "AT^THERMLDLOGSW?",
            "AT^TXPOWER?", "AT^VERSION?",
            // 带 cid/band 的查询型赋值
            "AT^NRRCCAPQRY=2", "AT^NRRCCAPQRY=3", "AT^NRRCCAPQRY=5",
        ] {
            assert!(is_read_command(c), "后端真实读命令被判成了写（闸门会漏）: {c}");
        }
    }

    #[test]
    fn topic_prefix_isolates_from_business_topics() {
        let t = raw_topic("AT+CPIN?");
        assert!(t.starts_with(TOPIC_RAW));
        assert!(!t.contains("sim")); // 不能和业务 topic 撞名
    }
}