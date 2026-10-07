#!/usr/bin/env node
'use strict';

/*
 * 概览页（status）一致性证明（LuCI 概览页 → 统一路由，前端最后一条 CLI 状态帧消失）
 *
 * 这一刀切掉的是概览页详情帧里的两条 CLI 文本帧：`mt5700m-at status`（CLI 把
 * 同一份 StateCache 渲染第二遍，4 行「实时补充」还自己另开 AT 通道现问模组 ——
 * 本项目明令禁止的「第二个 AT 串口持有者」）与 `mt5700m-at advanced session`
 * （八个慢命令的文本转储，前端拿正则从里面取值）。现在：
 *   - APN：`network.pdp_contexts`（+CGDCONT?/+CGACT?，旧 CLI 取的是同一条
 *     命令的 cid 1），取不到再退到 `qos.get` 的 ^DSAMBR APN；
 *   - QCI / AMBR：`qos.get`（WebUI 的 Info 页读同一组字段）；
 *   - 手机号：`sim.number`（+CNUM；`numberState=not_stored` 等于旧 CLI 的
 *     `phone_number_state=not_stored`，即 +CME ERROR: 22）；
 *   - 会话（移动 IP 卡）：`network.session`，与连接页同一条路由；
 *   - usb_state：`usb` 主题（串口存在性采集器），设备不在时与旧 CLI 一样映射
 *     成 `absent`；
 *   - 删除：`api.js` 的 `atStatus`/`atSession` —— 概览页不再有任何 CLI 调用。
 *
 * 用法：
 *   node scripts/prove-status-parity.js [基线]   # 基线默认 8a8c501（本刀之前）
 * 退出码 0 = 通过，1 = 不一致，2 = 基线选错了（基线里已经没有旧实现）。
 *
 * 关键设计：两侧喂的是**同一份事实**的两种表达 —— 旧侧是 CLI 文本帧
 * （cmd_status + print_cached_status + query_extras 的逐字形状），新侧是快照
 * 主题 + 三条路由的领域载荷。因此渲染结果逐字相同，等于说「路由给出的
 * APN/QCI/AMBR/号码/usb_state 与 CLI 现问模组得到的完全一致」。
 *
 * 另外两条关于旧实现的语义记录（本脚本都会断言）：
 *
 * 1. baseline 的 `mergeStatusLines(CLI, 快照)` 是「快照行覆盖 CLI 行」（同名键
 *    后者胜），所以 temperature 一直是快照里的原始浮点（45.1），CLI 那边的
 *    `round()` 取整从来没有显示过 —— 这一刀之后仍然是 45.1，不是「改了行为」。
 *
 * 2. **旧详情帧其实是坏的**：baseline 的 `refreshDetail` 把
 *    `mergeStatusLines(oldNative, native.stdout)` 的第二个参数传成了字符串
 *    （函数内部对它 forEach），必然抛 TypeError，被链尾的 `.catch` 抓住 →
 *    页面常年挂着一条「Some modem details could not be refreshed. …
 *    (overrides || []).forEach is not a function」告警条，而 CLI 帧里的
 *    **所有**行（不只是那 4 行补充值）都没进过 frame。
 *
 * 比较口径因此是三条：
 *   A. 主比较：**部署版基线**（不打补丁，就是用户在设备上看到的那一版）vs 新实现，
 *      正文（告警区与地址卡除外）必须逐字相同。那两块单独断言：
 *        · 告警区：旧侧是那条 TypeError 横幅；新侧要么没有，要么是 usb 主题真正
 *          驱动的「升级模式」横幅（旧实现写了这条横幅，却从来没有数据喂给它）。
 *        · 地址卡：旧侧被同一个 TypeError 一起打断（异常发生在设置
 *          `state.sessionDetail` 之前），所以「移动 IP」卡在部署版里**永远是空的**；
 *          新实现两条路径互不干扰。
 *   B. 打补丁的基线（把类型错误按行拆开修掉，= 旧实现*本来想*渲染的 UI）再跑一遍：
 *      只断言「CLI 帧带来的值」与「新实现从路由拿到的值」逐字相同 ——
 *      也就是路由 ≠ 另一套解码，而是同一条 AT 答案。
 *   C. 单向覆盖：新侧不许再出现任何 `at:` 调用。
 *
 * 会话夹具在 scripts/lib/at-fixtures.js：一份事实对象同时生成 CLI 帧与路由载荷
 * （会话那一刀之后连接页与概览页共用它，见 prove-connection-parity.js）。
 */

const fs = require('fs');
const path = require('path');
const cp = require('child_process');
const lib = require('./lib/luci-stub');
const fixtures = require('./lib/at-fixtures');

const REPO = path.resolve(__dirname, '..');
const RES = 'luci-app-mt5700m/htdocs/luci-static/resources';
const VIEW = RES + '/view/mt5700m/status.js';
const BASELINE = process.argv[2] || '8a8c501';

const oldSource = (rel) => cp.execSync('git show ' + BASELINE + ':' + rel, { cwd: REPO, maxBuffer: 64 * 1024 * 1024 }).toString();
const newSource = (rel) => fs.readFileSync(path.join(REPO, rel), 'utf8');

let failures = 0;
function check(name, ok, detail) {
	if (ok) { console.log('  ok   ' + name); return; }
	failures++;
	console.log('  FAIL ' + name + (detail ? '\n       ' + detail : ''));
}

/*
 * 唯一的取值差异：手机号的 `+` 前缀。
 *
 * 旧 CLI 的 `print_subscriber_number` 原样保留模组发来的 `+`；后端的领域模型
 * 给纯数字（+CNUM 的号码本身就是数字串，WebUI 的 normalizePhoneNumber 也是
 * 去 `+` 的）。数字逐位相同，所以比较时归一化，并单独把 `+` 的个数钉住。
 */
const NUM_PLUS = (t) => String(t).replace(/\+[\d]/g, (m) => m.slice(1));
function diffText(a, b) {
	const la = String(a).split('\n'), lb = String(b).split('\n');
	for (let i = 0; i < Math.max(la.length, lb.length); i++)
		if (la[i] !== lb[i]) return 'line ' + (i + 1) + ':\n       old: ' + la[i] + '\n       new: ' + lb[i];
	return '';
}

/* ------------------------------------------------------------ 共享事实
 * daemon StateCache 快照 + mt5700m-manager 的 ubus 状态：两侧都读它，
 * 所以「快照派生出来的行」两侧必然相同 —— 比较的意义在详情行。
 */
const MANAGER = {
	connected: true, at_port: '/dev/ttyUSB3', network: 'eth2',
	mode: 'serial', ipv4_address: '10.6.172.152'
};
const TRAFFIC = {
	interfaces: [ {
		name: 'eth2',
		updated: { date: { year: 2026, month: 10, day: 6 }, time: { hour: 12, minute: 30 } },
		traffic: {
			day: [
				{ date: { year: 2026, month: 10, day: 5 }, rx: 3435973836, tx: 1234567890 },
				{ date: { year: 2026, month: 10, day: 6 }, rx: 8589934592, tx: 2147483648 }
			],
			month: [ { date: { year: 2026, month: 10 }, rx: 12884901888, tx: 4294967296 } ],
			total: { rx: 429496729600, tx: 128849018880 }
		}
	} ]
};

function snapshotFor(shape) {
	return {
		signal: { value: { sysmode: 'NR5G', rsrp: -83, rsrq: -12, sinr: 12.8, rssi: -55, rscp: -95 } },
		network: { value: { operator: 'CHN-UNICOM', sysmode: 'NR5G', sysmode_detail: '5G SA' } },
		temperature: { value: { average: 45.1, modem1: 45.1, modem2: 44.3, ap1: 40 } },
		modem: { value: { manufacturer: 'Quectel', model: 'MT5700M', revision: 'MT5700M-2.5.0', imei: '862853030012345' } },
		sim: { value: { status: 'READY', iccid: '89860312345678901234', imsi: '460011234567890' } },
		cell: { value: { band: 41, channel: 504990, dlBandwidth: 100, sysmode: 'NR' } },
		endc: { value: { established: 1 } },
		usb: { value: { present: shape.usb.present, state: shape.usb.state, path: '/dev/ttyUSB3', available: shape.usb.present && shape.usb.state === 'normal', is_pcui: true } }
	};
}

/* ------------------------------------------------------------ 旧 CLI 帧
 * `cmd_status`（enabled/mode/usb_state+usb_pid+usb_slot/at_port/host/port/detected_gateway/channel/
 * connected）+ `print_cached_status` 的 key=value（signal / sim / identity /
 * operator / 载波块 / temperature）+ `query_extras` 的四行实时补充。
 * 载波段里的 dl 频点就是 CLI 的 `nr_arfcn_to_mhz()` 结果（对 NR 的 ARFCN 会
 * 算出量级离谱的数）——旧页面的 snapshotLines 只在「同一个小区」时把它搬进
 * carrier_1，而它只在多载波列表里渲染，单载波下不可见（证明里两侧 DOM 相同
 * 就是这条的证据）。
 */
function cliStatusFrame(shape) {
	const lines = [
		'enabled=1', 'mode=serial',
		'usb_state=' + (shape.usb.present ? shape.usb.state : 'absent'), 'usb_pid=3301', 'usb_slot=1',
		'at_port=/dev/ttyUSB3', 'host=127.0.0.1', 'port=8765',
		'detected_gateway=10.6.172.1', 'channel=serial', 'connected=1',
		// --- signal ---
		'sysmode=NR5G', 'rsrp=-83', 'rsrq=-12', 'sinr=12.8', 'rssi=-55', 'rscp=-95',
		// --- sim / identity ---
		'sim_state=READY', 'iccid=89860312345678901234', 'imsi=460011234567890',
		'imei=862853030012345', 'product_name=MT5700M', 'manufacturer=Quectel', 'revision=MT5700M-2.5.0',
		// --- network ---
		'operator=CHN-UNICOM', 'network_mode=NR5G', 'sysmode_detail=5G SA',
		// --- carrier（cell 主题 + nr_arfcn_to_mhz）---
		'carrier_count=1',
		'carrier_1=NR|B41|504990|2524950.00|100|2524950.00|0|100',
		'ca_active=0', 'dc_active=0', 'nr_carrier_count=1', 'lte_carrier_count=0',
		'ca_mode=NR', 'ca_dl_bandwidth=100', 'ca_ul_bandwidth=100',
		// --- temperature ---
		'temperature=45',
		// --- query_extras：APN / QCI / 号码 / 订阅速率 ---
		'active_apn=' + (shape.apn || ''),
		shape.qos.qci ? 'qci=' + shape.qos.qci : null,
		(typeof shape.qos.ambr_down_kbps === 'number' && typeof shape.qos.ambr_up_kbps === 'number')
			? 'ambr_down_mbps=' + (shape.qos.ambr_down_kbps / 1000).toFixed(1) : null,
		(typeof shape.qos.ambr_down_kbps === 'number' && typeof shape.qos.ambr_up_kbps === 'number')
			? 'ambr_up_mbps=' + (shape.qos.ambr_up_kbps / 1000).toFixed(1) : null,
		shape.numberState === 'not_stored' ? 'phone_number_state=not_stored'
			: (shape.number ? 'phone_number=+' + shape.number : null)
	].filter((line) => line !== null);
	return lines.join('\n') + '\n';
}

/* 兜底：夹具没覆盖时仍用旧版这八段（现在由 lib/at-fixtures 生成）。 */
function legacySessionFrame() {
	const section = (label, command, text) => '===== ' + label + ': ' + command + ' =====\n' + text + '\n\n';
	return section('Data session', 'AT^NDISSTATQRY?', 'AT^NDISSTATQRY?\r\n^NDISSTATQRY: 1,,,,"IPV4",1,,,"IPV6"\r\n\r\nOK')
		+ section('Detailed sessions', 'AT^DCONNSTAT?', 'AT^DCONNSTAT?\r\n^DCONNSTAT: 1,"cmnet",1,1,3,1\r\n\r\nOK')
		+ section('IPv4 lease', 'AT^DHCP?', 'AT^DHCP?\r\n^DHCP: 98AC060A,FFFFFF00,98AC0601,00000000,0A060B0B,0A060B0C,05DC,03E8\r\n\r\nOK')
		+ section('IPv6 lease', 'AT^DHCPV6?', 'AT^DHCPV6?\r\n^DHCPV6: 2408:8207::1,64,2408:8207::2,::,2408:8207::3,2408:8207::4,05DC,03E8\r\n\r\nOK')
		+ section('IP capability', 'AT^IPV6CAP?', 'AT^IPV6CAP?\r\n^IPV6CAP: 7\r\n\r\nOK')
		+ section('Data flow', 'AT^DSFLOWQRY', 'AT^DSFLOWQRY\r\n^DSFLOWQRY: 0000000A,0001E240,0002DC6C,00000064,02FAF080,05F5E100\r\n\r\nOK')
		+ section('MTU', 'AT^CGMTU=1', 'AT^CGMTU=1\r\n^CGMTU: 1,1500\r\n\r\nOK')
		+ section('PDP address', 'AT+CGPADDR=1', 'AT+CGPADDR=1\r\n+CGPADDR: 1,10.6.172.152\r\n\r\nOK');
}

/* ------------------------------------------------------------ 新路由载荷 */
function qosPayload(shape) {
	const payload = {};
	if (shape.qos.active_cid) payload.active_cid = shape.qos.active_cid;
	if (typeof shape.qos.ambr_down_kbps === 'number') payload.ambr_down_kbps = shape.qos.ambr_down_kbps;
	if (typeof shape.qos.ambr_up_kbps === 'number') payload.ambr_up_kbps = shape.qos.ambr_up_kbps;
	if (shape.qos.ambr_apn) payload.ambr_apn = shape.qos.ambr_apn;
	if (shape.qos.qci) payload.qci = shape.qos.qci;
	return payload;
}
function numberPayload(shape) {
	if (shape.numberState === 'not_stored') return { status: 'READY', numberState: 'not_stored' };
	return shape.number ? { status: 'READY', number: shape.number } : { status: 'READY' };
}
function contextsPayload(shape) {
	return {
		contexts: (shape.contexts || []).map((ctx) => ({
			cid: ctx.cid, type: 'IP', apn: ctx.apn, pdp_addr: ctx.active ? '10.6.172.152' : '', active: !!ctx.active
		}))
	};
}

/*
 * 两块区域单独断言，不参与逐字比较：
 *   alerts  —— 旧侧是那条 TypeError 横幅（缺陷产物），新侧要么没有、要么是 usb
 *              主题驱动的升级模式横幅；
 *   address —— 旧侧**永远是空的**：TypeError 发生在同一个 `.then` 里、就在设置
 *              `state.sessionDetail` 之前，所以「移动 IP」卡（advanced session
 *              那一路）也被一起打断了。新实现里两条路径互不干扰。
 */
const STRIPPED_REGIONS = [ 'alerts', 'address' ];
function stripRegions(node) {
	if (node === null || node === undefined || node === false || typeof node !== 'object' || !node.tagName) return node;
	const clone = Object.assign(Object.create(Object.getPrototypeOf(node)), node, {
		children: node.children
			.filter((ch) => !(ch && typeof ch === 'object' && ch.attrs
				&& STRIPPED_REGIONS.indexOf(ch.attrs['data-live-region']) !== -1))
			.map(stripRegions)
	});
	return clone;
}
function regionText(node, name) {
	const region = lib.collect(node, (x) => x.attrs && x.attrs['data-live-region'] === name)[0];
	return region ? lib.textOf(region) : '';
}

/* ------------------------------------------------------------ 形态表 */
const NORMAL_USB = { present: true, state: 'normal' };
const ACTIVE_APN = { contexts: [ { cid: 1, apn: 'cmnet', active: true }, { cid: 2, apn: 'ims', active: false } ], apn: 'cmnet' };
const SHAPES = [
	{
		key: '完整数据：活跃 PDP + QoS + 已存号码',
		...ACTIVE_APN,
		qos: { active_cid: 1, qci: '9', ambr_down_kbps: 102400, ambr_up_kbps: 51200, ambr_apn: 'cmnet' },
		number: '8613800138000', usb: NORMAL_USB,
		expect: [ 'cmnet', 'QCI 9', 'Down 102 Mbps / Up 51 Mbps', '8613800138000', '45.1' ]
	},
	{
		key: 'PDP 空闲但 cid 1 已配置 APN（无 QoS，APN 只能来自 +CGDCONT?）',
		contexts: [ { cid: 1, apn: '3gnet', active: false } ], apn: '3gnet',
		qos: {}, number: '8613800138000', usb: NORMAL_USB,
		expect: [ '3gnet' ]
	},
	{
		key: 'APN 取不到（无 PDP、无 QoS）→ 页面自己的「Carrier default」',
		contexts: [], apn: '', qos: {}, number: '8613800138000', usb: NORMAL_USB,
		expect: [ 'Carrier default' ]
	},
	{
		key: '卡里没存号码（+CME ERROR: 22）→ Not stored',
		...ACTIVE_APN,
		qos: { active_cid: 1, qci: '9', ambr_down_kbps: 102400, ambr_up_kbps: 51200, ambr_apn: 'cmnet' },
		number: null, numberState: 'not_stored', usb: NORMAL_USB,
		expect: [ 'Not stored' ]
	},
	{
		key: '亚 Mbps 速率 + 缺 QCI',
		contexts: [ { cid: 1, apn: 'cmnet', active: true } ], apn: 'cmnet',
		qos: { active_cid: 1, ambr_down_kbps: 512, ambr_up_kbps: 128, ambr_apn: 'cmnet' },
		number: '10086', usb: NORMAL_USB,
		expect: [ 'Down 500 Kbps / Up 100 Kbps', '10086' ]
	},
	{
		key: '升级模式（USB upgrade）→ 告警条 + 断开态',
		...ACTIVE_APN,
		qos: { active_cid: 1, qci: '6', ambr_down_kbps: 1024000, ambr_up_kbps: 102400, ambr_apn: 'cmnet' },
		number: '8613800138000', usb: { present: true, state: 'upgrade' },
		expect: []
	},
	{
		key: '模组不在（present=false → absent，与旧 CLI 同）',
		...ACTIVE_APN,
		qos: {}, number: '8613800138000', usb: { present: false, state: 'unknown' },
		expect: []
	},
	{
		key: '详情路由全不可用（null）→ 与「CLI 补充行全失败」等效',
		contexts: [], apn: '', qos: {}, number: null, usb: NORMAL_USB, detail: null,
		expect: [ 'Carrier default' ]
	}
];

/* ------------------------------------------------------------ 装载 */
function answers(kind, shape) {
	const sessionFacts = shape.sessions || {};
	if (kind === 'old') {
		return {
			'at:status': { stdout: cliStatusFrame(shape), stderr: '' },
			'at:advanced': { stdout: fixtures.advancedSessionFrame(sessionFacts), stderr: '' }
		};
	}
	const none = shape.detail === null || shape.noDetail === true;
	return {
		'route:network.session': none ? null : fixtures.sessionPayload(sessionFacts),
		[none ? 'route:__none__' : 'route:qos.get']: none ? null : qosPayload(shape),
		[none ? 'route:__none2__' : 'route:sim.number']: none ? null : numberPayload(shape),
		[none ? 'route:__none3__' : 'route:network.pdp_contexts']: none ? null : contextsPayload(shape)
	};
}

function side(kind, shape, opts) {
	opts = opts || {};
	const api = lib.makeApi(answers(kind, shape));
	if (kind === 'old') api.atStatus = () => api.at([ 'status' ]);
	api.atSession = () => api.at([ 'advanced', 'session' ]);
	api.managerStatus = () => Promise.resolve(MANAGER);
	api.trafficSummary = () => Promise.resolve(TRAFFIC);
	api.cachedSnapshot = () => Promise.resolve(snapshotFor(shape));
	const s = lib.loadSide(kind === 'old' ? oldSource : newSource, api, VIEW);
	if (kind === 'old' && opts.patchMerge !== false) {
		// 只在证明里修掉 baseline 的类型错误（native.stdout 是字符串），
		// 让 CLI 帧真的走完 mergeStatusLines → 页面渲染。见文件头注释第 2 条。
		const merge = s.view.mergeStatusLines;
		s.view.mergeStatusLines = function(base, overrides) {
			return merge.call(this, base, typeof overrides === 'string' ? overrides.split(/\r?\n/) : overrides);
		};
	}
	return s;
}

async function settle(holder) {
	for (let i = 0; i < 200 && holder.children.length === 0; i++) await lib.tick();
	for (let i = 0; i < 12; i++) await lib.tick();
}

async function render(kind, shape, opts) {
	const s = side(kind, shape, opts);
	s.view.load();
	const holder = s.view.render();
	// 详情帧的增量更新要求节点已经在文档里（页面的 updateRegions 会检查）
	s.scope.document.body.replaceChildren(holder);
	await settle(holder);
	return { side: s, holder: holder, text: lib.textOf(holder) };
}

/* ------------------------------------------------------------ 自检 */
if (oldSource(VIEW).indexOf('api.atStatus()') === -1) {
	console.error('基线 ' + BASELINE + ' 已经包含这一刀（status.js 里没有 api.atStatus()）——没有旧管线可比较。\n'
		+ '本刀的基线是 8a8c501：\n  node scripts/prove-status-parity.js 8a8c501');
	process.exit(2);
}
check('基线里旧侧确实读 CLI 状态帧（api.atStatus + atSession）',
	oldSource(VIEW).indexOf('api.atStatus()') !== -1 && oldSource(VIEW).indexOf('api.atSession()') !== -1);
check('新侧 status.js 零 CLI 调用（atStatus / atSession 都已删除）',
	newSource(VIEW).indexOf('api.atStatus') === -1 && newSource(VIEW).indexOf('atSession') === -1);
check('新侧 api.js 删除了 atSession 动词（概览页与连接页都不再用它）',
	newSource(RES + '/mt5700m/api.js').indexOf('atSession') === -1);
check('新侧 api.js 删除了 atStatus 动词',
	newSource(RES + '/mt5700m/api.js').indexOf('atStatus') === -1);

(async function () {
	console.log('渲染：整页 DOM 与文案（快照主题 + 详情路由 vs CLI 文本帧）');
	for (const shape of SHAPES) {
		const o = await render('old', shape, { patchMerge: false });  // 部署版基线（不打补丁）
		const n = await render('new', shape);                        // 新实现（真实路由）
		const nBare = await render('new', { ...shape, noDetail: true }); // 新实现（详情路由不可用）
		const oFixed = await render('old', shape, { patchMerge: true });  // 旧实现「本来想渲染」的 UI
		const body = (r) => NUM_PLUS(lib.serialize(stripRegions(r.holder), { attrs: true, keepValues: true }));
		const bodyText = (r) => NUM_PLUS(lib.textOf(stripRegions(r.holder)));
		const usbNormal = shape.usb.present && shape.usb.state === 'normal';

		if (usbNormal || !shape.usb.present) {
			// usb 正常 / 不在（旧 CLI 印 absent，同样不进异常分支）时：把详情数据抽掉，
			// 新实现必须与部署版基线逐字相同 —— 数据面换血本身对页面是透明的。
			check(shape.key + '：抽掉详情数据后新实现与部署版基线逐字相同（正文）',
				body(o) === body(nBare) && bodyText(o) === bodyText(nBare),
				diffText(body(o), body(nBare)) + '\n       ' + diffText(bodyText(o), bodyText(nBare)));
		} else {
			// upgrade/dump/unknown：旧实现从来没有把 usb_state 喂进 frame，hero 一直显示
			// 「已连接」；新实现照 usb 主题把页面切到断开态并挂出横幅 —— 这是修好，不是改 UI。
			check(shape.key + '：usb 主题驱动的差异被点名（旧=仍显示已连接，新=断开态）',
				bodyText(o).indexOf('Mobile network is connected and ready') !== -1
				&& bodyText(nBare).indexOf('The modem did not respond') !== -1,
				JSON.stringify([ bodyText(o).slice(0, 40), bodyText(nBare).slice(0, 40) ]));
		}

		// 详情路由有值时，多出来的就是那四行值（逐个点名，避免「同样为空」蒙混）
		shape.expect.forEach((needle) => {
			check(shape.key + '：新实现渲染出「' + needle + '」', bodyText(n).indexOf(needle) !== -1);
		});
		if (shape.key.indexOf('APN 取不到') === 0 || shape.key.indexOf('详情路由') === 0)
			check(shape.key + '：新实现仍然回落到页面自己的「Carrier default」', bodyText(n).indexOf('Carrier default') !== -1);

		// 手机号：旧 CLI 帧带 +（若模组给了），后端领域模型给纯数字
		check(shape.key + '：+ 前缀只出现在旧侧（' + (shape.number ? 1 : 0) + ' / 0）',
			(oFixed.text.match(/\+[\d]/g) || []).length === (shape.number ? 1 : 0)
			&& (n.text.match(/\+[\d]/g) || []).length === 0,
			JSON.stringify([ (oFixed.text.match(/\+[\d]/g) || []).length, (n.text.match(/\+[\d]/g) || []).length ]));

		// 地址卡（会话那一路）：旧侧被同一个 TypeError 一起打断 → 永远空
		if (shape.detail === null || shape.noDetail === true) {
			// 会话路由也不可用时，新侧退回「空卡」而不是崩溃/错位。
			check(shape.key + '：会话路由不可用时新侧退回空卡（Disconnected + --）',
				regionText(n.holder, 'address').indexOf('Disconnected') !== -1
				&& regionText(n.holder, 'address').indexOf('--') !== -1,
				JSON.stringify(regionText(n.holder, 'address').slice(0, 80)));
		} else {
			check(shape.key + '：地址卡差异被点名（旧=空卡，新=会话路由渲染出的地址）',
				regionText(o.holder, 'address').indexOf('Disconnected') !== -1
				&& regionText(n.holder, 'address').indexOf('10.6.172.152') !== -1
				&& regionText(n.holder, 'address').indexOf('2408:8207::1') !== -1
				&& regionText(n.holder, 'address').indexOf('MTU 1500') !== -1,
				JSON.stringify([ regionText(o.holder, 'address').slice(0, 60), regionText(n.holder, 'address').slice(0, 60) ]));
		}

		// 告警区：旧侧永远是那条 TypeError 横幅；新侧只在 usb 主题真的异常时挂横幅
		const usbAbnormal = shape.usb.present && (shape.usb.state === 'upgrade' || shape.usb.state === 'dump' || shape.usb.state === 'unknown');
		check(shape.key + '：告警区差异被点名（旧=TypeError 横幅' + (usbAbnormal ? '，新=升级模式横幅' : '，新=空') + '）',
			regionText(o.holder, 'alerts').indexOf('overrides') !== -1
			&& (usbAbnormal ? regionText(n.holder, 'alerts').indexOf('Upgrade mode') !== -1 : regionText(n.holder, 'alerts') === ''),
			JSON.stringify([ regionText(o.holder, 'alerts').slice(0, 60), regionText(n.holder, 'alerts').slice(0, 60) ]));
	}

	/* B：CLI 帧带来的值 = 新实现从路由拿到的值（补丁后的旧侧逐字相同） */
	{
		const shape = SHAPES[0];
		const fixedOld = await render('old', shape, { patchMerge: true });
		const n = await render('new', shape);
		[ 'cmnet', 'QCI 9', 'Down 102 Mbps / Up 51 Mbps', '8613800138000' ].forEach((needle) => {
			check('CLI 帧里的值「' + needle + '」在新实现里逐字相同（补丁基线 vs 新侧）',
				fixedOld.text.indexOf(needle) !== -1 && n.text.indexOf(needle) !== -1);
		});
		check('补丁基线：CLI 帧的四行补充值全部进过 frame（APN/QCI/速率/号码）',
			fixedOld.text.indexOf('could not be refreshed') === -1);
	}

	/* 数据源与调用面：新侧只用路由，绝不再出现 CLI 状态帧 */
	{
		const shape = SHAPES[0];
		const o = await render('old', shape);
		const n = await render('new', shape);
		const names = (r) => r.side.api.calls.map((c) => c.kind + ':' + (c.name || (c.args || [])[0] || ''));
		const oldCalls = names(o), newCalls = names(n);
		check('旧侧确实调了 CLI（at:status + at:advanced）',
			oldCalls.indexOf('at:status') !== -1 && oldCalls.indexOf('at:advanced') !== -1, JSON.stringify(oldCalls));
		check('新侧零 CLI 调用（概览页不再有 mt5700m-at 任何动词）',
			newCalls.filter((c) => c.indexOf('at:') === 0).length === 0, JSON.stringify(newCalls));
		check('新侧读的是四个统一路由（qos.get / sim.number / network.pdp_contexts / network.session）',
			newCalls.indexOf('route:qos.get') !== -1 && newCalls.indexOf('route:sim.number') !== -1
			&& newCalls.indexOf('route:network.pdp_contexts') !== -1 && newCalls.indexOf('route:network.session') !== -1,
			JSON.stringify(newCalls));
	}

	/* 详情帧到达后确实是「增量替换一块区域」，而不是整页重画 */
	{
		const shape = SHAPES[0];
		const s = side('new', shape);
		s.view.load();
		const holder = s.view.render();
		s.scope.document.body.replaceChildren(holder);
		// 等「首屏快照帧」落地（region 出现），再等详情帧落地
		for (let i = 0; i < 200; i++) {
			if (lib.collect(holder, (x) => x.attrs && x.attrs['data-live-region'] === 'facts').length) break;
			await lib.tick();
		}
		const countRegions = () => lib.collect(holder, (x) => x.attrs && x.attrs['data-live-region']).length;
		const countCards = () => lib.collect(holder, (x) => String(x.attrs && x.attrs['class'] || '').indexOf('mt-card') !== -1).length;
		const regionsBefore = countRegions(), cardsBefore = countCards();
		await settle(holder);
		const regionsAfter = countRegions(), cardsAfter = countCards();
		const unique = {};
		lib.collect(holder, (x) => x.attrs && x.attrs['data-live-region']).forEach((x) => {
			unique[x.attrs['data-live-region']] = (unique[x.attrs['data-live-region']] || 0) + 1;
		});
		check('增量替换没有丢卡片、也没有留下孤块（region ' + regionsBefore + '→' + regionsAfter
			+ '，卡片 ' + cardsBefore + '→' + cardsAfter + '）',
			regionsBefore === regionsAfter && cardsBefore === cardsAfter && regionsAfter >= 6
			&& Object.keys(unique).every((k) => unique[k] === 1));
		check('详情帧落地后 SIM 卡出现路由值（cmnet / QCI 9 / 号码）',
			lib.textOf(holder).indexOf('cmnet') !== -1 && lib.textOf(holder).indexOf('QCI 9') !== -1
			&& lib.textOf(holder).indexOf('8613800138000') !== -1);
	}

	console.log('');
	if (failures) {
		console.log('FAIL：' + failures + ' 项不一致');
		process.exit(1);
	}
	console.log('PASS：概览页的新旧渲染逐字一致（详情行改由 qos.get / sim.number / network.pdp_contexts 提供，'
		+ '页面不再调用 mt5700m-at status，也不再自己开 AT 通道）');
})();
