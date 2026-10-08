<div align="center">

# MT5700M OpenWrt 管理器

**面向移远 Quectel MT5700M-CN 5G 模组的高性能 OpenWrt LuCI 管理器与 Web 控制中心**

[![Version](https://img.shields.io/badge/Version-v3.2.0-blue.svg?style=flat-square)](https://github.com/LianXia233/luci-app-mt5700m/releases)
[![OpenWrt](https://img.shields.io/badge/OpenWrt-Filogic%20%7C%20ImmortalWrt-00C49F.svg?style=flat-square&logo=openwrt)](https://openwrt.org/)
[![Backend](https://img.shields.io/badge/Backend-Rust%20(std--only)-DEA584.svg?style=flat-square&logo=rust)](mt5700webui-openwrt-server/at-webserver/)
[![WebUI](https://img.shields.io/badge/WebUI-React%20%2B%20Semi%20Design-61DAFB.svg?style=flat-square&logo=react)](mt5700webui-openwrt-server/semi-tcpweb/)
[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg?style=flat-square)](luci-app-mt5700m/Makefile)

<p align="center">
  <b>单包交付</b>：LuCI 原生视图 + WebUI 独立前端 + Rust 双入口后端一键集成<br>
  不依赖外部云端 · 不上传 SIM 隐私 · 零第三方 Rust 依赖 · 原生内核流量统计
</p>

</div>

---

| 属性维度 | 说明与工程规范 |
|:--|:--|
| **软件包名** | `luci-app-mt5700m`（单包内嵌 LuCI 前端、独立 WebUI 与 Rust 后端） |
| **适配硬件** | 移远 Quectel MT5700M-CN 5G 模组 |
| **数据面拨号** | NCM 协议（基于 `kmod-usb-net-cdc-ncm`） |
| **控制面通讯** | Rust 后端独占 AT 串口（`TIOCEXCL`），LuCI 走本地控制套接字、WebUI 走 WebSocket |
| **访问入口** | LuCI：`调制解调器` → `MT5700M 管理` · WebUI：`http://<路由器IP>/5700/` |

---

## 架构 v2：单后端双前端（当前）

本版把「前端直连 AT」与「解析逻辑双份维护」两个历史包袱清零，确立**单一
AT 持有者**：Rust 后端 `at-webserver` 是唯一能下发 AT 命令的组件，LuCI 与
WebUI 的全部读写都走后端统一路由（路由清单见
[`docs/architecture-v2/api-contract.md`](docs/architecture-v2/api-contract.md)）。

### 两条机制性保证

| 验收标准 | 保证机制 |
|:--|:--|
| **前端不得访问 AT** | CI `frontend-proofs` job：10 个 `prove-*-parity.js` 在固定基线上逐字比对 UI 渲染产物 + `smoke-minified-luci.js` 确认压缩产物零 CLI 调用 |
| **无重复实现** | 死代码清零：LuCI shared 模块 9 个死导出（parser 29→24 / api 15→12 / components 45→44）、WebUI `at.ts` 8 个死方法；LuCI 模块按 `'require baseclass'` 协议盘点 |

UI 渲染不可变：任何影响渲染产物的改动会被 prove 逐字比对立刻报警——前端
重构（路由化、死代码清理）在 CI 层面被证明「用户看到的界面一个字节都没变」。

> 终端页是唯一保留原始 AT 通道的前端入口（有意保留，供调试）。

### 读写分离与 URC 解耦

写操作走 verbs 路由，后端做参数白名单校验（如 `traffic.pdcp_report_set` 的
interval 200–65535 ms）；URC 主动上报流（如 `^PDCPDATAINFO:`）由后端
`transport/urc.rs` 统一解析后经 `TOPIC_TRAFFIC` 推送——**前端开关只管 modem
侧使能，不碰数据流**。

详细设计见 [`docs/architecture-v2/`](docs/architecture-v2/)
（architecture / api-contract / data-flow / module-guide / migration /
remaining-work）。

---

## v3.1.0：异步化架构 —— 前后端彻底解耦

本版解决一个长期存在的体验问题：**LuCI 与 WebUI 同时使用时仍会互相拖慢**。

### 问题根因

「两端读同一份缓存」只保证数据一致，**不保证互不干扰**。所有 AT 指令进同一条
`AtArbiter` 队列，而 MT5700 的慢命令单条就要占 8~12 s（实测 `AT^NTXPOWER?` 11.6 s，
`AT^LENDC?` 不支持也占 8 s）。此前**渲染与轮询路径上的读命令仍会直压 AT 通道**，
两端同时刷新就在同一条队列里互相排队。

### 解决方案：后端命令闸门（机制性保证）

在**后端命令入口**加闸门，让读命令物理上到不了 AT 通道：

```
前端读命令 ──> 闸门 ──┬── 命中新鲜缓存 ──> 立即返回（零 AT、零阻塞）
                       │
                       ├── 命中过期缓存 ──> 立即返回旧值 + 后台刷新（SWR）
                       │
                       ├── 从未采集 ──────> 提交单飞后台采集 + 立刻返回「采集中」
                       │
                       └── 写命令 ────────> 直通 AT（行为与从前一致）
```

为什么选「后端闸门」而不是「前端逐处改读缓存」：后者是**约定**——160+ 处
`sendCommand` 漏一处即回归 AT，且新增读点会重新漏。机制性保证不依赖调用方自觉。

### 实测效果

| 指标 | 改造前 | 改造后 | 改善 |
|:--|--:|--:|--:|
| 单次读命令 `AT^DHCP?` | 0.32 s | 0.07 s | 4.6x |
| 10 次串行 | 2.03 s | 0.15 s | 13.5x |
| **边际斜率** | ~200 ms/次 | **8.9 ms/次** | **22.5x** |
| `mt5700m-at status` | 0.296 s | 0.08 s | 3.7x |
| 并发 4 worker × 6 轮（三路径混合） | — | 24 ms/次 | PASS |

**AT 流量与前端刷新率完全无关**：没人访问的二级页面，命令一次都不下发；
轰炸结束后 `raw:` age_ms 仍持续单调增长，证明零重采集。

### 三条后端路径均已覆盖

| 路径 | 入口 |
|:--|:--|
| JSON-RPC `at` | `daemon.rs: run_command`（WebUI WS 路径） |
| control socket `send` | `daemon.rs: handle_control_request`（**LuCI 全部读命令路径**） |
| CLI fork 子进程 | `cli_capture` → `cli.rs: run_at`（复用 control socket） |

> 第二处最容易漏：只接 `run_command` 时闸门等于没装，实测表现为
> `mt5700m-at cached` 里始终没有 `raw:` 主题。

完整设计见 [`docs/async-architecture.md`](docs/async-architecture.md)。

---

## 系统拓扑

双前端共用统一后端通道。Rust 后端 `at-webserver` 独占 AT 串口
（`TIOCEXCL` + 常驻描述符），彻底避免传统工具在 Web 与后台同时调用时的 TTY 串口锁死。

> **v3.0.0 起**：AT 串口由 Rust 后端直接用 termios ioctl 配置
> （`TCGETS`/`TCSETS`/`TCFLSH`），**不再依赖 BusyBox `stty` applet**
> （OpenWrt 镜像普遍不内置），并统一 `O_NONBLOCK` + `VMIN=1`/`VTIME=0`。
> 后端另提供 `atprobe` 诊断子命令，绕开守护进程独占锁直探 AT 口。

---

## 目录结构

```
luci-app-mt5700m/            OpenWrt 软件包（LuCI 前端 + ucode + init 脚本）
  htdocs/luci-static/resources/
    mt5700m/                 api.js / parser.js / components.js / style.css
    view/mt5700m/            各标签页视图（status / connection / network / sms /
                             system / advanced / terminal / settings）
mt5700webui-openwrt-server/
  at-webserver/src/          Rust 后端（std-only，零第三方依赖）
    daemon.rs                组合根：JSON-RPC / control socket / WS 接入
    scheduler/gate.rs        ★ 读命令缓存闸门
    scheduler/arbiter.rs     AT 仲裁器（优先级 + 去重 + 占位预算）
    scheduler/jobs.rs        任务生命周期与周期调度
    state/cache.rs           StateCache（SWR/TTL）
    state/bus.rs             EventBus（主题订阅 / 合并推送）
    core/                    错误模型 / JSON / 运行时 / 任务原语 / 频段表
    modules/<name>/          11 个领域模块（commands/parser/state/service/api）
    serial/ · transport/ · api/   串口独占 / 传输层 / 路由 registry
  semi-tcpweb/               WebUI 前端（React + Semi Design）
docs/async-architecture.md   异步化架构设计
docs/architecture-v2/        架构 v2 设计（api-contract / data-flow / migration）
scripts/                     prove-*-parity.js（前端等价证明）+ CI 静态检查
```

---

## 安装

```sh
# OpenWrt / ImmortalWrt
opkg update && opkg install luci-app-mt5700m
# 或 ImmortalWrt
apk add luci-app-mt5700m
```

安装后访问：

- LuCI：`调制解调器` → `MT5700M 管理`
- WebUI：`http://<路由器IP>/5700/`

---

## 故障排查

### AT 命令全部超时 / LuCI 与 WebUI 同时无数据

v3.0.0 前有**三层叠加根因**，缺任一层都会导致 AT 全哑：

1. **BusyBox 缺 `stty` applet**。旧实现完全依赖 `stty -F /dev/ttyUSB1 ... raw` 且忽略
   退出码，TTY 停留在内核默认 cooked 模式，回显与行缓冲让响应解析失真。
2. **`VMIN=0` 的 EOF 陷阱**。Linux tty 在 `VMIN=0`/`VTIME=0` 下空闲时 `read`
   **返回 0 字节**，被读循环当成 EOF，读取线程刚连上就退出。
3. **非阻塞下的 `EAGAIN` 被当成致命错误**。三处读取循环曾均为 `Err(_) => break`，
   而 `EAGAIN` 是正常空闲态。

```sh
# 1) 直探 AT 口（绕开 daemon / 仲裁器 / 采集器），期望 5/5 全部响应
at-webserver probe

# 2) 回读实际 termios，确认 cooked 位已清除、CBAUD=13(B115200)
AT_DEBUG_TERMIOS=1 at-webserver probe

# 3) 确认通道性能（0–1s 返回 26 行即为正常）
time mt5700m-at status

# 4) 确认并发不互相饿死（5 路并发应全部 RC=0）
for i in 1 2 3 4 5; do (timeout 30 mt5700m-at status >/dev/null 2>&1; echo "$i:$?") & done; wait
```

### `connected` 恒为 0 导致 LuCI 标签页全部判定为未连接

`connected` 是 **LuCI 的全页面总闸门**，一旦为 0，概览 / 移动数据 / 无线与小区 /
短信 / 模组与 SIM 卡 会同时显示未连接，**即使模组已正常注册**。

它的语义是「AT 通道可用」，**不是**「network 模式的 TCP 连接已建立」。
`AT+CONNECT?` 只存在于 network 通道，串口独占下恒返回 `ERROR`。

```sh
mt5700m-at status | grep -E '^connected=|^channel=|^at_port='
# 期望: connected=1 channel=serial at_port=/dev/ttyUSB2
```

### 页面某个卡片显示「未分配」但地址其实存在

移动数据页与概览页的 IP 显示依赖 `AT^DHCP?` / `AT^DHCPV6?` / `AT+CGPADDR=1`。
若某个模组或运营商对 `AT^NDISSTATQRY?` 不应答，v3.1.0 起会自动回退到
「是否真拿到地址」判定。

```sh
# 逐条实测，看哪条返回空
for c in 'AT^NDISSTATQRY?' 'AT^DHCP?' 'AT^DHCPV6?' 'AT+CGPADDR=1'; do
  printf '%-20s ' "$c"; mt5700m-at command "$c" | head -1
done
```

### 确认闸门是否生效

```sh
# 读一次命令后，raw topic 应出现
mt5700m-at command 'AT^DHCP?' >/dev/null; sleep 3
mt5700m-at cached | grep -o '"raw:AT\^DHCP?"'

# 紧急回滚：关掉闸门回到旧行为（排障用，重启后失效）
MT5700M_READ_GATE=0 mt5700m-at command 'AT^DHCP?'
```

---

## 开发

```sh
# 后端测试（292 个用例）
cd mt5700webui-openwrt-server/at-webserver && cargo test

# WebUI 构建
cd mt5700webui-openwrt-server/semi-tcpweb && npm install && npm run build

# 前端等价证明（UI 渲染逐字比对；CI 每次 push 自动跑全部 10 项）
node scripts/prove-connection-parity.js
```

**交叉编译**（本机 Windows 无 aarch64 链接器时借云端）：

```sh
RUSTFLAGS="-C linker=<rust-lld> -C linker-flavor=ld.lld" \
  cargo build --release --target aarch64-unknown-linux-musl
```

本项目 std-only、无 C 依赖，直接用 rustup 自带的 `rust-lld` 即可，
无需安装 `gcc-aarch64-linux-gnu`，也不污染系统环境。

---

## 安全声明

- 本项目**不**上传任何 SIM 隐私或遥测数据，全部在本机处理。
- v3.1.0 的异步化改造与全部验证脚本**未下发任何 IMEI 命令**。
  ⚠️ WebUI `pages/system/Info.tsx` 仍保留 IMEI 写入入口（连点 5 次隐藏入口），
  这是**遗留红线隐患**，待移除。

## License

Apache-2.0
