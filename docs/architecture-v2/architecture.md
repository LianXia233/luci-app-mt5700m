# 架构：分层、模块、依赖规则

## 1. 进程模型

```text
/usr/bin/at-webserver            daemon: sole AT owner, StateCache, EventBus,
                                 AT scheduler, module services, WS + TCP RPC
  ├── argv0 mt5700m-at           CLI client: forwards to the daemon, renders text
  └── argv0 atprobe              operator-only port probe (exclusive lock,
                                 refuses to run while the daemon holds the tty)

LuCI   ucode plugin ──nc──▶ daemon TCP RPC (newline JSON)  ── rpcd → views
WebUI  WebSocket ─────────▶ daemon WS RPC                  ── React pages
```

不存在第二个后端。历史上的 `mt5700m-manager` shell 脚本与
`mt5700m-traffic` 计费守护进程保留为*系统辅助*，由 init/rpcd 胶水层调用；
它们碰 AT 的行为一律经守护进程转发（见
[data-flow.md](data-flow.md) 第 6 节）。`mt5700m-traffic` 是流量历史文件的
唯一写入者，后端与 LuCI 都通过同一个 JSON 接口读它。

## 2. 后端分层（自底向上）

| 层 | 路径 | 职责 | 可依赖 |
| ----- | ---- | -------------- | ------------- |
| core | `src/core/` | 错误模型、JSON、运行时、任务原语、sha1、`AtChannel` trait | 仅 std |
| serial | `src/serial/` | 独占式打开 tty、USB 在位监控、探测 | core |
| scheduler | `src/scheduler/` | AT 仲裁器（队列、优先级、重试、去重、占空比）、任务管理器、读闸门、命令计划 | core、serial、state |
| state | `src/state/` | `StateCache`（主题 TTL）、`EventBus`（合并、历史）、`RefreshCtx` | core |
| modules | `src/modules/<name>/` | 每个模块一个业务领域：commands、parser、state、service、api | core、state、scheduler（仅 channel） |
| transport | `src/transport/` | WS 服务端、TCP RPC / 控制套接字、URC 分发器、AT 客户端（CLI 转发）、channel | core、scheduler、modules（仅 parser） |
| api | `src/api/` | registry + 信封 + CLI 适配器 | 以上全部 |
| daemon | `src/daemon.rs` | 组合根：把 serial → 仲裁器 → tasks → 模块 → transports 接线起来 | 全部 |

依赖方向**向下**。模块绝不会向上触达 `daemon`、
`transport` 或 `api`；是守护进程把模块接进来，模块不会反过来调用它。

## 3. 模块契约

每个模块都是一个目录，含同样的五个文件外加 `mod.rs`：

```text
src/modules/<name>/
├── mod.rs        what the module owns, in one paragraph
├── commands.rs   AT command construction (consts + builders)
├── parser.rs     AT response -> typed state (unit-tested here)
├── state.rs      domain model + to_json()/from_json() (wire shape lives here)
├── service.rs    refresh policy: AT via RefreshCtx, publish via store()/stale(),
│                 spawn() registering periodic jobs
└── api.rs        routes() -> Vec<Route> + text renderers for the CLI
```

以下规则靠两道闸把关：人工评审，加上 `scripts/` 下的一组静态检查工具
（工具与检查项见 [migration.md](migration.md)）：

* `service.rs` 只能看到 `RefreshCtx`（AT + 缓存 + 总线）—— 绝不能看到
  `AtArbiter`、tty，或其他模块状态结构体的内部。
* 跨模块数据通过已缓存主题（`ctx.cache.get(TOPIC_X)`）或某模块的
  `cached*` 服务函数流动；跨模块的 AT 绝不会重复执行两次。
* 所有线上 JSON 都由 `state.rs::to_json` 产生；CLI 的 `key=value` 文本
  是同一个结构体的另一种渲染（`to_text`），因此解析修复不可能只落到
  其中一个面上。

本次重构后存在的模块：

| 模块 | 主题 | 路由 | 拥有 |
| ------ | ------ | ------ | ---- |
| `signal` | `signal` | `signal.get`, `signal.cached` | `^HCSQ?` math |
| `network` | `network`, `registration` | `network.get`, `network.cached`, `registration.get`, `network.pdp`, `network.dhcp`, `network.ims`, `network.rrc`, `network.registration_urc`, `network.session`, `network.flow_clear`, `network.direct_ip`, `network.direct_ip_set`, `network.lock_get`, `network.lock_apply`, `network.c5goption`, `network.c5goption_set`, `network.radio`, `network.radio_set`, `network.syscfg`, `network.syscfg_set`, `network.autodial`, `network.autodial_set`, `network.pdp_contexts`, `network.pdp_set`, `network.pdp_remove`, `network.pdp_state`, `network.postroute_set`, `network.dmz_set`, `network.usb_mode`, `network.usb_mode_set`, `network.interface_cfg`, `network.interface_mode_set`, `network.schedule_get`, `network.schedule_set` | `+COPS?`, `^SYSINFOEX`, `+C5GREG/+CEREG/+CREG`, `^LTEFREQLOCK?`/`^NRFREQLOCK?`（含分组 CSV 写与射频周期应用）, `^C5GOPTION`, `^DHCP?`/`^DHCPV6?`/`^IPV6CAP?`, `^NDISSTATQRY`/`^DSFLOWQRY`/`^CGMTU`/`^DCONNSTAT` 聚合（session）, `+CGDCONT`/`+CGACT` 写, `^SETAUTODIAL` 写, `^SETDIRECTIP`/`^IPFILTERSWITCH`, `TDCFG` 写（mode/PostRoute/dmz）, `^RRCSTAT?`, `+CIREG?` |
| `cell` | `cell` | `cell.get`, `cell.cached`, `cell.neighbors`, `cell.scan_start`, `cell.scan_result`, `cell.scan_state`, `cell.scan_abort` | `^HFREQINFO?`, `^MONSC` (per-RAT offsets), `^MONNC` (neighbours + ARFCN→band table), `^CELLSCAN` (scan command builder + line parser + exclusive task, published on `scan`) |
| `beam` | `beam` | `beam.ssb` | `^NRSSBID?` (SSB ids per serving/neighbour cell) |
| `ca` | `ca` | `ca.get`, `ca.cached` | `^HFREQINFO?` groups, `^CASCELLINFO?`, `^MONSSC` (carrier aggregation) |
| `qos` | `qos` | `qos.get`, `qos.cached` | `+CGACT?` active context, `^DSAMBR` AMBR/APN, `+CGEQOSRDP` QCI |
| `sim` | `sim` | `sim.get`, `sim.cached`, `sim.number`, `sim.slot`, `sim.slot_set`, `sim.hotplug_set`, `sim.activation`, `sim.activation_set`, `sim.pin_status`, `sim.pin_apply` | `+CPIN?`（含 CME 错误分支）, `^ICCID?`, `+CIMI`, `+CNUM`, `^SIMSQ?`, `^SCICHG`/`^TDSIMHP`/`^HVSST`/`^HVSST?`（卡槽切换与供电路径）, `+CLCK`/`+CPWD`（PIN 启停改） |
| `modem` | `modem`, `txpower`, `nr_txpower`, `endc` | `modem.get`, `modem.cached`, `modem.txpower`, `modem.endc`, `modem.nr_txpower`, `modem.mcs`, `modem.reset`, `modem.imei_set`, `modem.nr_capability`, `modem.nr_capability_set` | `ATI`, `+CGSN`, `^TXPOWER?`, `^NTXPOWER?`, `^LENDC?`, `^MCS`, `AT^RESET`, `^PHYNUM=IMEI`, `^NRRCCAPQRY`/`^NRRCCAPCFG` (CA / VoNR / DSS) |
| `traffic` | `traffic`, `netrate` | `traffic.get`, `traffic.cached`, `traffic.netrate`, `traffic.clear`, `traffic.pdcp_report_set` | `^PDCPDATAINFO?`（读 + 写开关）, `^DSFLOWCLR`, interface counters, accounting report |
| `system` | `temperature` | `system.temperature`, `system.temperature.cached`, `system.device_control`, `system.nic_rate_set`, `system.power_control_set`, `system.factory_reset`, `system.service_mode`, `system.version`, `system.led`, `system.led_set`, `system.network_time`, `system.thermal`, `system.thermal_set`, `system.thermal_thresholds_set`, `system.thermal_log_set`, `system.fota_mode`, `system.fota`, `system.fota_start`, `system.fota_abort` | `^CHIPTEMP?`, `^TDPCIELANCFG`, `^TDPMCFG`, `AT&F`, `^VERSION?`, `^NWTIME?`, `^LEDSWITCH?`, `^THERMAUTOFUN`/`^THERMLDLOGSW`/`^THERMLDAUTOPARA`/`^THERMLDAUTOSTATUS`, `^FOTAMODE?` + FOTA 流程集（`^FOTASTATE?`/`^FOTADLQ`/`^FOTAMODE`/`^FOTAOEMDL`/`^FOTADL`/`^FWUP`，见 `fota.rs`） |
| `sms` | `sms` | `sms.status`, `sms.storage`, `sms.list`, `sms.send`, `sms.delete`, `sms.clear_all`, `sms.storage_set`, `sms.center_set`, `sms.ims_set`, `sms.analyze`, `sms.ussd_send`, `sms.ussd_cancel` | `+CMGF`/`+CMGL`/`+CMGD`/`+CPMS`/`+CSCA`/`^IMSSWITCH`/`+CUSD` (plus the IMS profile sequence's `+CGDCONT`/`+CEUS`/`+CFUN`) — SMS-SUBMIT PDU encoder and SMS-DELIVER decoder, multipart split/merge, send transaction, USSD codec (`ussd.rs`: GSM 7-bit pack/unpack, `<m>` copy) and the `sms.ussd` push the URC path publishes |

## 4. 前端

| 前端 | 入口 | 数据访问 | 拥有 |
| -------- | ----- | ----------- | ---- |
| LuCI | `luci-app-mt5700m/htdocs/.../view/mt5700m/*.js` | ucode 插件（`mt5700m.at/cached/events`）→ 守护进程 RPC | 渲染、标签页、弹窗、表单状态 |
| WebUI | `semi-tcpweb/src/pages/*` | `services/at.ts` WS 传输层 + `services/stateCache.ts` 主题存储 | 渲染、标签页、弹窗、表单状态 |

两个前端互不引用对方；它们不共享任何文件、任何存储键和任何
状态。每一个都可以独立构建、部署、更新或损坏而不影响另一个。
它们唯一的共同依赖就是 [api-contract.md](api-contract.md) 中的 API 契约。

## 5. 并发模型

* 每个传输层监听者一个 OS 线程（WS、控制套接字、TCP RPC），一个
  tty 读线程，仲裁器内一个执行线程。
* 所有 AT 请求都是仲裁器优先级队列上的*消息*（`AtRequestSpec`），
  并带有预算（排队超时 + 命令超时 + 余量）。除了当前正在执行的
  那条命令，没有任何东西会在持有 tty 时阻塞。
* 周期性工作是一个 `TaskManager` 任务：可取消（任务标志）、可超时
  （每任务预算）、可重试（按命令类别的策略）、可观测（任务注册表 +
  `task.*` 事件），且不泄漏（任务完成后被回收）。
* 失败的任务只能让它自己的那一跳失败：`RefreshCtx` 把超时转成
  「无应答」并回退到缓存值，因此一次糟糕的调制解调器响应
  不可能搞崩守护进程、某个路由或某个页面。
