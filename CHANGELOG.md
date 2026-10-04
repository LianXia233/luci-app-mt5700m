# Changelog

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
  - 实测：页面文本 712 → 2036 字符，「MOBILE DATA / Mobile data」标题与全部控件正常渲染。

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

    实测数据：NR-5GC / RSRP -63 / RSRQ -5 / SINR 27~29 / band 41 / SIM READY / ICCID `898604891823D0000381` / 45°C。

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
- 产物：CSS 724.02 kB / JS 986.95 kB，已同步至 `htdocs/5700/`（21 文件，清除旧 hash 产物）与 `root/usr/bin/at-webserver`（805,600 B）。
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

## [2.8.5] - 2026-10-03

### Added
- **LuCI / WebUI 共享同一 Rust AT 后端（8765 单端口双协议）**：
  - `at-webserver` daemon 在 8765 端口按首字节嗅探分流：WebSocket（WebUI 用）与 newline-JSON RPC（LuCI 经 `nc` 回环转发用）。两个前端共享同一 `StateCache` / `EventBus` / `AtArbiter`，数据互相可见、串口由 daemon 独占，互相不抢占、不干扰。
  - `EventBus` 新增全局事件历史（单调 `seq` + 有界队列），RPC `events(since)` 支持增量拉取，LuCI 无需重复下发只读 AT 命令。
  - 相同只读 AT 命令命中 `StateCache` 后不再重复下发（缓存优先，SWR）。
- **新增 rpcd ucode 插件 `mt5700.uc`**：`at` / `cached` / `events` / `netrate` / `usb` / `logs`。AT 数据面经 `nc` 转发到 Rust 后端；`netrate` / `usb` / `logs` 为本地 sysfs / 文件直读，全程零 AT 流量；`at` 支持参数数组透传（`args`），解决 `sms-send` 等多词参数的空格分词丢失。
- **LuCI api.js 数据通道重构**：AT 查询由 `fs.exec` 直调 shell 改为 `rpc.declare` 调 ucode `mt5700` 对象，保留 15s 硬超时兜底；`cachedSnapshot()` 兼容 daemon RPC 包装 / `snapshot` 键 / 裸 topic 三种输出形状。
- **WebUI 拨号页改为只读**：`network/Dial.tsx` 移除全部拨号操作（连接 / 断开 / 重拨 / 自动拨号 / APN 修改），改为只读展示共享状态并提示「拨号管理请使用 LuCI」。拨号（LuCI `mt5700m-manager`）与 WebUI 互不影响。
- **WebUI 版本号单一来源**：header / footer 不再硬编码 `V3.0.0`，由 `vite.config.ts` 构建时从 `package.json` 注入 `__APP_VERSION__`，显示与实际发布版本一致。

### Fixed
- **WebUI 排版异常（缺样式类导致的间距 / 对齐 / 空态缺失）**：`global.css` 补齐 `page-stack` / `form-stack` / `action-row` / `section-header*` / `field*` / `metric*` / `kv-label|kv-value`（widgets 实际类名）/ `cell-list*` / `panel-head` / `page-card--*` / `config-select*` / `at-console*` / `sms-*` / `device-control*` / `capability-*` / `temp-*` / `carrier-grid` / `quality-grid` 等全部页面引用类，并补响应式断点。
- **Kv 组件标签/值样式失效**：widgets 输出 `kv-label`/`kv-value`，旧样式表只有 `kv-k`/`kv-v`，已补齐。

### Note
- **拨号职责划分**：拨号（连接 / 断开 / 重拨 / 自动拨号 / APN / PDP 管理）由 LuCI「网络 → MT5700M → 移动数据」独占（`mt5700m-manager` + rpcd `mt5700m`）；WebUI `/network/dial` 仅只读展示共享后端状态，不发起任何 PDP 激活 / APN 写入操作。

## [2.8.4] - 2026-10-04

### Chore
- **版本号提升并触发重新编译**：LuCI 包 `2.8.3 → 2.8.4`、Rust AT 后端 `4.0.0 → 4.0.1`、WebUI 前端 `3.0.2 → 3.0.3`。无代码变更，用于触发 CI（ci.yml 静态检查 + `cargo test`）与 release.yml 的 OpenWrt SDK 全量构建，产出全新安装包。

## [2.8.3] - 2026-10-04

### Fixed
- **修复慢 AT 命令拖垮后台采集（signal/registration/network/cell 等缓存项长期空转）**。
  - Background：部分查询（`AT^HCSQ?`、`AT^COPS?`、`AT^SYSINFOEX`、`AT^CHIPTEMP?`、`AT^HFREQINFO?` 等）在模组响应慢或弱网环境下频繁超时，而采集器沿用 fast 规格（2s 超时、失败即丢弃），缓存项常年为空，LuCI / WebUI 页面相关字段持续显示 `--`。
  - Changes（`at-webserver/src/snapshot.rs`）：
    - 新增 `stale_refresh` 续命模式：采集失败时回退 StateCache 旧值重新 store（刷新 TTL + 重推事件），模组抖动期间页面数据不归零；缓存为空时返回 `Value::Null` 且不落库。
    - 新增 `slow_query` 三件套：宽松 AT 超时 + 不重试 + Low 优先级 + 失败 `set_ttl` 退避，用于 `COPS` / `SYSINFOEX` 等慢命令。
    - `collect_signal`：`^HCSQ` 改 12s 规格 + 失败 `stale_refresh`。
    - `collect_registration`：`C5GREG` → `CEREG` → `CREG` 逐级回退循环，`has_reg` 判定注册态。
    - `collect_network`：`COPS` / `SYSINFOEX` 走 `slow_query`。
    - `collect_temperature`：`^CHIPTEMP` 改 12s/8s Low 优先级规格 + `stale_refresh`，仅非 `Value::Null` 才 store。
    - `collect_cell`：`HFREQINFO` / `MONSC` / `COPS` 采集 + 从 registration 缓存补齐 lac/cid，`cell.updated` 事件补全 PCI / 频点 / MCC-MNC / TAC / 小区 ID 字段。
    - `collect_sim` / `collect_modem_info`：`CPIN` / `ICCID` / `CIMI` / `ATI` / `CGSN` 采集补位。
- **修复 SINR / 温度等浮点字段长尾精度（如 `25.000000000000004`）**：数值字段统一 round 归一，页面显示不再出现浮点尾差。
- **修复 WebUI header 状态栏文字重复（"AT 已连接已连接"）与布局缺失**。
  - Background：`AppLayout` 渲染 full（"AT 已连接"）与 compact（"已连接"）两个状态文本 span，但 `global.css` 仍是旧版类名（`.app-header-actions` / `.app-conn-pill`），JSX 新类名（`.at-status*` / `.app-header-*`）全部缺失样式，两段文字以默认 inline 拼接显示。
  - Changes（`semi-tcpweb/src/styles/global.css`）：补齐 `.app-header-left/right/eyebrow/name`、`.at-status` 及各连接状态配色（connected/connecting/authenticating/reconnecting/error/idle/disconnected）、`.at-status-label--full/compact`（桌面显示全称、移动端显示缩写）、`.app-icon-button`、`.app-scrim`、`.app-footer` 样式。

### Note
- **模组 AT 引擎死锁为设备侧/固件环境问题（非本包代码缺陷）**：目标设备（cmiot5g 物联网卡、4G 0% 弱信号）上模组只推送 `^PDCPDATAINFO:` URC、不对任何 AT 命令（含 `AT`/`ATI`/`^HCSQ`/`^C5GREG`/`^CHIPTEMP`）应答。多轮排查确认：无 at-webserver 触碰 3 分钟仍死锁、USB authorized 复位无效、整机重启短暂恢复后复现、`logread` 无 bandlock/scan/SETAUTODIAL 命令痕迹、scheduler 未启用——排除后台命令触发。代码修复（stale_refresh / 宽松采集规格 / cell 补位）已部署，模组恢复后全量字段可正常采集（温度采集实机验证通过）。

## [2.8.2] - 2026-10-04

### Fixed
- **修复 LuCI 概览页永久停留在骨架屏、页面加载不出来（P0）**。
  - Background：`admin/modem/mt5700m` 的渲染链路是「骨架屏 → StateCache 快照帧 → AT 完整帧」。AT 后端 daemon 以 `TIOCEXCL` 独占 AT 串口，而 `mt5700m-at` 在共享控制通道失败后会回落直连串口，与独占锁形成死锁；rpcd 的 `fs.exec` 没有超时概念，于是 `Promise.all` 永不 settle，页面永久停在骨架屏，挂起请求还会占满 uhttpd 的并发进程（本机 `-n 3`）导致整个 LuCI 无响应。
  - Changes：
    - `resources/mt5700m/api.js`：所有 `fs.exec`（`mt5700m-at`）调用套一层 15s 硬超时（`Promise.race`），AT 挂起时 Promise 必然 settle，页面必然完成渲染。
    - `at-webserver/src/at.rs`：新增 `daemon_owns_serial()`；`auto_cascade` / `serial_cascade` 在 daemon 存活（控制 socket 存在）或本次请求失败时**不再回落直连串口**，改为快速失败——避免打开被独占的 tty 造成死锁；仅在 daemon 确实不可达（socket 不存在）时才直连，此时无人持有端口，行为安全。
    - `at-webserver/src/sock.rs`：控制通道读超时由硬编码 20s 改为 `2 × timeout + 10s`。原值恰好等于 daemon 侧预算（`queued_timeout + timeout + 2s` ≈ 20s），两者边界重叠时 CLI 会误判 daemon 不可用并触发串口回落——这是死锁的直接触发器。
    - `htdocs/luci-static/resources/view/mt5700m/status.js`：渲染改为真正的异步化架构。首屏只依赖非阻塞数据源（`mt5700m-at cached` StateCache 快照 + rpcd `mt5700m status` + 流量统计），AT 详情查询移出渲染关键路径，由后台异步补齐并带超时；失败时保留快照帧而不是退回骨架屏；新增 15s 快照轮询实现零 AT 流量的持续自更新（页面卸载时自动清理定时器）。
- **修复快照帧从未生效（首屏空白的直接原因，P0）**。
  - Background：`mt5700m-at cached` 输出的是裸 topic 对象（`{"signal":{"value":…,"age_ms":…},…}`），而 `api.cachedSnapshot()` 只接受 `{ok:true,snapshot:{…}}` 包装，导致快照恒定返回 `null`。叠加上面的 AT 挂起后，页面既等不到快照帧也等不到完整帧，表现为彻底白屏。
  - Changes：`cachedSnapshot()` 兼容三种输出形状（控制通道包装、带 `snapshot` 键、裸 topic 映射），首屏立即渲染最近一次后台采集结果。
- **修复 `/5700` 原厂 WebUI 无法加载数据（P0）**。
  - Background：`build-release.sh` 只把 `files/www/5700` 折入包内，漏拷 `files/www/cgi-bin`，导致 `/cgi-bin/at-ws-info` 404；前端回退到 `config.json` 里构建期硬编码的 `at.host = 192.168.1.1`，在 192.168.10.x 等非默认网段上 WebSocket 连接到不可达主机，`/5700/#/network/info` 等页面永远拿不到数据。
  - Changes：`build-release.sh` 补拷 `files/www/cgi-bin`（`at-ws-info`、`at-log-clear`）并赋予 0755。`at-ws-info` 通过 `HTTP_HOST` 动态返回客户端实际访问的主机，任意网段均可正确发现后端。
- **修复 LuCI JS 视图在 luci-base 26.275 上无法加载（P0，运行时兜底）**。
  - Background：该版本 `luci.js` 在加载模块时调用 `String.prototype.format`，但该 polyfill 的定义只存在于 `cbi.js`，加载顺序滞后；缺失时 `L.require()` 抛错，所有 JS 视图（含登录表单）失效。
  - Changes：在目标设备的 `/www/luci-static/resources/luci.js` 头部注入幂等 polyfill（覆盖 `%s`/`%d`/`%i`/`%f`/`%%`），`cbi.js` 加载后由其完整实现自动接管。
  - 注意：该文件属于 `luci-base` 而非本包清单，此项是**设备侧运行时兜底**，不含本次代码提交；升级 `luci-base` 后需重新注入。已在 README「故障排查」记录该现象与注入片段。

### Changed
- **排版与视觉一致性修复**（`resources/mt5700m/style.css` 追加式补丁，不改动既有 `mt-` 类语义）：
  - 8 个基础色令牌（`--mt-surface/-soft/-inset`、`--mt-text/-muted/-faint`、`--mt-border/-soft`）原只认 luci-base 的 `--*-color-*` 变量，而 aurora 主题提供的是 `--surface/--text/--hairline` 等，导致明暗两模式均解析为写死浅色回退值——暗色下正文近黑压深底完全不可读、短信页出现刺眼白底。改为「aurora 令牌 → luci-base 令牌 → 硬编码回退」三级链，明暗各一套。
  - 长标识串溢出：`c.row()` 值节点补 `min-width:0` 与 `overflow-wrap:anywhere`，标签改为可收缩，修复 IMEI/ICCID/IMSI/IPv6 撑破卡片。
  - 窄屏栅格溢出：10 处 `minmax(Npx,1fr)` 改为 `minmax(min(Npx,100%),1fr)`，消除 320px 手机上的横向滚动条。
  - 信号柱容器由写死 16px 改为 `height:auto`，修复柱体（最高 47px）向上压住 RSRP 大数字。
  - 骨架屏尺寸对齐真实内容并保留视口高度，显著降低 CLS 抖动；暗色下骨架条改为可见。
  - 数值列统一 `tabular-nums`，消除 15s 轮询刷新时的数字左右抖动。
  - 补齐 JS 已使用但 CSS 未定义的类（`mt-view`、`mt-sms-chat-shell`、`mt-pdp-state`、`mt-session-columns`、`mt-ssb-serving-*`）与缺失的 `--mt-warn-border` 令牌。
  - 暗色下写死 rgba 改为新 `--mt-tint-*` 令牌；`svgCarrier()` 白字压亮蓝圆的对比度问题在暗色下改为深字。
  - 新增 768–1200px 断点调优与 `prefers-reduced-motion` 降级。

## [2.8.1] - 2026-10-02

### Chore
- `PKG_VERSION` 提升至 `2.8.1`，作为 2.8.0（玻璃拟态 UI 重构）的补丁级发布；前端样式表版本戳同步更新为 `style.css?v=2.8.1`，客户端升级后即时刷新样式缓存。
- 仓库维护：清理 GitHub 历史 Releases（删除旧 `manual-v2.7.0-*` 快照及关联 git tag）与陈旧分支（已合并 PR 的工作分支 `arena/01a0867a-*`、`copilot/refactor-*`、`refactor/luci-frontend-v25`、`fix-apn-dash-display`），仅保留 `main`。

## [2.8.0] - 2026-10-02

### Added
- **全新现代白色毛玻璃（Glassmorphism）视觉设计系统与全页面 UI 重构**：
  - 重构全部 8 个 LuCI 视图（`status` 概览、`connection` 移动数据、`network` 网络与小区、`sms` 短信、`system` 系统信息、`terminal` AT 终端、`advanced` 高级设置、`settings` 通信诊断），打造精致统一的白色半透明磨砂毛玻璃卡片质感（`backdrop-filter: blur(16px)`、半透明白底、双层立体光影微边框与悬浮微升交互）。
  - 新增环境流光背景网格（`mt-ambient-mesh`）与页面背景融合渲染，提升界面空间层次与现代科技感。
  - AT 终端升级为 macOS 亚克力磨砂终端窗口风格，集成红黄绿三色控制按钮、磨砂顶栏与专用等宽终端控制台。
- **高品质动态动画 SVG 矢量组件系统**：
  - `svgTower`：动态 5G 信号塔，具备 3 级同心射频波纹循环发射动画与高光信号脉冲。
  - `svgCircularGauge`：仪表级环形 SVG 进度环，带平滑渐变与动态旋转光晕，用于 5G/4G RSRP/RSRQ/SINR 信号质量与硬件温度监控。
  - `svgChip`：硬件 SoC 核心芯片矢量图形，内部配备呼吸流光与动态引脚脉冲，应用于系统、硬件及高级配置页面。
  - `svgCarrier`：多载波聚合（CA）轨道动态指示器，主辅载波环绕卫星轨道动态旋转。
  - `svgTrafficArrows`：双向收发流量动态光流箭头，直观指示当前实时数据上下行吞吐。
  - 动态状态呼吸光球（`svgStatusPulse`）、纸飞机发送图标（`svgSendIcon`）、旋转刷新图标（`svgRefreshIcon`）与 WebUI 跳转图标（`svgWebUiIcon`）。
- **独立 WebUI（`/5700`）全新 Kawaii Minimal 软萌粉彩视觉重构**：
  - 设计语言全面升级为 Kawaii Minimal 风格：采用马卡龙柔和粉彩配色（粉、紫、青、黄、暖白），搭配果冻弹性微交互（jelly bounce、squishy press）、圆润糖果卡片与柔和阴影，消除冷硬科技感。
  - 动态 SVG 组件系统升级：新增 `SvgAmbientMesh`（柔和浮动粉彩气泡背景）、`SvgBrandLogo`（萌系圆润基站/路由器 Logo）、`SvgConnectionPulse`（果冻状态呼吸灯）、`SvgSignalTower`（圆角糖果 5G 基站与多彩信号阶梯）、`SvgDataStream`（粉紫双向流动微粒数据流）与 `SvgRadarScanner`（粉彩雷达圆环扫描仪）。
  - 集成至全站布局（`AppLayout`）、网络信息页（`Info`）与基站扫频面板（`ScanPanel`），完成生产环境 bundle 打包与静态资源同步（`at-webserver/files/www/5700`）。
- **LuCI 前端 100% 完整中文化（Localization）**：
  - 补充补全 `po/zh_Hans/mt5700m.po` 中所有缺失的翻译词条（`OVERVIEW`、`MESSAGING`、`ADVANCED`、`TERMINAL`、`DIAGNOSTICS`、`SETTINGS`、`WebUI`、`No messages yet.`、`Phone number…`、`Received`、`Sent` 等）。
  - 规范并包裹所有前端视图中的用户可见文本与 Badge 标识（如 `_('AT Console')`、各页面 Kicker 导航标签），修复 PO 文件中多处格式换行缺失问题，实现前端界面 0 缺漏中文化覆盖。

### Changed
- 样式表版本号升级为 `2.8.0`（`style.css?v=2.8.0`），确保客户端升级后即时刷新样式缓存。
- `PKG_VERSION` 升级为 `2.8.0`。

## [2.7.0] - 2026-09-29

### Added
- **Rust 后端升级为异步化架构（Async Architecture）**：在「独占 AT 通道」基础上重构为**事件驱动 + 异步任务调度 + 状态缓存 + 实时事件推送**。任何单个慢 AT 操作都不能阻塞 LuCI / WebUI / WebSocket，页面打开不再等待模组，多个页面共享缓存、不再重复请求，后台轮询统一收敛。新增模块：
  - `error.rs` 统一错误模型：`BackendError`（含 `retryable` 标记的机器可读 `code` + 人类可读 `message`），错误码 `AT_TIMEOUT / AT_REJECTED / BUSY / MODEM_UNAVAILABLE / TRANSPORT_ERROR / TASK_TIMEOUT / TASK_CANCELLED / INTERNAL_ERROR` 等，禁止打印敏感数据。
  - `task.rs` 任务模型：任务生命周期 `Queued → Running → (Completed | Failed | Cancelled | Timeout)`，含 `Priority`；按耗时档次（Fast/Normal/Network/Scan/Long）使用不同超时与重试策略。
  - `task_manager.rs` 任务调度中心：注册/跟踪/取消/超时/周期任务，发布 `task.*` 生命周期事件，记录裁剪与不重叠执行，后台采集统一在固定间隔运行且不互相竞争。
  - `at_queue.rs` 统一 **AtArbiter**（AT 请求队列/仲裁器）：单执行线程串行化所有 AT 交换（WebSocket 命令、LuCI 控制套接字、后台采集、扫频、锁频），支持优先级调度、请求去重、超时、统一重试、独占通道、协作取消、背压。
  - `state_cache.rs` 状态缓存（SWR）：signal / network / registration / temperature / traffic / cell / sim / modem / usb 等每项含 `value + timestamp + ttl + source + status`；页面读到过期值立即返回（Stale-While-Revalidate），同时后台刷新。
  - `event_bus.rs` 事件总线：主题订阅/发布，高频遥测 100ms 窗口合并（coalescing），task / usb / modem / sms / scan 即时直推。
  - `snapshot.rs` 后台状态采集器：定期经 AT 抓取快照写入 StateCache，开机预热水军一次采集多个缓存项。
  - `device_monitor.rs` USBNotify 热插拔监视：USB add/remove 时 invalidate cache、取消无效任务、重连 transport、刷新模组状态并推事件。
  - `runtime.rs` 轻量线程运行时工具：`spawn_thread`、`next_id`、`now_ms`，保持 std-only 极低依赖、静态链接与 OpenWrt 交叉编译稳定。
- **CLI 新增 `cached` 子命令（加法兼容）**：`mt5700m-at cached [range]` 从 StateCache 读取最近状态快照，模组不可用时也能秒回；既有 CLI 契约不变。

### Changed
- **重接线 `daemon.rs`**：HTTP/控制套接字/WebSocket 全部改为「读缓存快照 + 下发任务」模式，不再同步等待 AT；即时写操作进入任务队列返回 `task_id`，慢操作后台执行，事件经 EventBus→WebSocket 推送。
- **重接线 `scheduler.rs`（band-lock 锁频）**：锁频过渡作为任务进入仲裁器，独占 AT 通道，不再与页面查询争抢。
- **WebSocket 改为事件驱动**：新增 `subscribe` / `unsubscribe` / `snapshot` 控制帧；前端可订阅主题，服务器按订阅推送 `{type, data, timestamp}` 事件，支持断线重连后重新订阅并拉取快照恢复。
- **LuCI 前端缓存优先 + 事件订阅**（`api.js` 新增 `cachedSnapshot()`、`status.js` 双阶段 SWR 渲染）：首屏优先读 StateCache 快照零 AT 秒开，再经 WebSocket 事件自动更新。
- **WebUI（React）事件驱动 + snapshot fallback**：`services/at.ts` 支持 `requestSnapshot()` 与状态事件；`network/Info`、`system/Info`、`NotificationHandler` 改为接状态事件并优先用快照渲染首屏。

### Performance
- 缓存命中 <1ms，页面打开不再等待 AT；空闲时后台轮询统一收敛、减少串口与 CPU 占用；重复请求合并为单次 AT。

### Docs
- `README.md` 新增「异步化架构（Async Architecture）」章节：总体分层、关键机制（统一 AT 仲裁器/去重/StateCache SWR/事件总线）与前端配合方案。

### Added
- **前端加载性能测量环境 `tests/perf-harness/`**：`server.js`（before/after 双资源树 + 可调路由器延迟的 ubus/AT mock，全部响应 `no-store` 保证冷首访）、`luci-shim.js`（复刻 luci.js「先取视图、逐层并行取依赖」加载时序的迷你加载器与框架垫片，标记 shell/content/style 时刻）、`page.html` 与 `measure.sh`（Playwright 批量采集 `window.__perf`）。模拟延迟（静态 6ms/文件、ubus 20~30ms、AT 150/180ms）下各跑 5 次取中位：前端可见 319ms → 82ms（−74%）、后端 5s 延迟时白屏 5119.8ms → ~70ms 出骨架、传输字节 121.6KB → 91.1KB（−25%）。
- **`scripts/minify-luci-frontend.sh`**：用 esbuild 只压缩 `luci-static/resources/{mt5700m,view/mt5700m}`（不触碰折入同一 htdocs 的 /5700 React 包，故 `LUCI_MINIFY_*=0` 保持不变），逐文件 `node --check` 并断言 LuCI 模块加载器赖以生存的引号 `require …` 指令压缩前后数量不变；`build-release.sh` 在 `make compile` 前调用，CI 新增 “Check LuCI frontend minification” 校验步骤。应用侧 12 个静态文件 207.9KB → 147.2KB（−29%）。

### Changed
- **LuCI 六个视图改为渐进渲染，前端首屏不再等待后端数据**：`status / connection / network / system / sms / advanced` 的 `load()` 发起数据请求后立即返回，`render()` 先绘制骨架屏（`components.js` 新增 `c.skeletonPage()`；`style.css` 新增 `.mt-skeleton-*` 脉冲样式，经 `--mt-*` 令牌自动适配暗色），ubus/AT 数据到达后由原渲染函数 `renderPage()` 整体替换骨架，数据链路失败时显示错误条。模组 AT 单次 200ms~数秒的延迟只影响填充时机，不再拖住首屏（实测后端 5s 下 70ms 出骨架）。数据请求同步并行化：概览页四路数据（managerStatus / AT×2 / 流量）全并行，移动数据页两条 AT 提前到与 manager/接口状态并行，完整页面再提前一个 ubus 往返。页脚与表单时序已对照 luci.js 源码核实——`addFooter()` 在 render 后统一创建、`handleSave`/`handleReset` 点击时才扫描 `.cbi-map`，骨架期不破坏 Save/Reset 绑定；渲染函数本体未改动，内容与改造前逐像素等价。
- **`style.css` 由 `render()` 内插入改为组件模块求值即注入 `<head>`，并携带 `?v=2.5.0` 版本戳**：原实现把 `<link>` 放在 `render()` 返回的 DOM 里，样式表排在整条数据瀑布之后（实测注入点 = 内容渲染时刻 303.7ms，比模块加载晚两波 ubus/AT 请求，内容先出现、样式后到，冷启动 34ms 无样式窗口）；现与数据请求并行下载（实测 67.9ms 注入、82ms 就绪）。模块 JS 本就走 `?v=resource_version`，`L.resource()` 的 CSS 此前无版本参数，升级后可能命中启发式缓存里的旧样式表——版本戳使包升级立即失效。

### Docs
- 「系统拓扑与多层协同架构」章节的 Mermaid 流程图替换为高清架构位图（`docs/architecture.png`），与后端独占串口、双前端共用控制通道的当前实现保持一致。

## [2.6.0] - 2026-09-22

### Changed
- **彻底重构 AT 后端，移除 `ubus-at-daemon` 与 `sms-tool_q` 两个第三方依赖**。Rust
  后端（daemon 模式）现在**独占**打开 MT5700M PCUI 串口（Linux `TIOCEXCL` + 常驻描述符）。
  LuCI 的 `mt5700m-at` 改为经本地控制套接字（`/var/run/at-webserver.sock`）向 daemon 下发
  AT 指令，WebUI 走 WebSocket——二者共用同一独占串口，天然串行化，替代旧版
  `ubus call at-daemon sendat` 的“共享通道”职能；daemon 不在时 CLI 自动退化为独立直连串口。
- **短信发送改为进程内纯 Rust PDU 编码**（`sms.rs`）：GSM-7 默认字母表 / UCS-2，长短信
  自动分片为多部分并携带拼接信息元（UDHI），中文短信可靠；短信读取沿用 `AT+CMGL` PDU 解码。
- **串口自动扫描 + 手动选择**：默认 `connection_type=SERIAL`、`serial_port=auto`，开机自动
  枚举 `/dev/ttyUSB*` 与 `/dev/ttyACM*`，按 VID/PID（`3466:3301`）、接口类型（`ff:06:12`）
  识别 PCUI 端口，必要时以 `AT` 应答探测兜底；`mt5700m-at port scan` 可查看、`port set
  <path|auto>` 可手动选择。
- **连接模式精简**：删除 `UBUS` 模式，仅保留 `SERIAL`（默认）/`NETWORK`；`AT^PDCPDATAINFO`
  等 URC 推送在 SERIAL 下原生生效，移除原先仅用于 UBUS 的轮询模拟。
- 构建与打包同步更新：`Makefile` 仅依赖 `+luci-base`，`build-release.sh` / `release.yml` 不再
  拉取 `qmodem` feed、不再编译/发布 `ubus-at-daemon` 与 `sms-tool_q`。

### Fixed
- **修复 SERIAL/NETWORK 模式下 `send()` 与 `stream_command()` 对同一非重入 `Mutex` 二次加锁
  导致的死锁**（旧代码默认 UBUS 模式掩盖了此问题）：改为仅由 `stream_command` 负责持锁。

## [2.5.0] - 2026-09-22

### Changed
- **LuCI 前端重构为分层 ES2018+ 架构（v2.5）**：共享设计系统样式（`resources/mt5700m/style.css`）、组件库（`components.js`）、统一数据通道（`api.js`）与纯 AT 解析层（`parser.js`）；基于 LuCI2 框架，使用 `:root[data-darkmode]` 主题变量与 `L.resource()` 样式加载，要求 luci-base >= 25.12。

### Fixed
- **修复 LuCI 概览页所有卡片内容显示为 `[object HTMLDivElement]` 的问题**。根因：`components.js` 的 `card()` 把「返回数组的 body」再包进一层 children 数组，而 LuCI 的 `E()`/`dom.append` 只展平顶层 children 数组——嵌套数组会被当作标量 `String()` 强转，逐项变成逗号连接的 `[object HTMLDivElement]`。现把 body 展平进顶层 children 数组，卡片正文正常渲染。

## [2.4.8] - 2026-09-10

### Changed
- **移除未使用的上游独立软件包**：删除 `mt5700webui-openwrt-server/at-webserver/Makefile`（OpenWrt 独立包 Makefile，Rust 后端实际由 cargo 编译后折入 `luci-app-mt5700m`）、`mt5700webui-openwrt-server/luci-app-at-webserver/`（上游 LuCI 集成包，含 config / debug / logs 页面，从未参与构建也从未被依赖）与 `mt5700webui-openwrt-server/prebuilt/` 下的旧预编译 APK（at-webserver 3.0.2 及 i18n 包，仅作历史参考）。发布构建入口 `scripts/build-release.sh` 本就不消费这些产物，因此对已安装包的运行时行为没有影响。
- 同步更新 `VENDOR.md`、`mt5700webui-openwrt-server/README.md`、`at-webserver/README.md`，明确所有内容以**单个 `luci-app-mt5700m` 包**交付，不再存在独立安装入口。

### Chore
- 仓库清理了 v2.4.7 及其之前的 Releases 页面历史条目（保留策略：仅留最新版本），历史修复记录仍完整保留在本文件中。

## [2.4.7] - 2026-09-07

### Fixed
- **消除开机启动竞争窗口（上电重启实测发现）**：at-webserver 与 ubus-at-daemon 同为 `START=99`，rc.d 按字典序执行使前者先启动。实机用 `/proc/<pid>/stat` starttime 测量：mt5700m-manager T+16s、at-webserver T+18s、ubus-at-daemon T+28s——即开机后有约 10 秒窗口，WebSocket 端口已监听但 `at-daemon` 对象尚未注册，此时 WebUI 的每条 AT 命令都会失败（表现为面板报错/全零），需等待或手动刷新才恢复。
  现 daemon 在 UBUS 模式下**绑定 8765 端口前**先轮询 `ubus list at-daemon` 就绪（间隔 500ms，上限 60s）：窗口期内端口不开放，前端表现为"连接失败→自动重连"，就绪后立即监听，不再产生失败命令。超时（如未安装 at-daemon 或走其它模式）仍照常启动，不阻塞开机；SERIAL/NETWORK 模式不受影响。
  注：判定启动时序不能用 `logread` 时间差——开机初期 NTP 校时会扭曲时间戳（本次日志显示间隔 33s，实测仅 10s），须用进程 starttime/CLK_TCK。
- 测试 36/36（新增 3 个就绪门控用例：轮询至成功、超时不早退、首次探测立即返回）。

## [2.4.6] - 2026-09-06

### Fixed
- **速率字段实值化（实机打流联调发现速率恒 0）**：2.4.5 的 `pdcp_data` 事件结构正确但 `ulPdcpRate`/`dlPdcpRate` 恒为 0。实机三路排查确认模组侧速率在 UBUS 模式物理不可达：(1) `AT^PDCPDATAINFO?` 查询的速率字段恒 0——模组只在周期性 URC 推送里计算速率，查询只报缓冲区统计与两个单调计数器；(2) 末尾两个计数器经下载/上传对照实验证实不是 UL/DL 字节计数（双向同步增长、差值仅几十字节），无法差分；(3) 使能模组推送后积累的 URC 被 `ubus-at-daemon` 丢弃、不随 sendat 应答返回，且对象无订阅方法、网络 AT 端点 20249 不可达。改由 daemon 从默认路由接口（/proc/net/route 解析，实机为 eth2）的 `/sys/class/net/<dev>/statistics/{rx,tx}_bytes` 差分计算实时速率（bytes/s，与前端 `*8/1e6` Mbps 换算一致），覆盖事件的速率字段；缓冲区统计仍取自模组查询。处理计数清零回绕（saturating_sub）与采样暂停后恢复的大间隔尖峰（>5s 窗口只刷新基线不计速率；<0.2s 抖动忽略）。前端速率曲线、峰值、累计图表全部可正常工作。

## [2.4.5] - 2026-09-06

### Fixed
- **修复 WS 事件广播信封格式（实机 PDCP 联调发现，自 2.4.0 起所有推送事件均不可被前端消费）**：Rust `broadcast()` 把事件对象平铺到消息顶层（`{"type":"pdcp_data","ulPdcpRate":...}`），而 Python 后端契约与前端消费路径统一为嵌套信封 `{"type":...,"data":{...}}`（`msg.data.xxx`；`raw_data` 还要求 `typeof data == "string"`）。平铺导致 `data` 为 undefined：PDCP 速率面板收不到字段、来电提醒（`t.data.number/time/state`）与短信通知（`t.data.sender/content/time`）条件永不满足、`raw_data` 类型检查失败。现统一按 Python `ws.broadcast(type, data)` 包一层 `data`；dispatcher 侧 `new_sms`/`raw_data` 的事件负载去掉多余内层包装（`raw_data` 负载改为纯字符串）。实机 WS 端到端验证：采样开关 ON 后 `pdcp_data` 事件按 750ms 到达且字段有值，OFF 后 3s 内零事件。回归测试 33/33（新增信封嵌套断言）。
- 注：2.4.4 的 PDCP 轮询模拟本身工作正常（事件节奏、开关启停均正确），仅信封格式错误；2.4.4 已发布但建议直接跳过安装。

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
