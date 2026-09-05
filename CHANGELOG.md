# Changelog

## [WebUI 3.0.2] - 2026-09-06

### Added
- 归档上游 [inotdream/mt5700webui-openwrt-server](https://github.com/inotdream/mt5700webui-openwrt-server) v3.0.2 完整源码（Go 后端 + React/Semi 前端 + LuCI 集成应用）至 `mt5700webui-openwrt-server/`，移除上游 CI 工作流与垃圾文件，其余原样保留。详见 [`VENDOR.md`](mt5700webui-openwrt-server/VENDOR.md)。
- 新增 **Go 后端的 Python 移植版** `mt5700webui-openwrt-server/at-webserver/at-webserver.py`（单文件，依赖 pyserial + websockets）与配套 procd init 脚本（`files-py/`）。协议与 Go 版严格一致：WS 文本帧即 AT 命令、`{success,data|error}` 应答、ping/pong 心跳、认证握手、`AT+CONNECT?`/`AT+SCHED?`/`AT^CELLSCAN` 伪命令、`raw_data`/`incoming_call`/`new_sms`/`pdcp_data`/`cellscan` 推送、短信 PDU 解码（GSM-7/UCS2、长短信分片拼装）、定时锁频（含频段/频点校验与 SCS 自动推断）、企业微信通知合并推送。移植背景：Go 版二进制在 ImmortalWrt 6.18 内核（n_tty 重构）上串口空闲读返回 0 字节被误判 EOF 陷入重连死循环；Python 版经实机验证稳定。
- 新增 `mt5700webui-openwrt-server/prebuilt/aarch64_cortex-a53/`：上游 v3.0.2 预编译 apk 存档（at-webserver 后端包 + luci-i18n 中文语言包）。

### Changed
- 实机（H5000M / ImmortalWrt 6.18.44 aarch64）WebUI 后端由旧 Python 服务（`at-server.py`，配套 umi 旧前端）切换为 3.0.2 前端 + Python 移植版后端：`connection_type` 由 `UBUS` 改为 `SERIAL` + 串口自动探测（`ubus-at-daemon` 停用，AT 口由 WebUI 后端独占）。旧前端与服务已备份（`/root/webui-old-backup-20260906.tar.gz`）。
- 实机验证通过：WS 认证/ping-pong/`AT+CONNECT?`（串口模式返回 1）/`AT+CGMR`（V200R001C20B014）/`AT^MONSC`（NR 实时小区）/`AT+C5GREG?`/`AT+SCHED?` 全链路正常。

## [2.3.40] - 2026-08-29

### Fixed
- 实机（ImmortalWrt 6.18.44 aarch64 / MT5700M）拨号健康检测与恢复回归，修复若干缺陷：
  - 修复 healthy 判定在模组侧谎报已拨号（`NDISSTATQRY` v4/v6=1）但主机侧网卡 `carrier=0`（链路 DOWN）时，被误判为 transient、永不触发恢复的问题。引入 **carrier 门控**：无 carrier 时即便模块报已拨号也判 unhealthy，交由恢复流程重新拉起链路。实机验证 eth2 由 DOWN 恢复正常并获得地址 `10.6.150.230/8`，外网 ping `223.5.5.5` 0% 丢包。
  - 修复 `acquire_lock()` 在并发初始化（owner 为空表示该进程仍在初始化）场景下的竞争：原逻辑会误删正在被其他进程初始化的锁，造成锁被抢/误删。现对空 owner 与存活 owner 统一退避，消除竞争窗口。
  - 修复 `state_read` 对空白/非数字内容的处理：`busybox tr` 不支持 POSIX 字符类 `[:space:]`，原 `tr -d '[:space:]'` 在设备上不过滤空格，导致 `12 34` 不被归一。改为显式 `tr -d ' \t\r\n'`，计数类字段（如 `recovery_failures`）解析更稳健。
  - 修复 `mt5700m-at` 的清理逻辑（`at_serial_cleanup` 临时文件泄漏、`at_response_ok` 未传播 ERROR）与 `usb.sh` 变量重命名（`wait`→`timeout_s`）。
- 测试桩补齐：`timeout` 在 busybox 下不走 PATH 而直接 exec，测试中需显式 stub；新增 `iface_carrier()` 可注入函数替代硬编码 `/sys/class/net` 读取，并用 `timeout`/`carrier` stub 确保测试可独立于真实网卡运行；新增用例覆盖 carrier 门控（用例 2.3b）。修复测试自身对退避时长断言的时序脆弱性（改用单一时间戳锚点，兼容慢速主机）。

### Changed
- 默认配置与 `90-mt5700m` 同步新增 `radio_settle='45'`（HEALTH_RADIO_SETTLE），与代码默认值对齐。

## [2.3.39] - 2026-08-28

### Changed
- 发布说明改为按版本自动生成，不再内联全量历史。原先 `release.yml` 里硬编码了一整段发布说明（v2.3.18 起的所有变更），导致每个 Release 页面都显示 5000+ 字符的过期日志。现在新增 `Build release notes` 步骤，用 awk 从 `CHANGELOG.md` 精确截取 `PKG_VERSION` 对应的一段（遇到下一个 `## [` 标题即停），拼上产物清单（文件名 / 大小 / SHA256 前 12 位）与安装示例，通过 `--notes-file` 发布；同名 Release 已存在时也会同步更新说明。实测正文从 5381 字符降到 2053 字符，且全部为当前版本内容。
- 清理了历史 Release（v2.3.36、v2.3.37），避免发版列表堆积过期条目。

### Added
- 新增 `Auto Clean Releases` 工作流（`.github/workflows/auto-clean.yml`）：每天 03:17 UTC 定时执行，也可手动触发。**默认只保留最近 1 个 Release**（即仅保留最新版），其余连同 git tag 一并删除；手动触发支持 `keep`（保留数量，默认 1）、`dry_run`（仅预览不删除，手动默认开、定时默认关）、`cleanup_tag`（是否同时删除 git tag，默认开）三个参数，执行结果写入 Job Summary 表格。

## [2.3.38] - 2026-08-28

### Fixed
- 修复拨号健康检测死循环：每约 90 秒重复「modem online but interface not healthy → ifdown/ifup → no IP after recycle → rebuilding modem link → still has no IP」，无退避、无次数上限、永不收敛。
  - 根因：地址判定使用 `jsonfilter -e '@.ipv4-address[0].address'`。当前 jsonfilter 构建不支持该点号语法（报 `Syntax error: Invalid escape sequence`），导致地址查询恒为空，`interface_healthy()` 永远为假，于是每个监控周期都触发一次完整恢复并重建模组数据链路。改用方括号引用形式 `@['ipv4-address'][0]['address']`，并在 jsonfilter 取不到时回退到 `ip -o addr` 解析，避免再有 jsonfilter 变更被误读成"无 IP"。
- 分层健康判定：接口是否存在（netifd）→ 是否已获得 IPv4/IPv6 地址 → 双栈均无地址时才执行一次真实连通性测试（绑定出口设备的 ICMP + 使用模组下发 DNS 的域名解析）。
  - 连通性正常即判定健康：只记录一条「transient state anomaly」警告并重置重试计数，不再重启链路。
  - 只有连通性测试也失败，才判定异常并进入恢复流程。
- 恢复动作频率控制：连续重试上限（默认 3 轮）、指数退避（60s→120s→…，上限 900s）、单轮操作超时（默认 90s 时间预算，`modem_cycle_link` / `ifdown` 均带 `timeout` 上限）。达到上限后停止重建链路，输出明确的 ERROR 日志并等待人工介入（每 600s 提醒一次）；`mt5700m-manager redial` 或 `reset-recovery` 可解除停机。
- 每轮恢复输出可诊断信息：接口名、设备名、IPv4/IPv6 地址状态、carrier、连通性测试结果、当前重试次数、下一步动作（单行 `recovery[<stage>] ... next=<action>`）。
- `recover_interface6` 增加限频（默认 300s），避免 DHCPv6/RA 尚未完成时的每个监控周期重拉 v6 接口。
- `status-json` 增加 `health` / `ipv4_address` / `ipv6_address` / `recovery` / `recovery_halted` 字段便于排障。
- 监控轮询间隔分级：健康 15s、异常/退避 30s、已停机等待人工介入 60s（原先失败时缩到 5s，反而加速冲击模组）。

### Added
- `mt5700m.recovery.*` UCI 配置段（`max_retries` / `backoff_base` / `backoff_max` / `round_timeout` / `probe_timeout` / `ping_host` / `ping_host6` / `dns_host` / `v6_cooldown` / `halt_notify`），缺省即使用内置默认值。

## [2.3.37] - 2026-08-28

### Fixed
- 修复"接口配置存在（MT5700M/MT5700Mv6）但无 IP"场景。恢复判据从接口 `up` 改为显式地址检查（ubus status 的 `ipv4-address`/`ipv6-address`）：DHCP 未拿到租约时不再误判为健康。恢复路径分级：等待 DHCP（最长 20s）→ carrier 缺失时重建 NDIS 链路 → ifdown/ifup 硬重拉 → 仍无 IP 时重建模组数据链路兜底。
- 双栈配置下 `MT5700Mv6` 无 IPv6 地址时单独重拉 v6 接口（`recover_interface6`），不打断已健康的 v4 链路。

## [2.3.36] - 2026-08-28

### Fixed
- 修复"模组在线但 OpenWrt 无接口"场景。sync_manager 新增拨号自愈：拉起接口前先等待 NCM 网卡枚举（`mt5700m_wait_netdev`），避免 cdc_ncm 慢枚举导致误报"无网络接口"；检测模组服务注册状态（`modem_connected`）与 USB 模式（`modem_usb_mode`），非 NCM(4) 时自动下发 `AT^SETMODE=4` 并 10 分钟限频；接口持续未 up 时重建 NDIS 数据链路（`modem_cycle_link`：NDISDUP 断开重建，仍无 carrier 则 CFUN 重启模组兜底），最后以 ifdown/ifup 强制重拉。
- 修复 USB 数据接口被非 cdc_ncm 驱动抢占导致"AT 端口在、无 eth 设备"的问题。新增 `mt5700m_force_ncm_rebind`：解绑抢占 CDC 接口的驱动，并强制绑定 NCM 控制/数据接口。
- 默认配置新增 `dial_mode=1`（USB 数传拨号），保证首次开机即按拨号模式建立数据连接。

## [2.3.35] - 2026-08-27

### Fixed
- 修复总览页 IMSI 显示 `--` 的问题。`print_sim_details()` 解析 AT+CIMI 输出时使用严格锚定行尾的数字正则，而模组返回行尾携带 CR（`<IMSI>\r\n`）导致匹配失败。现改为先剥离 CR 再匹配；系统信息页提取 IMSI 的正则同步容忍 CRLF 行尾。

## [2.3.34] - 2026-08-27

### Fixed
- 修复总览页 APN 显示 `--` 的问题。当模块使用运营商默认 APN（CID 1 上下文 APN 为空）时，`active_apn` 字段缺失导致前端显示 `--`。现改为回退到 UCI 拨号配置中的 APN，仍为空时显示 `Carrier default`（运营商默认）。
- 修复连接页 PDP 上下文编辑框、详细会话行和 APN 事实卡片中 `--` 文字泄漏的问题。

## [2.3.33] - 2026-08-18

### Fixed
- 修复空 APN 时自动拨号拼接空字段导致模组返回 ERROR、dial_mode 静默保持旧值的问题。空 APN 时改用 `AT^SETAUTODIAL=1,1,"IPV4V6"`（省略空字段），USB 数传拨号模式可正常生效。

## [2.3.32] - 2026-08-17

### Fixed
- 修复 init.d/at-webserver 缺少执行位导致 procd 跳过、8765 端口无监听、实时面板全零的问题。
