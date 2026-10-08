# 迁移日志与状态

## 1. 本次重构改了什么

**阶段 A —— 搬迁（提交 `375a36c`）。** 原先扁平的 20 文件 `src/` 目录树变成了分层目录树（`core/`、
`serial/`、`scheduler/`、`state/`、`transport/`、`api/`、`modules/`），并配有 `mod.rs` 文档，
遗留名称保留为局部别名，因此这次移动本身不改变任何行为。

**阶段 B —— 统一 API + 首个模块（提交 `448f0c8`）。**

* `api/registry.rs`、`api/rpc.rs`：一张路由表，三个薄封装（WebSocket、控制套接字、JSON-RPC）。
  `daemon.rs` 新增了 `api` 动词、`api` RPC 方法以及 `api.*` 分发，全部抵达注册表。
* `core/channel.rs`（`AtChannel`）、`scheduler/channel.rs`（`TaskChannel`、`DirectChannel`、
  `run_in_task`）、`transport/channel.rs`（`DaemonChannel`）、`state/refresh.rs`（`RefreshCtx`）：
  模块实际看到的接口。
* `modules/signal/*`：首个模块；HCSQ 计算只存在一份，五个解析器测试。
* 强制单一 AT 所有者：CLI 的"控制套接字 → 串口 → 网络"级联，以及它直连串口/裸 TCP 的短信回退
  路径被删除；客户端端口检测仅基于描述符；守护进程缺失是一个被上报的错误。
* 构建修复：`modules/sms/mod.rs` 在 `sms.rs` 移到 `modules/sms/pdu.rs` 之后缺失，导致
  `pub mod sms;` 无法解析。crate 在阶段 A 编译不过，现在可以了。

**阶段 C —— 每个主题都有自己的模块（提交 `dbb7eb0`）。**

* `state/collectors.rs`（1020 行）被删除。`cell`、`sim`、`modem`、`system` 与 `traffic` 模块
  现在各自拥有自己的采集器、解析器、退避键与采集节奏；`network` 拥有
  `+COPS`/`^SYSINFOEX`/`+CxxREG`。
* 去重：PDCP 字段表从 `transport/urc.rs` 移入 `modules/traffic/parser.rs`（URC 流与查询路径共用
  它）；CLI 中 COPS/SYSINFOEX 解码器与 HCSQ 计算的副本被删除（它改为别名到模块解析器并渲染
  `SignalState`）；cell 采集器不再第二次下发 `AT+COPS?` —— 它读取 network 模块的状态。
* `scripts/ci-annotate-cargo.py` + CI 步骤：cargo 诊断信息被重新输出为 GitHub 注解，因为本仓库的
  CI 日志来自一个 blob URL，某些环境无法抓取。
* `scripts/rs-static-check.py`：上文所述的免 cargo 结构检查器，现已成为仓库的一部分，并由 CI 在
  `cargo test` 之前运行（它还检查枚举变体，因此被重命名/删除的变体无法悄悄溜过）。
* `daemon.rs`：那个死掉的一次性短信写入器（`AtClient::send_sms` 与 `send_blocking`，它们绕过了
  仲裁器）被删除。发送短信现在只存在一处：传输层的 PDU 写入器 → `send_pdu` → 仲裁器上的 `+CMGS`，
  WebSocket 路由与 CLI 都走这条路（控制套接字那个阶段 C 仍在使用的独立 `sms` 动词，到阶段 D 已
  并入这一条路径）。

**阶段 D —— 短信模块（提交 `8383b7d` 及其后继提交）。**

* `modules/sms` 在 `pdu.rs` 之外新增了 `commands.rs`、`parser.rs`、`state.rs`、`service.rs` 与
  `api.rs`，现在拥有整条流程：`+CMGF`/`+CMGL`/`+CMGD`/`+CPMS`/`+CSCA`/`^IMSSWITCH` 的构造、应答
  解码、`SmsMessage`/`SmsStorage`/`SmsSettings` 领域模型，以及五步 IMS 时序。九条路由
  （`sms.status|storage|list|send|delete|clear_all|storage_set|center_set|ims_set|analyze`）与其
  它每个模块的路由注册在同一张表中。
* `pdu.rs` 成了一个双向编解码器：原有的 SMS-SUBMIT 编码器加上一个 SMS-DELIVER 解码器（GSM 7-bit
  字母表、UCS-2、字母数字发送方、半八位组 SCTS、UDH 串联）。多段的部分在解析器中被*合并*，因此
  列表就是消息的列表。DCS 判定现在检测 3..2 位（`0x08` 是 UCS-2，即 modem 实际发送的形式），而
  不再只看 0x04 位。
* `core::channel::SmsPart` 加上一个必需的 `AtChannel::send_sms_pdu` 取代了旧的号码/文本载荷：模块
  负责编码，通道/传输层执行 `AT+CMGS` 事务。`AtPayload::SMS { parts }`、
  `AtTransport::send_sms_pdu` 以及守护进程的 `send_pdu(length, hex, timeout)` 携带的是八位组而不
  是短信 PDU 类型，守护进程会指名失败的那一段（`第 i/n 条发送失败：…`）—— 即发送页面过去自行
  拼出的文案。
* 删除的重复实现：控制套接字的 `sms` 动词、WebUI 的 `modem/smsEncode.ts`（基于 node-pdu 的
  SMS-SUBMIT 构造器）、`modem/sms.ts` 的 `parseCMGL`/`processPDUMessage`/`mergeConcatenated`
  （node-pdu 解码 + 合并）、`@/types/node-pdu.d.ts` 以及 `node-pdu` 依赖本身、`services/at.ts`
  的 `listAllSMS`/`getSMSStorage`，以及 CLI 中裸 AT 的 IMS 时序 / `+CMGD` / `+CPMS` / `+CSCA`
  动词主体。
* 短信页面（`pages/sms/Center.tsx`、`pages/sms/Settings.tsx`）现在不含任何 AT 命令：列表、存储
  平面、中心号码、IMS 开关、清空以及撰写提示全部来自路由。撰写提示保留其原文案，但改为请求
  `sms.analyze`，因此承诺的分段数来自真正执行发送的那个编解码器。
* 已发送消息的缓存**故意**留在前端：UI 称之为本地缓存，并为它提供了导出/导入按钮，所以它是浏览器
  本地数据，不是 modem 状态。凡是从 modem 派生的，一律是后端状态。
* LuCI 自己的短信视图不再读取 CLI：它请求 `sms.list`/`sms.status`（与 WebUI 两个短信页面调用的
  路由相同），并通过 `sms.send`/`sms.delete`/`sms.clear_all`/`sms.center_set`/`sms.storage_set`/
  `sms.ims_set` 写入。`parser.js` 中短信那一半（`parseMessages`、`parseInfo`、`decodePdu`、
  `decodeGsm7`、`decodeUcs2`、`swapDigits`）随之被删除，因此 PDU/GSM-7/UCS-2 编解码器在代码树中
  只存在一份：`modules/sms::pdu`。只有 `groupMessages` 保留（对页面视图模型的纯展示分组）。

  两点值得记录的后果：

  * 解码后的发送方/接收方号码现在是后端的领域值（纯数字，无 `+` 前缀）。旧 JS 解码器在 PDU 的
    TOA 表明"国际号码"时会前置 `+`：数字完全相同，`+` 消失了，而 WebUI 从同一字段渲染的正是
    这个值。`prove-sms-parity.js` 把它钉死为*唯一*的值差异，并按每种形态统计 `+` 的个数，因此这个
    被允许的差异无法悄悄扩大。
  * 设置对话框的存储下拉框与卡槽徽标都读取 `+CPMS` 的**读**平面 —— 也就是旧 `parseInfo` 正则取到
    的那个（它的第一个匹配）。一致性证明使用的夹具三个平面各不相同，因此平面错一位不可能不被发现。

## 2. 已执行的验证

| 检查项 | 结果 |
| ----- | ------ |
| Rust 语法（lezer 解析器，全部 79 个文件） | 干净 |
| 静态检查（`scripts/rs-static-check.py`：338 条 `crate::` 路径、116 个模块、局部调用、trait 实现、结构体字面量） | 干净（已自测：它能报出缺失的 trait 方法和未知的结构体字段） |
| 随模块新增的单元测试 | 31 个测试，覆盖 `signal`、`network`、`cell`、`sim`、`modem`、`traffic`、`system` |
| `cargo test --locked`（CI） | 见 head 提交上的那次运行 |
| Shell/JS/JSON/PO 检查（CI `static-checks`） | 见 head 提交上的那次运行 |
| 本次重构触及的 UI 文件 | 无（有意为之） |
| LuCI 迁移一致性（`scripts/prove-neighbors-parity.js`、`prove-lock-parity.js`、`prove-network-parity.js`、`prove-cellscan-parity.js`、`prove-radio-parity.js`、`prove-sms-parity.js`） | 每个脚本都在同一个打过桩的 DOM 中，用同一份 modem 应答渲染基线视图与新视图并比对 DOM：结构与标签逐字节一致，只有列出的那些值修正不同。它们是迁移期的工具，不是 CI 检查 —— 每个脚本把各自切片前的提交作为参数（`663f989`、`dfb4810`、`24ed5ef`、`fceb6ea`、`4474436`、`812ba41`），当传入的修订版本已包含该切片时以退出码 2 结束。它们共用的 AT 侧夹具位于 `scripts/lib/at-fixtures.js`（`decodeSyscfg`/`decodeC5gOption`/`decodeNrCapability` 覆盖无线偏好类应答，方式与其它解码器覆盖各自模块相同），每个脚本都拿它们与对应的 Rust 单元测试对齐钉死；短信证明所用的消息向量来自 WebUI 的演示数据，以及 `modules/sms::pdu` 自身的七位组测试 |
| 压缩后代码树的冒烟测试（`scripts/smoke-minified-luci.js`，在 `scripts/minify-luci-frontend.sh` 之后） | 加载*压缩后*的无线页面与短信页面并驱动它们：预期的路由调用、无 CLI 调用、用户真正看到的读数/下拉框/侧边栏，以及一条写入路径。压缩后再做 grep 毫无意义（esbuild 会重命名和内联），因此对交付产物唯一诚实的检查就是把它渲染出来 |

## 3. 待办事项（按序）

1. **剩余模块**：`beam`（波束/扫描命令）、`diagnostics`（`cellscan`、端口扫描）。每个都遵循
   [module-guide.md](module-guide.md)；在它们迁移完成之前，CLI 动词是事实来源。`sms` 已落地：
   `modules/sms` 现在双向拥有 PDU 编解码器（SMS-SUBMIT 编码、SMS-DELIVER 解码、GSM 7-bit / UCS-2、
   多段拆分与合并），因此 `pages/sms/Center.tsx` 与 `pages/sms/Settings.tsx` 完全不含 AT 命令，
   `modem/smsEncode.ts` 和 `node-pdu` 依赖被删除，`modem/sms.ts` 只保留浏览器本地的已发送消息缓存
   与展示格式化函数，而 CLI 的 `sms-send`/`sms-delete`/`sms-clear`/`sms-set`/`sms-ims` 动词走的是
   同样的路由。LuCI 的短信视图也迁到了这些路由上（见下方切片说明），这使得 CLI 的 `sms-list`/
   `sms-info` 动词失去了调用方 —— 它们与 `advanced radio` 同属"给想看的人看的原始文本"这一类，
   被合并进 remaining-work 清单。（`ca`、`qos` 以及 Info 页面的数据业务字段已落地：载波聚合归入
   `modules/ca` —— CLI 的 `append_hfreqinfo_line`/`CaTotals` 已删除，冻结的 `carrier_*`/`ca_*`
   文本来自 `CaState::to_text()` —— `modules/qos` 拥有 `+CGACT?`/`^DSAMBR`/`+CGEQOSRDP` 的渲染，
   同时支撑 CLI 的两个动词（`qci_text()`、`ambr_text()`）并提供 `qos.get`，而 `modules/network` /
   `modules/modem` / `modules/traffic` 现在拥有 Info 页面剩余的原始读与写（`network.dhcp` 对应
   `AT^DHCP?`/`AT^DHCPV6?`/`AT^IPV6CAP?`，含 IPv4 十六进制解码；`modem.mcs` 对应 `AT^MCS=1`/`=0`；
   `network.registration_urc` 对应 `AT+CGREG=2` 的副作用；`traffic.clear` 对应 `AT^DSFLOWCLR`）。
   因此 `Info.tsx` 完全不含 AT 命令，而 `services/at.ts` 中那套死掉的 AT 动词（`sendSMS`、呼叫
   控制、`parsePDU`、注册查询）被删除，而不是留着腐烂。）
   频点锁定同理：`modules/network` 拥有 `^LTEFREQLOCK?`/`^NRFREQLOCK?`（行布局、十六进制 PCI）、
   带区间表的分组 CSV 写入、射频周期的应用时序，以及 `^C5GOPTION` 的读/写三元组，因此 CLI 的
   `lock`/`preview-lock` 动词、昼夜调度器与 Settings 页面都通过 `commands::lte_lock_command`/
   `nr_lock_command` + `service::apply_lock` 构造同一份写入，而 `modem/lock.ts` 不再拼装 AT 字符串。
   `modules/cell` 拥有邻区扫描（`cell.neighbors`、`AT^MONNC`），包括 WebUI 过去放在
   `modem/parse.ts` 里的 ARFCN→频段表。`modules/beam` 拥有 NR SSB 上报（`beam.ssb`、`AT^NRSSBID?`）
   及其固定偏移，因此 Settings 页面完全不含 AT 命令。随后 `modules/sim` 接管了系统页面整张
   SIM/PIN 卡片：`sim.slot`/`sim.hotplug_set` 负责卡槽与热插拔开关，`sim.pin_status` 负责 `+CPIN?`
   （含 CME 错误分支、`^SIMSQ?` 细化与 `+CLCK="SC",2`），`sim.pin_apply` 负责 PIN 动词，因此
   `modem/sim.ts` 只保留展示文本（码 → 标签）与 CME 消息翻译，而 `components/SimPinHandler.tsx`
   提交的是一个操作，而不是构造一条 AT 命令。接着 `modules/system` 接管了系统页面的开发板卡片
   （`system.device_control`、`system.nic_rate_set`、`system.power_control_set`、
   `system.factory_reset`，外加 `system.service_mode` —— 在 `core::modem` 中启动时一次性记录的
   串口/网络链路类型，因此页面不再用 `AT+CONNECT?` 探测），`modules/modem` 接管了 `modem.reset`
   （`AT^RESET`）与 `modem.imei_set`（`^PHYNUM=IMEI`，含规则），`modules/network` 接管了飞行模式
   开关（`network.radio`/`network.radio_set`）。系统页面的身份卡片与 NR 发射功率卡片现在读取
   `modem.get`/`modem.nr_txpower`。WebUI 中仅剩的裸 AT 界面就只有刻意保留的 `pages/at/Terminal.tsx`
   控制台。小区扫描面板接着迁移：`modules/cell/scan.rs` 拥有命令构造器（manual 5.35 的约束与频段
   位图）、`^CELLSCAN:` 行解析器，以及藏在 `cell.scan_start`/`cell.scan_state`/`cell.scan_abort`
   之后的独占任务，把解码出的小区推到 `cells[]` 上；`modem/cellscan.ts`（WebUI 自己的解析器与命令
   构造器）被删除，而仍然进入 WS 命令路径的裸 `AT^CELLSCAN*` 形式通过 `scan::pseudo_command` 汇入
   同一个任务，因此它是一个兼容垫片，而不是第二套实现。*遗留项，LuCI 侧：*它的扫描弹窗读取
   `mt5700m-at cellscan` 的文本，而该 CLI 动词是通过控制套接字的裸 `send` 抵达守护进程的（一个上限
   约 30 秒的透传，承载不了一次全频段扫描）—— 因此自扫描变成异步以来，弹窗就没有任何东西可渲染。
   它应该调用 `cell.scan_start` 并渲染 `cellscan` 推送，保留自己的弹窗标记；在那之前，CLI 动词为
   脚本逐字节保留。系统页面最后三张卡片紧随其后：`modules/modem` 拥有 NR 能力的读与写
   （`modem.nr_capability`/`_set`，`^NRRCCAPQRY`/`^NRRCCAPCFG`，面向 CA、VoNR 与 DSS，含应答类型
   匹配与区间规则），`modules/network` 拥有 `network.syscfg`/`network.syscfg_set`（七参数的
   `^SYSCFGEX` 写入，外加 CLI `set-radio-policy` 的校验），`modules/system` 拥有
   `system.thermal`/`system.thermal_set`（四份 `^THERMLD*` 上报与总开关）。因此
   `pages/system/Info.tsx` **完全不含 AT 命令** —— 本次会话开始时是 35 处裸调用点，现在是 0。）
2. **FOTA**：系统页面过去驱动整个升级流程 —— `ATE0`、`^FOTAMODE=0,1,0,1`、`^FOTAOEMDL="…"`（地址在
   页面里校验），然后是一个 1 秒轮询器，其状态机读取 `^FOTASTATE?` / `^FOTADLQ`，在 31 时恢复、
   在 40 时刷写 —— 因此关闭页面会让流程变成孤儿，重新加载既看不见也无法重新接入。现在
   `modules/system/fota.rs` 就是这条流程：一个具名任务，`system.fota_start` / `system.fota` /
   `system.fota_abort`，状态机与页面自己的步骤编号，以 `fota.progress` 发布。页面渲染快照，保留
   自己的标记、文案与流程；它不再下发哪怕一条 AT 命令（其版本卡片读取 `modem.get`，与它早已使用
   的 `modem` 主题同源）。
3. **CLI 适配层瘦身**：`api/cli.rs` 仍持有这些能力的逐动词文本格式化器；随着各模块落地，它们会
   迁入模块 `api.rs` 中的 `render_text`。
4. **拨号页面**：最后一个裸 AT 页面。它是只读的（每次写入都由 LuCI 负责），但自己构造了六个查询
   并在本地解码 —— 包括为两张卡片两次下发 `^TDCFG?`，以及重新实现"自动拨号关闭时使用 NDIS 会话"
   的回退逻辑。`modules/network` 现在拥有它们（`network.autodial`、`network.usb_mode`、
   `network.interface_cfg`、`network.pdp_contexts`），页面只渲染这些对象，因此除刻意保留的 `/at`
   终端外，WebUI 已无 AT 命令。
5. **USSD**：短信页面的 USSD 面板自己打包码流（`AT+CUSD=1,"<gsm7 hex>",15`）并解包网络的 `+CUSD:`
   URC，把 `<m>` 映射为中文文案 —— 如果 LuCI 侧哪天也需要，那将是同一份手册表格的第三份副本。
   `modules/sms/ussd.rs` 现在拥有打包、解码与文案；`sms.ussd_send`/`sms.ussd_cancel` 是路由，解码
   后的应答也作为 `sms.ussd` 事件发出，因此没有任何页面需要解析 `+CUSD` 行。
6. **Schedule（定时锁频）**：该面板通过夹带在伪 AT 命令（`AT+SCHED=`）里的一个 JSON blob 读写配置，
   而守护进程端口以扁平的 UCI 映射应答 —— 每个值都是字符串，没有 `night`/`day` 嵌套 —— 因此任何
   前端都无法渲染它，而一次保存又会写入并不存在的顶层 UCI 键。`modules/network/schedule.rs` 拥有该
   DTO 及其 UCI 映射，`network.schedule_get`/`schedule_set` 是路由，而 `AT+SCHED?`/`AT+SCHED=` 作为
   面向 CLI 与 LuCI 的别名保留下来。
7. **LuCI 扫描弹窗**：弹窗显示来自一次阻塞式 `mt5700m-at cellscan` 的扫描结果；一旦扫描变成耗时
   数分钟的独占任务，控制套接字的超时就让那次调用只应答一个即时确认，于是频点扫描卡片返回为空。
   现在该 CLI 动词打印两个快速区段，在没有扫描运行时启动扫描，并从 `cell.scan_result` 打印
   `^CELLSCAN` 行（扫描任务缓存自己的应答文本，TTL 30 分钟）；`cellscan-result` 是弹窗使用的轮询
   接口，弹窗的标记、文案与流程都不变 —— 它只是在扫描结束时自我刷新。
8. **载波聚合卡片的数据来源**：WebUI 的 `pages/network/Info.tsx` 从 `cell.carrierInfo`、
   `secondaryNR`/`secondaryLTE` 渲染载波列表与每载波信号 —— 当 AT 时代的数据源被移除后，没有任何
   东西给它们赋值，因此即使页面保留了合并辅助函数，卡片也渲染为空。修复办法是把缺失的解码移进
   已经拥有这些命令的模块：`modules/ca` **一次**解析 `^CASCELLINFO?`（载波列表与辅载波列表现在
   共用一张字段映射表）并按小区解析 `^MONSSC`（十六进制 PCI、手册中的无效值、±8 去标度启发式、
   `<MEASTYPE>`），而 `ca.get` 在 `carriers` 之外还回答一个 `secondary` 数组。页面获取 `ca.get`
   （缓存优先，按需 `refresh`），订阅 `ca` 主题，并用 `modem/ca.ts` 映射领域模型（仅展示；
   `modem/carrier.ts` 及其两个解析器已删除）。
9. **前端 AT 移除**：
   * WebUI —— 展示走守护进程的主题缓存（`services/stateCache.ts` + `useSharedStateTopic`），每个
     动作都走路由；仅剩的 AT 构造器是 `pages/at/Terminal.tsx`，即刻意保留的裸 AT 控制台（面向用户
     的诊断工具，仅管理员可用）。`modem/*.ts` 只保留类型、展示表与合并辅助函数。
   * LuCI —— `api.js` 新增了 `route(name, params)`（`mt5700.at` 之上的 `api.<route>` 命令路径，在
     `api-contract.md` 中有文档），无线页面整个诊断区块迁了过来：
     * SSB 面板与"NR neighbour cells"那一行读取 `beam.ssb`（`parser.js` 的 `parseNrsSbid` ——
       `^NRSSBID` 偏移量的第二份 JS 副本 —— 被删除）；
     * 两行调制方式读取 `modem.mcs`。该路由把所有 `^MCS` 行摊平成一个载波列表，因此模块的
       `McsCarrier` 携带应答自带的 `group` 与 `rat`，页面据此重建出与之前完全相同的区块
       （"NR Carrier 1"、"LTE Carrier 1"……）—— `parser.js` 的 `parseMcsSection` 被删除，调制表
       （`mcsModulation`）保留，因为它是展示；
     * NR PUSCH/PUCCH 功率与发射频率读取 `modem.nr_txpower`，QoS 等级读取 `qos.get`，辅载波计数
       读取 `ca.get`（`lte_secondary_count` / `secondary_connection_count`），数据注册读取
       `registration.get`，EN-DC 读取 `modem.endc`，IMS 注册读取新的 `network.ims` 路由（`+CIREG`，
       此前没有模块解码过）。
     * 一致性由一个 node 证明脚本验证：它在同一个打桩 DOM 中运行*旧*视图与*新*视图，并比较每一个
       渲染出的行（模拟 modem 上 12 行，外加九个边界场景：单载波与多载波 MCS 应答、空应答、缺失
       `+CGEQOSRDP`、999/0 发射功率哨兵值、缺失 `^CASCELLINFO`、`^MONSSC: NONE`、缺失 `+C5GREG`）：
       每种场景都逐字节一致，MCS 标签亦然。有两处差异，都在失败路径上，且都是**有意的**：当
       `+CIREG` 或 `^LENDC` 不应答时，旧代码会从一个由文本匹配器凭空造出的占位行打印
       "Not registered" / "Disabled"，而现在页面显示该行自己的 `--`（与其它每个空行所用的占位符
       相同）。
     * 邻区卡片紧随其后，覆盖它被绘制的全部三处：诊断区、SSB 面板与扫描弹窗的两个网格现在都渲染
       `cell.neighbors`（`parser.js` 的 `parseMonnc` —— `^MONNC` 布局的 JS 副本 —— 被删除）。
       ARFCN→频段表也离开了前端：它现在是 `core::radio::arfcn_to_band`（由 `modules::cell` 与
       `modules::beam` 共用，两者现在都会发布 `band`），而页面只负责把频段号映射为标签（`B3`/
       `n78`）。`mt5700m-at cellscan` 不再打印邻区区段（弹窗曾是它唯一的消费者，而打印它意味着要
       问两次 `AT^MONNC`）；`radio-diagnostics` 仍为人类保留它。一致性：
       `scripts/prove-neighbors-parity.js` 在同一个打桩 DOM 中用同一份 modem 应答渲染 HEAD 视图与
       新视图 —— 结构、标签与行数逐字节一致，而四处不同的值槽位恰好就是列出的那些修正（十六进制
       PCI `1DC`/`40` → 十进制 `476`/`64`，以及那些 n78 小区 —— 旧 JS 表只覆盖到 3 GHz，因此把它们
       打印成了光秃秃的 `NR`）。提交后可复跑：`node scripts/prove-neighbors-parity.js 663f989`
       （切片前的提交是基线；不传它脚本会说明并以退出码 2 结束）。
     * 频点锁定紧随其后，双向都是：两行锁定状态与锁定面板的预填读取 `network.lock_get`，而每次
       写入（面板的 Review-and-apply 与邻区卡片的 Lock 按钮）都通过 `network.lock_apply` 带上带类型
       的 `items` —— CLI 那种位置参数 `['lock', rat, type, bands, arfcns, scs, pcis]` 从前端消失了，
       随之消失的还有证明脚本钉死的两个缺陷：NR ARFCN 卡片传了一个空的 SCS 槽位（CLI 的构造器会
       拒绝它），而 LTE 卡片把 PCI 传到了被忽略的第 7 个槽位，于是 `pcis` 为空。面板的预填在真机上
       也是空的，因为 `collectFreqLock` 要求每一行应答都重复 `^…FREQLOCK:` 前缀，而固件只打印一次
       —— 路由改为交出解码后的条目（PCI 转回十进制，这正是写入所需要的）。`verify: true` 保留了
       CLI 流程的结果上报（模块轮询查询，页面显示消息）。`parser.js` 的 `collectFreqLock` 与
       `parseLockData` —— 锁定布局的第二、第三份 JS 副本 —— 被删除，`parser.arfcnToBand` 的最后一
       个调用方现在只剩扫描弹窗的服务小区。`scripts/prove-lock-parity.js dfb4810` 证明了这一点：
       面板结构与文案逐字节一致，预填等于领域值，条目与表单一一对应，四条结果路径（成功、modem
       拒绝、校验失败、传输失败）产生相同的通知，且成功时等待 2.5 秒后才重载。
       * CLI 的 `network` 动词不再打印 `LTE lock` / `NR lock` 区段（面板曾是它们唯一的消费者，而
         打印它们等于又向 modem 问了同样两个问题）；`mt5700m-at lock` 仍然写入，`status` 仍然上报
         `lte_lock=`/`nr_lock=`，`radio-diagnostics` 保留原始行。ucode 的超时预算为已迁移的写入路由
         长出了一张具名清单（`api.network.lock_apply` → 25 秒，与 `lock` 动词拿到的预算相同），而
         不再是读取类默认的 12 秒。
     * 无线页面的状态区块紧随其后（该页面最后一块仍在切分 `mt5700m-at network` 文本帧的部分）：
       信号仪表读取 `signal.get`，服务小区行读取 `cell.get`，注册读取 `registration.get`，运营商
       读取 `network.get`，RRC 行读取新的 `network.rrc` 路由（`^RRCSTAT?`，此前没有模块解码过），
       温度仪表读取 `system.temperature` 的 `peak`。各仪表按 RAT 划分的指标集合不变（NR：
       RSRP/RSRQ/SINR，LTE：RSRP/RSRQ/RSSI，WCDMA：RSCP/RXLEV/ECIO），但它们的数值现在来自
       `modules::signal` 里唯一的 `^HCSQ` 解码器，而不是 `^MONSC` 的第 7..10 字段 —— 两者是同一个
       测量量（MONSC 上报 dBm，HCSQ 上报可映射回 dBm 的索引），因此用户看到的读数就是 WebUI 与
       `status` 页面早已展示的那个。`parser.js` 的 `parseServingCell`（`^MONSC` 布局的第二份 JS
       副本）与 `api.js` 的 `atNetwork`（那个没有别人调用的 CLI 文本帧调用）被删除；页面的 `mt-row`
       集合其余部分逐字节一致，而"Technical details"折叠区 —— 过去会把整个 AT 文本帧 dump 出来 ——
       现在 dump 它消费过的路由载荷，因此没有任何 AT 应答文本再进入前端。一致性：
       `scripts/prove-network-parity.js` 在同一个打桩 DOM 中用一份一致的 modem 上报渲染旧视图与新
       视图并逐行比较：12 行，唯一的值差异是有意的 `PCI`/`Cell ID`/`TAC / LAC` 十六进制→十进制
       归一化（与邻区卡片得到的同一修正），每个仪表读数、标签、文案与卡片标题逐字节一致。提交后
       可复跑：`node scripts/prove-network-parity.js 24ed5ef`（切片前的提交是基线；不传它脚本会
       说明并以退出码 2 结束）。温度主题现在在十二个传感器与 `average` 之外还携带 `peak`/
       `peak_sensor`：仪表盘的仪表一直渲染的是 `average`（`status.js`），WebUI 的卡片渲染的是各个
       传感器，因此三个界面仍然读取同一读数的三个不同字段 —— 把它们合并成一个是 UI 决策而非解码
       决策，留给负责人决定（解码本身现在只有一份：一个 `TemperatureState`、一个 `peak()`）。
     * 扫描弹窗紧随其后。它的服务小区卡片读取 `^MONSC` 文本并用 `parser.arfcnToBand` 猜频段，测量
       条取自 `^MONSC` 的第 7..10 字段，扫描状态来自 `mt5700m-at cellscan-result` 的文本 JSON，而
       启动扫描是 `cellscan` 动词的一个副作用。现在：`cell.get`（频段来自
       `core::radio::arfcn_to_band`，PCI/CID 为十进制）+ `signal.get` 出测量条，`cell.scan_result`
       出状态，尚未扫描过时才用 `cell.scan_start` —— 后三者与 CLI 动词的行为一致（呈现一次已完成
       的扫描，而不是重启一次新的）。`parser.parseMonsc`（`^MONSC` 布局的第三份 JS 副本 ——
       `cell.get` 与 `cell.neighbors` 也都不再需要它）与 `parser.arfcnToBand`（那份只覆盖到 3 GHz
       的频段表：对于一个 n78 ARFCN，它做的是 0–3 GHz 的线性换算，落到所有区间之外，于是打印
       `NR · NR`）被删除，`api.atCellscan`/`api.atCellscanResult` 同样删除；无线页面只剩一个 CLI
       调用（`atRadio`，它的无线偏好区块），下一个切片也把它移除了 —— 见下。
       一致性：`scripts/prove-cellscan-parity.js fceb6ea` 在两个修订版上驱动真实的
       «Cell Scan → Continue» 路径 —— 弹窗标题、两张邻区卡片、标签与文案逐字节一致，而四行不同之处
       恰好是有意的值修正（`NR · NR` → `NR · n78` 频段、十进制 PCI/CID，以及那条 SINR 进度条 ——
       `^MONSC` 的 NR 布局并不携带它，所以旧卡片画的是 `--`）；它还钉死了无 `band` 时的回退，那正是
       旧表产生的同一个 `NR · NR`。弹窗的**"Frequency scan"卡片是被删除，而不是被恢复**：它从未被
       渲染过（下述 `parser.section` 前缀不匹配），因此移除文本路径删掉的是死代码而非 UI。把扫描
       结果弄回来是一个 UI 决策 —— `cell.scan_result` 已经携带了解码后的 `cells` 与原始 `^CELLSCAN`
       文本，只等有人决定这张卡片该显示什么。
     * 无线页面的无线偏好区块为这一页收了尾：接入技术行读取 `network.syscfg`（`^SYSCFGEX?`）、
       `network.c5goption`（`^C5GOPTION?`）与 `modem.nr_capability`（`^NRRCCAPQRY` 3/2/5），而它的
       五个按钮写入 `network.syscfg_set`、`network.c5goption_set` 与 `modem.nr_capability_set` ——
       与 `pages/network/Settings.tsx` 和 `pages/system/Info.tsx` 已在调用的三个读路由、三个写路由
       相同，因此两个前端编辑的是同一份契约。五次
       `advanced-set radio-policy|5g-access|carrier-aggregation|vonr|dss` 调用消失了，这使
       `mt5700m-at advanced radio` 变成死代码；`api.js` 的 `atRadio`（页面最后一个 CLI 调用，随之
       也是页面最后一个文本帧）与 `parser.js` 的 `matchValues`（其唯一调用方就是这个区块）被删除。
       预设→三元组的映射留在页面上（`1,0,1` = Option 2，`0,1,0` = Option 3，否则 `1,1,1`），而
       `ca` 以路由所校验的布尔值发送，而不是 CLI 的 `'1'`/`'0'`。缺失字段保留旧回退值
       （`080302`/`3FFFFFFF`/`1`/`2`/`7FFFFFFFFFFFFFFF`），因为 `SysCfgState` 会省略未读到的字段，
       一个不完整的 `^C5GOPTION` 三元组仍会落到 Option 2 + 3，而一次半读的 `^NRRCCAPQRY` 集合仍会
       让 VoNR/DSS 保持在先前的默认值上。两条警告横幅随填充它们的文本帧一起消失：页面顶部横幅与
       5G 卡片的横幅都渲染的是 `radio` 文本帧的 `stderr`（`status.stderr`/`radioSettings.stderr`），
       而读取路由没有 stderr 可显示 —— `api.route()` 是本页其它十次读取已在使用的、"永不拒绝，缺失
       数据渲染为空白"的刻意契约，因此一次失败的读取现在显示一个空控件或默认值，而不是一个原始
       CLI 错误框。ucode 的已迁移写入预算清单增加这三条路由（`c5goption_set` 会像旧的
       `advanced-set 5g-access` 那样循环一次飞行模式，因此需要 25 秒预算；另两条是多命令配置写入）。
       一致性：`scripts/prove-radio-parity.js 4474436` 用同一份应答渲染两个修订版 —— 卡片在八种
       形态下逐字节一致（所有字段缺失、空的 `^SYSCFGEX` 字段、roam/service 为 0、三种接入模式
       三元组、不完整三元组、CA 关闭且 VoNR FR2），五条写入路径全部通过真实弹窗驱动：旧 CLI 的
       argv 与新的带类型参数携带相同的值、相同的通知文案、相同的 900 毫秒重载。无线页面现在完全
       不发起 CLI 调用 —— 这是在调用面上断言的，而不是靠 grep 源码。
     * 短信页面紧随其后，随之消失的还有前端最后一个 PDU 解码器。页面现在读取 `sms.list`（PDU 解码、
       GSM-7/UCS-2、UDH 串联与分段合并全部发生在 `modules/sms` 中）与 `sms.status`（`+CPMS`/
       `+CSCA`/`^IMSSWITCH`），并写入 `sms.send`/`sms.delete`/`sms.clear_all`/`sms.center_set`/
       `sms.storage_set`/`sms.ims_set`。两次读取都走 `api.routeCall` 而不是 `api.route`，这是刻意的：
       消息列表*就是*这个页面，因此一次失败的读取必须被上报 —— 对一个从未应答的 modem 渲染
       "No messages yet."等于宣称收件箱是空的。这保留了页面的两条 `alert-message warning` 横幅
       （文案不变；文本是后端的消息，正如同一条横幅过去显示 CLI 文本帧的 stderr 时那样）。随文本
       帧一起删除的：`api.js` 的 `atSmsList`/`atSmsInfo` 与 `parser.js` 的 `parseMessages`/
       `parseInfo`/`decodePdu`/`decodeGsm7`/`decodeUcs2`/`swapDigits`；保留下来的 `groupMessages`
       是纯展示分组。ucode 的已迁移写入预算清单变成 `[route, seconds]` 对，因此每次写入都保留其
       CLI 动词曾有过的预算（`sms.send`/`clear_all`/`ims_set` 60 秒，其余 25 秒）。一处既存的死
       路径被保留并标记，而不是删除：`renderPage` 的 `deleteMessage` 对话框没有任何按钮接上它，
       因此页面无法触达它 —— 它那一行随其它代码一起迁到了 `sms.delete`，而是否应该存在一个"逐条
       删除"按钮是一个 UI 决策。
       一致性：`scripts/prove-sms-parity.js 812ba41` 用一组 modem 应答渲染两个修订版 —— 七种形态
       （五条收到的消息分布在三个不同的 `+CPMS` 平面上、WebUI 演示数据那种同平面存储、IMS 关闭、
       一次失败的状态读取、空收件箱，以及单会话的 UCS-2 与 GSM-7 收件箱）、设置对话框（预填 +
       三次写入 + 1500 毫秒重载）、带本地已发送历史写入的发送流程、清空流程，以及两条失败路径。
       消息夹具是 WebUI 的演示向量（`mockAT.ts` 的 RECEIVED_SMS），而 GSM-7 那条是由
       `modules/sms::pdu` 自己的 `pack_septets(["hello"])` 测试向量构造的，因此该证明比较的是后端
       解码与旧 JS 解码，而不是与从任一侧拷贝来的夹具比较。
     * 仍在文本帧上、下一步要迁移的：`parser.section`/`pick` 供设置页与拨号页的行使用，以及
       `status`/`system` 页面，它们仍调用 CLI 动词（`status` 还喂给仪表盘的
       `parser.signalQuality`/`operatorInfo` 行）。当其中最后一个迁移完，`parser.js`（`section`、
       `pick`）即被删除。仪表盘已经通过 `api.cachedSnapshot()` 读取守护进程缓存。
     * 在证明上述内容时发现的一个缺陷，**未**修改（它是 UI 变更，因此需要决策）：扫描弹窗的
       "Frequency scan"卡片从未渲染过，无论在 HEAD 还是现在。`parser.section()` 匹配前缀
       `'===== <label>:'`，而弹窗传的是 `'Frequency scan: AT^CELLSCAN'`，而 `cli.rs` 打印的是
       `===== Frequency scan: AT^CELLSCAN =====` —— 不匹配，因此这张卡片（及其"+CME ERROR: 3"注释）
       是死的。另外两个包含 AT 命令的标签之所以能存活，只是因为它们的调用方传了 `|| raw`。弹窗迁移
       把这张卡片连同文本路径一起删除，而不是恢复它 —— 它从来就没显示过，因此渲染出的 UI 没有
       变化，而这张卡片该显示什么是一个 UI 决策（解码后的 `cells` 或原始 `^CELLSCAN` 文本）。同一次
       改动还移除了弹窗对 `mt5700m-at cellscan` 的调用本身，控制套接字的超时承载不了一次全频段扫描。
10. **把 `mt5700m-traffic` 并入 `modules/traffic`**（最后一个非 Rust 业务进程），并把
    `mt5700m-manager` 的拨号胶水代码并入 `modules/network` 的动作。
11. **拆分两个上帝文件**（`daemon.rs` 约 1.9k 行，`api/cli.rs` 约 2.4k 行）：`daemon.rs` →
    `transport/{ws_server,rpc_server,control_server}` + `api/rpc.rs`；`api/cli.rs` → 每模块一个
    `render_text` + 一张小的动词表。两者现在都是纯粹的接线/适配层，因此拆分是机械性的。
12. **把 CLI 的射频写入动词并回路由。** 随着 LuCI 的无线偏好区块迁移完成，
    `advanced-set radio-policy|5g-access|carrier-aggregation|vonr|dss` 已无前端调用方，然而每一个
    仍在自行构造 AT 字符串（`AT^SYSCFGEX=`、`AT+CFUN=0` + `AT^C5GOPTION=`、`AT^NRRCCAPCFG=`），而
    `modules::network`/`modules::modem` 又为 `network.syscfg_set`、`network.c5goption_set` 与
    `modem.nr_capability_set` 构造了同样的命令 —— 同一份写入在后端内部实现了两遍。短信动词已经
    展示了目标形态（`run_api(settings, "api.sms.send", …)`：CLI 与两个前端一样是同一个 API 的客户
    端，端口上没有第二个写入者）。合并这五个动词意味着把 argv 翻译成路由参数（保留每个动词自己的
    校验与 `EXIT_USAGE` argc 检查），并接受短信动词已经做出的两处 CLI 可见变化：由路由支撑的动词
    成功时不打印任何内容，而模块/守护进程失败时以退出码 1 结束，而不是 AT 错误码。纯 Rust 改动 →
    CI 是唯一的编译器，因此它需要自己的切片，并配上 argv → params 映射的单元测试。`sms-list`/
    `sms-info` 处于同样状态（短信切片后已无前端调用方），可以直接删掉，或重做为来自 `modules/sms`
    的一个 `render_text` —— 无论哪种，它们都属于这一批。读取侧保留：`advanced radio` /
    `radio-diagnostics` 是 CLI 的裸 AT 诊断视图（与保留 `radio-diagnostics` 的理由相同）。

### 剩余 LuCI 页面清单（侦察，2026-10-07）

已对照路由表（`modules/*/api.rs`）与状态结构体核对，因此这是后续切片的起点，而非猜测：

| 页面 | 仍在切分的读取 | 仍在使用的写入动词 | 已被现有路由覆盖 |
| ---- | -------------------- | ------------------------- | -------------------------- |
| `status.js` | `status`、`advanced session` | — | `signal.get`、`cell.get`、`registration.get`、`network.get`、`modem.get`、`traffic.get`、`network.pdp`（`+CGPADDR`）、`network.dhcp`、`qos.get`、`ca.get`、`modem.txpower`/`nr_txpower`/`endc`；仪表盘已在为它的主题读取守护进程缓存 |
| `system.js` | `system` | `sim-pin`、`advanced-set thermal-thresholds`/`thermal-log`、`factory-reset` | 读：`modem.get`（厂商/型号/版本/imei）、`sim.get`（status/iccid/imsi/number/slot/hotplug）、`system.temperature`、`system.thermal`、`system.fota`；写：`sim.pin_apply`、`system.thermal_set`、`system.factory_reset`、`modem.imei_set`、`system.fota_start`/`fota_abort` |
| `connection.js` | `advanced connection-settings`、`advanced session` | `pdp-set`、`advanced-set autodial`/`direct-ip`/`postroute`/`dmz`/`pdp-state`/`pdp-remove` | 读：`network.autodial`、`network.interface_cfg`、`network.pdp_contexts`、`traffic.*`、`network.dhcp` |
| `advanced.js` | `advanced hardware` | `advanced-set usb-mode`/`pcie-controller`/`nic-speed`/`interface-mode`/`sim-hotplug`/`thermal` | 读：`network.usb_mode`、`network.interface_cfg`、`system.device_control`（nic_rate/power_control）、`sim.get`（hotplug）、`sim.slot`、`system.thermal`；写：`system.nic_rate_set`、`system.power_control_set`、`sim.hotplug_set`、`sim.slot_set`、`system.thermal_set` |
| `terminal.js` | `command` | — | **刻意保留**：终端*就是*一个裸 AT 控制台；这是用户唯一有意键入 AT 的地方 |

在这些页面能够迁移之前，需要**新后端路由**填补的空缺（每个都是一个 `service` 函数 + 一个 `Route`
+ 一份 JSON 形状，遵循 `module-guide.md`）：

* `system.js`：`^VERSION` 的字段（构建日期 / 软件 / 硬件）、网络时间（`^NWTIME`）、LED 开关
  （`^LEDSWITCH` —— `system.device_control` 目前只承载 `nic_rate`/`power_control`）以及 SIM 激活
  电源（`^HVSST`）。
* `connection.js`：`^SETAUTODIAL` 写入、转发三件套（`direct-ip`/`postroute`/`dmz`），以及 PDP 写入
  （`pdp-set`/`pdp-state`/`pdp-remove`）。`network.pdp` 目前只是 `+CGPADDR` 的读取。
* `advanced.js`：`usb-mode` 与 `interface-mode` 的写入（`^SETMODE` / `^TDCFG`）。

这些都是 Rust 切片（CI 是唯一的编译器），正因如此 LuCI 那一批要在它们之后按页面逐个拆分；字段已被
覆盖的两个页面（`status.js`，以及 `system.js` 的 SIM/thermal/FOTA 那一半）可以先走。

### 状态页：CLI 的 status 文本帧消失（切片，2026-10-07）

`view/mt5700m/status.js` 就是清单里那个"已被覆盖"的页面。此处完成，基线为 `8a8c501`：

* **详情帧**不再调用 `mt5700m-at status`（一个缓存的 key=value 文本帧*加上*四次实时 AT 查询*加上*
  一次可达性探测，全部经由 CLI 进程中的 `client::at_cmd` —— 这正是本架构禁止的第二个 AT 所有者）。
  它此前喂给的四行仪表盘数据现在是路由：`network.pdp_contexts` 取 cid-1 的 APN（`+CGDCONT?`/
  `+CGACT?`，与 CLI 读取的命令相同），`qos.get` 取 QCI + 签约速率（kbps → Mbps，沿用 CLI 的
  `{:.1}`），`sim.number` 取 `+CNUM`；
* `usb_state` 来自 `usb` 主题（在位采集器已经对 3301/3302/3303 做了分类；`present=false` 映射为
  `absent`，正是 `mt5700m_usb_info()` 什么都没找到时 CLI 打印的内容）；
* `api.js` 去掉了 `atStatus` 动词。`advanced session` 仍是 CLI 调用 —— 地址卡片是下一个切片；
* 后端：`modules/sim/parser.rs::cnum_number_state`（+CME ERROR 22 *及其* CMEE=2 文本 "not found" →
  `not_stored`，已加入 `CME_TEXTS`）、`SimState::number_state`（JSON 为 `numberState`）、
  `sim.number` 保留一条被拒绝的 `+CNUM` 应答而不是丢弃它（那条应答*就是*答案，与 `+CPIN?` 同一
  模式），而 `parse_cnum` 现在会归一化/校验该字段（去掉引号/空格，数字可带可选 `+`，长度 ≥ 5） ——
  这条规则 CLI 曾在内联重新实现过，现在在模块中单点提供。

旧详情帧中的两个缺陷，都被这次重写修复，且都被 `scripts/prove-status-parity.js` 钉死：

1. `refreshDetail` 把 `native.stdout`（**一个字符串**）作为 `mergeStatusLines` 的 `overrides` 传入，
   而后者函数体里有 `(overrides || []).forEach` → 必然抛出 `TypeError`，被调用链的 `.catch` 吞掉。
   因此 CLI 文本帧从未真正合并过：线上页面一直显示一条永久横幅"Some modem details could not be
   refreshed. … (overrides || []).forEach is not a function"，而 APN / QCI / 签约速率 / 号码这几行
   **始终为空白**。
2. 该异常在 `state.sessionDetail` 被赋值之前就触发了，因此 Mobile IP 卡片（`advanced session`）在
   线上页面上也**始终为空**。基于路由的版本独立设置会话帧，因此卡片能渲染出来。

两者都是这些卡片本就为之构建的 UI，而非新增 UI：除了页面一直想渲染的那些值，DOM 中没有添加任何
东西。一处刻意的、被计数的差异（见证明脚本中的 `NUM_PLUS`）：电话号码丢掉了 CLI 的 `+` 前缀，因为
后端的领域号码是裸数字（与 WebUI 的 `normalizePhoneNumber` 相同）。另有两条仅作记录：

* CLI 曾把 `network.sysmode` 以 `network_mode` 这个键发布，而 `parseStatus` 优先采用它而非
  `sysmode_detail` —— 沿用该做法会把"Network Mode"单元格从 `5G SA`（线上值，也是 WebUI 从同一主题
  推导出的值）变成粗糙的 `NR5G`。因此快照映射保留 `sysmode`/`sysmode_detail`，证明脚本断言该单元
  格与线上页面逐字节一致。
* 温度：`mergeStatusLines` 是后写覆盖，因此快照里的原始浮点数总会覆盖 CLI 的 `round()`。早在本
  切片之前很久，页面显示的就是 `45.1` 而非 `45` —— 映射保留这一点。
* 载波频率：CLI 用 `nr_arfcn_to_mhz()` 推导 `dl_freq`（LTE 的 ARFCN 规则套用到 NR ARFCN 上 → 差了
  好几个数量级），而页面自己的 `carrier_1` 行在合并时覆盖了它，因此对于单个服务载波它从未可见。
  页面的 `carrier_1` 现在把频率列留空；真正的多载波 MHz 值要等载波卡片长出多载波列表时，从 `ca`
  主题获取。

此后剩余的 CLI 调用点：`status.js` 1 处（`advanced session`）、`system.js` 5 处、`connection.js`
3 处读取 + 其写入动词、`advanced.js` 1 处、`terminal.js` 1 处（刻意保留）。

本切片中的脚手架工作：`scripts/lib/luci-stub.js` 增加了 `querySelector(All)` / `parentNode` /
`replaceChild` / `removeChild` 和 `L.url`，因为 `updateRegions` 会做真实的增量 DOM 替换 —— 没有它们，
详情帧的更新路径在证明脚本里根本跑不起来。`scripts/prove-status-parity.js`（基线 `8a8c501`）用三种
方式比较八种形态：线上基线 vs 详情路由被桩为 `null` 的新版（正文必须逐字节一致）、CLI 文本帧的值 vs
路由的值（打过补丁的基线），以及两个合理变化的区块（`alerts`、`address`）按内容断言。
`scripts/smoke-minified-luci.js` 现在也渲染状态页（16 项检查；压缩后代码树 297 500 → 186 254 B）。

### 会话帧消失：`network.session`（切片，2026-10-07）

同一件事的后半部分，基线 `c0268e0`。`advanced session` —— 那个把八条 AT 应答（`^NDISSTATQRY?`、
`^DHCP?`、`^DHCPV6?`、`^IPV6CAP?`、`+CGPADDR`、`^DSFLOWQRY`、`^CGMTU=1`、`^DCONNSTAT?`）作为文本
dump 出来、供 `parser.parseSession()` 用正则去抠的 CLI 动词 —— 现在是一条路由：

* `network.session`（`modules/network/{commands,parser,state,service,api}.rs`）返回解码后的快照：
  `{ipv4:{connected,address,gateway,dns[]}, ipv6:{connected,address,dns[]}, capability, mtu,
  maximum_down, maximum_up, flow:{current_duration,current_tx,current_rx,total_duration,total_tx,
  total_rx}, sessions:[{cid,apn,ipv4,ipv6,type,ethernet}]}`。两个界面的读取方（总览页的
  "Mobile IP"卡片、连接页面的面板）都调用它 —— 一个解码器。
* `network.flow_clear`（`AT^DSFLOWCLR`）取代了"Clear counters"按钮背后的 `flow-clear` CLI 动词；
  该按钮现在使用 `c.confirmRoute`。
* `api.js` 去掉了 `atSession`；`parser.js` 去掉了整个文本解码器 —— `parseSession`、`csvValues`、
  `hexIPv4`（那些最小的辅助函数存在只为读那个文本帧）—— 并新增了 `sessionInfo(payload)`，它保留
  面向页面的字段名（`ipv4Connected`、`maximumDown`、……），因此渲染代码没有挪动。那里剩下的三个
  映射是展示选择（capability 码 → 本地化标签、缺失 MTU → "Network default"、DNS 列表 → 用 " · "
  连接）。
* `DhcpLease` 保留了应答末尾的两个字段（`maximum_down`/`maximum_up`，原样保留 —— CLI 页面是通过
  自己的速率格式化器渲染它们的，因此在模块里解码它们会改变它显示的内容）。

两个解码器在迁移过程中变得更严格/更正确，都由单元测试断言：

* `parse_ipv6cap` 现在也读取十六进制形式。WebUI 一直渲染的是 `0x0B`（"separate APNs"），但该字段
  过去只按十进制解析，于是 `0B` 被当作"no answer"漏掉，两个页面因此丢了一个它们本就设计要显示的
  值。
* `parse_dconnstat` 会归一化固件偶尔发送的全角分隔符/引号（`，`、`“”`）—— 前端的正则容忍它们，而
  朴素的逗号切分不会。

本系列中更早的一个计划是让连接页面丢掉 `form.Map`，手工构建拨号表单。该计划被放弃了：`form.Map` 是
LuCI 自己的 UCI 表单机制（`settings.js` 用的是同一个东西），页面的 UCI 读写属于主机配置而非 modem
业务逻辑，而手工逐字节复现 CBI 的标记/CSS 是一种 UI 变更风险，却没有架构收益。页面保留框架表单；
只有 modem 数据路径迁到了路由。

CLI 动词本身保留。`mt5700m-at status`、`advanced session` 与 `flow-clear` 是有文档的工具
（`README.md` 在诊断配方里用到了 `status`），而且它们**不是**任何东西的第二套实现：`status` 以 CLI
自己的 `key=value` 形状渲染守护进程的 StateCache，而 `advanced <group>` 的 dump 是原始应答文本而非
解析器。删掉它们将是一次用户可见的损失，却没有架构收益 —— 重构禁止的是*前端*拥有第二套解码器，而
这正是本切片移除的东西。（它们的 AT 读取都经由守护进程通道，即那唯一的调度器；仅剩的"自己打开
端口"路径是 CLI 的无守护进程回退，那是下一个候选目标。）

`scripts/prove-connection-parity.js`（基线 `c0268e0`，30 项检查）渲染连接页面并比较五种形态：会话
夹具（`scripts/lib/at-fixtures.js::sessionFacts`）从同一个对象**同时**生成 CLI 文本帧与路由载荷，
因此"渲染一致"意味着模块的解码等于前端过去从 dump 里正则抠出的东西 —— 覆盖完整的 `^NDISSTATQRY`
规则、NDIS 为空时的回退（"has an address ⇒ connected"）、纯 IPv4、`0B` capability 场景，以及一次
彻底失败的会话读取（此时两侧渲染出同样的空卡片 —— 没有新增横幅，CLI 的失败路径本来也没有）。它还
钉死了写入路径（`flow-clear` → `network.flow_clear`）与 900 毫秒重载。

至此，总览页的 CLI 调用为 **零**，连接页面降到一处（`advanced connection-settings`，它是该页面仍
通过 `advanced-set` 写入的那套拨号设置的读取半边）；剩余的 CLI 调用点是 `system.js` 1 处、
`advanced.js` 1 处、`connection.js` 1 处、`terminal.js` 1 处（刻意保留）。

### 系统页面的写入迁到路由（切片，2026-10-08）

系统页面剩余的六处 CLI **写入**现在通过模块注册表分发。22 个区段的读取文本帧（`mt5700m-at system`）
未动，而七处写入刻意留在 CLI 动词上 —— 见下。

| 旧 CLI 动词 | 路由 | 等价性 |
| ------------ | ----- | ----------- |
| `airplane <0\|1>` | `network.radio_set {airplane}` | 同样的 `AT+CFUN`；页面的 `0\|1` 变成领域布尔值 |
| `advanced-set sim-slot <v>` | `sim.slot_set {slot}` | **故意差异** —— 模块执行厂商的完整切换时序，CLI 只发了 `AT^SCICHG` |
| `set-imei <v>` | `modem.imei_set {imei}` | 同样的 `AT^PHYNUM=IMEI`；15 位数字规则在模块里只存在一份 |
| `restart` | `modem.reset` | 同样的 `AT^RESET`（`commands::RESET`） |
| `sim-pin <op> a1 a2` | `sim.pin_apply {operation,pin,newPin}` | 五种操作一一对应；UI 隐藏该字段时 `newPin` 为 `""`，与 CLI 传的完全一致 |
| `factory-reset` | `system.factory_reset` | `AT&F` 对比 CLI 的 `AT&F0` —— `&F` 默认为 profile 0，因此无行为变化 |

`components.js` 早已把写入确认弹窗抽成 `confirmAction`，因此本切片对 `danger` + 自定义延时变体做了
同样的事：`runConfirmedAction` 现在是唯一实现，`runConfirmed`（CLI 动词）与新的 `runConfirmedRoute`
（路由）是它两个薄调用方。弹窗的 DOM、措辞、按钮顺序与恢复延时行为无法分叉。

本切片之后有七处写入留在 CLI 动词上：其中四处（`advanced-set led`、`advanced-set sim-activation`、
`advanced-set thermal-thresholds`、`advanced-set thermal-log`）是因为当时后端**没有**面向
`^LEDSWITCH`、`^HVSST=`、`^THERMLDAUTOPARA=` 或 `^THERMLDLOGSW=` 的写入路由，再加上 `fota-start` /
`fota-resume` / `fota-upgrade` —— 模块的形态是"启动一个任务，然后观察它"，而页面驱动的是一个
三步的下载/恢复/安装流程；把它转换过来是产品决策而非重构步骤。前四处在一个切片之后迁移
了 —— 待后端具备那四组命令后见
[下文](#系统页面另外四处写入迁到路由切片2026-10-08)；只有 FOTA 那三处仍
有意守着 CLI 这条线。

`scripts/prove-system-parity.js`（基线标签 `pre-system-route`，34 项检查）渲染六个读取文本帧 —— 完整
帧、SIM 需要 PIN、号码未存储、FOTA 安装完成、所有 thermal 区段缺失、以及飞行模式
—— 并要求每一行逐字节一致，因为本切片没有动解析。十二项映射检查同时钉死**两**侧：旧页面放到命令行
上的内容*以及*新页面发送的内容，因此参数写错的路由会失败，尽管其它什么都没变。另有七项检查断言
那些保留的动词**未被**迁移。

`scripts/smoke-minified-luci.js` 增加了一个系统页面区块（22 → 28 项检查）：它在*压缩后*的代码树上
驱动四处无危害的写入（airplane、SIM slot、restart、factory reset），并断言每次恰好一次 `routeCall`。

给接手读取文本帧的人的提示：`pre-system-route` 是一个**标签**，而不是硬编码的 SHA。其它证明脚本默认
使用其切片前提交的 SHA，而那在沙箱里一次全新克隆后无法存活 —— `scripts/lib/at-fixtures.js` 中的
那一份夹具（`SYSTEM_FACTS` / `systemCliFrame`）现在同时供给本证明与冒烟测试，因此二者不会漂移。

### 系统页面：另外四处写入迁到路由（切片，2026-10-08）

上一个切片交付时，后端还没有能承载这四处的路由，因此它们与其它写入一起留在 CLI 动词上。此后两半都
落地了 —— 后端新增了四对路由（`system.led` / `system.led_set`、`system.network_time`、
`system.thermal_thresholds_set` / `system.thermal_log_set`、`sim.activation` /
`sim.activation_set`）以及本切片 —— 它把最后四处*简单*写入从动词上移走。三处 FOTA 写入保留，理由
不变：模块回答的是"任务 + 观察"，UI 驱动的是一条流程，弥合这个缺口改变的是交互而非传输层。

| 旧 CLI 动词 | 路由 | 等价性 |
| ------------ | ----- | ----------- |
| `advanced-set led <0\|1>` | `system.led_set {enabled}` | 同样的 `AT^LEDSWITCH=<0\|1>`；下拉框的字符串在边界处转为布尔值 |
| `advanced-set sim-activation <0\|1>` | `sim.activation_set {active}` | 同样的 `AT^HVSST=1,<0\|1>`；现在由包住卡槽切换的那个构造器构造 |
| `advanced-set thermal-thresholds <9 values>` | `system.thermal_thresholds_set {thresholds}` | 同样的 `AT^THERMLDAUTOPARA=<9 values>` |
| `advanced-set thermal-log <s> <f>` | `system.thermal_log_set {serial,file}` | 同样的 `AT^THERMLDLOGSW=<s>,<f>` |

有两处是刻意**未**改动的：

* **页面保留自己的提交前校验**（0–150 °C / ladder check 及其两条 i18n 文案）。这是 `sim.pin_apply`
  定下的先例 —— 显示的文案是 UI，模块拥有 modem 的规则并会重新校验，而路由不会拿着一张 modem 会
  拒绝的表被调用到。
* **thermal 弹窗仍以一个 `Promise.all` 同时发起两次写入**，因此它的"Thermal settings saved." +
  reload 行为与过去逐字节相同。

`scripts/prove-system-parity.js` 在同一条 `pre-system-route` 基线上从 34 项检查增至 39 项：那四项
"动词消失"检查移入了 B 组，C 组新增了 LED（两个取值）、SIM activation（两种状态，通过 `^HVSST` 的
第二个字段驱动）与 thermal 那一对，而 D 组从七个保留动词缩减为三个。C 组仍然逐场景钉死**两**侧 ——
旧 argv *和*新参数 —— 因此参数打错会失败，而不是悄悄分叉。`scripts/smoke-minified-luci.js`
（28 → 31 项检查）在压缩后的代码树上驱动这些新写入，包括那个两次调用的 thermal 弹窗。
`scripts/prove-system-parity.js` 在同一条 `pre-system-route` 基线上从 34 项检查增至 39 项：那四项
"动词消失"检查移入了 B 组，C 组新增了 LED（两个取值）、SIM activation（两种状态，通过 `^HVSST` 的
第二个字段驱动）与 thermal 那一对，而 D 组从七个保留动词缩减为三个。C 组仍然逐场景钉死**两**侧 ——
旧 argv *和*新参数 —— 因此参数打错会失败，而不是悄悄分叉。`scripts/smoke-minified-luci.js`
（28 → 31 项检查）在压缩后的代码树上驱动这些新写入，包括那个两次调用的 thermal 弹窗。

### 系统页面：22 段读帧切成 15 条路由（切片，2026-10-08）

这是系统页的最后一批迁移，也是整个前端里**最大的一段 CLI 耦合**：`load()` 里那一次
`api.atSystem()`（`mt5700m-at system`，22 段文本帧）换成了 15 条
`api.route(...)` 的 `Promise.all`，`mt5700m/api.js` 的 `atSystem` 速记随之删除
（没有第二个调用者）。旧 JS 里那份 22 段的切分/取值规则（SIM PIN 位数、
温控阶梯、DSAMBR 字段序……）是这些语义的**第二份实现**，本批后只剩后端一份。

`renderPage` 的入参从 `{stdout, stderr}` 变成路由结果数组，但**渲染代码一行没动**：
领域载荷在函数头部映射回与旧版同名同型的局部变量（`sim / iccid / imsi / phone /
subscription / operator / networkTime / functionLevel / ledState / simActivation /
simSlots / temperature / fotaMode / fotaState / total / received / revision / imei /
buildDate / software / hardware / thermalLevel / thermalThresholds / thermalLog`）。
缺失回退也照旧仿出来：`^HVSST` / `^SCICHG` / `^THERMLDLOGSW` 三段原本解析成
字符串数组，select 的默认值与 `length > 1` 判断依赖这个形态 —— 整段不答时
仍然是 `['']`。

三处**有意**的行为决策（逐字等价的代价是保留旧版的小怪癖）：

* **Protection level 恒显示**：旧版判据是 `thermalValues.length`，而 `''` 切逗号
  得到 `['']`，length 恒为 1 —— "段没答"也显示「Normal」。路由化后沿用同一个
  公式（`Number(currentLevel) || 0`），不引入新的空态。
* **stderr 告警条删除**：旧版 `renderPage` 顶部有 `res.stderr` 的 warning 横幅。
  路由世界里没有 stderr —— 任一路由失败会让 `Promise.all` 整体 reject，由
  `render()` 的失败分支画整页 `alert-message error`（与 network 页一致）。
* **Technical details 改倒路由载荷**：折叠块仍在（同一 class、同一标题），
  内容从 22 段 AT 原文改为 `api.<route>` + JSON（15 条，照 network 页先例）。
  新版页面里不再出现任何 AT 应答原文。

三处**渲染数值漂移风险**在动工前逐项取证排除：

1. 订阅速率 —— `qos.ambr_*_kbps` 就是 kbps 原值，页面 `parser.subscriptionRate`
   的 `/1000` 与旧帧 `^DSAMBR` 同源同算；
2. 运营商 —— `network.get.operator` 即 `+COPS` 引号内字段（旧页面第 3 字段）；
3. 英雄区峰值温度 —— `system.temperature.peak`（后端过滤 0 / 65535 后取最大）
   与页面旧公式 `(max(raw)/10).toFixed(1)` 等价（network 页早已在用同一个字段）。

`scripts/prove-system-parity.js` 扩到 **48 项**：A 组改为同一份 `SYSTEM_FACTS`
双形态喂数 —— 旧侧 `systemCliFrame`（22 段文本帧）、新侧
`systemRouteResults`（15 条路由载荷，出自同一份事实对象），六个病态用例
逐行比对；Technical details 块作为有意差异抹平后单独断言（块仍在、旧侧含
`^VERSION` / `^CHIPTEMP` 原文、新侧含 `api.system.version` 且无任何 `^` 帧）；
E 组改为断言 `atSystem` 在 system.js 与 api.js 两处都已消失；新增 F 组
（调用面）：整页 `load()/render()` 真跑，旧版恰一次 `at system`，新版零 CLI、
15 条路由且顺序即 `Promise.all`。`scripts/smoke-minified-luci.js`（31 → 33 项）
在压缩树上做同样的断言 —— 不喂 `at:system` 帧、不挂 `atSystem` 桩，
任何残留的 CLI 读帧调用都会直接崩掉。

保留的 3 条 FOTA 写动词不变；FOTA state / progress 两段读已并入
`system.fota` 路由（`state / total / received`），页面的 percent 计算
保持旧公式。

### 连接页面：设置帧与七条写入切成 11 条路由（切片，2026-10-08）

`advanced connection-settings` 的五段读帧与七条写入动词离开 `connection.js`，
页面至此**零 CLI**（FOTA / 终端除外，本页本就没有）：

- 读：`network.autodial` / `network.interface_cfg` / `network.pdp_contexts` /
  `network.direct_ip` 四条路由替换 `api.atConnectionSettings()`（`load()` 里
  与 session 一起进 `Promise.all`）；前端那份第二解析（`autoMatch` 正则、
  `parseContexts` 调用）随本批离开连接页 —— `parser.js` 的死导出留给清理批次。
- 写：`pdp-set` / `pdp-state` / `pdp-remove` → `network.pdp_set` /
  `network.pdp_state` / `network.pdp_remove`，`advanced-set autodial` /
  `direct-ip` / `postroute` / `dmz` → `network.autodial_set` /
  `network.direct_ip_set` / `network.postroute_set` / `network.dmz_set`。
  校验全部搬进 `commands.rs` 构造器（cid 1–11、`safe_at_field`、
  **尾部空字段必须省略**的 SETAUTODIAL 形态、postroute 的两笔串行且首笔
  失败即止）；`cli.rs` 改薄转发（cid 的 `01` 保真拒绝照旧）。
- 删除：`api.js` 的 `atConnectionSettings` 速记（定义 + 导出，均无第二个
  调用者）。

两处怪癖在载荷层原样仿制 + 一处**有意修复**（`prove-connection-parity.js`
逐字比对覆盖）：

1. **autoKnown**：旧 `autoMatch` 要求 `^SETAUTODIAL` 应答带 auth 字段，缩短
   形态（`1,0,"IPV4V6"`）整行不识别、控件回落默认值 —— 载荷层 `authType`
   缺席走进同一分支；
2. **directIp / postRoute 的「值不在合法集就禁用」**：载荷缺席（`undefined`）
   与旧帧解析失败（`''`）同一条路 —— 控件禁用、Apply 按钮隐藏、DMZ 输入框
   回落空串；`Dmz: not cfg` 即 `enabled: false`（`parser.rs` 单测钉住）；
3. **有意修复（不是漂移）**：真实抓包的 TDCFG 应答是 `PostRoute : 1` ——
   冒号前带空格（`parse_interface_cfg` 单测样本与 doc 注释；姊妹项目
   `luci-app-mt5700` 的 `dial.js` 用 `/PostRoute\s*:\s*(\d+)/` 两种形态都认）。
   mt5700m 旧前端正则漏了冒号前的 `\s*`，真实设备上 Post-routing 控件**永远
   Unavailable**（Apply post-routing 按钮从未出现过）。新链路把值解出来后
   控件恢复可用 —— prove 脚本以「盲区形态」组单独点名断言（旧侧 Unavailable
   vs 新侧取值可用），DMZ 不受影响（`Dmz:` 冒号紧跟，旧正则认）。

`scripts/prove-connection-parity.js` 重定位为「连接设置批」证明（基线
`7a02417`，**58 项**）：8 个渲染形态（5 会话 + 3 设置病态）整页逐字比对 +
控件取值 / DMZ 输入框单独断言；数据源组断言旧侧恰一条 `advanced
connection-settings` 帧、新侧零 CLI 且 5 条路由顺序即 `load()` 的
`Promise.all`；**七条写路径逐条驱动**（按压按钮 → 确认弹窗），旧侧断言
CLI argv、新侧断言路由名 + params 逐键一致；盲区形态组单独点名差异。
`scripts/smoke-minified-luci.js` 扩到 **38 项**：连接页不喂设置帧、不挂
`atConnectionSettings` 桩，断言零 CLI + 5 条读路由 + 控件取值来自载荷 +
三条写入抽样（autodial_set / pdp_set 弹窗 / dmz_set）。`cargo test` **290**。
