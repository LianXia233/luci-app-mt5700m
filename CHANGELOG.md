# Changelog

## [2.4.4] - 2026-09-06

### Fixed
- **修复 WebUI network/info 网络速率面板恒为 0 / "等待速率上报…无数据"**。根因：速率图依赖 `^PDCPDATAINFO:` URC 推送，而默认 UBUS 连接模式下 `ubus-at-daemon` 只有请求/响应通道（`open`/`sendat`/`list`/`close`），URC 永远到不了 WebUI daemon——这是上游 Python 版就写明文档的已知限制，速率面板从未在 UBUS 模式工作过。修复分两层：
  - **UBUS 模式轮询模拟**：daemon 拦截前端的采样开关命令 `AT^PDCPDATAINFO=1[,间隔ms]`（默认 750ms，钳位 250-5000ms）与 `AT^PDCPDATAINFO=0`，不起用模组推送（避免污染共享 PCUI 串口的应答流），改为按间隔轮询 `AT^PDCPDATAINFO?` 并以与 URC 流完全一致的 `{"type":"pdcp_data","data":{...}}` 事件广播；无 WS 客户端时暂停查询。其余命令不受影响，SERIAL/NETWORK 模式不经过此路径（走原生 URC 推送）。
  - **补回 dispatcher 缺失的 PDCP 解析**：Rust 移植时丢失了 Python `Dispatcher.handle_pdcp`，`^PDCPDATAINFO:` 在 URC 流（SERIAL/NETWORK 模式）中同样被丢弃。现按 Python `PDCP_FIELDS` 表解析 14 个统计字段（`avgDelay` 等 5 个字段按十分位换算），16 字段查询响应（末尾 2 个累计字节计数）只取前 14。实机验证：`AT^PDCPDATAINFO?` 经 ubus 返回 16 字段、66ms；测试 32/32 通过（新增 6 个：字段映射/短行拒绝/事件类型/开关解析钳位/UBUS 应答文本解析）。

## [2.4.3] - 2026-09-06

### Fixed
- **修复 `status` 在 NR 单载波实机上 panic（LuCI 载波面板不显示）**：`AT^HFREQINFO?` 解析的 `num` 闭包写成了 `field[i + k]`（k 为相对偏移），而调用方传的是绝对索引 `num(i + 5)` 等，索引被加了两次——模组实答 `^HFREQINFO: 0,7,41,513000,2565000,100000,513000,2565000,100000`（9 个字段）时访问 `field[9]` 越界，`thread 'main' panicked ... len is 9 but the index is 9`，整个 status 子命令中断，载波段及其后输出全部丢失。修正为绝对索引 `field[k]`；解析逻辑提取为纯函数 `append_hfreqinfo_line` 并新增 3 个用真实模组应答的回归测试（NR 单载波/非数字组跳过/LTE 单载波），26/26 通过。实机验证：`status` 输出 `carrier_1=NR|n41|513000|2565.00|100.0|513000|2565.00|100.0`，ca_mode=NR、CA 带宽 100MHz，不再 panic。

## [2.4.2] - 2026-09-06

### Fixed
- **修复 CLI 在 UBUS 模式设备上必然挂死的问题（实机 H5000M/MT5700M 发现）**：`mt5700m_pcui_port()` 将 `/dev/ttyUSB1` 全路径传给 `port_is_pcui()`，而后者拼出 `/sys/class/tty//dev/ttyUSB1/device` 这类非法 sysfs 路径（移植时丢失了 shell 版的 basename 处理），PCUI 探测恒为 None，`auto` 级联跳过 UBUS/串口直接落入 network 通道。现统一在 sysfs 入口做 basename 归一。实机验证：`command 'AT+CSQ'` 由无限挂死变为 0.08s 经 UBUS 返回。
- **修复 network 通道 TCP connect 无超时**：模组网络 AT 端点不可达时 `TcpStream::connect` 停留在 SYN_SENT 数分钟（内核默认超时），CLI/守护进程级联被冻死。改用 `TcpStream::connect_timeout`（按配置超时，逐候选地址尝试）。
- **修复 init.d 自杀缺陷**：脚本进程名即 `at-webserver`，`start_service`/`stop_service` 里的 `killall -q "$PROG_NAME"` 会把 init 脚本自己 TERM 掉（rc=143，procd 实例从未注册，服务无法启动/停止）。改为 `pidof` 枚举并排除自身 PID。实机取证：同名脚本 `killall` 必现自杀。
- 修复 LuCI shell 入口残留：v2.4.0 重写后 `root/usr/sbin/mt5700m-at` 仍以旧 shell 文件随包安装，uci-defaults 的 symlink 逻辑被"文件已存在"挡住，LuCI 实际仍调用旧 shell。该文件已删除，由 `93-mt5700m-webui` 创建 symlink 指向 Rust 单二进制。

## [2.4.1] - 2026-09-06

### Fixed
- CI 静态检查适配 Rust 后端（`4a40a91` 推送后暴露，`b2bcb66`/`383a822` 修复）：`sh -n` 与 `test -x` 的 WebUI init.d 路径 `files-py/etc/init.d` -> `files/etc/init.d`；"Check Python AT backend"（py_compile + imports）替换为 "Check Rust backend"（`cargo test --locked`）；`files/etc/init.d/at-webserver` 修正 git mode 为 100755（该脚本由上游以 0644 归档，procd 只运行 +x 的 init 脚本，早期 2.3.31 曾因同一问题导致 WebUI 面板全零）。
- Release 构建修复 aarch64 Rust 链接（run 34013382900 失败暴露，`ad88ca9` 修复）：rustc 对 `aarch64-unknown-linux-musl` 默认调宿主 `cc`，目标特有的 `-Wl,--fix-cortex-a53-843419` 被 x86_64 模式 GNU ld 拒绝（`unrecognized option`）；`build-release.sh` 现显式 `RUSTFLAGS="-C link-self-contained=yes -C linker=rust-lld"`，用捆绑 rust-lld 完成自包含静态链接（本机已验证产出 647KB AArch64 ELF，machine 0xb7）。
- 文档同步：`mt5700webui-openwrt-server/VENDOR.md` 更新为 Rust 4.0 归档说明（上游 Go 源码、Python 移植版移除，归档表与实机部署记录重写）。

## [2.4.0] - 2026-09-06

### Added
- **AT 后端 Rust 重写（at-webserver 4.0，双前端单二进制）**。`mt5700webui-openwrt-server/at-webserver/src/` 新增 std-only 零第三方依赖的 Rust 实现，按 argv[0] 分发双入口：以 `at-webserver` 运行为 WebSocket AT daemon（WebUI 后端，协议与 Go/Python 版一致：auth_key 认证、文本帧 ping/pong、`AT+CONNECT?`/`AT+SCHED?`/`AT+SCHED=` 伪命令、`AT^CELLSCAN` 异步扫频、URC 推送），经 `/usr/sbin/mt5700m-at` symlink 调用进入 LuCI shell 后端模式（`mt5700m-at` 全部子命令 stdout 契约逐条对齐，LuCI 前端零改动）。
- AT 通道三模式级联与原 shell `at_cmd()` 逐条对齐：UBUS（`ubus call at-daemon sendat`，payload 键 `at_port`/`at_cmd`/`timeout`）优先，at-daemon 缺失（rc 127）回退直连串口，auto 模式串口失败落网络端口（网关探测 + host 候选列表）；anchored ERROR 终止符判定与 `at_response_ok` 一致。
- `usb.sh` 移植：`mt5700m_usb_info`（3466:3301/3302/3303 状态识别）与 PCUI 口探测（USB 接口类 `ff:06:12` 或接口描述匹配），status 输出契约不变。
- daemon 侧新增 URC 分发器（移植 urc.go/Python Dispatcher）：来电（RING/+CLIP 去重 30s/^CEND）、新短信（+CMTI）、存储满（^SMMEMFULL）、信号变化（^HCSQ，阈值 1dB）与 passthrough（^REJINFO/带逗号 +CUSD）。
- daemon 侧新增昼夜定时锁频调度器（移植 Python Scheduler 控制环）：时段判定（支持跨午夜窗口）、LTE/NR 锁频命令构建（type 0/1/2/3，频点数校验不一致回退解锁、FR2 频段 SCS 自动 120kHz）、飞行模式切换包络、无服务超时强制解锁恢复；UBUS 模式同样可用。
- 硬约束：IMEI 路径（`set-imei` -> `AT^PHYNUM=IMEI`）按原 shell 逻辑原样移植，无任何行为改动。

### Removed
- 删除 Go 版后端 `mt5700webui-openwrt-server/at-webserver/src/*.go`（含 vendor 与预编译 apk）：审计确认其未接入主包部署路径（`build-release.sh` 只折入 Python 版），且缺少 UBUS transport、在 ImmortalWrt 6.18 内核存在串口空闲读 EOF 误判问题。
- 删除 Python 后端 `at-webserver.py` 与 `files-py/`：职责由 Rust daemon 全面接替。

### Changed
- `scripts/build-release.sh`：改为在 runner 上以 `aarch64-unknown-linux-musl` 目标交叉编译 Rust 后端（std-only，rust-lld 自包含链接，无需交叉工具链），产物与 init.d 折叠进 `luci-app-mt5700m` 包。
- `luci-app-mt5700m/Makefile`：`LUCI_DEPENDS` 移除 `+python3 +python3-websockets +python3-pyserial`；版本 2.3.44 -> 2.4.0。
- `93-mt5700m-webui`：后端检查改为 `/usr/bin/at-webserver`，新增 `/usr/sbin/mt5700m-at` symlink 建立与 3.x Python 残留清理。
- `at-webserver/Makefile`：Go 打包改为 Rust 打包（feed 集成走 packages feed `lang/rust`）。

### Fixed
- 彻底消除三套后端（shell/Python/Go）间锁频命令构建、AT 解析行为的分叉：LuCI 与 WebUI 现共享同一份 AT 实现，`AT^SYSCFGEX` 参数补引号逻辑（normalizeSyscfgex）统一收口。

## [2.3.44] - 2026-09-06

### Fixed
- **修复 WebUI（`/5700/`）所有按钮渲染为实心红色块、文字不可见的问题。** 根因：LuCI 构建期的 `csstidy --template=highest`（`LUCI_MINIFY_CSS` 默认开启）会把 Semi Design 主题中依赖出现顺序的重复规则合并去重——`.semi-button-light` 的浅色背景规则（`background-color: var(--semi-color-fill-0)`，在 `.semi-button-primary` 之后重复声明以恢复浅色底）被合并到前面，导致同特异性的 `.semi-button-primary { background-color: var(--semi-color-primary) }` 按级联顺序获胜；按钮图标/文字为 `currentColor`（主色红），最终背景色 = 文字色 = `#c7000b`，整颗按钮变成不可辨识的红色色块。与 2.3.41 时 jsmin 破坏 JS 同类，属构建期二次压缩破坏已压缩产物。修复：Makefile 增加 `LUCI_MINIFY_CSS:=0`，vendor CSS 原样随包分发（实测定测：修复后按钮 `background-color` 为浅灰 `rgba(100,116,139,.07)`、文字为深色，正常可读）。

### Changed
- `luci-app-mt5700m/Makefile` 版本 2.3.43 -> 2.3.44。

## [2.3.43] - 2026-09-06

### Fixed
- **修复全新安装（卸载后重装）后 `at-webserver` 配置缺失、服务无法自启/启动的问题。** 根因有两处，均在 `luci-app-mt5700m/root/etc/uci-defaults/93-mt5700m-webui`：
  1. 守卫 `[ -x /usr/bin/at-webserver.py ]` 依赖可执行位，但 OpenWrt 构建系统把 `root/usr/bin/at-webserver.py` 装成 `0644`（源文件虽为 `0755`），导致 `-x` 为假、脚本直接 `exit 0` 跳过全部初始化逻辑。改为 `[ -f ... ]`（仅检查存在性）并在脚本内 `chmod 0755` 恢复可执行位。
  2. `uci batch` / `uci set <cfg>.<sec>=<type>` 在本机 ImmortalWrt 的 uci 上，若 `/etc/config/at-webserver` 尚不存在会报 `Entry not found` 而静默失败。在 batch 前增加 `touch /etc/config/at-webserver` 确保配置文件存在。
- 此前 2.3.41/2.3.42 实机能用，是因为开发期手动 `uci set` 建过配置，并非 uci-defaults 生效；干净重装路径此前从未被验证。本版本起该路径已闭环。

### Changed
- `luci-app-mt5700m/Makefile` 版本 2.3.42 -> 2.3.43。

## [2.3.42] - 2026-09-06

### Added
- Python 后端 `at-webserver.py` 新增 **UBUS 连接模式**（经 `ubus call at-daemon sendat` 转发）并设为默认：AT 口由 `ubus-at-daemon` 独占并串行化，WebUI 后端与 `mt5700m-at` 作为客户端共享同一通道，LuCI 管理页与 WebUI 可同时使用。AT 口按 USB 接口类型 `ff:06:12`（PCUI）自动探测，回退 `/dev/ttyUSB1`，可用 `ubus_at_port` 指定。
- 新增 `luci-app-mt5700m/root/etc/uci-defaults/93-mt5700m-webui`：首次安装时写入默认 UCI 配置（`connection_type=UBUS` 等）并 `enable` `at-webserver` 与 `ubus-at-daemon`。OpenWrt 的 procd 只运行 `/etc/rc.d` 中有链接的 init 脚本，缺少此步骤会导致服务不开机自启。

### Fixed
- 修复 LuCI 管理页（`/cgi-bin/luci/admin/modem/mt5700m/status`）加载失败：v2.3.41 部署时 WebUI 后端以 SERIAL 模式独占 PCUI 串口并停用 `ubus-at-daemon`，导致管理页依赖的 `ubus call at-daemon` 通道不存在。改由 UBUS 模式共享 AT 口后恢复。
- 修复 `at-webserver` 未开机自启（`/etc/rc.d` 无链接）：旧栈的 `93-mt5700m-webui` 被移除后没有替代的启用步骤。

### Changed
- `luci-app-mt5700m/Makefile` 版本 2.3.41 -> 2.3.42。
- `.github/workflows/ci.yml` 恢复对 `93-mt5700m-webui` 的语法与可执行位检查。
- 文档：`VENDOR.md` 增加三种连接模式对比与 UBUS 限制说明，README 同步。

## [2.3.41] - 2026-09-06

### Removed
- 移除旧版 WebUI 全套：umi 旧前端（`htdocs/5700/`，88 个静态文件）、旧 Python 后端 `root/usr/bin/at-server.py`、旧 `root/etc/init.d/at-webserver`（Python/pyserial 版，服务 `at-server.py`）、`root/etc/uci-defaults/93-mt5700m-webui`、`root/etc/config/at-webserver`、`htdocs/cgi-bin/at-ws-info`、`htdocs/cgi-bin/at-log-clear` 及配套 `htdocs/scripts`。WebUI 由 mt5700webui 3.0.2 前端 + Python 移植版后端（`mt5700webui-openwrt-server/`）全面接替。

### Changed
- `scripts/build-release.sh`：构建时将 mt5700webui 3.0.2 前端（`www/5700`）与 Python 后端（`at-webserver.py` + `files-py/etc/init.d/at-webserver`）折叠进 `luci-app-mt5700m` 包源码，单个安装包包含前端 + 后端 + LuCI 管理页；SDK 截断防护（pristine 重拷贝 + node --check）保留，数据源换为新前端。
- `luci-app-mt5700m/Makefile`：版本升至 2.3.41；JSMin 禁用注释更新为新前端背景。
- `.github/workflows/ci.yml`：移除已删文件（旧 init.d、cgi-bin、93-mt5700m-webui、at-server.py）的检查，新增 mt5700webui 前端 bundle node --check、后端 py_compile 与关键 import 校验。
- `mt5700webui-openwrt-server/at-webserver/files-py/etc/init.d/at-webserver` git mode 修正为 100755（init.d 必须可执行，procd 只跑 +x 脚本）。

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
