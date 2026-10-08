# AT WebServer 软件包（Rust 后端 v4.0）

为 MT5700 提供 WebSocket AT 命令服务和 Web 界面。后端是单个静态 Rust 二进制，
std-only 零第三方依赖，不依赖 Python 运行时，也不用 cargo 索引/vendor。

## 单二进制双入口

同一份二进制按 argv[0] 分发，服务两个前端：

| 入口 | 行为 |
| --- | --- |
| `/usr/bin/at-webserver` | WebSocket AT daemon（WebUI 后端，:8765） |
| `/usr/sbin/mt5700m-at`（symlink） | LuCI shell 后端：子命令与输出契约和原 shell 版逐条对齐，LuCI 前端零改动 |

## 文件结构

```
/usr/bin/at-webserver          # 单二进制（daemon / cli 双模式）
/usr/sbin/mt5700m-at           # -> /usr/bin/at-webserver（LuCI 入口 symlink）
/etc/init.d/at-webserver       # 系统服务脚本
/etc/config/at-webserver       # UCI 配置文件
/www/5700/index.html           # Web 前端界面
/www/cgi-bin/at-ws-info        # 前端获取 WebSocket 地址的接口
```

源码在 `src/`（std-only，无第三方 crate），`cargo build --release --offline`
即可编译，无需联网。

## 连接模式

> v2.6 彻底重构：`UBUS` 模式与 `ubus-at-daemon` 已移除。LuCI 的 `mt5700m-at` 通过本地
> 控制套接字（`/var/run/at-webserver.sock`）复用 daemon 独占的串口，`sms-tool_q` 也已
> 移除（短信改为进程内纯 Rust PDU 编码）。

| connection_type | 说明 | URC 推送 | 定时锁频调度器 |
| --- | --- | --- | --- |
| `SERIAL`（默认） | daemon 独占 PCUI 串口（`TIOCEXCL`）；LuCI 与 WebUI 共用 | 原生（来电/新短信/存储满/信号变化） | 可用 |
| `NETWORK` | 直连模组网络 AT 端口（host:20249） | 原生 | 可用 |

串口由 daemon 独占打开。`serial_port=auto` 时开机自动扫描 `/dev/ttyUSB*` 按 VID/PID
（`3466:3301`）+ 接口类型（`ff:06:12`）识别 PCUI，必要时以 `AT` 应答探测兜底；也可手动
指定路径，或运行 `mt5700m-at port set <path|auto>` 一键选择。

## 配置

```bash
# 启用/禁用服务
uci set at-webserver.config.enabled='1'

# 连接类型 (SERIAL / NETWORK)
uci set at-webserver.config.connection_type='SERIAL'

# 串口模式：AT 命令走 PCUI 口
# serial_port='auto' 自动扫描；或手动指定（如 /dev/ttyUSB1）
uci set at-webserver.config.serial_port='auto'
uci set at-webserver.config.serial_timeout='10'

# 网络模式
uci set at-webserver.config.network_host='192.168.8.1'
uci set at-webserver.config.network_port='20249'

# WebSocket 端口与连接密钥（密钥留空表示不校验）
uci set at-webserver.config.websocket_port='8765'
uci set at-webserver.config.websocket_auth_key=''

# 查看/选择串口
mt5700m-at port scan
mt5700m-at port set /dev/ttyUSB1
mt5700m-at port auto

# 定时锁频调度器（SERIAL/NETWORK 均可用）
uci set at-webserver.config.schedule_enabled='0'
uci set at-webserver.config.schedule_check_interval='60'
uci set at-webserver.config.schedule_timeout='180'
uci set at-webserver.config.schedule_toggle_airplane='1'
uci set at-webserver.config.schedule_night_enabled='1'
uci set at-webserver.config.schedule_night_start='22:00'
uci set at-webserver.config.schedule_night_end='06:00'

# 夜间/日间各自 LTE/NR 锁频参数：
# schedule_{night,day}_{lte,nr}_{type,bands,arfcns,scs_types,pcis}
# type 0=解锁 1=频点 2=频点+PCI 3=仅频段

uci commit at-webserver
/etc/init.d/at-webserver restart
```

## 使用

```bash
/etc/init.d/at-webserver start
/etc/init.d/at-webserver enable   # 开机自启
```

Web 界面：`http://路由器IP/5700/`

检查状态：

```bash
/etc/init.d/at-webserver status
ps | grep at-webserver
netstat -lntp | grep 8765
```

WebSocket 协议与 Go/Python 版完全一致：文本帧承载 AT 命令，JSON 帧
（`{"auth_key": ...}` 认证、URC 推送、`AT+SCHED?`/`AT+SCHED=` 伪命令、
`AT^CELLSCAN` 异步扫频）语义不变。详见 [API.md](API.md)。

## 构建

由仓库根 `scripts/build-release.sh` 统一构建：

1. `cargo build --release --locked --target aarch64-unknown-linux-musl` 交叉编译 Rust 二进制
2. 编译产物（`at-webserver`）连同 `files/` 下的前端、init 脚本、UCI 配置一起折叠进
   `luci-app-mt5700m` 包源码目录
3. OpenWrt SDK 打包为单个 apk/ipk

不产生独立的 `at-webserver` 包。
