# MT5700M 架构 v2 重构 — 剩余工作清单

> 基线：`main` = `6a4eabd`（PR #7 已合并）
> 当前分支：`arena/d1c7aed1-luci-app-mt5700m`，头 = `a7ab0a3d`
> 原始 20 条验收标准：**18 条已达成**，剩余 2 条见第五节

本文件每完成一批都应就地更新，避免重复盘点。

---

## 〇、本批已完成（2026-10-08，提交 `2643af75` + `a7ab0a3d`）

**系统页 6 条写入从 CLI 迁到路由**，保留 7 条（原因见下）。

| 原 CLI 动词 | 目标路由 | 等价性 |
|---|---|---|
| `airplane <0/1>` | `network.radio_set {airplane}` | ✅ 同为 `AT+CFUN` |
| `advanced-set sim-slot <v>` | `sim.slot_set {slot}` | ⚠️ 有意差异：模块走完整厂商序列，CLI 只发 `AT^SCICHG` |
| `set-imei <v>` | `modem.imei_set {imei}` | ✅ 同为 `AT^PHYNUM=IMEI` |
| `restart` | `modem.reset` | ✅ 同为 `AT^RESET` |
| `sim-pin <op> a1 a2` | `sim.pin_apply {operation,pin,newPin}` | ✅ 五种操作一一对应 |
| `factory-reset` | `system.factory_reset` | `AT&F` vs `AT&F0`（`&F` 默认即 profile 0，无行为变化） |

`components.js`：抽出 `runConfirmedAction` 作为写入确认弹窗的**唯一**实现，
`runConfirmed`（CLI）与 `runConfirmedRoute`（路由）是它的两个薄调用者 ——
弹窗的 DOM / 文案 / 按钮顺序 / 恢复延迟不可能分叉。

新增 `scripts/prove-system-parity.js`（**34 项**，`tag pre-system-route`），
`scripts/smoke-minified-luci.js` 扩到 **28 项**（新增系统页 6 项）。

### 第二批 + 第三批（2026-10-08，追加）

**系统页余下 4 条写入 + 22 段读帧也全部迁完**，`system.js` 至此**零 CLI**
（仅保留 FOTA 三步动词，见 1.3）。

第二批（4 条写入，后端能力先行补齐）：

| 原 CLI 动词 | 目标路由 | 等价性 |
|---|---|---|
| `advanced-set led <0/1>` | `system.led_set {enabled}` | ✅ 同为 `AT^LEDSWITCH=` |
| `advanced-set sim-activation <0/1>` | `sim.activation_set {active}` | ✅ 同为 `AT^HVSST=1,<0\|1>` |
| `advanced-set thermal-thresholds <9 值>` | `system.thermal_thresholds_set {thresholds}` | ✅ 九值 0–150 + 阶梯校验在后端（`valid_thermal_thresholds` 自 cli.rs 上移，一份实现） |
| `advanced-set thermal-log <s> <f>` | `system.thermal_log_set {serial,file}` | ✅ 同为 `AT^THERMLDLOGSW=` |

第三批（读帧）：`api.atSystem()`（`mt5700m-at system`，22 段文本帧）删除，
改 15 条路由 —— `modem.get / system.version / sim.get / sim.number / sim.slot /
sim.activation / qos.get / network.get / network.radio / system.temperature /
system.thermal / system.led / system.network_time / system.fota_mode /
system.fota`；api.js 的 `atSystem` 速记一并删除。渲染逐字未变：

- `prove-system-parity.js` 扩到 **48 项**：A 组改为同一份 `SYSTEM_FACTS`
  双形态喂数（旧侧 22 段文本帧、新侧 15 条路由载荷），Technical details 块
  照 network 页先例改倒 `api.<route>` + JSON（有意差异，单独断言）；
- `smoke-minified-luci.js` 扩到 **33 项**：系统页断言改为零 CLI + 15 条路由
  （不喂 `at:system` 帧、不挂 `atSystem` 桩，残留调用会直接崩掉）；
- `cargo test` 286 passed。

三处 ⚠ 漂移风险已逐项取证排除（见 1.3 表）。

### 第四批：连接页（2026-10-08，后端 `f441e65` + 前端本批）

**连接设置 1 读帧 + 7 写全部迁完，`connection.js` 至此零 CLI**。

后端（`f441e65`，施工图 1.2 逐条执行）：`network.direct_ip` 读路由 +
七条写路由（`pdp_set` / `pdp_remove` / `pdp_state` / `autodial_set` /
`direct_ip_set` / `postroute_set` / `dmz_set`），命令串一律 commands.rs
构造器产出（`safe_at_field` / `valid_cid` / `valid_pdp_type` /
`valid_dmz_host` 自 cli.rs 上移，一份实现），cli.rs 薄转发（cid 的 `01`
保真拒绝照旧）；`cargo test` 286 → **290**。

前端（本批）：`load()` 的设置区块改 `Promise.all` 四条路由
（autodial / interface_cfg / pdp_contexts / direct_ip），renderPage 以
载荷映射替换帧解析；七处写入改 `routeCall` / `confirmRoute`；
`api.js` 删 `atConnectionSettings` 速记。验证：

- `prove-connection-parity.js` 重定位为「连接设置批」证明（基线 `7a02417`，
  **58 项**）：8 形态整页逐字比对 + 七条写路径旧 argv vs 新 params 逐键
  断言 + 盲区形态有意差异组（PostRoute 冒号前带空格 → 旧正则盲区修复，
  证据：姊妹项目 luci-app-mt5700 的 dial.js 与 parse_interface_cfg 单测）；
- `smoke-minified-luci.js` 33 → **38 项**（连接页零 CLI + 5 条读路由 +
  三条写入抽样）；
- `cargo test` **290/290**、`prove-system-parity.js` 回归全绿。

### 第五批：高级设置页（2026-10-08，后端本批 + 前端本批）

**hardware 1 读帧 + 6 写全部迁完，`advanced.js` 至此零 CLI**（本页无拨号
表单无 FOTA，一次到位）。后端仅新增 `network.usb_mode_set` /
`network.interface_mode_set` 两条写路由（构造器 `tdcfg_mode` + cli.rs 薄
转发），PCIe / PHY / SIM 热插拔 / 温控四条复用系统页与 SIM 模块已有路由；
前端读写双切 + `atHardware` 速记删除。验证：`prove-advanced-parity.js`
**37 项**（新脚本，基线 tag `pre-advanced-route`；Mode 冒号空格盲区与
PCIe 写命令短形态两个有意差异组单独点名）、`smoke` 38 → **45 项**、
`cargo test` 290 → **291**（+1 `tdcfg_mode` 形态测试）。

---

## 一、LuCI 前端剩余 CLI 调用点：4 处

| 页面 | 调用点 | 说明 |
|---|---|---|
| `advanced.js` | 0 | **已清零**（hardware 1 读帧 + 6 写全部迁完，见 1.1） |
| `connection.js` | 0 | **已清零**（连接设置 1 读帧 + 7 写全部迁完，见第四批） |
| `system.js` | 3 | **3 条保留写**（FOTA 三步；4 条写入 + 22 段读帧已全部迁完，系统页零 CLI） |
| `terminal.js` | 1 | **有意保留**：原始 AT 控制台是产品功能本身 |
| `network.js` / `sms.js` / `status.js` / `settings.js` | 0 | 已清零 |

### 1.1 `advanced.js` — 0 处（✅ 已清零，2026-10-08；原 1 读 + 6 写）

> 施工图（下方保留）已按条执行：`network.usb_mode_set` /
> `network.interface_mode_set` 两条写路由本批新增（构造器校验 + cli.rs
> 薄转发），PCIe/PHY/SIM 热插拔/温控四条直接复用已有路由；前端读写双切 +
> `atHardware` 速记删除（本批），migration.md 对应切片。

| 位置 | 现状 | 结论 |
|---|---|---|
| `atHardware()` 读帧 | 8 段（USB/接口/NIC/PCIe/LED/SIM 热插拔/卡槽/温控） | 除 **LED** 外全部已有路由，缺 `^LEDSWITCH?` 读 |
| USB 模式写 `AT^SETMODE=` | 只有读路由 | 需后端新增写路由 |
| PCIe 控制器写 `AT^TDPMCFG=` | `system.power_control_set` 已存在 | ✅ 可直接迁移 |
| PHY 档位写 `AT^TDPCIELANCFG=` | `system.nic_rate_set` 同命令 | ✅ 可直接迁移 |
| 接口模式写 `AT^TDCFG="infcfg","mode"` | 只有读 | 需后端新增写路由 |
| SIM 热插拔写 | `sim.hotplug_set` 已存在 | ✅ 可直接迁移 |
| 温控写 `AT^THERMAUTOFUN=` | `system.thermal_set` 已存在 | ✅ 可直接迁移 |

**✅ 已执行（2026-10-08）**：读侧 5 条路由（LED 段页面不读，不需要路由；
`system.device_control` 一条覆盖 NIC + PCIe 两段），写侧 6 条路由
（`usb_mode_set` / `interface_mode_set` 新增，其余四条复用）；
`prove-advanced-parity.js` 37 项全绿（含 Mode 冒号空格盲区与 PCIe 写命令
短形态两个有意差异组，见第五批）；`smoke` 45 项。

| 位置 | 现状 | 结论 |
|---|---|---|
| `atHardware()` 读帧 | 8 段（USB/接口/NIC/PCIe/LED/SIM 热插拔/卡槽/温控） | 除 **LED** 外全部已有路由，缺 `^LEDSWITCH?` 读 |
| USB 模式写 `AT^SETMODE=` | 只有读路由 | 需后端新增写路由 |
| PCIe 控制器写 `AT^TDPMCFG=` | `system.power_control_set` 已存在 | ✅ 可直接迁移 |
| PHY 档位写 `AT^TDPCIELANCFG=` | `system.nic_rate_set` 同命令 | ✅ 可直接迁移 |
| 接口模式写 `AT^TDCFG="infcfg","mode"` | 只有读 | 需后端新增写路由 |
| SIM 热插拔写 | `sim.hotplug_set` 已存在 | ✅ 可直接迁移 |
| 温控写 `AT^THERMAUTOFUN=` | `system.thermal_set` 已存在 | ✅ 可直接迁移 |

### 1.2 `connection.js` — 0 处（✅ 已清零，2026-10-08；原 1 读 + 7 写）

> 施工图（下方保留）已按条执行：七条写路由 + `network.direct_ip` 读路由
> 在后端补齐（构造器校验 + cli.rs 薄转发，`f441e65`），前端读写双切 +
> `atConnectionSettings` 速记删除（本批），详见第四批与
> migration.md 对应切片。

读侧现状：`network.autodial` / `network.pdp_contexts` / `network.session` /
`TDCFG_QUERY` / `CGDCONT_QUERY` / `CGACT_QUERY` 均已在 modules/network；
`network.flow_clear` 前端已接（连接页第 8 处已消）。写侧缺口与
`cli.rs` 实发 AT 串（逐条核实）：

| # | CLI 动词 → 目标路由 | AT 串（cli.rs 原样） | 校验规则（须搬进模块） |
|---|---|---|---|
| 1 | `pdp-set` → `network.pdp_set {cid,type,apn}` | `AT+CGDCONT=<cid>,"<type>","<apn>"` | cid 1–11；type ∈ IP/IPV6/IPV4V6；`safe_at_field(apn)` 且 ≤99 |
| 2 | `pdp-remove` → `network.pdp_remove {cid}` | `AT+CGDCONT=<cid>` | cid 1–11 |
| 3 | `pdp-state` → `network.pdp_state {state,cid}` | `AT+CGACT=<state>,<cid>` | state/cid 同上 |
| 4 | `autodial` → `network.autodial_set {...}` | `AT^SETAUTODIAL=0`；开启时 `AT^SETAUTODIAL=<1>,<mode>,"<proto>"[,"<apn>"[,"<user>"[,"<pass>",<auth>]]]`（**尾部空字段必须省略**，MT5700M 拒绝） | enable 0/1；mode 0–2；proto 同 PDP；auth 0–2；apn≤99 / user≤31 / pass≤31，均 `safe_at_field` |
| 5 | `direct-ip` → `network.direct_ip_set {enabled}` | `AT^SETDIRECTIP=<0\|1>`（不是 TDCFG 系列） | 0/1 |
| 6 | `postroute` → `network.postroute_set {mode}` | mode=2：`AT^TDCFG="infcfg","PostRoute",2`；mode=1：同串 `,1` 后追加 `AT^IPFILTERSWITCH=0`（两笔，首笔失败即止） | mode ∈ 1/2 |
| 7 | `dmz` → `network.dmz_set {host}` | `AT^TDCFG="infcfg","dmz","<v>"`（`0` 即关闭，同一串） | `0` 或 4 段 IPv4（每段数字 ≤255，禁止空段）；前端另有「选中主机暴露告警」文案，保留 |

注意：`IPFILTERSWITCH` / `SETDIRECTIP` 动词目前仅存在于 `cli.rs`，
commands.rs 需新增常量与构造器；`SETAUTODIAL_QUERY` 已有，写构造器需新增。
可复用 `qos`/`system` 批次的模式：命令串一律由 commands.rs 构造器产出，
cli.rs 改为薄转发（参照 `valid_thermal_thresholds` 上移先例）。

前端读帧 `api.atConnectionSettings()`（`advanced connection-settings`）
在 7 条写齐后最后一并切换（settings 区块照 system.js 模式改
`Promise.all([network.autodial, network.pdp_contexts, …])`）。
**✅ 已执行（2026-10-08）**：`connection.js` 读写双切完成，4 条读路由 +
7 条写路由；`prove-connection-parity.js` 58 项全绿（含盲区形态有意差异组，
见第四批）。

### 1.3 `system.js` — 剩 3 处（3 条保留写，读帧已切完）

#### 读帧：22 段的路由对照表（✅ 已切完，2026-10-08）

本批前补的后端能力已覆盖缺列；下表把每段钉到具体字段。
三处曾标 ⚠ 的渲染漂移风险已逐项取证排除：

1. **订阅速率**：`qos.ambr_*_kbps` 给的就是 kbps 原值，页面对它调
   `parser.subscriptionRate`（内部 `/1000`），与旧帧 `^DSAMBR` 字段同源同算；
2. **运营商**：路由 `network.get.operator` 就是 `+COPS` 引号内字段
   （旧页面逗号切分后的第 3 字段）；
3. **英雄区峰值温度**：路由 `system.temperature` 自带 `peak / peak_sensor /
   average`（后端过滤 0 / 65535 后取最大），与页面旧公式
   `(max(raw)/10).toFixed(1)` 等价，页面直接用 `peak`。

| 段（AT） | 路由 | 字段 | 备注 |
|---|---|---|---|
| Identity（`ATI`） | `modem.get` | `manufacturer / model / revision / imei` | 页面 model 是硬编码 `MT5700M`，revision 取 Identity |
| Version（`^VERSION?`） | `system.version` ✨ | `buildDate / software / hardware` | ✨ 本批新增；此前 Rust 侧**无人读** `^VERSION?` |
| SIM（`+CPIN?`） | `sim.get` | `status` | |
| ICCID | `sim.get` | `iccid` | |
| IMSI（`AT+CIMI`） | `sim.get` | `imsi` | |
| Subscriber number（`+CNUM`） | `sim.number` | `number` | 未存储的分支（`+CME ERROR: 22`）已在 `number_state` 里 |
| Subscription rate（`^DSAMBR?`） | `qos.get` | `ambr_down_kbps / ambr_up_kbps` | ✅ `ambr_*_kbps` 即 kbps 原值，页面 `subscriptionRate` 的 `/1000` 原样保留 |
| Operator（`+COPS?`） | `network.get` | `operator` | ✅ 路由 `operator` 即 `+COPS` 引号内字段（原第 3 字段） |
| Network time（`^NWTIME?`） | `system.network_time` ✨ | `time` | 字符串逐字传递 |
| Function level（`+CFUN?`） | `network.radio` | `cfun / airplane` | 页面用 `cfun` 做 `'0'/'1'` 判断，路由给数字 |
| LED（`^LEDSWITCH?`） | `system.led` ✨ | `led` | |
| SIM activation（`^HVSST?`） | `sim.activation` ✨ | `active / slot` | `slot` 是页面回退用的第三字段 |
| SIM slot（`^SCICHG?`） | `sim.slot` | `slot / hotplug` | |
| Temperature（`^CHIPTEMP?`） | `system.temperature` | `peak / peak_sensor / average` | ✅ 后端过滤 0 / 65535 后取最大得 `peak`，与旧公式 `(max(raw)/10).toFixed(1)` 等价 |
| FOTA mode（`^FOTAMODE?`） | `system.fota_mode` ✨ | `mode` | ✨ 本批新增；`0,1,0,1` → “HTTP update mode”的译名保留在页面（UI 文案） |
| FOTA state / progress | `system.fota` | `state / stateName / total / received` | 替代 `^FOTASTATE?` + `^FOTADLQ` 两段的自算 percent |
| Thermal status | `system.thermal` | `currentLevel` | 后端取第 6 字段，与页面一致 |
| Thermal thresholds | `system.thermal` | `thresholds[]` | 页面用索引 0–8 |
| Thermal log | `system.thermal` | `logSwitch.consoleLog / fileLog` | 页面按 `1/0` 判断“Enabled/Disabled” |

#### 保留的 3 条写

FOTA 下载 / 续传 / 安装 —— 模块是「任务 + 观察 + 中止」形态，
**迁移会改变交互，需产品决策**。
（温控阈值 / 温控日志 / LED / SIM 激活四条已在第二批迁到路由，
详见 migration.md。）

### 1.4 死导出（✅ 已清零，2026-10-08，第六批）

> 旧盘点基于 CommonJS 正则，**失配**：LuCI shared 模块用
> `'require baseclass'` + `return baseclass.extend({...})` 协议，不是
> `module.exports`。本批按协议重新解析导出面 + 全仓引用计数后执行：

| 模块 | 处置 | 明细 |
|---|---|---|
| `parser.js` | 29 → 24 导出 | 删 5 个全死项（定义 + 导出行）：`section`、`pick`、`parseContexts`、`countLines`、`hexNumber` |
| `api.js` | 15 → 12 导出 | `atTimeoutMs`、`netrate`（全死，连带 `callNetrate` rpc 声明）删定义；`atSafe` 内部仍用，仅去导出 |
| `components.js` | 45 → 44 导出 | `signalPercent` 内部仍用（信号条），仅去导出 |

旧清单过时项更正：`createSvg` 本就**没有导出行**（纯内部函数）；
`countLines`/`hexNumber`/`netrate` 属全死（函数 + 导出 + rpc 声明一并删）。

验证：`node --check` 三模块通过；`prove-advanced` 37 / `smoke` 45 /
`prove-connection` 58 全绿；四个老 prove 脚本 stash 前后输出逐字一致
（预存行为不受影响）。

盘点脚本要点（可复用）：取 `return baseclass.extend({` 之后的顶层
`\t(\w+)\s*:` 为导出面；引用方含 view 页面（按各页 require 头解析别名，
components 别名是 `c`）、shared 互相调用、测试脚本 stub。

---

## 二、Rust 后端：12 组能力缺口

| # | 能力 | AT 命令 | 现状 |
|---|---|---|---|
| 1 | LED 读/写 | `AT^LEDSWITCH?` / `=` | ✅ 已落地：`system.led` / `system.led_set` |
| 2 | SIM 激活读 | `AT^HVSST?` | ✅ 已落地：`sim.activation` |
| 3 | SIM 激活写（独立） | `AT^HVSST=1,<0/1>` | ✅ 已落地：`sim.activation_set`（`switch_slot` 的启停括号与 CLI 写统一由 `commands::hvsst_power` 构造） |
| 4 | 网络时间读 | `AT^NWTIME?` | ✅ 已落地：`system.network_time` |
| 5 | 温控阈值写 | `AT^THERMLDAUTOPARA=` | ✅ 已落地：`system.thermal_thresholds_set`（校验规则从 `cli.rs` 上移到 `system::commands`，CLI 改为复用，仅剩一份实现） |
| 6 | 温控日志写 | `AT^THERMLDLOGSW=` | ✅ 已落地：`system.thermal_log_set` |
| 7 | 拨号设置写 | `AT^SETAUTODIAL=…` | 只有 QUERY + 解析 |
| 8 | 接口模式写 | `AT^TDCFG="infcfg","mode",` | 只有读 |
| 9 | PostRoute / DMZ / 直通写 | `TDCFG` + `AT^IPFILTERSWITCH=0` | 仅 `cli.rs` |
| 10 | PDP 上下文写 | `AT+CGDCONT=` / `AT+CGACT=` | 只有读 + 解析 |
| 11 | USB 模式写 | `AT^SETMODE=` | 只有读 |
| 12 | 自定义 DNS / 路由 metric | （UCI 侧） | 不涉及 AT（`form.Map` 保留决策） |

**另有三处清理**：
① 两端都未引用的 10 条路由（`*.cached` 7 条 + `sim.get`、`traffic.get`、`traffic.netrate`）需明确归属或删除；
② ~~`transport/client.rs` 文档漂移~~ ✅ 已完成（第六批）：`at_cmd` doc 由
「control socket → direct serial → network 三级级联」更正为单通道 daemon
转发；删除 Network 段孤儿注释与 daemon-holding 孤儿 doc 块；`AtError` 的
shell contract 举例更正为实际映射（2/1/124）；顺带删除死代码
`NetworkFailed` 变体（零构造点，`core/error.rs` 的 From 转换同步收口）、
`libc_eagain`（零调用）、`network_hosts`（零调用）；`cargo test` 291 passed；
③ CLI 的 `sms-list` / `sms-info` 已无调用者（其余 `status` / `advanced` / `flow-clear` 按既定决策作为诊断工具保留）。

---

## 三、WebUI 前端（`mt5700webui-openwrt-server/semi-tcpweb/src`）

源码**在库内**（此前误认为只有 bundle）。

### 3.1 原始 AT 写（✅ 已清零，2026-10-08，第七批）

`pages/network/Info.tsx` 的 `at().setPDCPDataReport()` 原直发
`AT^PDCPDATAINFO=1[,<ms>]` / `=0`。后端新增
`traffic.pdcp_report_set {enabled, interval}` 写路由
（`traffic::commands::pdcp_report` 构造器：关=0 不带 interval；
开=1 + interval 白名单 200–65535 ms，对齐页面 InputNumber 界限；
URC 流本身继续由 `pdcp_data` 事件推送，与本开关解耦）。
WebUI `setPDCPDataReport` 改走 `apiCommand`（签名不变，5 个调用点零改动），
mock 演示模式的本地 URC 模拟触发点同步迁到 api 帧。
**至此「前端不得访问 AT」验收达成** —— pages 下唯一的原始 AT 通道只剩
终端页（产品功能本身，有意保留）。

### 3.2 `services/at.ts` 死方法（✅ 已删除，2026-10-08，第七批）

按方法定义处计 8 处：`getConnectionState`（适配器 ×2 + 门面 ×1）、
`isAuthRequired`（同 ×3）、`readCommand`（连带 `pendingReads` 合并机制）、
`getIMEI`（`AT+CGSN` 纯残留，IMEI 由 `modem.get` 路由提供）。
**更正**：旧清单把 `subscribeSMS`/`unsubscribeSMS` 也列为死方法 ——
两者是 `ATService.subscribe/unsubscribe` 的适配器层活转发，**保留**。
连带确认存活：`getConnectionSnapshot` / `isReady` /
`onConnectionStateChange` / `ATConnectionState` 类型（均有外部引用）。
bundle 已重建并同步 `at-webserver/files/www/5700/`
（`index-BbP5KO1B.js` → `index-BRUXWNeG.js`，legacy 跳转页重建），
`tsc --noEmit` + `vite build` 通过。

### 3.3 其他

- AT 文本解析残留仅 2 处（应答前缀匹配、演示模式间隔解析），**均非业务解析**。
- `modem/parse.ts` 的频段/运营商/QCI 文案为**显示层映射**，与后端 `core/radio.rs` 的
  `ARFCN→band` 职责不同，**不算重复实现**（可选：未来由路由下发标签）。
- **任何 WebUI 改动都必须重建 bundle 并同步** `htdocs/5700/index.html` 的哈希引用。
  注意：该目录被 `.gitignore` 排除（CI 由 `scripts/build-release.sh` 折入软件包），
  但远端存在**历史遗留的误提交副本**，推送时**不得删除**（见 6.2）。

---

## 四、测试与工具链（6 项）

| # | 问题 | 建议 |
|---|---|---|
| 1 | ~~6 个老证明默认基线是 `HEAD`，误用会自校验退出 2~~ | ✅ 部分完成：`pre-system-route` / `pre-advanced-route` 两个 tag 锚定为远端 orphan 提交（`tools/push-baseline-tag.py`，树 = 本地重建快照）；connection 默认基线 tag 化；CI 显式传参 |
| 2 | ~~8/9 个证明在非完整克隆下无法运行~~ | ✅ CI 已解决：`frontend-proofs` job `fetch-depth: 0` + `fetch-tags: true`；7 个远端原生基线提交（8a8c501/4474436/24ed5ef/fceb6ea/dfb4810/663f989/812ba41）API 实测可达 |
| 3 | ~~前端证明与 minify smoke 未进 CI~~ | ✅ 已完成（第八批）：ci.yml 新增 `frontend-proofs` job —— 10 个 prove（固定基线）+ smoke |
| 4 | WebUI 无自动化测试 | 至少加「路由调用面」静态检查 |
| 5 | `minify-luci-frontend.sh` 要求目标已是 htdocs 树 | 自检提示已存在，CI 用法见 ci.yml |
| 6 | 沙箱每轮重克隆、`.git` 不持久 | **每个可验证阶段立即推送**；见 6.2 的 API 推送工具 |

> 基线快照与工作区 shared 模块的兼容性已验证：`lib/luci-stub.js` 的
> `loadSide` 对旧侧**全套读基线**（parser/components/页面同源自洽），
> 基线快照自带第六批删除的死函数定义，不存在 ReferenceError 风险。

---

## 五、尚未达成的 2 条验收标准 → ✅ 全部达成

1. ~~**前端不得访问 AT**~~ ✅ **已达成**（第七批，2026-10-08）：WebUI 的
   PDCP 上报开关已切 `traffic.pdcp_report_set` 路由，pages 下唯一的原始
   AT 通道是终端页（LuCI/WebUI 的终端页为有意保留，不计入）；
2. ~~**无重复实现**~~ ✅ **已达成**（第六 + 第七批，2026-10-08）：
   LuCI 9 个死导出（见 1.4）+ WebUI 8 处死方法（见 3.2）全部清零。

---

## 六、环境约束（重要）

### 6.1 沙箱网络：git 协议被阻断

**现象**：`git clone` / `git ls-remote` 超时或
`fetch-pack: unexpected disconnect`；而 `api.github.com` 与
`codeload.github.com` 直连**完全可用**（DNS 均解析到 20.205.243.16x 网段）。
用户提供的 `gh.acg2.mom` 只做静态页托管，**不支持** git 转发。

**对策**：
- 取源码：`https://codeload.github.com/<owner>/<repo>/tar.gz/refs/heads/<branch>`（约 3 分钟）
- 推送：`tools/ghgit.py`，走 REST API 的 Git Database
  （blob → tree → commit → update-ref），保留逐 commit 粒度：

```bash
GITHUB_TOKEN=<token> python3 tools/ghgit.py plan                    # 只看变更
GITHUB_TOKEN=<token> python3 tools/ghgit.py pushall --since <已推送的本地 commit>
```

### 6.2 两个必须记得的坑

1. **删除保护**：远端存在 5 个被 `.gitignore` 排除的文件
   （`htdocs/5700/`、`htdocs/cgi-bin/`、`root/usr/bin/`、`root/etc/init.d/at-webserver`），
   系历史遗留误提交。`ghgit.py` 默认**不删除**，需显式 `--allow-delete`。
   本批推送验证了这一点：本地 302 文件 vs 远端 306，「删除 5」是预期且被拦截的。
2. **LF 换行**：所有提交文件强 LF，写盘通过 Python 转换后校验。

### 6.3 其它

- **UI 不可变**：所有迁移必须用 `prove-*-parity.js` 的「同数据 → 新旧逐字比对」证明；
  拿不准的交互（如 FOTA 三步）保持原样。
- 两处**有意**语义差异已记录：`sim.slot_set`（完整厂商序列）、
  `system.factory_reset`（`AT&F`）。

---

## 七、建议下一步顺序（按依赖）

1. ~~**后端补 4 组命令**（LED 读写、SIM 激活读写、网络时间读、温控表写）~~
   ✅ 已完成（`13e7b8f` / `61c4ad3` / `662c2c3` 三笔）；
2. ~~**系统页读帧切路由**（22 段 → 现有 + 新路由）~~
   ✅ 已完成（含 4 条写入 + FOTA 形态对齐，系统页零 CLI）；
3. **连接页**（PDP 3 写 + 拨号写 + PostRoute/DMZ）— 校验规则须逐字保留；
4. **高级页**（4 条直接迁移 + 读帧 + USB/接口模式写）；
5. ~~**WebUI**（PDCP 路由 + 删死方法 + 重建 bundle）~~ ✅ 已完成（第七批）；
6. ~~**清理**（死导出、CLI 无调用动词、`client.rs` 文档漂移、10 条路由归属）~~
   死导出与 `client.rs` 文档漂移已完成（第六批）；CLI 无调用动词与 10 条路由归属待做；
7. **工具链**（证明基线全部改为 tag + CI 前端 job + `--unshallow` 跑全量）。
