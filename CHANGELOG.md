# Changelog



---

## [3.2.0] - 2026-10-08

### Changed

- **架构 v2 收官（PR #7 + PR #8）：单 Rust 后端、双 AT-free 前端**：
  - Background：架构 v2 的目标是把「前端直连 AT」与「解析逻辑双份维护」两个
    历史包袱清零，两条验收标准：**前端不得访问 AT**（终端页有意保留）与
    **无重复实现**。PR #7 完成主体路由化，PR #8 为收官批：死代码清零 +
    WebUI 路由化收尾 + CI 证明链。
  - Changes（PR #7，全页面读写路由化）：
    - 系统页：22 段读帧切成 15 条路由（页面零 CLI）；LED / 网络时间 / 温控 /
      SIM 激活（`^HVSST` 动词收敛）写入进后端；补 Version / FOTA 模式两组读能力。
    - 连接页：7 条写路由 + Direct IP 读路由，随后整页读写双切路由化。
    - 高级设置页：读写双切路由化。
    - 全部路由在 `docs/architecture-v2/api-contract.md` 登记，migration /
      remaining-work 同步登记。
  - Changes（PR #8，收官批）：
    - LuCI 死导出清零：旧盘点基于 CommonJS 正则失配（shared 模块实为
      `'require baseclass'` + `return baseclass.extend({...})` 协议），按协议
      重盘后删除 9 个——`parser.js` 29→24、`api.js` 15→12、`components.js` 45→44。
    - `client.rs` 文档漂移修正 + 3 处死代码清理：at_cmd doc「三级级联」→
      单通道转发；删 `NetworkFailed` 零构造变体（`core/error.rs` 同步收口）、
      `libc_eagain`、`network_hosts`。
    - WebUI：`setPDCPDataReport` 签名不变改走 `traffic.pdcp_report_set` 路由
      （后端构造器白名单 interval 200–65535 ms，URC 数据流与开关解耦）；删
      8 处死方法（`getConnectionState`×3 / `isAuthRequired`×3 / `readCommand`
      连带 `pendingReads` / `getIMEI`）；bundle 重建同步（`index-BRUXWNeG.js`，
      legacy 跳转页重建）。
    - 验证：cargo test 291→**292**；`tsc --noEmit` + `vite build` 通过；
      本地 prove 全绿（advanced 37 / system 48 / connection 58 / smoke 45）。

### Added

- **CI 证明链 `frontend-proofs` job（PR #8）**：`fetch-depth: 0` +
  `fetch-tags: true`，跑 10 个 `prove-*-parity.js`（固定基线）+
  `smoke-minified-luci.js`。基线锚定两条路：孤儿基线提交经增量上传为远端
  orphan 提交并打 tag（`pre-system-route` / `pre-advanced-route`），其余
  6 个脚本显式传远端原生基线 sha。兼容性预验证：`loadSide` 旧侧全套读基线
  （parser / components / 页面同源自洽），基线自带被删死函数定义，无
  ReferenceError 路径。UI 渲染不可变由逐字比对锁定：前端重构在 CI 层面被
  证明「用户看到的界面一个字节都没变」。

### Chore

- **版本号提升并触发重新编译**：LuCI 包 `3.1.2 → 3.2.0`。架构 v2 收官（PR #7
  全页面路由化 + PR #8 收官批 + PR #9 文档同步）作为 minor 里程碑发版，
  循 3.1.0 异步化架构的 minor 提升先例。
- **发布构建默认关闭 LuCI 前端 JS 压缩**：`build-release.sh` 不再调用
  `scripts/minify-luci-frontend.sh`，安装包内的 LuCI 前端 JS/CSS 保持源码可读，
  方便设备端调试、比对与 grep；压缩脚本与 CI 证明链（static-checks +
  frontend-proofs）原样保留，需要恢复压缩时
  `MINIFY_LUCI_FRONTEND=1 ./scripts/build-release.sh` 一键打开。
  WebUI（/5700 React bundle）为仓库内置 prebuilt 产物，本就不参与编译期压缩，
  维持不变。
- **修复 release 构建的 SDK 下载偶发失败**：`build-release.sh` 下载
  `downloads.openwrt.org` 的 SDK 时出现过
  `curl: (92) HTTP/2 stream 1 was not closed cleanly: PROTOCOL_ERROR`，
  直接中断发版。两处下载统一改用 curl 选项数组强制 HTTP/1.1 并加
  `--retry-all-errors`（`--retry` 对协议错误默认不生效）、`--retry-delay 5`、
  `--connect-timeout 30`；原先 `sha256sums` 那次下载连重试都没有，一并补上。

---

## [3.1.2] - 2026-10-06

### Chore
- **版本号提升并触发重新编译**：LuCI 包 `3.1.0 → 3.1.2`（跳过已被 manual-v3.1.1
  手动构建占用的 3.1.1）。无功能性代码变更，用于触发 CI 静态检查 + `cargo test`
  与 release.yml 的 OpenWrt SDK 全量构建，产出全新安装包并补挂正式 tag。
- 本版本包含 v3.1.0 提交之后合入的 PR #6「Refactor modem status views around
  shared async state」：模组状态视图重构为共享异步状态。该改动此前仅存在于
  main 分支，从未进入任何正式发版。

---

## [3.1.0] - 2026-10-05

### Added

- **后端读命令缓存闸门（`read_gate`）—— 前端刷新率与 AT 通道下发量彻底解耦**：
  - Background：此前 LuCI 与 WebUI 虽然读同一份缓存，但**渲染/轮询路径上的读命令
    仍会直压 AT 通道**。MT5700 独占串口，慢命令单条占 8~12 s（实测 `AT^NTXPOWER?`
    11.6 s、`AT^LENDC?` 不支持也占 8 s），两端同时刷新就在同一条队列里互相排队——
    这正是「LuCI 与 WebUI 同时使用仍互相影响」的根因。
  - Changes（`mt5700webui-openwrt-server/at-webserver/src/read_gate.rs`，新增）：
    - 在后端命令入口加**机制性闸门**，读命令物理上到不了 AT 通道。选这条路线而不是
      「前端逐处改读缓存」，是因为后者是约定：160+ 处 `sendCommand` 漏一处即回归 AT，
      且新增读点会重新漏。机制性保证不依赖调用方自觉。
    - 三级响应：`Cached`（命中新鲜缓存，零 AT 零阻塞）/ `Pending`（无缓存，提交
      **单飞**后台采集后**立刻**返回占位，绝不阻塞页面）/ `Passthrough`（写命令直通）。
    - **惰性采集**而非 40+ 个周期采集器：后者会让 AT 流量暴涨十几倍，反而把独占通道
      占死。改为惰性后，AT 流量 = 实际被访问的命令数 × 各自最小重采间隔，**与前端
      刷新率完全无关**；没人访问的二级页面，命令一次都不下发。
    - raw 缓存用 `raw:` **前缀式** topic 而非固定枚举：读命令有 40+ 条且会持续新增，
      枚举必然漏；加前缀后闸门对任何读命令自动适用，新增读点无需改后端。
    - TTL 按实测响应耗时分三档：慢 300 s / 中 60 s / 快 10 s。
    - 采集任务用 `Priority::Low`：**永远排在用户写操作之后**，即使用户正在拨号，
      采集器也不插队。
  - **三条后端路径分别接入**（`src/daemon.rs`）：
    | 路径 | 入口 | 说明 |
    |:--|:--|:--|
    | JSON-RPC `at` | `run_command` | WebUI WS 路径 |
    | control socket `send` | `handle_control_request` | **LuCI 全部读命令路径** |
    | CLI fork 子进程 | `cli_capture` → `cli.rs: run_at` | 复用 control socket，自动覆盖 |

    第二处最容易漏：只接 `run_command` 时闸门等于没装，实测表现为
    `mt5700m-at cached` 里始终没有 `raw:` topic（`mt5700m-at command` 实际走
    `auto_cascade → daemon_transport → control socket`，不在 `run_command` 里）。
  - Changes（`src/services/at.ts`）：新增 `pending` 字段；WebSocket 应答匹配里
    **pending 判定必须提到 `matchesLastCommand` 之前**——pending 的 `data` 是空串，
    走命令匹配会因对不上任何命令而被当串号丢弃，前端白等一个 `commandTimeout`。

- **三个验证脚本**（`tools/`）：
  - `verify_async_gate.py`：频率阶梯压测 + raw age_ms 单调性。
  - `verify_cli_path.py`：验证 CLI fork 路径的闸门覆盖与 extras 查询入缓存。
  - `verify_coexist_async.py`：4 worker × 6 轮混合三条路径并发轰炸。

### Fixed

- **无参查询命令被误判为写命令，导致每次刷新仍真发 AT**：
  - Background：`AT+CNUM`（本机号码）在 `cmd_status` 的 4 条 extras 查询里是唯一
    没进 raw 缓存的，实测边际成本 368 ms/次就是它贡献的。根因是它属于**无参查询**
    ——既无 `?` 后缀也无 `=`，且不在动词白名单里，于是被「含 `=` 即写」之外
    的兜底判成了写。
  - 修复：补入白名单 `AT+CNUM` `AT+CIMI` `AT+CSQ` `AT^MONSC` `AT^MONSSC`
    `AT^DSFLOWQRY` `AT^FOTADLQ` `AT^SYSINFOEX`。
  - 新增 `all_backend_read_commands_are_classified_as_read` **全量反查测试**，
    清单来自 `grep -ohE '"AT[+^&][^"]*"' src/cli.rs src/snapshot.rs`。这条测试当场
    抓出漏判的 **`AT+CSQ`**（信号强度，`at_queue.rs` 网速测量依赖它）与
    **`AT^SYSINFOEX`**。新增读命令若忘加白名单会立刻测试失败，而不是等到实机
    表现为「页面偶尔不显示数据」。
  - 反向约束同步补齐：`AT^CELLSCAN`（启动扫描）/ `AT&F0`（恢复默认）判写；
    `AT^NRRCCAPCFG=5,1,0`（真赋值）不得被 `AT^NRRCCAPQRY=`（查询）前缀规则误收。
  - 查询型赋值白名单扩充：`AT+CGPADDR=`、`AT^NRRCCAPQRY=`（=2 VoNR / =3 NR CA /
    =5 DSS 都是查能力），仍要求 `=` 后**恰好一个纯数字**。

- **LuCI 概览页 Mobile IP 卡片恒显示 Disconnected（一条「luci 经常不显示数据」的成因）**：
  - Background：`parser.js::parseSession()` 里
    `ipv4Connected = ndis[0]==='1' && ndis[4]==='IPV4'`，`ndis` 来自
    `AT^NDISSTATQRY?`。**实测该命令在中国移动网络下返回空应答** → `ndis` 恒空 →
    `ipv4Connected` 恒 false。而同一批实测：`AT^DHCP?` 正常返回
    `^DHCP: 98AC060A,...`（little-endian 即 `10.6.172.152`，与 eth2 实际地址一致）、
    `AT^DHCPV6?` 正常返回 IPv6、`AT+CGPADDR=1` 正常返回 `10.6.172.152`。
    即地址就在手里，只因判据依赖一条不响应的命令而显示未连接。
  - Changes（`htdocs/luci-static/resources/mt5700m/parser.js`）：NDIS 可用
    （`ndis.length >= 9`）时保持原判定；不可用时**回退到「是否真拿到地址」**——
    有 DHCP 租约或 PDP 地址即算已连接。宁可极端情况多显示一次「已连接」，
    也不要拿着真实地址却告诉用户「未分配」。
  - 实测：Mobile IP 从 `Disconnected` / `Not assigned` 变为 `Active`，
    IPv4 `10.6.172.152`、IPv6 `2409:8d5a:374:117:5c24:66f3:a74:16d8` 均显示。

- **IPv6 地址在窄容器下末位被孤立成一行**：
  - `htdocs/luci-static/resources/view/mt5700m/status.js`：IPv6 是 39 字符无空格
    长串，默认只在 `:` 处断行，实测把末位（如 `...a74:16d` + `8`）孤立到下一行。
    加 `word-break:break-all`。

### Performance

实机（192.168.10.1，MT5700M-CN，中国移动）实测，后端
818,328 B / md5 `077bff7191b9455265b66c60fbaac361`，`cargo test` 104 passed / 0 failed。

| 指标 | 改造前 | 改造后 | 改善 |
|:--|--:|--:|--:|
| 单次读命令 `AT^DHCP?` | 0.32 s | 0.07 s | 4.6x |
| 10 次串行 | 2.03 s | 0.15 s | 13.5x |
| **边际斜率** | ~200 ms/次 | **8.9 ms/次** | **22.5x** |
| `mt5700m-at status` | 0.296 s | 0.08 s | 3.7x |
| 并发 4 worker × 6 轮（三路径混合） | — | 24 ms/次 | PASS |

- **验证判据用「边际斜率」而非「总耗时 < 单次 × N」**：单档里含 SSH 建连与进程首启的
  一次性开销（~50 ms），拿它当基线会误判（实测 0.15 s vs 0.07×2 = 0.14 s，只差
  0.01 s 就 FAIL）。真正要证明的是「每多刷一次多花多少毫秒」。
- 轰炸结束后 `raw:` age_ms 仍持续单调增长（3471 → 7482 → 11492 ms），证明**零重采集**。
- 页面层：LuCI 7/7 标签页正常、零插件 JS 错误、无「未连接」标记（桌面 1080×900 与
  移动 390×844 均验证）；WebUI 直开零 JS 错误；双标签页各刷新 3 轮后数据同源。

### Known Issues（非本次引入，仅记录）

- **ImmortalWrt 上游 LuCI 核心的 `E is not defined` / `findParent is not defined`**：
  堆栈在 `/luci-static/resources/luci.js:184` 与 `ui.js:287 showTooltip`，
  **未进入本插件即复现**，属上游既有缺陷，本次未越界修改。
- **`AT+CNUM` 模组不应答**：中国移动网络下返回空串，故采集失败且**故意不写缓存**
  ——把空值缓存起来等于把「不支持」固化，之后每次访问都白下发一条 AT。属正确行为。
- **WebUI 的 IMEI 写入入口仍在**（`pages/system/Info.tsx`，连点 5 次隐藏入口下发
  `AT^PHYNUM=IMEI,...`）。按用户指示「暂不动，只做异步改造」保留原样，**是遗留红线
  隐患**，待决定何时移除。

### Security

- 本轮改造与全部验证脚本**未下发任何 IMEI 命令**。`AT^PHYNUM` 未纳入白名单、
  未写入任何测试断言；`AT+CGSN` 保持既有白名单条目不动（无新增测试）。
- **本轮未新增任何截图**。`docs/preview/` 下4 张图与
  `mt5700webui-openwrt-server/docs/screenshots/` 下 8 张均为**历史提交已在仓库中**
  的旧图，本轮原样保留、未重新生成。
  需要说明的是：这些旧图含设备真实身份与位置线索（IMEI / ICCID / IMSI / 内网 IP /
  基站ID）。本轮尝试重新脱敏时发现脚本按「值特征」判定（15 位数字 / IPv4 / IPv6 /
  长 hex），在概览页与模组页验证命中，但**移动数据页的 `.mt-session-row` 容器尚未
  覆盖**（实测该页 IP 未被涂住），因此**未提交任何新图**。脱敏补全与旧图治理是
  一项独立待办，不在本轮范围内。

### Build

- `luci-app-mt5700m` 3.0.0 → **3.1.0**（PKG_RELEASE 1）
- `at-webserver` (Cargo) 4.0.4 → **4.1.0**，`Cargo.lock` 同步
- `mt5700m-webui` 3.0.0 → **3.1.0**
- 交叉编译：云端 aarch64-unknown-linux-musl + `rust-lld`
  （`RUSTFLAGS="-C linker=<rust-lld> -C linker-flavor=ld.lld"`），
  避开宿主 `/usr/bin/ld: unrecognized option '--fix-cortex-a53-843419'`。
  std-only 项目无需 `gcc-aarch64-linux-gnu`，不污染系统环境。

---

## [3.0.0] - 2026-10-04

### Fixed
- **修复 `connected` 恒为 0 导致 LuCI 全部标签页判定为未连接（P0，本次修复引入的回归）**：
  - Background：本次为压缩 `status` 耗时，把 `connected` 判定从「短探测 `AT`」改成了
    「查 `AT+CONNECT?` 是否回 `+CONNECT: 1`」。但 `AT+CONNECT?` **只存在于 network
    传输通道**，串口独占通道下必然回 `ERROR` —— 于是 `connected` 永久为 0。而
    `parser.js` 用它做**全页面总闸门**（`connected = manager.connected && reachable &&
    sysmode 有效`），表现为概览 / 移动数据 / 无线与小区 / 短信 / 模组与 SIM 卡 /
    高级设置**同时显示未连接**，而 `mt5700m-manager status` 明明报 `connected: true`。
  - Changes（`at-webserver/src/cli.rs::cmd_status`）：语义纠正为「AT 通道是否可用」。
    daemon 快照中 `signal` / `registration` / `sim` / `operator` 任一非空即视为可用
    （零 AT 流量，因为上一行已经调过 `daemon_cached()`）；daemon 不可达时才回退一次
    `AT` 短探测。
  - 实测：`connected=0` → `connected=1`，`channel=serial`，`at_port=/dev/ttyUSB2`
    （与 daemon 实际上报的 `serial_port` 及 `/proc/<pid>/fd` 三方一致），
    `status` 26 行 / 0s / RC=0，5 路并发全部 RC=0。

- **修复 LuCI「无线与小区」页整页渲染中断（P0）**：
  - Background：`view/mt5700m/network.js` 调用 `c.circularGaugeCard(...)`，但组件库
    `components.js` 从未导出该函数（导出名是 `svgCircularGauge`）。`TypeError: c.circularGaugeCard
    is not a function` 在渲染期抛出，**中断整页渲染**。另外即便补上函数，参数顺序也是错的：
    调用处传 `(label, value, unit, cls, min, max)`，而 `svgCircularGauge` 签名是
    `(val, min, max, unit, label, cls)`。
  - Changes：按 label 查表取量程（`gaugeScale`）而不是按下标硬编码 ——
    `parser.js` 的 `cell.metrics` 会随 RAT 变化（NR: RSRP/RSRQ/SINR，LTE: RSRP/RSRQ/RSSI，
    WCDMA: RSCP/RXLEV/ECIO），下标固定会在 LTE/WCDMA 下量程错配、指针顶到刻度外；
    只渲染当前 RAT 实际存在的项。
  - 验证：全量脚本比对 `components.js` 导出清单与 8 个视图的全部 `c.xxx()` 调用，
    确认**这是唯一一处不存在的组件调用**。实测该页由 686 字符（残缺）恢复为 4183
    字符完整渲染，含 Registered / RSRP -63 / RSRQ -10 / SINR 27 / 48°C。

- **修复 LuCI「移动数据」页整页显示字面量 `[object Promise]`（P0）**：
  - Background：`view/mt5700m/connection.js` 的 `load()` 写的是
    `return m.render().then(...)`。LuCI 的 view 契约要求 `load()` **同步返回一个 DOM 节点**，
    返回 Promise 会被当字符串塞进容器 —— 整个正文被这一行占满（页面文本仅 712 字符）。
  - Changes：改为同步返回外层容器 + 骨架屏占位，`form.Map` 渲染完成后异步注入正文，
    并补 `.catch` 把渲染失败显示为可读错误而非空白。与项目既有异步化架构一致。
  - 注意：LuCI 的 `dom` 模块**只有 `dom.content()`，没有 `dom.prepend()`** —— 用原生
    `appendChild` 组装容器。
  - 骨架屏清理方式：不能用 `dom.content(skeleton, null)`，那只是清空内容、节点仍在，
    会留下一个空的 `.mt-page .mt-skeleton-page` 继续占 padding 与间距；必须
    `parentNode.removeChild(skeleton)`。
  - `.mt-page` 不能嵌套：`c.skeletonPage()` 返回的元素**自带 `mt-page` 类**，放进另一个
    `.mt-page` 里会双倍内缩。改为外层容器不带 `mt-page`、`mt-page` 放在注入 slot 上，
    骨架与 slot 平级。
  - 实测：页面文本 712 → 2040 字符；`.mt-page` 数量 2→1、嵌套 1→0、残留骨架 1→0、
    `cbi-map` 正常渲染、文档无横向溢出；「MOBILE DATA / Mobile data」标题、
    事实卡（自动拨号 / eth2 / Automatic / IPv4+IPv6）、已分配地址（IPv4 10.6.172.152、
    双栈 DNS、IPv6 PD、MTU）、流量计数（1.68 GiB / 8.37 Gbps）、拨号表单与
    Save & Apply 栏均正常。

- **修复 LuCI 与 WebUI 的 AT 通道全哑（P0，实机三重根因叠加，缺任一层都导致全部 AT 命令超时）**：
  - Background：
    1. **BusyBox 缺 `stty` applet**。原 `serial.rs` 完全依赖外部 `stty -F /dev/ttyUSB1 ... raw` 配置串口，且**忽略退出码**。OpenWrt 镜像普遍不内置该 applet（实机 `stty: applet not found`），TTY 一直停留在内核默认 cooked 模式——`ICRNL`/`OPOST`/`ICANON`/`ECHO` 全开，回显与行缓冲让 AT 响应解析彻底失真。
    2. **`VMIN=0` 的 EOF 陷阱**。Linux tty 在 `VMIN=0`/`VTIME=0` 下，空闲时 `read` **返回 0 字节**，被读循环当成 EOF → 读取线程刚连上就退出，后续所有 AT 命令必然超时。
    3. **非阻塞下的 `EAGAIN` 被当成致命错误**。补上 `O_NONBLOCK` 后，`at.rs::spawn_reader`、`probe.rs`、`serial.rs::at_probe_ok` 三处读取循环均为 `Err(_) => break`；而 `EAGAIN` 是**正常空闲态**，等于每次空闲都拆掉读取线程。
  - Changes（`mt5700webui-openwrt-server/at-webserver/src/`，实现对照姊妹项目 [`luci-app-mt5700`](https://github.com/LianXia233/luci-app-mt5700) 的 `src/rust/src/serial_linux.rs`，**std-only 零第三方依赖**）：
    - `serial.rs` 新增 `termios_raw` 模块：`TCGETS`/`TCSETS`/`TCFLSH` ioctl 直配，必须**清除** cooked 位（`c_lflag &= !(ICANON|ECHO|...)`、`c_oflag &= !OPOST`、`c_iflag &= !(ICRNL|...)`），波特率 `(c_cflag & !CBAUD) | B115200`，8N1 且 `c_cflag |= CS8|CREAD|CLOCAL`，`VMIN=1`/`VTIME=0`，配 `tcflush(TCIOFLUSH)` 清残留；补齐缺失的 `CSTOPB` 常量。
    - `serial.rs` 新增 `open_tty_raw()`：FFI `open(2)` 带 `O_RDWR|O_NOCTTY|O_NONBLOCK|O_CLOEXEC`（`OpenOptions` 无法表达后两个 flag），并新增 `set_modem_lines()` 经 `TIOCMGET`/`TIOCMSET` 拉高 DTR/RTS（`option` 驱动 open 后默认拉低）。
    - `at.rs` / `probe.rs` / `serial.rs::at_probe_ok` 三处读取循环统一容忍 `WouldBlock`/`Interrupted`（短睡重试），不再拆线程。
    - `probe.rs` 新增 **`atprobe` 诊断子命令**，绕开 daemon / 仲裁器 / 后台采集器直探 AT 口；配合 `AT_DEBUG_TERMIOS=1` 回读 termios 实际值。
  - 实测（Airpi AP3000M + Fibocom MT5700M-CN）：

    | 指标 | 修复前 | 修复后 |
    |:--|:--|:--|
    | 单次 `mt5700m-at status` | 25s 超时被杀（RC=143） | 0–1s RC=0，26 行完整 |
    | 5 路并发 `status` | 全部 143 超时（30s） | 全部 0 成功 |
    | `atprobe` 探测 5 条只读命令 | 5/5 TIMEOUT，0 字节 | 5/5 全部响应 |
    | `cached` 快照 | 无数据 | 全字段 `fresh:true` |

    实测数据：NR-5GC / RSRP -63 / RSRQ -5 / SINR 27~29 / band 41 / SIM READY / ICCID 已读到（号码脱敏）/ 45°C。

- **修复 `at_port` 上报值与 daemon 实际持有的串口不符**：USB 重新枚举时 `option` 驱动重编号 ttyUSB*（PCUI 口在 ttyUSB1 ↔ ttyUSB2 间互换），内核会把旧 fd 转移到新编号节点，而 daemon 日志里的 `attached to serial` 是**启动时快照**，因此 `status` 报告的端口可能并非实际在用的端口。`daemon.rs` 新增 `AtClient.attached_port` 字段，`cached` 响应新增 `serial_port`；`cli.rs::cmd_status` 优先采用 daemon 上报值，实机确认 CLI 报的 ttyUSB2 与 `/proc/<pid>/fd` 一致。

- **修复 `interruptible_aborts_on_cancel` 测试竞态**：`at_queue.rs` 该测试的阻塞时长仅 20ms，却在 15ms 后置 cancel，留下 5ms 窗口，并行执行下必然失败。改为 `set_interruptible_block(600ms)` + 置位前 sleep 30ms，**连续 4 次全量 94 passed**。

- **修复 WebUI 图表组件 21 个 class 零样式定义（P0，经线上产物 grep 实证）**：`sparkline` / `sparkline-canvas` / `sparkline-svg` / `sparkline-dot` / `chart-grid-line` / `sparkline-legend` / `quality-bar` / `quality-bar-track` / `quality-bar-fill` / `ring-gauge` / `ring-gauge-dial` / `ring-gauge-track` / `ring-gauge-arc` / `ring-gauge-value` / `ring-gauge-label` / `lock-row` / `chart-empty` 等在 `app.css` 中**全部 0 匹配**。后果：质量条与 7 路温度条渲染为 **0 高度（完全不可见）**、环形仪表数值掉到 SVG 下方、曲线采样点不定位。已补齐全套样式段。

- **修复 `.net-hero` 在 961~1010px 视口静默丢内容（P0）**：该区硬编码 `minmax(720px, 1fr)` 双列，而 `.app-viewport` 是 `overflow-x: hidden` —— 视口窄于 1440px 时右侧内容**直接消失且无法滚动**。5 处网格统一改为 `repeat(auto-fit, minmax(…, 1fr))`，元素数量与列数不再错配。

- **修复布局间距双重叠加**：`.page-card` / `.panel` 的 `margin-bottom:20px` 与父级 `.page-stack` 的 `gap:20px` 叠加成实际 **40px**，且 `.two-col` 内出现单侧偏置。改为间距统一由容器 `gap` 控制。

- **修复 `.kv-grid` 无任何降列规则**：2~4 列键值表在任何断点都不降为单列，手机上 IPv6 长值只能挤在两列换行。手机档统一降为单列。

- **修复 `Kv columns={1}` 静默失效**：该值生成不存在的 `kv-grid--1` 类，样式表未定义 → **静默退回 2 列**。新增 `kv-grid--single`。

- **修复 `.quality-grid` 定义 4 列只放 2 项** → 右半永久空白；**`.net-cell-params` 内嵌 3 列 Kv 只占第 1 列** → 3 列挤在半宽、右半空。均改为 auto-fit / 跨列修正。

- **修复窄屏溢出**：`.at-input-row` 无 `flex-wrap` 导致 3 个按钮永不换行、Input 被压至近 0 宽；`.sms-shell` 在 375px 下联系人栏与线程并排、正文不可读（改纵向堆叠）；`UssdPanel` 200px Input + 2 按钮 ≈408px > 375px 溢出。

- **修复三级边框嵌套**：PageCard → Panel → kv-item 三层描边。`Panel` 新增 `variant="flat"`（`pages/network/Info.tsx` 三处启用）。

- **修复 Semi Card 头部挤压**：Semi 实际类名是 `semi-card-header-wrapper`（且为 `row-reverse`），原先只写了 `semi-card-header`，补 `flex-wrap` + `row-gap`，并让 `header-extra` 用 `margin-left:auto` 保持右对齐。

- **修复概览页「载波状态」卡与 QCI / APN / 签约速率永久空白（P0）**：
  - Background：`status` 切到 StateCache 快照后（`print_cached_status`），
    **只覆盖 5 个 topic**（signal / sim / modem / network / temperature），
    `print_carrier_aggregation()` 只在「无 daemon」的 fallback 分支里被调用，
    从未进入缓存路径。结果 `status` 输出里**完全没有**
    `carrier_count` / `carrier_N` / `ca_*` / `dc_*` / `active_apn` / `qci` /
    `ambr_*` / `phone_number` 这几组字段，而 `parser.js` 的 `carrierInfo()` 与
    `status.js` 的 SIM 卡正是靠它们渲染 —— 于是「载波状态」恒显示
    "Current carrier information is unavailable."，QCI / APN / 签约速率恒为空。
  - Changes：
    - `cli.rs::print_cached_status` 新增 carrier/CA 段：从已有的 `cell` 快照
      （`AT^HFREQINFO?` 的采集结果，含 band / NR-ARFCN / dlBandwidth）重建
      `carrier_count` / `carrier_1`（8 段 `|` 分隔，字段顺序与 parser.js 严格对齐）、
      `ca_active` / `dc_active` / `ca_mode` / `ca_dl_bandwidth` / `ca_ul_bandwidth`，
      零 AT 流量。
    - 新增 `nr_arfcn_to_mhz()`（3GPP TS 38.104）换算 NR-ARFCN → 中心频率，
      填 `carrier_1` 的 dl/ul_freq 字段。
    - `Settings` 新增 `query_extras` 开关（默认 off），`cmd_status` 置 on 后
      补回 `print_active_apn` / `print_qos` / `print_subscriber_number` /
      `print_subscription_rate` 四项 —— 它们没有后台采集器，只能现查；
      `timeout_s` 已被压到 3s，单次 status 仍在 0–1s 内完成。
  - 实测新增输出：`carrier_count=1`、`carrier_1=NR|B41|504990|2524950.00|100|…`、
    `ca_dl_bandwidth=100`、`qci=6`、`ambr_down_mbps=1000.0` / `ambr_up_mbps=100.0`。
    概览页实测渲染：B41 / NR / Single carrier / ARFCN 504990 / 上下行各 100 MHz。

- **修复 SIM 状态恒显示 Unknown**：`status` 输出的键名是 `sim_state`，而
  `status.js` 读的是 `data.sim`，两边不一致。`parser.js::parseStatus` 补
  `data.sim = data.sim || data.sim_state || ''`。实测 SIM Status 由 Unknown 变 Ready。

- **修复运营商 logo 压在名称上（概览页事实卡）**：`mt-facts-value` 是块级容器，
  运营商 logo（`<img>`）与文本节点各占一行叠压，单元格高度从 24px 翻到 44px。
  新增 `.mt-facts-value--inline` flex 变体 + `.mt-facts-logo`，不改动共用类以免
  影响其余 20+ 处用法。实测高度回落到 24px，logo 20×20 同行对齐。

### Added
- **新增前端构建配置（`semi-tcpweb/` 原先只有 `src/`，无任何构建入口，无法产出可发布产物）**：补齐 `package.json`（React 18 + Semi 2.103 + Vite 5 + TypeScript）、`vite.config.ts`（`base:'/5700/'`、构建期注入 `__APP_VERSION__`、hash 产物名 `assets/index-[hash].*`）、`tsconfig.json`、`index.html`。
- **新增 `src/styles/breakpoints.ts` 断点契约**：CSS 与 JS 共用 `mobile 768 / compact 480 / tablet 1024`。改造前 CSS 断点（960/767）与 JS 断点（767/640/520）**互不重合**，≤640px 区间无任何 CSS 规则；5 处调用点（`Dial.tsx` / `Settings.tsx` / `ScanPanel.tsx` / `Upgrade.tsx` / `AppLayout.tsx`）已改为引用共享常量。响应式统一为**移动优先三档**：≥1024px 多列网格、768–1023px 两列、<768px 单列纵向堆叠、<480px 按钮全宽与 Steps 缩进复位。
- **新增 `atprobe` 诊断子命令**：绕过 daemon 独占锁与仲裁器直探 AT 口，用于区分「通道/串口问题」与「模组注册态问题」。详见 Fixed 第一条。
- **新增仲裁器通道占空比预算**：`at_queue.rs` 引入 `DutyWindow` 滑动窗口（`BACKGROUND_DUTY_LIMIT_PCT = 60`）限制后台采集器占用率，为用户可见流量保留带宽；用户请求被拒时置**用户饥饿锁存**，采集器据此退避。
- **新增命令级超时表与 `AtRequestSpec::ui_query`**：UI 可见请求统一 High 优先级 + 零重试；`run_command` 与 daemon 控制通道 `send` 分支改用 `ui_query`。
- **新增工具脚本**：`tools/stress.sh`（5 路并发 `mt5700m-at status` 压测，记录 RC/耗时/行数）、`tools/deploy_webui.py`（备份 → 空间检查 → `.new` 暂存上传 → 原子 `mv` 提升 → HTTP 验证，输出回滚命令）、`tools/shoot_webui.py`（三档 375/768/1440 截图 + 溢出探针，输出 `layout-report.json`）。

- **新增 WebUI 动效体系（v3.1）**：`global.css` 末尾新增「动效体系」一节，并建立
  分级动效令牌（`--app-dur-fast/base/slow`、`--app-t-lift/press/fade/slide/tint/shadow`）。
  - **消除 `transition: all`**：原 `--app-transition-spring: all 0.22s` 让每次 hover 都
    要监听全部可动画属性（width/height/margin/padding/border…），触发布局与重绘。
    改为按属性分类，transform/opacity 走合成层。产物内 `transition:all` 从 13 处降到
    6 处（剩余为 Semi 组件库自带，不可控）。
  - **补 `prefers-reduced-motion` 分级降级**：此前是一条全局 `!important` 把所有动画
    压到 0.01ms —— 连**承载语义的状态反馈**（色块切换、数值变化）也被压掉，依赖动画
    确认操作结果的用户得不到任何反馈。改为分级：装饰性常驻循环（呼吸光晕、气泡、
    骨架扫光）彻底 `animation: none`；状态切换与图表数值变化保留（去位移留颜色/宽度）。
  - **图表数值不再瞬跳**：环形仪表按 `stroke-dashoffset` 从空环长到目标值（周长经
    `--ring-circumference` 内联，不写死常量，`size` 变化时起点仍是空环）；质量条按
    `width` 生长；迷你曲线按 `stroke-dasharray` 逐段画出 + 采样点按 `--dot-i` 依次点亮
    （点越靠右越晚，与时间轴同向）。虚线序列只淡入不生长 —— `stroke-dasharray` 属性
    复用会互相覆盖，且虚线本不该「长出来」。
  - **新增加载骨架**（`Skeleton` / `SkeletonCards`）：此前数据到位前只渲染「--」，
    「还在加载」与「加载完成但确实没数据」视觉上完全一样，弱信号区尤其误导。
  - **指标数值变化轻抬**（`Metric` 加 `metric--updated`）：轮询场景下静默跳变会让人
    怀疑「这次刷新到底生效没有」。
  - **收敛重复断点**：删除两处遗留的 `@media (max-width: 960px)`（规则已被统一响应式层
    覆盖，双份定义正是漂移来源），并合并文件中部与末尾**两份** `prefers-reduced-motion`
    块（靠前那份与末尾分级版互相矛盾）。
  - 移动端（<768px）停用装饰性循环动画，采样点 stagger 上限压缩到 6 个点。
  - 验证：`tsc --noEmit` 全绿；Playwright 实测入场动画进行中
    （10 个元素处于 `--enter`、`ring-gauge-sweep` 运行、`stroke-dashoffset: 216.77px`、
    条宽 `0px` 起始），`prefers-reduced-motion: reduce` 下无限循环动画数从 23 降到 0。

### Changed
- **`make package` 版本 `2.8.7` → `3.0.0`（破坏性）**：AT 串口不再依赖 BusyBox `stty` applet，串口打开一律 `O_NONBLOCK` + `VMIN=1`/`VTIME=0`。旧版二进制在不修正 termios 的环境下会把 TTY 留在 cooked 模式导致 AT 全哑。
- **`cmd_status` 瘦身**：派生值改从 StateCache 快照读取（零 AT 流量），仅保留一次短探测判断 `connected`，`status` 响应从 25s 超时降到 0–1s。
- **快照慢查询加占空比门控与失败退避**（`snapshot.rs`），避免长周期采集挤占用户通道。
- **包内补齐 `root/etc/init.d/at-webserver`**（原包内缺失）：`at-webserver` 是**独立 procd 服务**，不由 `mt5700m-manager monitor` 拉起（monitor 只管 netifd 与接口），缺失时 8765 端口永远不监听、`/5700` 无数据。
- **修复 npm 侧构建阻塞**（两条均为真实 resolve 失败，非配置错误）：
  - Semi 2.103 的 `exports` 只白名单 `lib/es` / `lib/cjs`，**不含 `dist/*`**，导致 `import '@douyinfe/semi-ui/dist/css/semi.min.css'` 被 resolve 拒绝（`Missing "./dist/css/semi.min.css" specifier`）。`vite.config.ts` 用 alias 指向物理文件绕过。
  - `src/main.tsx` 原用 `../node_modules/...` 相对路径绕过解析器，改为 bare specifier + alias。

### Verified
- `cargo test`：连续 4 次全量 **94 passed / 0 failed**。
- `tsc --noEmit` 全绿；顺带修两处既有类型错误（Semi `Collapse.onChange` 可能传 `undefined`，`SchedulePanel.tsx` / `Settings.tsx` 需窄化为 `[]`）。
- 产物：CSS 724.02 kB / JS 986.95 kB；Rust 二进制 807,936 B（补齐载波字段），已同步至 `htdocs/5700/`（21 文件，清除旧 hash 产物）与 `root/usr/bin/at-webserver`（805,600 B）。
- Playwright 三档实测**均无横向溢出**（`scrollWidth == clientWidth`）：

  | 视口 | 375 | 768 | 1440 |
  |:--|:--|:--|:--|
  | 结果 | 无横向滚动 | 无横向滚动 | 无横向滚动 |

  实测期间发现 `.metric-row` 的 `minmax(150px,1fr)` 下限在 485px 宽容器内只能排 2 列、第三个 Metric 落单，已将下限调至 130px（三列需 430px < 485px），实测三指标整齐同行。
- **LF 换行校验：30 个文本文件全部合规**。

### Note
- **IMEI 红线**：全程仅通过**只读** `AT+CGSN` 读取并缓存；`atprobe` 默认命令集（`AT` / `AT+CSQ` / `AT+COPS?` / `AT+CGSN` / `AT^HCSQ?`）全部只读；本次全部代码改动、AT 指令与前端交互**未引入任何写入、修改、擦除 IMEI 或变更其相关存储的指令与入口**。
- **排查手法记录**：本次 termios 修正中，「方向写反」把 raw 标志当置位用（等于主动打开 cooked 模式）是通过 `AT_DEBUG_TERMIOS=1` **回读实际 termios** 暴露的（`cflag=0x8bd`，CBAUD 位为 0）；波特率改用 `BOTHER`+`c_ispeed` 时内核不认，回读 `ispeed=0x0 ospeed=0x0`。裸 ioctl 配置必须回读验证，不能只看代码。
- **luci-base 自身的两个加载顺序缺陷（非本包代码，实机确认）**：`luci.js` 调用
  `String.prototype.format`（定义在 `cbi.js`，加载滞后），以及 `ui.js` 调用
  `findParent`（定义在 `cbi.js`）。两者都会在控制台抛 `pageerror`，在 404 页面尤其明显。
  文件属于 `luci-base` 而非本包，需设备侧运行时兜底（见 README 故障排查一节）。
- **遗留观察（非本次修复范围）**：
  - USB 偶发重新枚举（`dmesg: option1 ttyUSB4 disconnected` → `usb 2-1 new SuperSpeed device`），端口编号随之互换。daemon 能通过内核 fd 转移继续工作，但 `hotplug.d/usb/60-mt5700m` 是否需要触发 daemon 重 attach 待后续单独排查。
  - 模组注册态瞬态：`AT+COPS?` 返回 `4`（registration denied）时 WebUI 会瞬时显示「4G LTE 0% / 暂无测量」，而同期 `AT^HCSQ?` 仍报 `"NR",78,246,30` —— 属模组注册态瞬态，非前端问题。

## [2.8.7] - 2026-10-03

### Fixed
- **修复 ucode 插件 nc 无超时导致整个 LuCI 被拖死**：
  - 根因：`rpcCall()` 的 `p.read('line')` 无超时，rpcd 的 ucode 插件是「每请求一个进程」，AT 命令挂起时 nc 永久等待，rpcd 并发进程被占满 → 整个 LuCI 拖死。前端 15s 超时只救浏览器、救不了 rpcd。
  - ucode 侧：nc 转发套超时外壳（`timeout` applet，缺失退化 `nc -w`），到点杀 nc 立即释放 rpcd 进程；按命令类型分配超时——读类 12s / 写类（`sms-*`、`command` 前缀、`pdp-set`、`factory-reset`、`sim-pin`、`restart`、`unlock`、`advanced-set`）25s。
  - Rust 侧：`cli_capture` 的 `Command::output()` 改为 `spawn` + `try_wait` 轮询 + 25s 硬超时 `kill`，避免 CLI 子进程卡死时永久占住 RPC 线程（跨平台 std API）。
  - 前端侧：`AT_TIMEOUT_MS` 15s → 30s，保证前端兜底 > ucode 外壳（25s），三层防挂起闭环。

## [2.8.6] - 2026-10-03

### Chore
- **版本号提升并触发重新编译**：LuCI 包 `2.8.5 → 2.8.6`、Rust AT 后端 `4.0.2 → 4.0.3`、WebUI 前端 `3.0.4 → 3.0.5`。无代码变更，用于触发 CI 静态检查 + `cargo test` 与 release.yml 的 OpenWrt SDK 全量构建，产出全新安装包。
