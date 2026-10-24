# AT 命令手册核对记录

对照依据：`MT5700M-CN 5G系列模组AT命令手册`（文档版本 01，2024-05-17）、
`FOTA升级指南`、`USB接口应用指南`、`高精度授时功能验证指导` 与
`sms-at-protocol.md`。

核对方法：提取后端所有 `AT` 字面量，与手册中的命令/语法/参数表逐条比对；
对格式类命令同时核对响应字段、单位、无效值和查询后缀。测试用模拟串口序列，
不依赖真实模组。

## 1. 已核对一致的命令组

| 领域 | 命令 | 核对结果 |
|:---|:---|:---|
| 身份/版本 | `ATI`、`AT+CGSN`、`AT+GSN`、`AT+CGMR`、`AT^VERSION?`、`AT^ICCID?`、`AT+CNUM` | 命令与查询形式一致；`^VERSION` 的 `BDT`/`EXTS`/`EXTH` 行按原始字符串解析 |
| SIM | `AT+CPIN?`、`AT^SIMSQ?`、`AT+CLCK="SC",2`、`AT+CPWD`、`AT^SCICHG`、`AT^HVSST`、`AT^TDSIMHP` | `+CPIN` 的 CME 10/11/12/17/18 分支与手册一致；`^SIMSQ?` 用于 dead/present 细化 |
| 注册 | `AT+CREG?`/`AT+CEREG?`/`AT+C5GREG?`、`AT+COPS?`、`AT+CGREG=2` | 字段布局按 27.007；厂商 MCC/MNC 前缀变体已兼容 |
| 信号 | `AT+CSQ`、`AT^HCSQ?`、`AT^DSAMBR`、`AT^NTXPOWER?`、`AT^TXPOWER?` | `^HCSQ` 索引换算与手册一致：RSRP `n-141`（≥97 → -44）、RSCP `n-121`（≥96 → -25）、SINR `-20.2+0.2n`（≥251 → 30）、RSRQ `-20+0.5n`（≥34 → -3）、255 无效 |
| 小区 | `AT^MONSC`、`AT^MONNC`、`AT^MONSSC`、`AT^CASCELLINFO?`、`AT^NRSSBID?`、`AT^HFREQINFO?` | 手册中这些命令**没有 `?` 后缀**，代码按无参查询发送；十六进制 PCI、1/8 dB 单位与无效值 255/32767/-1256/-348/-188 已在解析层处理 |
| 扫描 | `AT^CELLSCAN`、`AT^CELLSCAN=` + band 位图、终止符 `abcd` | 参数约束（band/freq 互斥、NR 需 SCS）与位图构造已核对 |
| 数据连接 | `AT+CGDCONT`、`AT+CGACT`、`AT+CGPADDR`、`AT+CIREG?`、`AT^NDISSTATQRY?`、`AT^DCONNSTAT?`、`AT^DSFLOWQRY`、`AT^DSFLOWCLR`、`AT^CGMTU`、`AT^IPV6CAP?`、`AT^DHCP?`、`AT^DHCPV6?` | `^DHCP` 八字段 `<clip>,<netmask>,<gate>,<dhcp>,<pDNS>,<sDNS>,<max_rx_data>,<max_tx_data>`；IPv4 为 8 位十六进制小端（`0100A8C0` → `192.168.0.1`），IPv6 为文本；少于 6 字段不猜测槽位 |
| 拨号控制 | `AT^SETAUTODIAL?`、`AT^SETAUTODIAL=`（七参数，尾空字段省略）、`AT^SETMODE?`、`AT^SETMODE=`、`AT^TDCFG?`、`AT^TDCFG=` | 七参数顺序与手册一致；`^SETAUTODIAL` 关闭时只回开关，代码用 `^NDISSTATQRY?` 补齐 dialMode |
| 射频策略 | `AT+CFUN?`、`AT+CFUN=`、`AT^SYSCFGEX?`、`AT^SYSCFGEX=`、`AT^C5GOPTION?`、`AT^C5GOPTION=`、`AT^LTEFREQLOCK?`/`=`、`AT^NRFREQLOCK?`/=` | 锁频返回 `<type>` 行 + `<mobility>,<num>` + 每载波行；LTE 3 字段、NR 4 字段（含 SCS），PCI 十六进制 |
| 温度/电源 | `AT^CHIPTEMP?`、`AT^THERMAUTOFUN?`/`=`、`AT^THERMLDLOGSW?`/`=`、`AT^THERMLDAUTOPARA?`/`=`、`AT^THERMLDAUTOSTATUS?`、`AT^TDPCIELANCFG?`/`=`、`AT^TDPMCFG?`/`=`、`AT^LEDSWITCH?`/`=` | `^CHIPTEMP` 12 个传感器、0.1 °C、65535 失败值；阈值 9 项、0–150 °C、触发点递增 |
| 短信 | `AT+CMGF?`/`=0`、`AT+CMGL=4`、`AT+CMGR`、`AT+CMGD=<index>[,<delflag>]`、`AT+CMGS=<length>` + PDU（单行主形态）、`AT+CPMS?`/`=`、`AT+CSCA?`/`=`、`AT+CMEE=2`、`AT+CNMI=2,1,0,2,0`、`AT+CUSD=1,"<hex>",15`、`AT+CUSD=2` | PDU 模式 `=0`；`<length>` 只计 TPDU 八位组（不含 SMSC）。**主形态为单行 `AT+CMGS=<len>\r<PDU>`**，其中 `\r` 是字面「反斜杠+r」两字节（手册 9.14.5 演示），该模组**不实现 `>` 两段式提示**；两段式仅在本地 `ERROR`/`+CME ERROR` 后一次性回退，`+CMS ERROR`（网络拒收）与静默超时**绝不回退**（重发＝二次投递），任何失败一律 ESC(0x1B) 清理。`CMGL` 每行头后跟 PDU，按 TP-MTI 分派 DELIVER（收）/ SUBMIT（发）。SUBMIT 首字节 SRR=1（`0x31`/`0x71`）、相对有效期 TP-VP=`0xA7`（24h，边界 167→24h/168→48h）、目的地址 TOA 国际 `0x91` / 国内短号 `0x81`，地址长度字段填**位数**而非补齐后的 nibble 数。TP-DCS 按**高半字节分组**判编码（general/MWI/UCS2/message-class），不再用扁平 `(dcs>>2)&3`。SCTS 仅前 6 字节按 BCD 校验、时区字节单独取，避免把带合法时区的消息整条丢弃。长短信 UDH 接受 8-bit（IEI `0x00`）与 16-bit（IEI `0x08`）参考号。`CMGD=1,4` 删除全部，且 clear_all 结束后**还原原 `+CPMS` 三面**（mem1 决定后续 list/delete/CMGR 落在哪个存储面）。USSD 使用 GSM 7-bit 十六进制。CMEE/CNMI 为进程内一次性会话引导（`+CNMI` 掉电不保存，故每次连接重设） |
| FOTA | `ATE0`、`AT^FOTAMODE=0,1,0,1`、`AT^FOTAOEMDL="<url>"`、`AT^FOTASTATE?`、`AT^FOTADLQ`、`AT^FOTADL=0|1|2`、`AT^FWUP` | 状态 10/11/12/30/31/40。`^FOTAOEMDL` **要求模组处于 IDLE（`^FOTASTATE:10`）前置**：非 IDLE 时先拒绝并提示，卡在 30/31/40 时用 `^FOTADL=0` 复位回 IDLE 再判一次。31 续传前使用 `^FOTADL=1`；`^FOTADL=0` 取消；`^FWUP` 启动升级。下载 URL 仅 **HTTP**（HTTPS 暂未支持）；升级前须停语音/数据/拨号；`^SSLMNG`/`^SSLCFG` 不随掉电保存 |
| 授时 | `AT^NWTIME?`（已接入）、`AT^HPTCFG`（仅核对、**未接线**，见 §5） | `^NWTIME?` 返回网络时间字符串，原样展示，仅作诊断，不参与系统时钟校准。高精度授时 `AT^HPTCFG`：手册 §11.14 定义为 2 参数 `<tsnEn>,<ueUncertainty>`，而《高精度授时功能验证指导》示例用 3 参数（`1,0,32`）且要求**重启生效**；两文档在参数个数与生效方式上冲突，且该功能与本项目自有的 NTP 时间模块相互独立（不得被其触碰）。在无硬件复验前**不接线、不做 UI**，仅在此登记为待验证项 |

## 2. 本轮修正

| 位置 | 问题 | 处理 |
|:---|:---|:---|
| `scheduler/arbiter.rs` | 超时分类里出现手册未定义的 `AT^SIMSTATE?` | 改为手册中的 `AT^SIMSQ?` |
| `scheduler/arbiter.rs` | 超时分类里出现 `AT^COPS?`（手册只有 `AT+COPS?`） | 删除未定义项，保留 `AT+COPS?` |
| `scheduler/gate.rs` | 只读白名单含 `AT^CPIN?`（手册只有 `AT+CPIN?`） | 删除；`AT+CPIN?` 仍由 `?` 后缀规则识别 |
| `scheduler/gate.rs`、`daemon.rs` | 注释写成 `AT^CGEQOSRDP` / `AT^CGPADDR` | 更正为 `AT+CGEQOSRDP` / `AT+CGPADDR` |
| `scheduler/gate.rs` | 缓存 TTL 前缀 `AT^THERM` 会误匹配多组命令 | 删除冗余前缀，保留精确的 `AT^THERMLDAUTOPARA?` |

## 3. 手册未记载、保留的厂商命令

这些命令来自原项目/原 WebUI，**不在本次提供的手册中**。保留是为了不破坏已有
功能，但都按“模组不认就明确失败”处理，不伪造结果：

| 命令 | 使用位置 | 安全行为 |
|:---|:---|:---|
| `AT^SETDIRECTIP?` / `AT^SETDIRECTIP=<0\|1>` | `network.direct_ip`、`network.direct_ip_set`、连接页 IP 直通 | 查询返回 `ERROR`/非 0/1 时 `enabled` 键缺席，页面禁用控件并隐藏写入按钮；写入错误原样返回 |
| `AT^PHYNUM=IMEI,<imei>` | `modem.imei_set`、CLI `set-imei`、系统页“设备身份实验室” | 页面文案明确标注“不支持的操作”，要求确认；模组拒绝时返回 `+CME ERROR`/`ERROR` 文本，成功必须收到 `OK` |

任何依赖这两条命令的自动化测试只使用模拟应答；真机可用性仍需在对应固件上单独
验证（见 §5）。

## 4. 自动化覆盖

- `scheduler/arbiter.rs`：25 ms 起始间隔（含重试）、优先级抢占、去重、独占、
  队列截止时间、取消（排队中/可中断命令）、SMS 事务期间暂停非 SMS、SMS 审计为
  P0、事务释放、嵌套守卫、SMS 挂起时拒绝长独占任务。
- `transport/urc.rs`：URC 分流、PDCP 行解析、短行拒绝。
- `modules/sms/*`：PDU 编解码、分片合并（8-bit / 16-bit 参考号）、SUBMIT/DELIVER
  按 MTI 分派、TOA 国际/国内判定、TP-VP 相对有效期、TP-DCS 高半字节分组、SCTS
  时区不再误判、`+CMGL`/`+CPMS`/`+CSCA` 解析、clear_all 三面还原、CMEE/CNMI 会话
  引导、USSD 7-bit。`daemon.rs`：`AT+CMGS` 单行主形态与 `ERROR`/`+CME`/`+CMS`/
  超时四类分派（仅本地错误一次性回退两段式）。
- `modules/network/parser.rs`：`^DHCP`/`^DHCPV6` 字段与字节序、`+CGPADDR`
  IPv4/IPv6、`^LTEFREQLOCK`/`^NRFREQLOCK`、`^SYSCFGEX`、`^SETAUTODIAL`。
- `modules/system/*`：`^CHIPTEMP` 无效值、FOTA 状态机（含 IDLE 前置与 `^FOTADL=0`
  复位的 `wait_for_idle`）、阈值规则。
- `tests/ui-time.js` + `_test_v2313.js`：LuCI 统一 Tab、时间/DNS 页面渲染、
  未保存输入保留、URL 内层 Tab、校验、页面卸载释放。

## 5. 仍无法在当前环境验证的部分

- 真实模组的 AT 时序、URC 插入点、SMS 网络下发结果、FOTA 实际下载/升级；
- 真机 `AT^SETDIRECTIP` / `AT^PHYNUM` 支持情况；
- **高精度授时 `AT^HPTCFG`（有意不接线）**：手册 §11.14 与《高精度授时功能验证
  指导》在参数个数（2 vs 3）与生效方式（是否必须重启）上互相冲突，且要求与本项目
  自有的 NTP 时间模块解耦、不得相互触碰。在无 MT5700M 硬件复验前不实现、不做 UI，
  避免把「盲写的、需要重启验证、可能覆盖 NTP」的功能塞进代码。此项与上面几条同属
  硬件边界，留待真机验证后再落地；
- 目标板运行：`at-webserver` 已用 `aarch64-unknown-linux-musl`
  （rustc 1.98.1 + 官方 rust-std）交叉编译为静态 ELF，并用
  `file` / `strings` 确认架构与 `time.*` / `dns.*` / `task.*` 路由存在；
  当前环境没有 aarch64 运行器或 MT5700M 硬件，未执行目标板启动、
  真实串口与拨号/短信联调。
- OpenWrt SDK 整包构建（`make package/luci-app-mt5700m/compile`）未执行：
  环境没有预装 SDK，完整 SDK 约 263 MB 且构建脚本依赖可用的下载链路；
  仓库内的 `scripts/build-release.sh` 保留为发布环境入口。
