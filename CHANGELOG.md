# Changelog

## [2.3.39] - 2026-08-28

### Changed
- 发布说明改为按版本自动生成，不再内联全量历史。原先 `release.yml` 里硬编码了一整段发布说明（v2.3.18 起的所有变更），导致每个 Release 页面都显示 5000+ 字符的过期日志。现在新增 `Build release notes` 步骤，用 awk 从 `CHANGELOG.md` 精确截取 `PKG_VERSION` 对应的一段（遇到下一个 `## [` 标题即停），拼上产物清单（文件名 / 大小 / SHA256 前 12 位）与安装示例，通过 `--notes-file` 发布；同名 Release 已存在时也会同步更新说明。实测正文从 5381 字符降到 2053 字符，且全部为当前版本内容。
- 清理了历史 Release（v2.3.36、v2.3.37），避免发版列表堆积过期条目。

### Added
- 新增 `Auto Clean Releases` 工作流（`.github/workflows/auto-clean.yml`）：每天 03:17 UTC 定时执行，也可手动触发。默认保留最近 5 个 Release，其余删除；手动触发支持 `keep`（保留数量）、`dry_run`（仅预览不删除，默认开）、`cleanup_tag`（是否同时删除对应 git tag）三个参数，执行结果写入 Job Summary 表格。

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
