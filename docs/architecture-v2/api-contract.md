# API 契约

API 就是边界。前端只知道路由名和领域 JSON；它们对 AT 命令、
串口、调度器、缓存键或解析器一无所知。

## 1. 路由

由模块注册在 `api/registry.rs`；从每个传输层分发的逻辑完全一致。
`*.get` 以缓存优先作答，当主题仍是冷的最多执行一次有界刷新；
`*.cached` 绝不触碰调制解调器。

路由分两类，在表本身中声明（`Route::display` / `Route::on_demand`）：

* **display**（`signal.get`、`system.temperature`……）—— 每次页面加载
  都会调用它们，因此即使调制解调器缺失或繁忙，它们也必须返回领域
  JSON；刷新失败则降级为上一次的值。
* **on-demand**（`sim.number`、`network.pdp`、`network.dhcp`、`modem.mcs`、
  `traffic.clear`、`network.registration_urc`、`network.lock_get`、
  `network.lock_apply`、`network.c5goption`、`network.c5goption_set`、
  `cell.neighbors`、`cell.scan_start`、`cell.scan_abort`、`beam.ssb`、
  `sim.slot_set`、`sim.hotplug_set`、`sim.activation_set`、
  `sim.pin_status`、`sim.pin_apply`、`system.nic_rate_set`、
  `system.power_control_set`、`system.factory_reset`、`modem.reset`、
  `modem.imei_set`、`network.radio_set`、`network.syscfg_set`、
  `system.thermal_set`、`system.led_set`、`system.thermal_thresholds_set`、
  `system.thermal_log_set`、`system.fota_start`、`system.fota_abort`、
  `modem.nr_capability_set`）—— 一次显式的用户操作，执行实时读取；
  它可能以调制解调器错误失败，由 UI 呈现。它绝不能因参数/内部
  错误而失败（registry 测试对每一条已注册路由都做了该断言）。

| 路由 | 返回（领域 JSON） |
| ----- | --------------------- |
| `signal.get` / `signal.cached` | `{sysmode, rssi, rsrp, rsrq, sinr, rscp, ecio}` |
| `network.get` / `network.cached` | `{operator, sysmode, sysmode_detail}` |
| `network.ims` | `{enabled?, registered?}` — 按需 `AT+CIREG?`；无线页面的 "IMS registration" 行读 `registered`（1 = 已注册）。拒绝该查询的调制解调器应答空对象，页面渲染为无值 |
| `registration.get` | `{state, tac, ci, act, nssai, mcc, mnc, lac}` — `state` 的 1/5 = 已注册/漫游，也就是无线页面渲染为 "Home network"/"Roaming" 的那两个值 |
| `network.rrc` | `{state?, camped?}` — 按需 `AT^RRCSTAT?`；`state` 取值 0..3（页面把它们标注为 Idle/Connected/Inactive/Invalid），`camped` 取值 98/99。固件两种应答形式都被接受：`^RRCSTAT: 1,1,98`（先 state 后标志）与 `^RRCSTAT: 1,98`（没有 state 字段 —— 此时标志不会被当成 state 读） |
| `qos.get` / `qos.cached` | `{active_cid, ambr_down_kbps, ambr_up_kbps, ambr_apn, qci}` |
| `network.pdp` | `{addresses: [{cid, address, family}]}` — 按需 `AT+CGPADDR`，供诊断面板使用 |
| `network.dhcp` | `{ipv4: {address, netmask, gateway, dhcp_server, primary_dns, secondary_dns}, ipv6: {…}, ipv6_capability}` — 按需 `AT^DHCP?`/`AT^DHCPV6?`/`AT^IPV6CAP?`（十六进制小端解码发生在这里，而不是页面里）；部分应答保留已读到的字段 |
| `network.registration_urc` | `{enabled: true}` — 幂等写 `AT+CGREG=2`；即 Info 页过去用裸 AT 开启的那份详细 PS 注册上报 |
| `network.lock_get` | `{lock_type, mobility, items: [{band, arfcn, pci, scs}]}` — `rat=lte\|nr`，按需 `AT^LTEFREQLOCK?`/`AT^NRFREQLOCK?`；`mobility,num` 行、每个载波的行以及十六进制 PCI 都在这里解码 |
| `network.lock_apply` | `{cycled_radio, results: [{rat, applied, error?, code?, verified?, verify_error?}]}` — 接受 `{rat, lock_type, mobility, items}` 或 `{locks: [ … ]}`，在一个射频周期内完成两种 RAT；分组 CSV 写在这里拼装，每个 RAT 的失败分别上报。`"verify": true` 按 CLI `lock` 动词的方式轮询锁频查询（射频周期后尝试 8 次；若射频本来就是关的则只探一次，因为固件是异步发布新锁的），并追加 `verified`/`verify_error`，这样调用方无需了解固件的时序 |
| `network.c5goption` | `{nr_sa_support_flag, nr_dc_mode, gc_access_mode}` — 按需 `AT^C5GOPTION?` |
| `network.c5goption_set` | `{applied: true, cycled_radio}` — 写操作，射频在其前后各翻转一次 |
| `network.radio` | `{airplane: bool, cfun}` — 基于 `+CFUN?` 的 display 路由；调制解调器不应答时返回空对象，这样开关保持原位 |
| `network.radio_set` | `{applied: true, airplane}` — `{airplane: bool}` → `AT+CFUN=0\|1`；network/registration 两个快照被失效 |
| `network.syscfg` | `{acqorder, band, roam, srvdomain, lteband}` — 基于 `^SYSCFGEX?` 的 display 路由；应答中带引号或不带引号的 order 字段以及末尾两个保留位都在这里处理，读取无应答则返回空对象 |
| `network.syscfg_set` | `{applied: true}` — `{acqorder, band, roam, srvdomain, lteband}` → 七参数的 `AT^SYSCFGEX` 写；已知 acqorder 列表、十六进制 band 掩码以及漫游/服务域取值范围都在这里校验（即 CLI `set-radio-policy` 的规则） |
| `network.autodial` | `{enable, dialMode, protocol, apn, username, password, authType}` — `^SETAUTODIAL?`；自动拨号关闭时固件会省略 mode，模块改用 `^NDISSTATQRY?` 补齐（`dialMode: 1` = 调制解调器自行拨号）；状态读不到时应答 `{}`，这样只读的拨号页保持它当前显示的内容 |
| `network.usb_mode` | `{mode}` — `^SETMODE?` |
| `network.interface_cfg` | `{mode, postRoute, dmz: {enabled, host}}` — 一次 `^TDCFG?` 读取同时应答拨号页的两张卡片（过去这条命令要发两次）；`Dmz: not cfg` 即 `enabled: false` |
| `network.pdp_contexts` | `{contexts: [{cid, type, apn, pdp_addr, active}]}` — `+CGDCONT?` 与 `+CGACT?` 按 cid 连接，只取 cid 1–20（0 是调制解调器自身的上下文） |
| `network.session` | `{ipv4: {connected, address, gateway, dns}, ipv6: {connected, address, dns}, capability?, mtu?, maximum_down?, maximum_up?, flow: {current_duration, current_tx, current_rx, total_duration, total_tx, total_rx}, sessions: [{cid, apn, ipv4, ipv6, type, ethernet}]}` — 一次聚合 `^NDISSTATQRY?`/`^DHCP?`/`^DHCPV6?`/`^IPV6CAP?`/`^DSFLOWQRY`/`^CGMTU=1`/`+CGPADDR`/`^DCONNSTAT?`（十六进制小端解码发生在这里，而不是页面里）；满 9 字段的 NDIS 应答由它判定连通，固件空应答退回「有没有地址」；整份不可用时返回 `null`，页面渲染为空卡。概览页的「移动 IP」卡与连接页的会话面板同读这一条 |
| `network.flow_clear` | `{applied: true}` — `AT^DSFLOWCLR`；「清空模组计数」按钮的目标 |
| `network.direct_ip` | `{enabled?}` — `^SETDIRECTIP?`；应答不是 0/1 时键缺席，页面据此禁用控件并隐藏写入按钮（旧帧解析同一条规则） |
| `network.pdp_set` | `{applied: true}` — `{cid, type, apn}` → `AT+CGDCONT=<cid>,"<type>","<apn>"`；cid 1–11、type ∈ IP/IPV6/IPV4V6、`safe_at_field` 且 APN ≤99 在这里校验 |
| `network.pdp_remove` | `{applied: true}` — `{cid}` → `AT+CGDCONT=<cid>`；cid 1–11 |
| `network.pdp_state` | `{applied: true}` — `{cid, active}` → `AT+CGACT=<0\|1>,<cid>`；cid 1–11 |
| `network.autodial_set` | `{applied: true}` — `{enabled, dialMode, protocol, apn, username, password, auth}` → `AT^SETAUTODIAL=0`（关闭时短路，不校验其余字段）或七参数形态；**尾部空字段必须省略**（MT5700M 拒绝）：apn 空时止于 `=1,<mode>,"<proto>"`、user+pass 空时止于 `"<apn>"`；mode 0–2、proto 同 PDP、auth 0–2 与 `safe_at_field`（apn ≤99 / user ≤31 / pass ≤31）在这里校验 |
| `network.direct_ip_set` | `{applied: true}` — `{enabled}` → `AT^SETDIRECTIP=<0\|1>` |
| `network.postroute_set` | `{applied, filterCleared}` — `{mode: 1\|2}` → `AT^TDCFG="infcfg","PostRoute",<mode>`；mode=1 追加 `AT^IPFILTERSWITCH=0`（两笔串行，首笔失败即止） |
| `network.dmz_set` | `{applied: true}` — `{host}` → `AT^TDCFG="infcfg","dmz","<host>"`；`"0"` 即关闭（同一串），四段 IPv4 每段 ≤255 且禁止空段在这里校验 |
| `network.schedule_get` | `{enabled, check_interval, timeout, unlock_lte, unlock_nr, toggle_airplane, night: {enabled, start, end, lte, nr}, day: {enabled, lte, nr}, status: {current_mode, next_switch, switch_count, applied}}` — 来自 UCI 的昼夜频段锁定配置（`schedule_*`），带类型且已嵌套，外加 applier 的实时状态。不访问调制解调器，因此即使调制解调器已死也能应答 |
| `network.schedule_set` | `{applied: true}` — 校验（HH:MM 时间窗、interval ≥10 s、timeout ≥30 s、锁类型 0–3）后写 `uci set at-webserver.config.schedule_*` + `commit`；applier 每 15 s 重读。`enabled`（LuCI 总开关）与 `status` 在此只读，因此没有调用方能把自己锁在该功能之外 |
| `cell.neighbors` | `{cells: [{type, arfcn, pci, rsrp, rsrq, sinr, rxlev, band}]}` — 按需 `AT^MONNC`；十六进制 PCI 转十进制、NR 的 1/8 单位换算、"no measurement" 哨兵值（255/32767/-1256/-348/-188，被丢弃因此字段缺失）以及 ARFCN→band 编号全部在这里。`band` 来自 `core::radio::arfcn_to_band`，即后端唯一的频段表；`band` 缺失意味着该 ARFCN 落在所有已列频段之外 |
| `cell.scan_start` | `{started: true}` — `{rat?, plmn?, freq?, pci?, band?, scs?}`；校验手册给出的约束（band 与 freq 互斥、PCI 仅用于 LTE/NR、NR 的 freq/PCI 必须带 SCS、band 1–512），构造 `AT^CELLSCAN[=…]`，其中包含 band **位图**（`1 << (band-1)` 的十六进制形式，按半字节拼装，因为 n78 需要第 77 位），并提交独占任务；扫描进行中以 `BUSY` 拒绝 |
| `cell.scan_result` | `{running, state, cells, count, raw?, error?}` — 最近一次扫描，来自任务注册表加 `scan` 缓存（TTL 30 分钟）；`raw` 是调制解调器 `^CELLSCAN` 应答文本的逐字副本，也就是 LuCI 弹窗与 `mt5700m-at cellscan` 打印的内容。绝不触碰调制解调器，因此扫描结束后重新加载页面仍能渲染它 |
| `cell.scan_state` | `{running}` — 仅任务自省，绝不触碰调制解调器，因此重新加载页面能找到一次比页面挂载活得更久的扫描 |
| `cell.scan_abort` | `{aborted}` — 取消扫描任务（仲裁器在线路上注入固件的中止令牌）；`{aborted: false}` 表示没有任务在跑，因此与扫描自身完成相竞的取消不算错误 |
| push `cellscan` | `{state: "done"\|"aborted"\|"error", cells: [{rat, ratName, plmn, freq, pci, band, lac, cid, rxlev, bsic, psc, scs, rsrp, rsrq, sinr, raw}], count, error?}` — 每次扫描在 `scan` 主题上发布一次；行的布局（`^CELLSCAN:`、十六进制 band/lac/cid、1/2-dB 的 RSRQ/SINR、1/8-dB 的 LTE SINR、15 字段与 14 字段的怪癖）在 `modules/cell/scan.rs` 中解码，因此没有前端需要解析它 |
| `beam.ssb` | `{servingCell: {arfcn, cid, pci, band?, rsrp, sinr, ta, ssbs: [{ssbId, rsrp}]}, neighborCells: [{pci, arfcn, band?, rsrp, sinr, ssbs}]}` — 按需 `AT^NRSSBID?`；固定偏移（服务小区 8 个槽位、每个邻区 4 个、步长 12）、"not measured" 槽位（255/32767）以及 `band` 编号（`core::radio`）都在这里处理，因此没有前端需要一张 ARFCN 表 |
| `ca.get` / `ca.cached` | `{carriers: [{radio, band, source, dl_arfcn, ul_arfcn, dl_frequency_mhz, ul_frequency_mhz, dl_bandwidth_mhz, ul_bandwidth_mhz}], secondary: [{radio: "NR", arfcn, pci, rsrp?, rsrq?, sinr?, measType} \| {radio: "LTE", index, pci, band, rssi?, rsrp?, rsrq?, ulArfcn?, dlArfcn?, ulFreq?, dlFreq?, ulBandwidth?, dlBandwidth?}], carrier_count, ca_active, dc_active, nr_carrier_count, lte_carrier_count, lte_secondary_count, secondary_connection_count, ca_mode, ca_dl_bandwidth, ca_ul_bandwidth}` (`ca.get?refresh=1` forces a live read) — `secondary` 来自 `^CASCELLINFO?` + `^MONSSC`：即 `^HFREQINFO?` 列表没有携带的每载波信号（十六进制 PCI、手册给出的无效值 -1256/-348/-188、±8 的反缩放启发式以及 `<MEASTYPE>` 都在这里解码），因此信息页按下行 ARFCN 合并两者，而不必自己解析 `^MONSSC` |
| `cell.get` / `cell.cached` | `{band, channel, dlBandwidth, arfcn, sysmode, mcc, mnc, cid, pci, lac, scs, operator, raw}` — `scs` 是 `^MONSC` 上报的 NR 子载波间隔码（0 = 15 kHz）；LTE 没有该字段，因此它缺失，无线页面也就省略它的 "SCS type" 行 |
| `sim.get` / `sim.cached` | `{status, iccid, imsi, slot, hotplug}` (+ `number` once read) — 周期快照还携带当前卡槽与热插拔开关 |
| `sim.number` | `{…, number}` — 按需读取 `+CNUM` |
| `sim.slot` | `{slot, hotplug}` — 基于 `sim` 主题的 display 路由（0 = 外置，1 = 内置） |
| `sim.slot_set` | `{switched: true, slot}` — `{slot: 0\|1}`；厂商序列（围绕 `^SCICHG` 的 `^HVSST` 去激活/激活，射频关/开）在这里执行，且只此一份 |
| `sim.hotplug_set` | `{applied: true, hotplug}` — `{hotplug: bool}` → `^TDSIMHP` |
| `sim.activation` | `{active?: bool, slot?: int}` — `^HVSST?`，即系统页显示为 "SIM power path" 的 SIM 供电路径；`slot` 是第三个字段，LuCI 也曾把它当作当前卡槽的回退值。查询不应答时返回 `{}` |
| `sim.activation_set` | `{applied: true, active}` — `{active: bool}` → `^HVSST=1,<0\|1>`；由包裹卡槽切换的同一个构造器生成，因此该动词只有一种写法 |
| `sim.pin_status` | `{code, lock, blocked, needsNewPin, card: {status, dead, present}, pinEnabled}` — `+CPIN?` 带 CME 错误分支（10 → `ABSENT`，11/12/17/18 → 对应的锁），`^SIMSQ?` 用于细化 dead/present，卡片就绪时再发 `+CLCK="SC",2` |
| `sim.pin_apply` | `{applied: true}` — `{operation: verify\|unblock\|enable\|disable\|change, pin, newPin?, pin2?}`；操作决定走 CPIN/CLCK/CPWD，4–8 位数字规则在这里校验 |
| `modem.get` / `modem.cached` | `{manufacturer, model, revision, imei}` |
| `modem.txpower` | `{total, pusch, pucch, srs, prach}` |
| `modem.nr_txpower` | `{carriers: [{pusch, pucch, srs, prach, freq}]}` |
| `modem.endc` | `{available, plmnAvailable, restricted, established}` |
| `modem.mcs` | `{downlink: {rat, carriers: [{index, group, rat, mcs_table_index, code0, code1}], avg_mcs}, uplink: {…}}` — 按需 `AT^MCS=1` / `AT^MCS=0`；`group`/`rat` 是应答自身的按行分组（LuCI 每 `^MCS` 行打印一个块，WebUI 忽略它们并按位置把载波与 `^HFREQINFO` 配对），方向层的 `rat` 是合并视图（NR 优先）；前端把 `code0` 映射为调制/等级标签 |
| `modem.reset` | `{rebooting: true}` — `AT^RESET`；快照被丢弃，因此下一次读到的是重启后的状态 |
| `modem.imei_set` | `{applied: true, imei}` — `{imei: "15 digits"}` → `^PHYNUM=IMEI,<imei>`；位数规则在这里校验 |
| `modem.nr_capability` | `{ca, vonr, dss: {rateMatchingLTE, additionalDMRS}}` — 基于 `^NRRCCAPQRY=3/2/5` 的 display 路由；每条应答都回显自己的种类（解析器据此匹配），没有应答的能力保持缺失，包括 DSS 只有一半的情况 |
| `modem.nr_capability_set` | `{applied: true, wrote: [kind…]}` — `{ca?, vonr?, dss?: {rateMatchingLTE, additionalDMRS}}`，至少给一个；每个能力各自成为一条 `^NRRCCAPCFG` 写，VoNR 的 0–3 与 DSS 的 0/1 取值范围在这里校验 |
| `traffic.get` / `traffic.cached` | PDCP 字段映射（`id`、`pduSessionId`、……、`dlDiscardCnt`） |
| `traffic.netrate` | `{available, device, rx_bytes, tx_bytes, timestamp, source, traffic}` |
| `traffic.clear` | `{cleared: true}` — 按需写 `AT^DSFLOWCLR`；调度器的动作失效机制会丢弃过期计数器 |
| `system.temperature` / `system.temperature.cached` | 12 个传感器字段 + `average` + `peak`/`peak_sensor` — 取最热的合理传感器（>0 °C、≤150 °C，`TemperatureState::peak()`），也就是 CLI 文本形态打印为 `temperature=`/`temperature_sensor=` 的那个值，也正是无线页面那一个温度仪表所显示的值 |
| `system.device_control` | `{nic_rate: 1\|2, power_control: bool}` — `^TDPCIELANCFG?` 和 `^TDPMCFG?`；不应答的那个开关被省略，页面保持它显示的值 |
| `system.nic_rate_set` | `{applied: true, nic_rate}` — `{rate: 1\|2}` → `^TDPCIELANCFG=<rate>`（重启后生效） |
| `system.power_control_set` | `{applied: true, power_control}` — `{enabled: bool}` → `^TDPMCFG=<0\|1>` |
| `system.factory_reset` | `{restored: true}` — `AT&F`（AT 默认值；调制解调器不会重启） |
| `system.service_mode` | `{mode: "serial"\|"network"}` — 守护进程如何访问调制解调器，启动时记录一次（`core::modem`）；取代了页面里的 `AT+CONNECT?` 探测 |
| `system.thermal` | `{enabled, caMimoSwitch, interval, logSwitch: {consoleLog, fileLog}, thresholds: [...], currentLevel}` — `^THERMAUTOFUN?` / `^THERMLDLOGSW?` / `^THERMLDAUTOPARA?` / `^THERMLDAUTOSTATUS?`（level 是状态行的第 6 个字段，在这里解码）；某条查询不应答则其字段保持缺失 |
| `system.thermal_set` | `{applied: true}` — `{enabled, caMimoSwitch?, interval}` → `^THERMAUTOFUN=<on>,<caMimo>,<interval>`；interval 取值范围在这里校验 |
| `system.led` | `{led?: bool}` — `^LEDSWITCH?`，状态指示灯；查询不应答时返回 `{}`，这样页面把开关保持在用户留下的位置 |
| `system.led_set` | `{applied: true, led}` — `{enabled: bool}` → `^LEDSWITCH=<0\|1>`；由模块保存，重启后生效 |
| `system.network_time` | `{time?: "…"}` — `^NWTIME?`，字符串与调制解调器格式化的结果完全一致（LuCI 页面只去掉引号）；没有时间行（即未注册）时该字段缺失 |
| `system.version` | `{buildDate?, software?, hardware?}` — `^VERSION?`（`BDT` / `EXTS` / `EXTH`）；每个键仅在调制解调器应答了对应那一行时出现，因此页面的回退逻辑（`software || revision`）继续有效 |
| `system.fota_mode` | `{mode?: "…"}` — `^FOTAMODE?` 原样透传、不做解码：把 `0,1,0,1` 称为 "HTTP update mode" 属于 UI 文案，而不是调制解调器的事实 |
| `system.thermal_thresholds_set` | `{applied: true, thresholds}` — `{thresholds: [i64; 9]}` → `^THERMLDAUTOPARA=<9 values>`；阶梯规则（9 个值、0–150 °C、触发点递增、每个恢复点低于自己的触发点）在这里强制，且只在这里 |
| `system.thermal_log_set` | `{applied: true, serial, file}` — `{serial: bool, file: bool}` → `^THERMLDLOGSW=<serial>,<file>` |
| `system.fota` | `{running, phase: "idle"\|"running"\|"done"\|"error", step, progress, state, stateName, total, received, error?}` — 升级流程的状态来自**任务注册表加已发布的快照**，绝不是一次 AT 访问，因此重新加载页面（或它的 1 s 轮询）不可能排在自己正在上报的那个下载之后 |
| `system.fota_start` | `{started: true}` — `{url}`；地址规则（只允许 `http://`、拒绝空值、补上结尾斜杠、引号防护）在这里校验，随后模块提交它的 `fota.system` 任务，该任务跑完整个流程：`ATE0`、`^FOTAMODE=0,1,0,1`、`^FOTAOEMDL="…"`，然后是状态机（每 1 s `^FOTASTATE?`，状态 30 时取 `^FOTADLQ` 进度，状态 31 时每 5 s 最多续传一次，状态 40 时 `^FWUP`）。流程进行中以 `BUSY` 拒绝 |
| `system.fota_abort` | `{aborted}` — 取消流程任务；`{aborted: false}` 表示没有任务在跑。WebUI 的页面没有取消按钮（UI 不变），因此这是终端/API 的逃生口 |
| push `fota.progress` | `{running, phase, step, progress, state, stateName, total, received, error?}` — 每次状态变化时在 `fota` 主题上发布（即时投递），与 `system.fota` 应答的是同一个对象 |
| `sms.status` | `{enabled, imsOn?, center?, storage?}` — 页面加载时的快照：`+CMGF?`（决定短信是否开启的那次读取）、`^IMSSWITCH?`、`+CSCA?`（仅当 IMS 开启，与页面一贯做法一致）和 `+CPMS?`；缺失/读不到的字段被省略，而 `enabled` 始终存在 |
| `sms.storage` | `{read, write, receive, storages: [name…]}` — `+CPMS?` 解码结果（每个存储面 `name`、`used`、`total`，清空全部短信的循环用到去重后的名字） |
| `sms.list` | `{messages: [{index, content, number, time, type, isConcatenated?, concatenatedRef/Seq/Total?}]}` — 必要时先 `+CMGF=0`，再 `+CMGL=4`；每个 PDU 都被解码，多段短信按序号合并，因此没有前端需要重组 PDU |
| `sms.send` | `{sent: true, parts}` — `{number, text}`；目标号码在这里归一化（11 位本地号码补国家码 86）并编码（GSM 7-bit / UCS-2、多段拆分、短信中心取自 `+CSCA?` — `00` 当调制解调器没上报时），然后在仲裁器线程上按每条分段一次 `AT+CMGS` 事务发送（`AT+CMGS=<len>` 只计 TPDU 的字节数）；多段发送失败时读到的文案是 `第 i/n 条发送失败：<cause>` |
| `sms.delete` | `{deleted: true}` — `{index}` → `+CMGD=<index>` |
| `sms.clear_all` | `{cleared: [storage…]}` — 自行读取 `+CPMS?`，然后按每个存储面执行 `+CPMS="X","X","X"` + `+CMGD=1,4`，并遵循固件的稳定时间（即设置页的清空所有短信） |
| `sms.storage_set` | `{applied: true}` — `{read, write?, receive?}`（缺省取 `read`）→ `+CPMS=…`；存储名在这里按 `SM`/`ME` 校验 |
| `sms.center_set` | `{applied: true}` — `{number}` → `+CSCA="<number>"`；空值/非法字符在这里被拒绝 |
| `sms.ims_set` | `{applied: true}` — `{enabled}` 执行模块的五步 IMS 序列（`+CFUN=0` → IMS PDP profile → `+CEUS` → `^IMSSWITCH` → `+CFUN=1`）及其稳定时间，两个前端与 CLI 共用同一份实现 |
| `sms.analyze` | `{encoding: "7bit"\|"UCS2", chars, parts}` — `{text}`；撰写提示里的分段数来自 `sms.send` 所使用的同一个编解码器，因此承诺与实际发送不可能互相矛盾 |
| `sms.ussd_send` | `{sent: true, reply?}` — `{code}`；码值在这里校验并打包（GSM 7-bit，手册自带的例子 `*133#` → `AAD86C3602`，以 `AT+CUSD=1,"…",15` 发出），被拒绝的码值原样回显面板的文案，若固件把应答内联返回则解码进 `reply`（`{m, mText, text, needsReply}`） |
| `sms.ussd_cancel` | `{cancelled: true}` — `AT+CUSD=2`，释放会话 |
| push `network.reject` | `{plmn, domain, domainText, cause, causeText, rat, ratText, rejectType, rejectTypeText, originalCause, lac, rac, cellId, esmCause?, raw, at}` — 由 `modules/network/reject.rs` 解码的一条 `^REJINFO` 行（手册 13.14）：原因表、USIM 的 65537–65543 区间以及 domain/rat/type 标签都在这里，而不是在页面里（`network` 主题） |
| push `qos.ambr` | `{ambr_down_kbps, ambr_up_kbps, ambr_apn?}` — 由 `modules/qos` 解码的一条主动上报 `^DSAMBR` 行（手册 5.33），字段与 `qos.get` 所用的完全一致，因此信息页无需轮询即可更新 APN/AMBR（`qos` 主题） |
| push `sim.changed` | the raw line (`+CPIN:` / `^SIMSQ:` / `^SIMST`) — `sim` 主题上的一次轻推：卡片处理器改为重新读取 `sim.pin_status`，而不是在浏览器里对 URC 文本做模式匹配 |
| push `sms.ussd` | `{m, mText, text, needsReply}` — 网络带外送来的 `+CUSD:` 行，由同一个编解码器解码（`sms` 主题）。原始行仍作为 `raw_data` 发出，但没有页面会解析它 |

字段名与类型与重构前缓存发布的完全一致，因此既有消费方
（LuCI 主题、WebUI `stateCache`、`mt5700m-at cached`）无需改动即可
继续工作。缺失字段是被省略，而不是发成 `null`，与之前的写入者
保持一致。

## 2. 传输层与信封

| 传输层 | 调用形态 | 成功 | 失败 |
| --------- | ---------- | ------- | ------- |
| WebSocket (WebUI) | `{"method":"api","params":{"path":"signal.get"}}` | `{success: true, data: {…}}` | `{success: false, error, code, retryable}` |
| WS command line (WebUI service layer) | `api.signal.get` / `api.ca.get {"refresh":true}` | `{success: true, data: {…}}` | as above |
| WS legacy | `{"method":"at","params":{"cmd":"AT+CSQ"}}` | `{success: true, data: "<text>"}` | as above |
| Control socket (CLI) | `{"cmd":"api","method":"signal.get","params":{}}\n` | `{ok: true, result: {…}}` | `{ok: false, error, code}` |
| TCP RPC (ucode/LuCI) | `{"id":1,"method":"api","params":{"path":"signal.get"}}` | `{success: true, data: {…}}` | as above |
| TCP RPC `at` (LuCI `mt5700.at`) | `{"cmd":"api.beam.ssb"}` (optional trailing JSON = params) | `{success: true, data: {…}}` | as above |
| `mt5700m-at` CLI | `mt5700m-at <verb>` | legacy text on stdout | exit code + stderr (`124` on timeout) |

信封里的 `error` 是 `BackendError::detail()`：对于被拒绝的参数，
它就是那句裸的拒绝文案（"频段与频点不能同时指定"）—— 也就是页面在
本地校验时显示的同一份文案；其他情况则是日志与 CLI 打印的同一个
`message()`。`code` 始终是 `Error::code()`。

一条路由可以通过每一种传输层访问，`api.` 前缀在 `registry::dispatch`
内部被归一化，因此 `api.signal.get`（WS/LuCI 命令行，
`split_api_command` 把可选的尾部 JSON 作为 `params` 携带）与
`signal.get`（控制套接字、JSON-RPC、CLI）命中的是同一个处理器。
前端调用路由；只有有意保留的裸 AT 控制台（`WebUI /at` terminal、LuCI
`terminal.js`）仍在发 AT。

LuCI 通过 `api.js` 的 `route(name, params)` 辅助函数触达 registry ——
即用 `cmd = api.<route> [json]` 调 `mt5700.at` —— 因此一个页面请求的
是领域模型（`api.route('beam.ssb')`），而不是一帧还需要切分的带标签
文本。纯渲染的辅助函数仍留在 CLI 动词上；页面按区块逐个迁移
（见 `migration.md` 第 9 项）。

遗留动词逐字节保留：控制套接字 `send|cached|scan`（外加 `api`；旧的
`sms` 动词已消失 —— `sms.send` 是唯一的发送路径）、TCP RPC
`at|cached|events|scan|ping`（外加 `api`）、WS 事件
`{type, data, timestamp}`，其主题名包括 `signal[.updated]`、
`network.updated`、`cell.updated`、`temperature.updated`、`traffic.updated`、
`netrate.updated`、`registration.updated`、`endc.updated`、`txpower.updated`、
`nr_txpower.updated`、`sim.updated`、`modem.info` 以及
`^(usb|modem|task|scan|beam|sms)\.` 系列。鉴权失败仍保持精确字符串
`Authentication failed`、`Authentication timeout`、`Invalid authentication`。

## 3. 超时预算（不变）

```text
frontend hard timeout 30 s
  > ucode write class 25 s / read class 12 s
    > daemon queued 10 s + exec 8 s + 2 s slack
CLI capture hard cap 25 s (exit 124)
```

## 4. 新增一项能力

1. 把路由加进所属模块的 `api.rs`（`Route { name, handler }`）。
2. 在 `api/registry.rs::routes()` 中注册它。
3. 两个前端（以及 CLI）立刻就能调用它 —— 无需传输层改动、无需
   重复解析，除非模块自己要发布，否则也不需要新的缓存键。

任何路由都必须在冷缓存、未接调制解调器的情况下作出应答：
registry 测试对每一条已注册路由都确切地断言了这一点。
