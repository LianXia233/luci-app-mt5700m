#!/usr/bin/env node
'use strict';

/*
 * 连接页一致性证明（连接设置批：CLI 帧 + 7 条 CLI 写 → 4 条读路由 + 7 条写路由）
 *
 * 本批切掉的是连接页自己的最后一段 CLI：`mt5700m-at advanced
 * connection-settings` 的五段读帧（Auto dial / Interface mode / PDP contexts /
 * PDP activation / Direct IP）与七条写入动词（pdp-set / pdp-state / pdp-remove /
 * advanced-set autodial|direct-ip|postroute|dmz）：
 *   - 读：4 条路由 network.autodial / network.interface_cfg /
 *     network.pdp_contexts / network.direct_ip（会话面板的 network.session
 *     在基线里已经是路由，本批不动）；
 *   - 写：七处 c.confirmRun([…argv]) 与 editPdp 的 api.at(['pdp-set',…]) 全部
 *     改成 c.confirmRoute(name, params) / api.routeCall(name, params)；
 *   - 删除：api.js 的 atConnectionSettings 速记、旧版对模块设置帧的第二份
 *     前端解析（autoMatch 正则 / parseContexts 调用随本批离开连接页）。
 *
 * 用法：
 *   node scripts/prove-connection-parity.js [基线]   # 基线默认 pre-system-route tag（连接页迁移前）
 * 退出码 0 = 通过，1 = 不一致，2 = 基线选错了（基线里已经没有旧实现）。
 *
 * 关键设计：夹具（scripts/lib/at-fixtures.js）用**一份 SETTINGS_FACTS** 同时
 * 生成 CLI 帧与 4 条路由载荷，所以两侧渲染逐字相同就等于说「后端从同一批 AT
 * 应答解出来的值，与旧前端自己正则出来的完全一致」。三种病态形态专门钉住旧
 * 正则的怪癖在载荷层被原样仿制（docs/architecture-v2/migration.md）：
 *   - ^SETAUTODIAL 缩短应答（auth 字段缺席）：旧正则要求尾部 ,<auth>，整行
 *     不识别、控件回落默认值 —— 载荷层 authType 缺席走进同一分支（autoKnown）；
 *   - ^SETDIRECTIP 非法值：两侧同样禁用控件并隐藏 Apply 按钮；
 *   - PostRoute 越界 + Dmz: not cfg：两侧同样显示 Unsupported value 并禁用。
 *
 * 已知且在断言里点名的差异（有意修复，不是漂移）：真实抓包的 TDCFG 应答是
 * `PostRoute : 1` —— 冒号前有空格（modules/network/parser.rs 单测样本与
 * parse_interface_cfg 的 doc 注释；姊妹项目 luci-app-mt5700 的 dial.js 用
 * /PostRoute\s*:\s*(\d+)/ 两种形态都认）。mt5700m 旧前端的正则
 * /PostRoute:\s*(\d+)/ 漏了冒号前的 `\s*`，导致真实设备上 Post-routing 控件
 * **永远 Unavailable**（Apply post-routing 按钮从未出现过）。新链路把值解出
 * 来后控件恢复可用 —— 单独一组断言把这个差异钉住；DMZ 不受影响（`Dmz:`
 * 冒号紧跟，旧正则认）。
 *
 * 写路径逐条驱动（按压按钮 → 确认弹窗 → 记录调用）：旧侧断言 CLI argv，
 * 新侧断言路由名 + params（键序一并钉住，防参数悄悄改名）。
 */

const fs = require('fs');
const path = require('path');
const cp = require('child_process');
const lib = require('./lib/luci-stub');
const fixtures = require('./lib/at-fixtures');

const REPO = path.resolve(__dirname, '..');
const RES = 'luci-app-mt5700m/htdocs/luci-static/resources';
const VIEW = RES + '/view/mt5700m/connection.js';
const BASELINE = process.argv[2] || 'pre-system-route';

const oldSource = (rel) => cp.execSync('git show ' + BASELINE + ':' + rel, { cwd: REPO, maxBuffer: 64 * 1024 * 1024 }).toString();
const newSource = (rel) => fs.readFileSync(path.join(REPO, rel), 'utf8');

let failures = 0;
function check(name, ok, detail) {
	if (ok) { console.log('  ok   ' + name); return; }
	failures++;
	console.log('  FAIL ' + name + (detail ? '\n       ' + detail : ''));
}
const sameJson = (a, b) => JSON.stringify(a) === JSON.stringify(b);
const NUM = (t) => String(t).replace(/\d+/g, '#');
function diffText(a, b) {
	const la = String(a).split('\n'), lb = String(b).split('\n');
	for (let i = 0; i < Math.max(la.length, lb.length); i++)
		if (la[i] !== lb[i]) return 'line ' + (i + 1) + ':\n       old: ' + la[i] + '\n       new: ' + lb[i];
	return '';
}

/* ------------------------------------------------------------ uci / manager
 * uci.load('mt5700m') 之后页面读 connection 段的三个键 —— 两侧同一份。
 */
const UCI = {
	'mt5700m.connection.enabled': '1',
	'mt5700m.connection.apn': 'cmnet',
	'mt5700m.connection.pdp_type': 'ipv4v6'
};

/* ------------------------------------------------------------ 形态表 */
/* facts 是会话侧（network.session 路由）的事实覆盖；settings 是连接设置侧
 * （本批）的事实覆盖，缺席即默认 SETTINGS_FACTS。前五个形态沿用会话批的
 * 五形态（回归钉），后三个是本批新增的 settings 病态形态。 */
const FULL_NDIS = { ndis: true };
const EMPTY_NDIS = { ndis: false };
const SHAPES = [
	{
		key: '正常双栈（设置满字段）', facts: FULL_NDIS,
		expect: [ '10.6.172.152', '223.5.5.5', '2408:8207::1', 'IPv4 / IPv6 · same APN', 'MTU', '1500', 'CID 1 · cmnet' ],
		// 控件取值：enabled / dialMode / protocol / auth / directIp / postRoute（DOM 顺序）
		selects: [ '1', '2', 'IPV4V6', '1', '0', '1' ],
		dmzValue: '192.168.8.100'
	},
	{ key: 'NDIS 空应答（退回地址判定）', facts: EMPTY_NDIS, expect: [ '10.6.172.152', 'Connected' ] },
	{
		key: '只有 IPv4（无 v6 租约/无能力码）',
		facts: { ndis: false, ipv6: '', ipv6Dns: [], capability: null, maximumDown: null, maximumUp: null, sessions: [] },
		expect: [ '10.6.172.152' ],
		absent: [ '2408:8207::1' ]
	},
	{
		key: '独立 APN 的双栈（能力码 0B → 11）',
		facts: { capability: 11 },
		expect: [ 'IPv4 / IPv6 · separate APNs' ]
	},
	{ key: '会话数据不可用（路由返回 null / CLI 帧读取失败）', facts: null, expect: [ '--', '0 B' ] },
	{
		/* 缩短应答：`^SETAUTODIAL: 1,0,"IPV4V6"` 没有 auth 字段，旧 autoMatch
		 * 整行不匹配 → 控件全默认（1 / 1 / IPV4V6 / … / 0）。新载荷层
		 * authType 缺席 → autoKnown=false，同一分支。 */
		key: '缩短 autodial 应答（auth 缺席 → 两侧控件回落默认值）', facts: FULL_NDIS,
		settings: { autodial: '1,0,"IPV4V6"' },
		selects: [ '1', '1', 'IPV4V6', '0', '0', '1' ],
		dmzValue: '192.168.8.100'
	},
	{
		/* `^SETDIRECTIP: 5` 不是 0|1：旧 pick 出 '5' 但 directIpKnown=false；
		 * 新载荷 enabled 缺席 → 同样禁用。Apply 按钮两侧都被隐藏。 */
		key: 'Direct IP 应答非法值（5 → 两侧禁用控件、隐藏 Apply）', facts: FULL_NDIS,
		settings: { directip: '5' },
		selects: [ '1', '2', 'IPV4V6', '1', '', '1' ],
		dmzValue: '192.168.8.100'
	},
	{
		/* `PostRoute: 3` 越界（只认 1|2）+ `Dmz: not cfg`：两侧都显示
		 * Unsupported value 并禁用 postRoute、隐藏 Apply post-routing；
		 * DMZ 输入框回落空串。样本用冒号紧跟形态 —— 带空格形态旧正则根本
		 * 不识别，属于下面「真实抓包」组的有意差异，不混进这条。 */
		key: 'PostRoute 越界 + Dmz not cfg（两侧禁用/回落空）', facts: FULL_NDIS,
		settings: { tdcfg: 'Mode : 2\nPostRoute: 3\nDmz: not cfg' },
		selects: [ '1', '2', 'IPV4V6', '1', '0', '' ],
		dmzValue: ''
	}
];

/* ------------------------------------------------------------ 装载 */
function answers(kind, shape) {
	const settingsFacts = fixtures.settingsFacts(shape.settings);
	const shared = {
		'route:network.session': shape.facts === null ? null : fixtures.sessionPayload(shape.facts)
	};
	if (kind === 'old') {
		// 旧侧的设置区块整份来自这条 CLI 帧（makeApi 按 at:<首词> 分发）。
		return Object.assign(shared, {
			'at:advanced': { stdout: fixtures.connectionSettingsFrame(settingsFacts), stderr: '' }
		});
	}
	return Object.assign(shared, fixtures.settingsRouteAnswers(settingsFacts));
}

function side(kind, shape) {
	const api = lib.makeApi(answers(kind, shape));
	if (kind === 'old')
		api.atConnectionSettings = () => api.at([ 'advanced', 'connection-settings' ]);
	// 新侧刻意**不挂** atConnectionSettings：若新页面残留调用，这里直接崩。
	api.managerStatus = () => Promise.resolve({ connected: true, network: 'eth2', at_port: '/dev/ttyUSB3' });
	api.deviceStatus = () => Promise.resolve({ up: true, carrier: true });
	api.dialLog = () => Promise.resolve({ log: '' });
	return lib.loadSide(kind === 'old' ? oldSource : newSource, api, VIEW, { uci: uciStub(), form: formStub() });
}

/* 桩 uci / form：连接页用它们建拨号表单（页面自身的能力，与数据源无关） */
function uciStub() {
	return {
		load: () => Promise.resolve(),
		get: (pkg, sec, opt) => UCI[pkg + '.' + sec + '.' + opt],
		set: () => {}, save: () => Promise.resolve(), apply: () => Promise.resolve()
	};
}
function formStub() {
	const option = () => {
		const o = {
			value: () => o, default: undefined, placeholder: undefined, rmempty: undefined,
			datatype: undefined, description: undefined, depends: () => o, password: undefined
		};
		return o;
	};
	const section = { anonymous: false, option: () => option() };
	return {
		Map: function () {
			return {
				section: () => section,
				render: () => Promise.resolve({ tagName: 'DIV', attrs: { 'class': 'cbi-map' }, children: [] })
			};
		},
		NamedSection: function () {}, Flag: 'flag', Value: 'value', ListValue: 'list',
		DynamicList: 'dynamic', Section: function () {}
	};
}

async function render(kind, shape) {
	const s = side(kind, shape);
	s.view.load();
	const holder = s.view.render();
	await s.view.contentReady;
	for (let i = 0; i < 20; i++) await lib.tick();
	return { side: s, holder: holder, text: lib.textOf(holder) };
}

/* 下拉取值（moduleControls 的 6 个 select；form 桩不产 select） */
const selectsOf = (r) => lib.collect(r.holder, (n) => n.tagName === 'SELECT').map((n) => String(n.value));
/* DMZ 输入框的值（input.value 不进 textOf，单独取） */
const dmzOf = (r) => {
	const inputs = lib.collect(r.holder, (n) => n.tagName === 'INPUT'
		&& String(n.attrs['placeholder'] || '') === '192.168.8.100');
	return inputs.length ? String(inputs[0].value) : null;
};

/* ------------------------------------------------------------ 自检 */
if (oldSource(VIEW).indexOf('api.atConnectionSettings()') === -1) {
	console.error('基线 ' + BASELINE + ' 已经包含本批（connection.js 里没有 api.atConnectionSettings()）——没有旧管线可比较。\n'
		+ '本批的基线是 pre-system-route tag（树与 7a02417 快照一致，远端可达）：\n  node scripts/prove-connection-parity.js pre-system-route');
	process.exit(2);
}
check('基线里旧侧确实读连接设置 CLI 帧 + 七条 CLI 写（atConnectionSettings / confirmRun / pdp-set）',
	oldSource(VIEW).indexOf('api.atConnectionSettings()') !== -1
	&& oldSource(VIEW).indexOf('c.confirmRun') !== -1
	&& oldSource(VIEW).indexOf("'pdp-set'") !== -1
	&& oldSource(VIEW).indexOf("'advanced-set'") !== -1);
check('新侧连接页不再有 CLI 帧解析代码（atConnectionSettings / confirmRun / SETAUTODIAL 正则 / parseContexts 全部离开）',
	newSource(VIEW).indexOf('atConnectionSettings') === -1
	&& newSource(VIEW).indexOf('confirmRun(') === -1
	&& newSource(VIEW).indexOf('SETAUTODIAL:') === -1
	&& newSource(VIEW).indexOf('parseContexts(') === -1
	&& newSource(VIEW).indexOf('parser.section(') === -1
	&& newSource(VIEW).indexOf('parser.pick(') === -1);
check('新侧七条写入全部走 confirmRoute/routeCall（路由名逐条出现）',
	[ 'network.pdp_set', 'network.pdp_state', 'network.pdp_remove', 'network.autodial_set',
		'network.direct_ip_set', 'network.postroute_set', 'network.dmz_set' ]
		.every((name) => newSource(VIEW).indexOf("'" + name + "'") !== -1));
check('新侧 api.js 删除了 atConnectionSettings 速记', newSource(RES + '/mt5700m/api.js').indexOf('atConnectionSettings') === -1);

/* ------------------------------------------------------------ 写路径表 */
/* 每条写入两侧分别驱动：按压按钮 →（可选改弹窗取值）→ 确认弹窗按钮。
 * 旧侧预期 argv（api.at 的 args），新侧预期 routeCall 的名字 + params。 */
const WRITES = [
	{
		key: 'PDP 新建（Add profile → Save）',
		button: 'Add profile', apply: 'Save',
		prepare: (nodes) => {
			const inputs = lib.inputsIn(nodes); // [CID, APN]（中间的 IP protocol 是 select）
			inputs[0].value = '3';
			inputs[1].value = 'iot.apn';
		},
		oldArgs: [ 'pdp-set', '3', 'IPV4V6', 'iot.apn' ],
		newCall: { name: 'network.pdp_set', params: { cid: 3, type: 'IPV4V6', apn: 'iot.apn' } }
	},
	{
		key: 'PDP 停用（CID 1 Deactivate）',
		button: 'Deactivate', apply: 'Apply',
		oldArgs: [ 'pdp-state', '0', '1' ],
		newCall: { name: 'network.pdp_state', params: { cid: 1, active: false } }
	},
	{
		key: 'PDP 删除（CID 1 Remove）',
		button: 'Remove', apply: 'Apply',
		oldArgs: [ 'pdp-remove', '1' ],
		newCall: { name: 'network.pdp_remove', params: { cid: 1 } }
	},
	{
		key: '模块拨号策略（Apply module dialing）',
		button: 'Apply module dialing', apply: 'Apply',
		// 旧 argv 直接搬控件取值（SETTINGS_FACTS 的 autodial 解析产物）；
		// 新 params 的 enabled/dialMode/auth 是 Number，键序与页面代码一致。
		oldArgs: [ 'advanced-set', 'autodial', '1', '2', 'IPV4V6', 'cmnet', '', '', '1' ],
		newCall: { name: 'network.autodial_set', params: { enabled: true, dialMode: 2, protocol: 'IPV4V6', apn: 'cmnet', username: '', password: '', auth: 1 } }
	},
	{
		key: 'IP 直通（Apply IP passthrough）',
		button: 'Apply IP passthrough', apply: 'Apply',
		oldArgs: [ 'advanced-set', 'direct-ip', '0' ],
		newCall: { name: 'network.direct_ip_set', params: { enabled: false } }
	},
	{
		key: 'Post-routing（Apply post-routing）',
		button: 'Apply post-routing', apply: 'Apply',
		oldArgs: [ 'advanced-set', 'postroute', '1' ],
		newCall: { name: 'network.postroute_set', params: { mode: 1 } }
	},
	{
		key: 'DMZ（Apply DMZ）',
		button: 'Apply DMZ', apply: 'Apply',
		oldArgs: [ 'advanced-set', 'dmz', '192.168.8.100' ],
		newCall: { name: 'network.dmz_set', params: { host: '192.168.8.100' } }
	}
];

async function fireWrite(r, w) {
	r.side.api.calls.length = 0;
	lib.pressButton(r.holder, w.button);
	if (w.prepare) w.prepare(lib.lastModal(r.side.scope).children);
	lib.modalButton(r.side.scope, w.apply);
	for (let i = 0; i < 6; i++) await lib.tick();
	return r.side.api.calls;
}

(async function () {
	console.log('渲染：整页 DOM 与文案（CLI 设置帧 vs 4 条设置路由）');
	for (const shape of SHAPES) {
		const o = await render('old', shape);
		const n = await render('new', shape);
		const oShape = NUM(lib.serialize(o.holder, { attrs: true, keepValues: true }));
		const nShape = NUM(lib.serialize(n.holder, { attrs: true, keepValues: true }));
		check(shape.key + '：结构与文案逐字一致（数字归一化后）',
			oShape === nShape && NUM(o.text) === NUM(n.text),
			diffText(oShape, nShape) + '\n       ' + diffText(NUM(o.text), NUM(n.text)));
		(shape.expect || []).forEach((needle) => {
			check(shape.key + '：两边都渲染出「' + needle + '」',
				o.text.indexOf(needle) !== -1 && n.text.indexOf(needle) !== -1,
				JSON.stringify([ o.text.indexOf(needle), n.text.indexOf(needle) ]));
		});
		(shape.absent || []).forEach((needle) => {
			check(shape.key + '：两边都没有「' + needle + '」（该栏为空）',
				o.text.indexOf(needle) === -1 && n.text.indexOf(needle) === -1,
				JSON.stringify([ o.text.indexOf(needle), n.text.indexOf(needle) ]));
		});
		if (shape.selects) {
			check(shape.key + '：设置控件取值两侧一致且等于期望（enabled/dialMode/protocol/auth/directIp/postRoute）',
				sameJson(selectsOf(o), selectsOf(n)) && sameJson(selectsOf(n), shape.selects),
				JSON.stringify([ selectsOf(o), selectsOf(n), shape.selects ]));
		}
		if (shape.dmzValue !== undefined) {
			check(shape.key + '：DMZ 输入框取值两侧一致（' + JSON.stringify(shape.dmzValue) + '）',
				dmzOf(o) === shape.dmzValue && dmzOf(n) === shape.dmzValue,
				JSON.stringify([ dmzOf(o), dmzOf(n) ]));
		}
		if (shape.key.indexOf('Direct IP 应答非法值') === 0) {
			check(shape.key + '：两侧都没有 Apply IP passthrough 按钮',
				!lib.findButton(o.holder, 'Apply IP passthrough') && !lib.findButton(n.holder, 'Apply IP passthrough'));
		}
		if (shape.key.indexOf('PostRoute 越界') === 0) {
			check(shape.key + '：两侧都渲染出 Unsupported value 且没有 Apply post-routing 按钮',
				NUM(o.text).indexOf('Unsupported value: #') !== -1 && NUM(n.text).indexOf('Unsupported value: #') !== -1
				&& !lib.findButton(o.holder, 'Apply post-routing') && !lib.findButton(n.holder, 'Apply post-routing'),
				JSON.stringify({ old: NUM(o.text).indexOf('Unsupported value: #'), new: NUM(n.text).indexOf('Unsupported value: #'),
					oldBtn: !!lib.findButton(o.holder, 'Apply post-routing'), newBtn: !!lib.findButton(n.holder, 'Apply post-routing') }));
		}
	}

	/* 有意差异（不是漂移）：真实抓包的 TDCFG 应答 `PostRoute : 1` 冒号前带
	 * 空格，旧正则 /PostRoute:\s*(\d+)/ 不识别 → Post-routing 控件在真实设备
	 * 上永远 Unavailable；新链路（parse_interface_cfg 的宽松解析）把值解出来，
	 * 控件恢复可用 —— 与姊妹项目 luci-app-mt5700 的 dial.js 行为对齐。
	 * 样本取 parse_interface_cfg 单测的原样本（CRLF + 带空格）。 */
	{
		console.log('有意差异：真实抓包 TDCFG（PostRoute 冒号前带空格 → 旧正则盲区）');
		const shape = {
			facts: FULL_NDIS,
			settings: { tdcfg: 'Mode : 1\r\nPostRoute : 1\r\nDmz: 192.168.8.100' }
		};
		const o = await render('old', shape);
		const n = await render('new', shape);
		const selOf = (r) => lib.collect(r.holder, (x) => x.tagName === 'SELECT').map((x) => String(x.value));
		check('盲区形态：旧侧 postRoute 不可识别（Unavailable、禁用、无 Apply post-routing）',
			selOf(o)[5] === '' && !lib.findButton(o.holder, 'Apply post-routing')
			&& o.text.indexOf('The modem reported a post-routing value') !== -1,
			JSON.stringify(selOf(o)));
		check('盲区形态：新侧 postRoute=1 可用（有 Apply post-routing）',
			selOf(n)[5] === '1' && !!lib.findButton(n.holder, 'Apply post-routing'),
			JSON.stringify(selOf(n)));
		check('盲区形态：DMZ 与 IP 直通不受影响（两侧同值，证明差异只在 PostRoute）',
			dmzOf(o) === '192.168.8.100' && dmzOf(n) === '192.168.8.100'
			&& selOf(o)[4] === selOf(n)[4] && selOf(o)[4] === '0');
	}

	/* 数据源：旧侧一条 CLI 设置帧，新侧四条设置路由；两侧都读 network.session */
	{
		const shape = SHAPES[0];
		const o = await render('old', shape);
		const n = await render('new', shape);
		const routes = (r) => r.side.api.calls.filter((c) => c.kind === 'route').map((c) => c.name);
		const ats = (r) => r.side.api.calls.filter((c) => c.kind === 'at').map((c) => (c.args || []).join(' '));
		check('旧侧读 CLI 连接设置帧（advanced connection-settings）+ network.session',
			ats(o).indexOf('advanced connection-settings') !== -1
			&& routes(o).indexOf('network.session') !== -1, JSON.stringify([ ats(o), routes(o) ]));
		check('新侧按 load() 顺序读 5 条路由（4 条设置 + session），零 CLI',
			sameJson(routes(n), [ 'network.autodial', 'network.interface_cfg', 'network.pdp_contexts', 'network.direct_ip', 'network.session' ])
			&& ats(n).length === 0, JSON.stringify([ ats(n), routes(n) ]));
	}

	/* 写路径：七条写入逐条驱动 —— 旧 argv vs 新 route 参数 */
	console.log('写入：七条设置逐条驱动（旧 CLI argv vs 新路由 params）');
	{
		const shape = SHAPES[0];
		const o = await render('old', shape);
		const n = await render('new', shape);
		for (const w of WRITES) {
			const oldCalls = await fireWrite(o, w);
			const newCalls = await fireWrite(n, w);
			const oldAt = oldCalls.filter((c) => c.kind === 'at').map((c) => c.args);
			const newRoute = newCalls.filter((c) => c.kind === 'routeCall')[0];
			check(w.key + '：旧侧发出 CLI argv ' + JSON.stringify(w.oldArgs),
				oldAt.length === 1 && sameJson(oldAt[0], w.oldArgs), JSON.stringify(oldCalls));
			check(w.key + '：新侧发出 ' + w.newCall.name + ' 路由（params 逐键一致）',
				newRoute && newRoute.name === w.newCall.name && sameJson(newRoute.params, w.newCall.params)
				&& newCalls.every((c) => c.kind === 'routeCall'), JSON.stringify(newCalls));
		}
		/* 确认弹窗成功后的 900 ms 重载节奏两侧一致（confirmAction 共用） */
		check('确认成功后两侧都挂 900 ms 重载（confirmAction 同一份实现）',
			o.side.scope.window.pending.some((p) => p.delay === 900)
			&& n.side.scope.window.pending.some((p) => p.delay === 900));
	}

	/* 「清空计数」在基线里已是路由（上一批迁完）：两侧同走 network.flow_clear，
	 * 这里钉住它没有被本批破坏。 */
	{
		const shape = SHAPES[0];
		const o = await render('old', shape);
		const n = await render('new', shape);
		o.side.api.calls.length = 0;
		n.side.api.calls.length = 0;
		lib.pressButton(o.holder, 'Clear counters');
		lib.modalButton(o.side.scope, 'Apply');
		lib.pressButton(n.holder, 'Clear counters');
		lib.modalButton(n.side.scope, 'Apply');
		for (let i = 0; i < 4; i++) await lib.tick();
		check('「清空计数」两侧都走 network.flow_clear（上一批成果未被本批破坏）',
			o.side.api.calls.some((c) => c.kind === 'routeCall' && c.name === 'network.flow_clear')
			&& n.side.api.calls.some((c) => c.kind === 'routeCall' && c.name === 'network.flow_clear'),
			JSON.stringify([ o.side.api.calls, n.side.api.calls ]));
	}

	/* 会话不可用时页面不崩、也不加新横幅（与旧版 CLI 帧读不到时同一观感） */
	{
		const o = await render('old', SHAPES[4]);
		const n = await render('new', SHAPES[4]);
		check('会话不可用：两侧渲染出同一张空卡（骨架 + 「--」占位，无新增告警条）',
			n.text.indexOf('Assigned addresses') !== -1 && n.text.indexOf('--') !== -1
			&& n.text.indexOf('[object') === -1
			&& NUM(n.text) === NUM(o.text)
			&& lib.collect(n.holder, (x) => String(x.attrs && x.attrs['class'] || '').indexOf('alert-message') !== -1).length
				=== lib.collect(o.holder, (x) => String(x.attrs && x.attrs['class'] || '').indexOf('alert-message') !== -1).length,
			diffText(NUM(o.text), NUM(n.text)));
	}

	console.log('');
	if (failures) {
		console.log('FAIL：' + failures + ' 项不一致');
		process.exit(1);
	}
	console.log('PASS：连接页的新旧渲染逐字一致（连接设置帧与七条 CLI 写已切成'
		+ ' 4 条读路由 + 7 条写路由；缩短 autodial / 非法 Direct IP / 越界 PostRoute'
		+ ' 三种怪癖形态在载荷层原样仿制；真实抓包的 PostRoute 带空格盲区作为有意'
		+ ' 修复单独点名 —— 对齐姊妹项目 luci-app-mt5700 的行为）');
})();
