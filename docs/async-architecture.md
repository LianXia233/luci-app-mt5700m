# MT5700M 后端异步化架构（Async Architecture）

> 从 v2.7 起，Rust 后端在「独占 AT 通道」基础上升级为 **事件驱动 + 异步任务调度 +
> 状态缓存 + 实时事件推送**。核心目标：**任何单个慢 AT 操作都不能阻塞 LuCI /
> WebUI / WebSocket**；页面打开不等待模组；多页面共享缓存、不再重复请求；后台轮询
> 统一收敛、不浪费串口与 CPU。

## 1. 为什么异步化

旧实现是「请求驱动 + 同步等待 AT」：

- 每个 HTTP / WebSocket / LuCI 请求直接触发 AT，并**阻塞到模组响应**。
- 多个页面同时查询产生**重复 AT**，抢占同一串口。
- 后台轮询由各页面用 `setInterval` 独立驱动，白白消耗串口与 CPU。
- 一个慢 AT（扫描、PDP、短信）会**卡住整个请求线程**，乃至整个后端。

异步化把「请求」与「模组 I/O」解耦：请求要么读共享缓存（秒回），要么进入统一任务
队列（返回 `task_id`），AT 交换由单一仲裁器串行化，结果通过事件推送，**没有任何
路径直接阻塞等模组**。

## 2. 总体分层

```
┌──────────────────────────┐   LuCI / WebUI 前端
│  快速读缓存 / 下发任务    │   读 StateCache 快照（SWR），订阅事件
└────────────┬─────────────┘
             ▼
┌──────────────────────────┐
│  WebSocket（事件驱动）    │   subscribe / unsubscribe / snapshot 控制帧
└────────────┬─────────────┘   实时推送 {type, data, timestamp} 事件
             ▼
┌──────────────────────────────────────────────────────────┐
│ Async Application Core                                     │
│   ┌───────────┐ ┌──────────────┐ ┌────────────────────┐  │
│   │ StateCache │ │ TaskManager  │ │ EventBus           │  │
│   │  (SWR/TTL) │ │ 生命周期/周期 │ │ (主题/合并/即时)    │  │
│   └─────┬─────┘ └──────┬───────┘ └─────────┬──────────┘  │
└─────────┼──────────────┼──────────────────┼──────────────┘
          │              │                  │
          └──────────────┼──────────────────┘
                         ▼
┌────────────────────────────────────────┐
│ AtArbiter：AT 请求队列 / 仲裁器          │  单执行线程 · 优先级 · 去重 ·
│ 超时 · 重试 · 独占 · 背压 · 协作取消     │  慢 AT 不再卡死任何路径
└──────────────────┬─────────────────────┘
                   ▼
┌────────────────────────────────────────┐
│ AtTransport：串口(TIOCEXCL) / 网络 AT    │
└──────────────────┬─────────────────────┘
                   ▼
              MT5700M 模组
```

三条原则贯穿始终：

- **UI ≠ Modem**：UI 只经 API/WebSocket 与 Application Core 对话。
- **状态单向流**：Modem → State Collector → StateCache → EventBus → UI。
- **任务单向流**：UI → Task Request → Scheduler → Worker → Result → Event → UI。

## 3. 模块职责

| 模块 | 职责 |
|:---|:---|
| `core/runtime.rs` | 轻量线程运行时工具（`spawn_thread` / `next_id` / `now_ms`），std-only 低依赖，保持静态链接与 OpenWrt 交叉编译稳定 |
| `core/error.rs` | 统一错误模型 `BackendError`：机器可读 `code` + 人类可读 `message` + `retryable`；禁止打印 IMEI/IMSI/短信内容等敏感数据 |
| `core/task.rs` | 任务模型：生命周期、`Priority` 档次、超时/重试策略、JSON 序列化 |
| `core/radio.rs` | 全后端唯一的 ARFCN → 频段表 |
| `scheduler/jobs.rs` | 任务调度中心（`TaskManager`）：注册/跟踪/取消/超时/周期任务（周期抓取快照写入 StateCache、开机预热），发布 `task.*` 生命周期事件 |
| `scheduler/arbiter.rs` | **AtArbiter**：单执行线程串行化全部 AT，优先级/去重/超时/重试/独占/协作取消/背压 |
| `scheduler/gate.rs` | **读命令缓存闸门**：读命令命中 raw 缓存即零 AT 返回，写命令直通 |
| `scheduler/plan.rs` | 昼夜频段锁定计划：按时间应用 `^LTEFREQLOCK` / `^NRFREQLOCK` |
| `state/cache.rs` | **StateCache（SWR）**：signal/network/registration/temperature/traffic/cell/sim/modem/usb |
| `state/bus.rs` | **EventBus**：主题订阅/发布，高频遥测 100ms 合并（coalescing），task/usb/modem/sms/scan 即时直推 |
| `state/refresh.rs` | **RefreshCtx**：模块 service 共用的刷新上下文（AT + 缓存 + 总线），退避/陈旧回退/占空比让行策略只此一份 |
| `serial/presence.rs` | **设备监视器**：USB 热插拔监督（在位 / 模式变化 → 缓存失效、取消无效任务、重连传输层、推事件） |
| `daemon.rs` | 接线层：HTTP/控制套接字/WebSocket→「读缓存 / 下发任务」，WebSocket 事件推送 |

## 4. Task Scheduler 设计

**生命周期**：`Queued → Running → (Completed | Failed | Cancelled | Timeout)`

**任务类型**：
- **Fast Query**（IMEI/RSRP/RSRQ/SINR/温度/IP）：优先读 StateCache。
- **Background Query**（SIM/运营商/CA/流量/注册）：后台定期刷新。
- **Interactive Task**（改 APN/频段/PDP/短信）：立即入队。
- **Long Running Task**（小区/邻区/Beam/频段扫描、网络恢复、模组初始化）：后台执行，不阻塞任何路径。
- **Exclusive Task**（固件升级、扫描、模组重启、恢复出厂、特定长 AT）：独占 AT 通道，同一时间仅一个。
- **Periodic Task**：signal 2s / network 2s / registration 3s / temperature 5s / traffic 5s / cell 5~10s / sim 30s / modem 60s，周期任务**互不重叠**执行，background 优先级避免饿死用户操作。

**优先级**：`Critical > Interactive > High > Normal > Background`。用户手动 AT > 网络恢复 >
页面查询 > 后台刷新，后台轮询不会把用户操作排到很后面。

## 5. AtArbiter（AT 调度器）设计

所有 AT 交换的**唯一入口**，单执行线程驱动：

- **串行化**：一次一条命令，响应不交错，杜绝串口争抢与响应错位。
- **去重（Deduplication）**：同一时间相同只读请求（如多页面 `AT+CSQ`）合并为一次 AT，N 个请求 → 1 次交换，共享结果。
- **超时（Timeout）**：Fast 1~3s、Normal 3~5s、Network 5~15s、Scan 30~120s、Long 自定义；超时释放资源、记录错误、必要时重新调度，不卡死队列。
- **重试（Retry）**：timeout / USB 临时不可用 / UBUS 临时失败才重试（带 backoff）；参数错误、命令不存在、权限错误、模组明确拒绝**不重试**，避免重试风暴。
- **独占（Exclusive）**：扫描/重启等独占时，普通请求排队并在队列截止时间内干净过期。
- **背压（Backpressure）**：队列上限 + 截止时间，100 个 status 请求最终只会产生少数实际 AT 快照，绝不无限造任务。
- **协作取消（Cancellation）**：`send_interruptible` 在每次读周期轮询取消标志，可注入 abort token（如扫描中止），直到模组平静。

## 6. StateCache 设计（Stale-While-Revalidate）

每项状态含 `value + timestamp + ttl + source + status`。页面读取逻辑：

```
Cache Fresh  ──► 立即返回（缓存命中 <1ms）
Cache Expired ──► 立即返回旧值，同时后台刷新（SWR），绝不因过期而同步等待 AT
```

后台刷新完成后：`Cache Update → EventBus → WebSocket → 前端自动更新`。

写操作（APN/PDP/频段/重启/SIM）会**主动使受影响缓存失效**，再后台重取。

## 7. EventBus 设计

主题订阅/发布。高频遥测（RSRP/SINR/温度/流量）做 100ms 窗口**合并**，只推最新值；
task/usb/modem/sms/scan 即时直推。事件示例：

```json
{ "event": "signal.updated", "data": { "rsrp": -86, "rsrq": -11, "sinr": 18 }, "timestamp": 1234567890 }
```

典型事件：`SignalUpdated` `NetworkUpdated` `CellUpdated` `TemperatureUpdated`
`TrafficUpdated` `SimUpdated` `RegistrationChanged` `UsbStateChanged`
`ModemStateChanged` `TaskStarted/Progress/Completed/Failed` `ScanStarted/Progress/Completed`
`SmsReceived`。

## 8. WebSocket 设计（事件驱动）

禁止 `while: query modem sleep`。服务器由 EventBus 驱动向订阅者推事件：

```
connect → 可选 { action: subscribe, topics:[...] } → 服务端只推订阅主题
       → { action: snapshot } → 返回 StateCache 快照（零 AT）
       → 断线重连 → 重新订阅 + 拉 snapshot 恢复
```

前端状态模型统一为 `loading / ready / stale / updating / unavailable / error`
（而非裸 `null`），可区分「没数据 / 正在更新 / 模组不存在 / 请求失败」。

## 9. HTTP API 兼容性

- **保留旧同步接口**，内部转换为 `request → scheduler → await task → result`，不破坏既有调用。
- **新增异步接口**：`GET /api/state`、`GET /api/state/:topic`（读缓存）、`POST /api/task`（返回 `task_id`）、
  `GET /api/task/:id`、`POST /api/task/:id/cancel`、`WS /api/events`；旧调用仍可用 `fire-and-observe`。
- CLI 新增 `cached` 子命令读取缓存快照，既有契约不变。

## 10. 关键不变项

- 统一走 AtArbiter，禁止页面/后台/WebSocket 各自直连串口。
- 慢 I/O（`std::process::Command` / `std::fs` / 阻塞串口）若不可避免则用 `spawn_thread` 隔离，**禁止在异步路径直接阻塞**。
- 并发安全按场景选 `Mutex` / `Arc` / 原子 / 通道，不滥用锁。
- 空闲后台任务数量可控，避免 Task Explosion（去重、节流、背压、周期不重叠）。
- OpenWrt mediatek/filogic aarch64_cortex-a53 交叉编译稳定，`release static strip`，固件体积可控。
---

## 11. 实测验证结果（2026-10-05）

改造已部署到实机（192.168.10.1），后端二进制
`aarch64-unknown-linux-musl` 818,328 B / md5 `077bff7191b9455265b66c60fbaac361`，
`cargo test` 104 passed / 0 failed（2026-10-05 时点数字；架构 v2 收官后
用例已增长到 **292**，见 `docs/architecture-v2/migration.md`）。

### 11.1 闸门生效证据

| 指标 | 改造前 | 改造后 | 改善 |
|---|---|---|---|
| 单次读命令 `AT^DHCP?` | 0.32 s | 0.07 s | 4.6x |
| 10 次串行 | 2.03 s | 0.15 s | 13.5x |
| **边际斜率** | ~200 ms/次 | **8.9 ms/次** | **22.5x** |
| `raw:` age_ms | 每次被刷回 0 | 单调增长 | 缓存复用 |

判据用**边际斜率**而非「总耗时 < 单次 × N」：单档里含 SSH 建连与进程首启的
一次性开销（~50 ms），拿它当基线会误判（实测0.15 s vs 0.07×2 = 0.14 s，
只差 0.01 s 就FAIL）。真正要证明的是「每多刷一次多花多少毫秒」。

### 11.2 三条后端路径均已覆盖

| 路径 | 入口 | 覆盖情况 |
|---|---|---|
| JSON-RPC `at` | `daemon.rs: run_command` | 已接闸门 |
| control socket `send` | `daemon.rs: handle_control_request` | 已接闸门（**LuCI 全部读命令走这条**） |
| CLI fork 子进程 | `daemon.rs: cli_capture` → `cli.rs: run_at` | 复用 control socket，自动覆盖 |

第二处是最容易漏的：只接 `run_command` 时闸门等于没装，实测表现为
`mt5700m-at cached` 里始终没有 `raw:` 主题。

### 11.3 判定漏洞修复

实机验证暴露：`AT+CNUM` 未入缓存。根因是**无参查询**（既无 `?` 也无 `=`
也不在动词白名单）被判成写命令。修复后新增 4 组测试锁死：

- `paramless_queries_count_as_read` —— 无参查询判读
- `paramless_actions_stay_write` —— `AT^CELLSCAN` / `AT&F0` 等动作仍判写
- `all_backend_read_commands_are_classified_as_read` —— **全量反查**：清单来自
  `grep -ohE '"AT[+^&][^"]*"' src/api/cli.rs src/modules/*/commands.rs`，新增读命令忘加白名单
  会立刻测试失败，而不是等到实机表现为「页面偶尔不显示数据」。这条测试当场
  抓出漏判的 `AT+CSQ`（信号强度，arbiter 网速测量依赖它）与 `AT^SYSINFOEX`。
- `non_cid_param_variants_stay_write` —— `AT^NRRCCAPCFG=5,1,0`（真赋值）
  不得被 `AT^NRRCCAPQRY=`（查询）的前缀规则误收。

补进白名单的无参查询：`AT+CNUM`、`AT+CIMI`、`AT+CSQ`、`AT^MONSC`、
`AT^MONSSC`、`AT^DSFLOWQRY`、`AT^FOTADLQ`、`AT^SYSINFOEX`。

### 11.4 并发共存验证

4 worker × 6 轮，混合 control socket / JSON-RPC / CLI fork 三条路径，
全程无 sleep：

```
完成 24 次调用，总墙钟 0.57 s（平均 24 ms/次）
  control  n=12  平均 61 ms
  rpc      n=6   平均 51 ms
  fork     n=6   平均 77 ms
age_ms 单调增长: 是   [3471 → 7482 → 11492]
后端进程存活 / 端口 8765 LISTEN / netrate 快照 fresh
结论: PASS
```

`age_ms` 在轰炸结束后仍持续单调增长，证明**零重采集**——AT 流量与前端
刷新率完全无关。

### 11.5 页面层实测

- LuCI 7/7 标签页正常，无「未连接」标记，零 JS 错误（独立标签页）。
- WebUI 直开零 JS 错误，`/5700/` 与两个 asset 均 200。
- 双标签页各刷新 3 轮后，LuCI 2058 字 / WebUI 1151 字，数据同源。
- LuCI 数据方法 `cached` / `netrate` / `traffic` / `logs` / `usb` 均
  0.05~0.09 s 返回；`mt5700m-at status` 0.08 s（改造前 0.296 s）。

### 11.6 两个非本次引入的问题（仅记录，不越界修改）

1. **LuCI 核心 `E is not defined` / `findParent is not defined`**
   堆栈指向 `/luci-static/resources/luci.js:184` 与 `ui.js:287`
   `showTooltip`，**未进入本插件即复现**，是 ImmortalWrt 上游 LuCI 既有缺陷。

2. **`AT+CNUM` 模组不应答**
   中国移动网络下返回空串，故采集失败且**故意不写缓存** —— 把空值缓存
   起来等于把「不支持」固化，之后每次访问都白下发一条 AT。属正确行为。

### 11.7 IMEI

本轮及验证脚本全程未下发任何 IMEI 命令。`AT^PHYNUM` 未纳入白名单、
未写入任何测试断言；`AT+CGSN` 保持既有白名单条目不动（无新增测试）。
WebUI `pages/system/Info.tsx` 的 IMEI 写入入口按用户指示「暂不动」，
仍是遗留红线隐患。
