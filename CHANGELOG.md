# Changelog

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
