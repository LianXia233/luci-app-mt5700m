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

| connection_type | 说明 | URC 推送 | 定时锁频调度器 |
| --- | --- | --- | --- |
| `UBUS`（默认） | 经 `ubus call at-daemon sendat` 转发，与 luci-app-mt5700m 共享 AT 口 | 不可用（请求/响应式） | 可用 |
| `SERIAL` | 直连 PCUI 串口（独占） | 来电/新短信/存储满/信号变化 | 可用 |
| `NETWORK` | 直连模组网络 AT 端口（host:20249） | 同 SERIAL | 可用 |

## 配置

```bash
# 启用/禁用服务
uci set at-webserver.config.enabled='1'

# 连接类型 (UBUS / NETWORK / SERIAL)
uci set at-webserver.config.connection_type='UBUS'

# UBUS 模式：AT 口留空则按 PCUI（ff:06:12）自动探测，回退 /dev/ttyUSB1
uci set at-webserver.config.ubus_at_port=''
uci set at-webserver.config.ubus_timeout='10'

# 网络模式
uci set at-webserver.config.network_host='192.168.8.1'
uci set at-webserver.config.network_port='20249'

# 串口模式：AT 命令要走 PCUI 口，MT5700M-CN 上是 ttyUSB1
# 端口映射 ttyUSB0=Application Interface / ttyUSB1=PCUI / ttyUSB2=SerialB
#          ttyUSB3=SerialC / ttyUSB4=GPS
uci set at-webserver.config.serial_port='/dev/ttyUSB1'

# WebSocket 端口与连接密钥（密钥留空表示不校验）
uci set at-webserver.config.websocket_port='8765'
uci set at-webserver.config.websocket_auth_key=''

# 定时锁频调度器（SERIAL/NETWORK/UBUS 均可用）
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
