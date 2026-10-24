# 时间同步与 DNS

两个能力彼此独立，但共享一条原则：**保存、应用、运行时验证是三种状态**，
任何一层失败都不能被另一层的成功掩盖。

## 1. 时间服务

### 1.1 统一时间接口

所有模块只通过 `time.now`（或后端 `service::time_json()`）读取时间：

| 字段 | 含义 |
|:---|:---|
| `unix` / `unixMs` | 权威 Unix 时间戳，与时区无关 |
| `local` | 设备时区的本地时间文本，仅用于显示 |
| `timezone` | 设备配置（`system.system.timezone` / `/etc/TZ`），插件不硬编码 |
| `plausible` | 系统时钟是否晚于 2020 年 |
| `synced` / `trusted` / `lastSuccessAt` | NTP 运行时证据，未同步时为 `false`/`null` |

时区只影响 `local` 的显示；`unix` 永远不做二次偏移。未同步时页面明确显示
“不可信”，绝不把“配置了服务器”当成“已同步”。

### 1.2 提供方检测与所有权

`modules/timesync/provider.rs` 按以下顺序检测，**只使用一个时钟控制器**：

1. 正在运行的 `chronyd` → chrony（读取 `chronyc tracking` / `sources`）；
2. `/etc/init.d/sysntpd` 存在 → OpenWrt 系统 `sysntpd`（写 `system.ntp`，
   由 procd 管理）；
3. 正在运行的外部 `ntpd` → 只读状态，不修改其配置；
4. 存在 chrony 配置/初始化脚本 → chrony（应用时通过系统 init 启动/重启，
   仍由系统服务持有，不产生插件自己的进程）；
5. 都没有 → `none`，保存配置可用，应用明确失败。

chrony 配置使用 `# BEGIN mt5700m-managed` / `# END mt5700m-managed` 标记块，
只替换插件自己的块，保留其它行。`sysntpd` 路径只改 `system.ntp` 的
`enabled`/`server`，不新增第二个守护进程。

### 1.3 应用、同步与恢复

- `time.apply`：校验配置 → 备份原配置（仅首次）→ 写入检测到的服务 →
  reread/restart（或 `chronyc reload sources`）→ 写入运行时 `apply.result`。
- `time.sync`：只通过系统时钟控制器触发，绝不并排再起一个 `ntpd`：
  - 运行中的 chrony → `chronyc burst` + `makestep`；
  - `sysntpd` → 重启 procd 管理的系统实例（busybox ntpd 没有“立即查询”命令），
    再用 `ntpq`/`adjtimex` 轮询证据；
  - 外部 `ntpd` → 只读，明确返回“不会启动第二个 NTP 进程”；
  - 完全没有系统服务 → 才使用单次 `ntpd -nq -p <server>`，该进程在返回前退出。
  保存但未应用时拒绝同步并提示先“应用到系统”，避免用另一组服务器冒充结果。
  结果分 `dns`/`network`/`timeout`/`server`/`clock`/`config`/`internal` 错误类别；
  系统服务不提供可读状态时明确记为 `unverified`，不伪造成功。
- 自动重试有退避上限（6 次后停止），日志去重，避免离线时刷屏。
- 服务重启、网络断开、时钟异常：状态文件是运行时文件，重启后从“未知”开始；
  `sync` 失败不阻断页面、拨号或 AT 通道。NTP 与 AT 调度完全解耦，不占用
  AT 串口（`^NWTIME` 只用于诊断展示）。

### 1.4 默认服务器

1. `ntp.tencent.com`
2. `time.cloudflare.com`
3. `ntp.aliyun.com`

保存在 `mt5700m.ntp.server`（有序列表），`mt5700m.ntp.enabled` 控制插件是否
管理。升级时已有 section（包括空列表）不被覆盖。

## 2. DNS

### 2.1 唯一应用路径

`/usr/share/mt5700m/dns.sh apply` 是唯一写接口 DNS 的地方：

- 只写 `network.MT5700M.dns` / `network.MT5700Mv6.dns` 并设置 `peerdns=0`；
- 不改全局 DNS、LAN、WAN、DHCP、dnsmasq、路由；
- 每次调用都重新读取 `mt5700m.dns`，不缓存上次的值；
- 先校验字面量；启用族必须有主地址；无效值回退到默认并记录 `partial`；
- 写前比较当前值，只有变化才 `uci commit network` + `network reload`；
- 运行时状态写入 `/var/run/mt5700m/dns.state`（原子替换）。

### 2.2 所有启动/up 入口

| 入口 | 行为 |
|:---|:---|
| `/etc/init.d/mt5700m-manager start` | 先 `sync`，再 `dns.sh apply` |
| `reload` | 同上 |
| `mt5700m-manager monitor` → `ensure_network()` | 每轮（15 s）在接口配置后调用 `dns.sh apply` |
| `mt5700m-manager connect/redial` | 经 `sync_manager` → `ensure_network` |
| USB add hotplug | 1 s 后 `mt5700m-manager sync` |
| `/etc/hotplug.d/iface/95-mt5700m-dns` | `MT5700M`/`MT5700Mv6` `ifup` 时立即 `dns.sh apply` |
| LuCI `dns.apply` | 异步任务调用同一个脚本并验证运行时 |

脚本可重入；`flock` 串行化热插拔与管理器轮询，避免并发 `network reload`。

### 2.3 保存 / 应用 / 验证

- `saved`：UCI `mt5700m.dns`；
- `applied`：`dns.state` 中上次脚本写入的服务器与结果；
- `runtime`：后端读取 `network.interface.MT5700M` / `MT5700Mv6` 的
  `ipv4-address`/`ipv6-address` 与 `dns-server`，逐族比较；
- `verified`：运行时解析器与保存值完全一致才为 `true`；接口不存在/未 up 时
  为 `null` 并给出原因，IPv6 不可用不影响 IPv4。

### 2.4 默认值

| 族 | 主 | 备 |
|:---|:---|:---|
| IPv4 | `119.29.29.29` | `223.5.5.5` |
| IPv6 | `2402:4e00::` | `2400:3200::1` |

旧版 `mt5700m.connection.dns_list` 在升级时一次性迁移到新 section；之后不再读取。

## 3. 依赖顺序

NTP 服务器是域名，必须先有 DNS 与网络。插件启动顺序：

1. netifd/dhcp 拿到地址与插件 DNS（`dns.sh` 已设置 `peerdns=0`）；
2. `sysntpd`/chrony 启动并按已保存服务器解析域名；
3. 解析失败归类为 `dns`，网络不可达归类为 `network`；DNS 更新后下一次重试会
   重新解析，不需要重启插件；
4. 时钟未同步不会阻止 NTP 重试，也不会阻止拨号、短信、按键等核心功能。

## 4. LuCI 页面

`view/mt5700m/time.js`：

- 顶部统一 Tab（URL 路由选择页面），内层 `ntp`/`dns` Tab 写在
  `?timeTab=`，刷新/后退/前进都恢复；
- 面板只构建一次，轮询与 Tab 切换不重建表单，未保存输入保留；
- NTP：总开关、服务器有序列表（启用/禁用/上移/下移/删除/新增）、恢复默认、
  保存、应用到系统、立即同步；长操作走 `task_id` + `waitTask()`；
- DNS：IPv4/IPv6 分族开关、主/备地址、恢复默认、保存、保存并应用；
- 明确展示“已保存 / 已应用 / 运行时验证”、上次错误类别、IPv6 跳过原因；
- 页面卸载停止轮询并中止任务观察；后端任务继续执行。
