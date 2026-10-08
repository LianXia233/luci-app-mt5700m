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

## 连接模式：SERIAL / NETWORK（v2.6 起）

> v2.6 彻底重构：`UBUS` 模式与 `ubus-at-daemon` 已移除，串口改由 Rust 后端独占
> （`TIOCEXCL`）。LuCI 的 `mt5700m-at` 通过本地控制套接字（`/var/run/at-webserver.sock`）
> 复用该独占串口，与 WebUI 共用，不再争抢 PCUI 口；`sms-tool_q` 也已移除，短信改为进程内
> 纯 Rust PDU 编码。

后端支持两种 AT 通道，`connection_type` 在 `/etc/config/at-webserver` 中配置：

| 模式 | 通道 | 主动上报（URC） | 与 luci-app-mt5700m 共存 |
| --- | --- | --- | --- |
| `SERIAL`（默认） | daemon 独占打开 PCUI 串口（`serial_port=auto` 自动扫描或手动指定） | 支持 | 支持（管理页经控制套接字，WebUI 经 WebSocket，共用同一串口） |
| `NETWORK` | TCP 连模组网络 AT 口（默认 `192.168.8.1:20249`） | 支持 | 支持 |

SERIAL 模式要点：

- 仅 Rust 后端 `at-webserver` 持有该 TTY 文件描述符，并请求内核 `TIOCEXCL` 排斥其它打开者；
  `mt5700m-at`（LuCI）经由 `/var/run/at-webserver.sock` 控制套接字向它发指令，天然串行化。
- 串口自动探测依据 USB 接口类型 `bInterfaceClass:SubClass:Protocol = ff:06:12`（PCUI）与
  VID/PID（`3466:3301`），必要时以 `AT` 应答探测兜底；`serial_port` 填具体路径（或运行
  `mt5700m-at port set <path>`）则手动选择。
- URC（`raw_data`/`incoming_call`/`new_sms`/`pdcp_data` 等）在 SERIAL 下原生实时推送，
  无需轮询模拟。短信读取/写入均走同一个独占通道。

## 构建流程

`scripts/build-release.sh` 负责编译与打包：

1. 以 `aarch64-unknown-linux-musl` 交叉编译 Rust 后端（`cargo build --release --locked`）
2. 将编译产物 `at-webserver` 二进制、`www/5700` 前端、`etc/init.d/at-webserver` init 脚本
   折叠进 `luci-app-mt5700m` 包源码目录
3. 由 OpenWrt SDK 打包为单个 apk/ipk，包含 LuCI 管理页 + WebUI 前端 + AT 后端

最终发布的包：`luci-app-mt5700m`（含前端+后端）。不再发布 `ubus-at-daemon` 与 `sms-tool_q`。
