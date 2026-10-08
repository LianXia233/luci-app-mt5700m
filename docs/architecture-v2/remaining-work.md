# MT5700M 架构 v2 重构 — 剩余工作清单

> 基线：`main` = `6a4eabd`（PR #7 已合并）
> 当前分支：`arena/d1c7aed1-luci-app-mt5700m`，头 = `a7ab0a3d`
> 原始 20 条验收标准：**18 条已达成**，剩余 2 条见 §5

本文件每完成一刀都应就地更新，避免重复盘点。

---

## 〇、本刀已完成（2026-10-08，提交 `2643af75` + `a7ab0a3d`）

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

---

## 一、LuCI 前端剩余 CLI 调用点：20 处

| 页面 | 调用点 | 说明 |
|---|---|---|
| `advanced.js` | 7 | 1 读帧 + 6 写 |
| `connection.js` | 8 | 1 读帧 + 7 写 |
| `system.js` | 4 | **1 读帧 + 3 保留写**（写入已从 13 处降到 3 处，均为 FOTA 三步） |
| `terminal.js` | 1 | **有意保留**：原始 AT 控制台是产品功能本身 |
| `network.js` / `sms.js` / `status.js` / `settings.js` | 0 | 已清零 |

### 1.1 `advanced.js` — 7 处（1 读 + 6 写）

| 位置 | 现状 | 结论 |
|---|---|---|
| `atHardware()` 读帧 | 8 段（USB/接口/NIC/PCIe/LED/SIM 热插拔/卡槽/温控） | 除 **LED** 外全部已有路由，缺 `^LEDSWITCH?` 读 |
| USB 模式写 `AT^SETMODE=` | 只有读路由 | 需后端新增写路由 |
| PCIe 控制器写 `AT^TDPMCFG=` | `system.power_control_set` 已存在 | ✅ 可直接迁移 |
| PHY 档位写 `AT^TDPCIELANCFG=` | `system.nic_rate_set` 同命令 | ✅ 可直接迁移 |
| 接口模式写 `AT^TDCFG="infcfg","mode"` | 只有读 | 需后端新增写路由 |
| SIM 热插拔写 | `sim.hotplug_set` 已存在 | ✅ 可直接迁移 |
| 温控写 `AT^THERMAUTOFUN=` | `system.thermal_set` 已存在 | ✅ 可直接迁移 |

### 1.2 `connection.js` — 8 处（1 读 + 7 写）

| 位置 | 缺口 |
|---|---|
| 设置读帧 `advanced connection-settings` | 读侧基本已覆盖 |
| 新增 PDP `AT+CGDCONT=` | **无写路由** |
| PDP 激活/去激活 `AT+CGACT=` | **无写路由** |
| 删除 PDP `AT+CGDCONT=<cid>` | **无写路由** |
| 拨号设置写 `AT^SETAUTODIAL=` | 只有 QUERY |
| 直通写（TDCFG 系列） | 待确认命令 |
| PostRoute `TDCFG PostRoute` + `AT^IPFILTERSWITCH=0` | **无写路由**；`IPFILTERSWITCH` 仅存在于 `cli.rs` |
| DMZ `TDCFG dmz "<ip>"`（含 IPv4 校验） | **无写路由**；校验需从 `cli.rs` 搬进模块 |

### 1.3 `system.js` — 剩 4 处（1 读帧 + 3 保留写）

#### 读帧：22 段的路由对照表（2026-10-08 就地复核）

本刀前补的后端能力已覆盖缺列；下表把每段钉到具体字段，
三处标 ⚠ 的是**渲染数值可能漂移**的点，切之前必须逐项解决。

| 段（AT） | 路由 | 字段 | 备注 |
|---|---|---|---|
| Identity（`ATI`） | `modem.get` | `manufacturer / model / revision / imei` | 页面 model 是硬编码 `MT5700M`，revision 取 Identity |
| Version（`^VERSION?`） | `system.version` ✨ | `buildDate / software / hardware` | ✨ 本刀新增；此前 Rust 侧**无人读** `^VERSION?` |
| SIM（`+CPIN?`） | `sim.get` | `status` | |
| ICCID | `sim.get` | `iccid` | |
| IMSI（`AT+CIMI`） | `sim.get` | `imsi` | |
| Subscriber number（`+CNUM`） | `sim.number` | `number` | 未存储的分支（`+CME ERROR: 22`）已在 `number_state` 里 |
| Subscription rate（`^DSAMBR?`） | `qos.get` | `ambr_down_kbps / ambr_up_kbps` | ⚠ 页面对**原始字段**调 `parser.subscriptionRate`，而路由给的是已换算 kbps，需确认换算与文案一致 |
| Operator（`+COPS?`） | `network.get` | `operator` | ⚠ 页面取 `+COPS` 逗号切分后的**第 3 字段**（长名），需确认路由的 `operator` 就是它 |
| Network time（`^NWTIME?`） | `system.network_time` ✨ | `time` | 字符串逐字传递 |
| Function level（`+CFUN?`） | `network.radio` | `cfun / airplane` | 页面用 `cfun` 做 `'0'/'1'` 判断，路由给数字 |
| LED（`^LEDSWITCH?`） | `system.led` ✨ | `led` | |
| SIM activation（`^HVSST?`） | `sim.activation` ✨ | `active / slot` | `slot` 是页面回退用的第三字段 |
| SIM slot（`^SCICHG?`） | `sim.slot` | `slot / hotplug` | |
| Temperature（`^CHIPTEMP?`） | `system.temperature` | `sensors[] / average` | ⚠ 英雄区是**峰值**：`(max(raw)/10).toFixed(1)`。路由只有 `average`（均值），必须用 `sensors`（已是 °C）自算最大值；此外页面过滤 `-1000 < raw < 2000`，与后端的 `65535 / >1500` 判据不完全等价 |
| FOTA mode（`^FOTAMODE?`） | `system.fota_mode` ✨ | `mode` | ✨ 本刀新增；`0,1,0,1` → “HTTP update mode”的译名保留在页面（UI 文案） |
| FOTA state / progress | `system.fota` | `state / stateName / total / received` | 替代 `^FOTASTATE?` + `^FOTADLQ` 两段的自算 percent |
| Thermal status | `system.thermal` | `currentLevel` | 后端取第 6 字段，与页面一致 |
| Thermal thresholds | `system.thermal` | `thresholds[]` | 页面用索引 0–8 |
| Thermal log | `system.thermal` | `logSwitch.consoleLog / fileLog` | 页面按 `1/0` 判断“Enabled/Disabled” |

#### 保留的 3 条写

FOTA 下载 / 续传 / 安装 —— 模块是「任务 + 观察 + 中止」形态，
**迁移会改变交互，需产品决策**。
（温控阈值 / 温控日志 / LED / SIM 激活四条已在第二刀迁到路由，
详见 migration.md。）

### 1.4 死导出（7 个）

`parser.js`: `countLines`、`hexNumber`
`components.js`: `createSvg`、`signalPercent`
`api.js`: `netrate`、`atTimeoutMs`、`atSafe`（内部仍用，仅去导出）

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
② `transport/client.rs` 顶部注释仍写「control socket → direct serial → network 三级级联」与「daemon 不可达时直连串口安全」，但实现已无任何直连路径 —— **文档漂移**，应删上半段、保留「唯一所有者」说明；
③ CLI 的 `sms-list` / `sms-info` 已无调用者（其余 `status` / `advanced` / `flow-clear` 按既定决策作为诊断工具保留）。

---

## 三、WebUI 前端（`mt5700webui-openwrt-server/semi-tcpweb/src`）

源码**在库内**（此前误认为只有 bundle）。

### 3.1 原始 AT 写（1 处，属验收缺口）

`pages/network/Info.tsx` → `at().setPDCPDataReport()` → `AT^PDCPDATAINFO=1[,<ms>]` / `=0`。
后端 `traffic` 模块**已有**该 URC 的解析与分发，只缺写路由。
建议新增 `traffic.pdcp_report_set {enabled, interval}` 后删除该方法。

### 3.2 `services/at.ts` 死方法（8 个，外部零引用）

`getConnectionState`、`isAuthRequired`、`subscribeSMS`、`unsubscribeSMS`（三个适配器中重复出现）、`readCommand`、`getIMEI`（纯残留）。

### 3.3 其他

- AT 文本解析残留仅 2 处（应答前缀匹配、演示模式间隔解析），**均非业务解析**。
- `modem/parse.ts` 的频段/运营商/QCI 文案为**显示层映射**，与后端 `core/radio.rs` 的
  `ARFCN→band` 职责不同，**不算重复实现**（可选：未来由路由下发标签）。
- **任何 WebUI 改动都必须重建 bundle 并同步** `htdocs/5700/index.html` 的哈希引用。
  注意：该目录被 `.gitignore` 排除（CI 由 `scripts/build-release.sh` 折入软件包），
  但远端存在**历史遗留的误提交副本**，推送时**不得删除**（见 §6.2）。

---

## 四、测试与工具链（6 项）

| # | 问题 | 建议 |
|---|---|---|
| 1 | 6 个老证明默认基线是 `HEAD`，误用会自校验退出 2 | 像本刀这样把基线内嵌为 tag（已为 system 页示范：`pre-system-route`） |
| 2 | 8/9 个证明在非完整克隆下无法运行（基线对象不在本地） | CI 用 `--unshallow` 后跑全部 10 个 |
| 3 | 前端证明与 minify smoke **未进 CI** | 新增前端 job（见 §5 待办） |
| 4 | WebUI 无自动化测试 | 至少加「路由调用面」静态检查 |
| 5 | `minify-luci-frontend.sh` 要求目标已是 htdocs 树 | 自检提示已存在，CI 用法见 ci.yml |
| 6 | 沙箱每轮重克隆、`.git` 不持久 | **每个可验证阶段立即推送**；见 §6.2 的 API 推送工具 |

---

## 五、尚未达成的 2 条验收标准

1. **前端不得访问 AT** —— WebUI 的 PDCP 上报开关仍是原始 AT 写
   （LuCI/WebUI 的终端页为有意保留，不计入）；
2. **无重复实现** —— 残留 LuCI 7 个死导出 + WebUI 8 个死方法。

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
   本刀推送验证了这一点：本地 302 文件 vs 远端 306，「删除 5」是预期且被拦截的。
2. **LF 换行**：所有提交文件强 LF，写盘通过 Python 转换后校验。

### 6.3 其它

- **UI 不可变**：所有迁移必须用 `prove-*-parity.js` 的「同数据 → 新旧逐字比对」证明；
  拿不准的交互（如 FOTA 三步）保持原样。
- 两处**有意**语义差异已记录：`sim.slot_set`（完整厂商序列）、
  `system.factory_reset`（`AT&F`）。

---

## 七、建议下一步顺序（按依赖）

1. **后端补 4 组命令**（LED 读写、SIM 激活读写、网络时间读、温控表写）
   — 纯新增，风险低；完成后 system.js 的读帧与保留的 4 条写可一并收尾；
2. **系统页读帧切路由**（22 段 → 现有 + 新路由）— 需处理 FOTA 形态差异；
3. **连接页**（PDP 3 写 + 拨号写 + PostRoute/DMZ）— 校验规则须逐字保留；
4. **高级页**（4 条直接迁移 + 读帧 + USB/接口模式写）；
5. **WebUI**（PDCP 路由 + 删死方法 + 重建 bundle）；
6. **清理**（7 个死导出、CLI 无调用动词、`client.rs` 文档漂移、10 条路由归属）；
7. **工具链**（证明基线全部改为 tag + CI 前端 job + `--unshallow` 跑全量）。
