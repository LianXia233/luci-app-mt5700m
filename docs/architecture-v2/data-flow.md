# 数据流与单一 AT 所有者

## 1. 流水线

```text
AT response ──▶ modules/<x>/parser.rs ──▶ modules/<x>/state.rs (typed)
                                             │
                              to_json()      │      to_text()   (CLI only)
                                 ▼           ▼
                        StateCache topic ──▶ EventBus ──▶ WS push / events RPC
                                 │
                                 ▼
                     api/registry.rs route ──▶ WS envelope / control socket /
                                               JSON-RPC / CLI stdout
```

单向流动，每个主题一个写入者，每种响应一个解码器。前端
无法触达 AT 层：它只能调用路由或读取主题。

## 2. 谁写哪个主题

| 主题 | 写入者（模块 service） | 周期 | 失败行为 |
| ----- | ----------------------- | ------- | ----------------- |
| `signal` | `signal::service::refresh` | 15 s | 回退到旧值，120 s 退避 |
| `network` | `network::service::refresh` | 30 s | 按命令 60 s / 300 s 退避 |
| `registration` | `network::service::refresh_registration` | 20 s | 主题始终被写入 |
| `cell` | `cell::service::refresh` | 120 s | 按命令退避，TAC/CI 来自 `registration` |
| `ca` | `ca::service::refresh` | **按需**（`ca.get`） | 保留上一次的图景，按命令 60/120/300 s 退避 |
| `qos` | `qos::service::refresh` | **按需**（`qos.get`） | 保留部分值，按命令 60/120 s 退避 |
| `sim` | `sim::service::refresh` | 60 s | 按命令退避 |
| `modem` | `modem::service::refresh_info` | 60 s | 60 s 退避，显式 60 s TTL |
| `txpower` / `endc` | `modem::service::refresh_*` | 300 s | 600 s 退避 |
| `nr_txpower` | `modem::service::refresh_nr_txpower` | 180 s | 600 s 退避 |
| `temperature` | `system::service::refresh` | 60 s | 回退到旧值 |
| `traffic` | `traffic::service::refresh` | 30 s | 空对象 |
| `netrate` | `traffic::service::refresh_netrate` | 5 s | `available:false` + 原因 |
| `scan` | `cell::scan::start`（独占任务） | **按需**（`cell.scan_start`） | 每次扫描推送一次 `{state, cells, count}`；`cell.scan_state`/`cell.scan_abort` 属于任务自省 |
| `fota` | `system::fota::start`（长跑任务） | **按需**（`system.fota_start`） | 每次状态变化推送一次 `fota.progress`；`system.fota` 从快照作答，`system.fota_abort` 取消该任务 |
| `schedule` | `network::schedule::write`（UCI） | 保存时 | 不是调制解调器主题：`scheduler::plan` 每 15 s 重读 UCI，`network.schedule_get` 在页面加载时读取它 |
| `sms`, `task`, `usb`, `beam`, `raw:*` | 守护进程/URC/分发路径 | 事件驱动 | 各主题不同。URC 路径是调制解调器主动上报的一切的解码器：`+CUSD` → `sms.ussd`（`modules::sms::ussd`）、`^REJINFO` → `network.reject`（`modules::network::reject`）、`^DSAMBR` → `qos.ambr`（`modules::qos`）、`+CPIN`/`^SIMSQ`/`^SIMST` → `sim.changed`，而原始行仍作为 `raw_data` 发出，供诊断视图使用 |

TTL 定义在 `state/cache.rs`；发布统一走
`RefreshCtx::store`/`stale`，这是唯一既设置主题*又*
发出其事件的地方（因此主题不可能被静默更新）。

## 3. 单一 AT 所有者，时序如下

```text
module service / route handler
        │  ctx.read(cmd, at_timeout, queued_timeout, prio)  |  ctx.slow(...)
        ▼
core::channel::AtChannel            (trait the modules see)
        │
        ├── scheduler::channel::TaskChannel    (inside a periodic job)
        └── scheduler::channel::DirectChannel  (inside a request handler)
        ▼
scheduler::arbiter::AtArbiter  ── priority queue + read gate + dedup + retry
        ▼
scheduler::plan + transport::urc (URC demux) ──▶ daemon AtClient
        ▼
serial::manager  ── exclusive tty (TIOCEXCL, O_NOCTTY, O_NONBLOCK)
        ▼
modem
```

进程外的调用方走的是同一个仲裁器：

```text
mt5700m-at  ──▶ transport::client::at_cmd ──▶ control socket "send" verb ──┐
LuCI ucode  ──▶ daemon TCP RPC              ──▶ transport::urc path   ─────┤
WebUI       ──▶ WS RPC / api.<module>.<verb> ───────────────────────────▶ arbiter
```

CLI **没有**直连串口或裸 TCP 的回退：若守护进程未运行，`at_cmd`
返回 `DaemonFailed` 错误并指名 init 脚本。这是有意为之 ——
tty 上出现第二个写入者不是降级模式，而是一次被破坏的
调制解调器会话（并且会与守护进程自己的锁形成死锁）。同样的道理
也适用于 SMS：`mt5700m-at sms-send` 调用守护进程的 `sms.send` 路由，
由该路由编码 PDU 并在仲裁器之下跑完整个 `+CMGS` 事务 ——
CLI 既不拥有 PDU 编解码器，也不拥有发送序列。

客户端侧的端口识别只基于描述符（`detect_pcui_port`）；只有
守护进程可以跑 AT 探测（`auto_detect_serial`），且只在它取得所有权之前。
`atprobe` 为现场诊断而存在，它以独占方式打开端口，因此
守护进程存活时会拒绝运行。

## 4. 任务生命周期

```text
TaskManager::add_periodic / add_periodic_keepalive / add_periodic_always
   │
   ├── spawns at daemon start (modules::spawn_all → <module>::service::spawn)
   ├── each tick: run_in_task(ctx, f) builds TaskChannel + RefreshCtx
   │      └── RefreshCtx.priority = 任务优先级 → AT 仲裁器优先级队列
   ├── 前端活动闸门（state::activity::ActivityGate）
   │      ├── 活跃源：WS 连接（reader_enter/leave）+ 8765 RPC 请求时间戳（touch）
   │      ├── 控制套接字（mt5700m-at）不计入，否则常驻拨号守护会永久唤醒后端
   │      ├── 空闲时：add_periodic 暂停；keepalive 降为 idle_interval；
   │      │            always 保持全速（锁频调度器等系统功能）
   │      └── 恢复时：last_run 早于 interval 的任务立即触发 → 首帧按优先级刷新
   ├── deadline: queue timeout + AT timeout; on overrun the tick is abandoned
   ├── cancel: task flag checked by the arbiter before/while writing
   ├── errors: mapped to BackendError (never a panic); stale fallback keeps UI
   └── observability: task registry + `task.*` events → both frontends
```

## 5. 错误模型

`core::error::BackendError` 是跨越每一个边界（串口、仲裁器、模块、
路由、CLI）的唯一错误类型。每个变体都带有一个稳定的 `code()`
（`AT_TIMEOUT`、`BUSY`、`MODEM_UNAVAILABLE`、`INVALID_PARAMETER`……）
和一个 `retryable()` 标志，WS 信封会把它暴露出来：

```json
{"success": false, "error": "…", "code": "AT_TIMEOUT", "retryable": true}
```

前端只按 `code` 分支，绝不按消息文本。

## 6. 剩下两个系统辅助

| 辅助 | 角色 | 计划 |
| ------ | ---- | ---- |
| `mt5700m-traffic`（init-daemon + rpcd） | 把接口字节数计入 `/etc/mt5700m/traffic-history`，为 LuCI 与后端应答 `json`/`summary` | 今天已通过同一个 JSON 接口读取；把计数循环折进 `modules/traffic` 是下一步（移除最后一个非 Rust 业务进程） |
| `mt5700m-manager` + rpcd shim | 拨号胶水层（`AT^NDISDUP`、`AT+CFUN`、`^SETMODE`），由 hotplug/init 与 LuCI 的拨号按钮调用 | 它的 AT 调用已经走 `mt5700m-at` → 守护进程；把这些折进 `modules/network` 的动作（dial/redial/airplane）是再下一步 |
