# mt5700webui-openwrt-server（上游 3.0.2 源码 + Python 后端移植）

本目录是上游 [inotdream/mt5700webui-openwrt-server](https://github.com/inotdream/mt5700webui-openwrt-server) v3.0.2 的完整源码归档，供本仓库集成与二次开发使用。

## 归档内容

| 路径 | 说明 |
| --- | --- |
| `at-webserver/src/` | 上游 Go 后端源码（原样保留） |
| `at-webserver/at-webserver.py` | **本地新增**：Go 后端的 Python 移植版（单文件，pyserial + websockets） |
| `at-webserver/files-py/etc/init.d/at-webserver` | **本地新增**：Python 版 procd init 脚本 |
| `at-webserver/files/` | 上游打包文件（默认 UCI 配置、init.d、`www/5700` 前端构建产物） |
| `luci-app-at-webserver/` | 上游 LuCI 集成应用（config/debug/logs 页面） |
| `semi-tcpweb/` | 上游 React + Semi Design 前端源码 |
| `docs/` | 上游文档与截图 |
| `prebuilt/aarch64_cortex-a53/` | 上游 v3.0.2 预编译 apk（aarch64_cortex-a53，H5000M 用） |

## 与上游的差异

1. 移除了上游 `.github/`（CI 工作流）与 `config.js.backup` 垃圾文件，其余源码原样保留。
2. 新增 `at-webserver.py`：在不改变 WebSocket 协议的前提下，用 Python 复刻 Go 后端全部行为
   （命令收发/URC 分发/短信 PDU 解码/定时锁频/全网扫频/通知推送）。
   - 背景：Go 版二进制在 ImmortalWrt 6.18 内核（n_tty 重构）上串口空闲读返回 0 字节，
     被误判为连接中断进入重连死循环；pyserial 的串口姿势在同一内核上经实机验证可靠。
   - 依赖：`python3-pyserial`、`python3-websockets`（OpenWrt/ImmortalWrt 官方源可装）。

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
- 实机验证：AT 通道自动探测选中 ttyUSB1（PCUI），`AT+CGMR`/`AT^MONSC`/`AT+C5GREG?` 等均正常应答。
