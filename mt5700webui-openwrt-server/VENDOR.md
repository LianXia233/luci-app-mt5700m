# mt5700webui-openwrt-server（上游 3.0.2 前端归档 + Rust 后端重写）

本目录是上游 [inotdream/mt5700webui-openwrt-server](https://github.com/inotdream/mt5700webui-openwrt-server) v3.0.2 的源码归档（前端为主），AT 后端已由本仓库用 Rust 重写（v4.0）。构建时由 `scripts/build-release.sh` 以 `cargo build` 编译 Rust 二进制，连同前端一起折叠进 `luci-app-mt5700m` 包，**不产生独立的 `at-webserver` 包**。

## 归档内容

| 路径 | 说明 |
| --- | --- |
| `at-webserver/src/` | **本地新增**：Rust 后端源码（v4.0 重写，std-only 零第三方依赖） |
| `at-webserver/Cargo.toml` | **本地新增**：Rust 工程（release profile：opt-level=z + lto + strip） |
| `at-webserver/files/` | 上游打包文件 + 本地改造的 init.d（默认 UCI 配置、`www/5700` 前端构建产物） |
| `semi-tcpweb/` | 上游 React + Semi Design 前端源码 |
| `docs/` | 上游文档与截图 |

上游 Go 后端源码、Python 移植版（`at-webserver.py` + `files-py/`）已在 v2.4.0 移除：
Go 版在 ImmortalWrt 6.18 内核存在串口空闲读被误判 EOF 的兼容问题且缺 UBUS transport；
Python 版由 Rust 重写接替。重写背景与行为对齐说明见仓库根 `CHANGELOG.md`。

## 与上游的差异

1. 移除了上游 `.github/`（CI 工作流）与 `config.js.backup` 垃圾文件；v2.4.0 起进一步
   移除上游 Go 后端源码与 vendor 目录。
2. 移除了上游独立包定义（`at-webserver/Makefile`、`luci-app-at-webserver/`、`prebuilt/`）——
   本仓库不发布独立的 `at-webserver` 包，Rust 二进制由 `cargo build` 编译后折叠进
   `luci-app-mt5700m`。
3. **v2.4.0：AT 后端 Rust 重写**（`at-webserver` 4.0，std-only 单二进制，argv[0] 分发）：
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

## 构建流程

`scripts/build-release.sh` 负责编译与打包：

1. 以 `aarch64-unknown-linux-musl` 交叉编译 Rust 后端（`cargo build --release --locked`）
2. 将编译产物 `at-webserver` 二进制、`www/5700` 前端、`etc/init.d/at-webserver` init 脚本
   折叠进 `luci-app-mt5700m` 包源码目录
3. 由 OpenWrt SDK 打包为单个 apk/ipk，包含 LuCI 管理页 + WebUI 前端 + AT 后端

最终发布的包：`luci-app-mt5700m`（含前端+后端）、`ubus-at-daemon`、`sms-tool_q`。
