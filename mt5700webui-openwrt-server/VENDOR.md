# mt5700webui-openwrt-server（上游 3.0.2 前端归档 + Rust 后端重写）

本目录是上游 [inotdream/mt5700webui-openwrt-server](https://github.com/inotdream/mt5700webui-openwrt-server) v3.0.2 的源码归档（前端为主），AT 后端已由本仓库用 Rust 重写（v4.0），供本仓库集成与二次开发使用。

## 归档内容

| 路径 | 说明 |
| --- | --- |
| `at-webserver/src/` | **本地新增**：Rust 后端源码（v4.0 重写，std-only 零第三方依赖） |
| `at-webserver/Cargo.toml` | **本地新增**：Rust 工程（release profile：opt-level=z + lto + strip） |
| `at-webserver/Makefile` | **本地新增**：OpenWrt 构建集成（packages feed `lang/rust`） |
| `at-webserver/files/` | 上游打包文件 + 本地改造的 init.d（默认 UCI 配置、`www/5700` 前端构建产物） |
| `luci-app-at-webserver/` | 上游 LuCI 集成应用（config/debug/logs 页面） |
| `semi-tcpweb/` | 上游 React + Semi Design 前端源码 |
| `docs/` | 上游文档与截图 |
| `prebuilt/aarch64_cortex-a53/` | 上游 v3.0.2 预编译 apk（aarch64_cortex-a53，仅作历史参考） |

上游 Go 后端源码、Python 移植版（`at-webserver.py` + `files-py/`）已在 v2.4.0 移除：
Go 版在 ImmortalWrt 6.18 内核存在串口空闲读被误判 EOF 的兼容问题且缺 UBUS transport；
Python 版由 Rust 重写接替。重写背景与行为对齐说明见仓库根 `CHANGELOG.md`。

## 与上游的差异

1. 移除了上游 `.github/`（CI 工作流）与 `config.js.backup` 垃圾文件；v2.4.0 起进一步
   移除上游 Go 后端源码与 vendor 目录。
2. **v2.4.0：AT 后端 Rust 重写**（`at-webserver` 4.0，std-only 单二进制，argv[0] 分发）：
   以 `at-webserver` 运行是 WebSocket daemon（复刻 Python 版全部行为：命令收发/URC 分发/
   短信通知/定时锁频/全网扫频）；经 `/usr/sbin/mt5700m-at` symlink 调用则进入 LuCI shell
   后端模式（stdout/退出码契约与原 shell 逐条对齐）。零第三方 crate，firmware 友好。

## 连接模式：UBUS / SERIAL / NETWORK

后端支持三种 AT 通道，`connection_type` 在 `/etc/config/at-webserver` 中配置：

| 模式 | 通道 | 主动上报（URC） | 与 luci-app-mt5700m 共存 | 适用 |
| --- | --- | --- | --- | --- |
| `UBUS`（默认） | 经 `ubus call at-daemon sendat` 转发 | 不支持 | 支持（共享同一 AT 口，由 daemon 串行化） | **推荐**：WebUI 与管理页同时可用 |
| `SERIAL` | 直接打开 PCUI 串口（`/dev/ttyUSB1`） | 支持 | 不支持（需停用 `ubus-at-daemon`） | 需要来电/短信实时推送时 |
| `NETWORK` | TCP 连模组网络 AT 口（默认 `192.168.8.1:20249`） | 支持 | 支持 | 模组开启网络 AT 服务时 |

UBUS 模式要点：

- 由 `ubus-at-daemon` 独占串口并串行化所有 AT 请求，WebUI 后端与 `mt5700m-at` 均为其客户端，
  因此管理页与 WebUI 可同时使用，不会争抢 PCUI 口。
- AT 口自动探测依据 USB 接口类型 `bInterfaceClass:SubClass:Protocol = ff:06:12`（PCUI），
  失败时回退 `/dev/ttyUSB1`；也可用 `ubus_at_port` 显式指定。
- 代价：ubus 是请求/响应式，**没有主动上报通道**，`raw_data`/`incoming_call`/`new_sms`/
  `pdcp_data` 等推送与定时锁频的实时性依赖轮询，短信/来电通知在 UBUS 模式下不可用。
  需要这些能力时切 `SERIAL`（并停用 `ubus-at-daemon`）。

## 预编译包（aarch64_cortex-a53）

| 文件 | SHA256（前 12 位） |
| --- | --- |
| `at-webserver-3.0.2-r1_aarch64_cortex-a53.apk` | `040aff57bbaf` |
| `luci-i18n-at-webserver-zh-cn-26.243.16773.4bf3ae2_aarch64_cortex-a53.apk` | `041fb6451a50` |

安装（OpenWrt SNAPSHOT / ImmortalWrt，apk）：

```sh
apk add --allow-untrusted ./at-webserver-3.0.2-r1_aarch64_cortex-a53.apk
```

注意：`luci-i18n-at-webserver` 依赖 `luci-app-at-webserver`，单独装 i18n 包会因依赖不满足而失败。

## 实机部署记录（H5000M / ImmortalWrt 6.18.44 aarch64，2026-09-06）

- 上游 Go 版 apk 与本仓库 `luci-app-mt5700m` 存在文件所有权冲突
  （`/etc/init.d/at-webserver`、`/etc/config/at-webserver`、`/www/5700/*`），
  需 `apk add --allow-untrusted --force-overwrite` 安装。
- Go 版二进制存在 6.18 内核兼容问题（见上），实机改用 Python 版后端：
  - `/usr/bin/at-webserver.py` + `files-py/etc/init.d/at-webserver`
  - `uci set at-webserver.config.connection_type='SERIAL'`、`serial_port='auto'`
  - 需停用 `ubus-at-daemon`（Python 后端以 SERIAL 模式独占 AT 口，与其互斥）
- v2.3.41 起：前端与后端由 `scripts/build-release.sh` 折叠进 `luci-app-mt5700m` 包，
  单个安装包即包含前端 + 后端 + LuCI 管理页；旧 umi 前端与旧 Python 服务已从仓库移除。
- v2.4.0 起：AT 后端替换为 **Rust 重写版**（`at-webserver` 4.0，UBUS 默认 + SERIAL/NETWORK
  可选），构建时以 `aarch64-unknown-linux-musl` 目标交叉编译（rust-lld 自包含链接），
  前端与 Rust 二进制折叠进 `luci-app-mt5700m` 包；Python/Go 后端不再随包分发。
- v2.3.42 起：Python 后端新增 **UBUS 连接模式并设为默认**。
  背景：SERIAL 模式下后端独占 PCUI 串口，与 `luci-app-mt5700m` 的 `mt5700m-at`
  （走 `ubus call at-daemon`）互斥，导致 LuCI 管理页加载失败。
  改为 UBUS 后两者共享同一 AT 口，实测并发（管理页连发 `AT+CSQ` 的同时 WebUI 查询
  `AT+CGMR`/`AT^MONSC`/`AT+C5GREG?`/`AT+SCHED?`）全部正常应答。
- 服务自启：`luci-app-mt5700m/root/etc/uci-defaults/93-mt5700m-webui` 负责写入默认 UCI
  配置并 `enable` `at-webserver` 与 `ubus-at-daemon`。
  注意：OpenWrt 的 procd 只运行 `/etc/rc.d` 中存在链接的 init 脚本，缺少该步骤时
  服务安装后不会开机自启（v2.3.41 曾因此遗漏）。
- 实机验证（2026-09-06）：AT 通道 `/dev/ttyUSB1`（PCUI），
  `AT+CGMR`=V200R001C20B014、`AT^MONSC` NR 小区 RSRP -67/SINR 32、
  `AT+C5GREG?`、`AT+SCHED?`、ping/pong 心跳均正常；拨号 10.22.99.43（NCM/ECM）不受影响。
