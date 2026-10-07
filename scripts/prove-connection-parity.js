#!/usr/bin/env node
'use strict';

/*
 * 连接页一致性证明（LuCI 连接页 → network.session 路由）
 *
 * 这一刀切掉的是连接页与概览页共用的最后一条 CLI 文本帧：
 * `mt5700m-at advanced session`（八个慢命令 `^NDISSTATQRY?` / `^DHCP?` /
 * `^DHCPV6?` / `^IPV6CAP?` / `^CGPADDR?` / `^DSFLOWQRY` / `^CGMTU=1` /
 * `^DCONNSTAT?` 的 dump，前端用 `parser.parseSession()` 正则取值）：
 *   - 读：`network.session`（连接页与概览页的「移动 IP」卡同一条路由）；
 *   - 写：「清空模组计数」按钮从 `c.confirmRun([ 'flow-clear' ])` 改为
 *     `c.confirmRoute('network.flow_clear')`；
 *   - 删除：`parser.parseSession()`、`json 的 csvValues/hexIPv4`、
 *     `api.js` 的 `atSession` —— 前端不再有第二份会话解码。
 *
 * 用法：
 *   node scripts/prove-connection-parity.js [基线]   # 基线默认 c0268e0（本刀之前）
 * 退出码 0 = 通过，1 = 不一致，2 = 基线选错了（基线里已经没有旧实现）。
 *
 * 关键设计：夹具（scripts/lib/at-fixtures.js）用**一份事实对象**同时生成 CLI 帧与
 * 路由载荷，所以两侧渲染逐字相同就等于说「后端从这八个命令解出来的值，与旧前端
 * 自己正则出来的完全一致」。四种形态覆盖：正常双栈（含 ^NDISSTATQRY 判定）、
 * NDIS 空应答（退回「有地址即已连接」）、仅 IPv6、以及会话数据整体不可用
 * （路由返回 null，对应旧版 CLI 帧读取失败）。
 *
 * 已知且在断言里点名的差异：DSL 能力码在旧侧是**文本帧的原文**（`7` / `0B`），
 * 新侧是**数字**（7 / 11）—— 页面把它渲染成同一句文案（`ipv6CapDescription` 的
 * 4 种取值，映射在后端解析成数字之后仍然成立），所以两侧可见文字相同。
 */

const fs = require('fs');
const path = require('path');
const cp = require('child_process');
const lib = require('./lib/luci-stub');
const fixtures = require('./lib/at-fixtures');

const REPO = path.resolve(__dirname, '..');
const RES = 'luci-app-mt5700m/htdocs/luci-static/resources';
const VIEW = RES + '/view/mt5700m/connection.js';
const BASELINE = process.argv[2] || 'c0268e0';

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
 * uci.load('mt5700m') 之后页面读 connection 段的四个键 —— 两侧同一份。
 */
const UCI = {
	'mt5700m.connection.enabled': '1',
	'mt5700m.connection.apn': 'cmnet',
	'mt5700m.connection.pdp_type': 'ipv4v6',
	'mt5700m.connection.auto': '1'
};

/* ------------------------------------------------------------ 形态表 */
/** `^NDISSTATQRY` 满 9 字段：由模组判定连通（第 1 字段 =1 且第 5 = IPV4）。 */
const FULL_NDIS = { ndis: true };
/** NDIS 空应答（实测中国移动下的真实行为）：退回「有没有地址」。 */
const EMPTY_NDIS = { ndis: false };
const SHAPES = [
	{ key: '正常双栈（^NDISSTATQRY 满字段）', facts: FULL_NDIS, expect: [ '10.6.172.152', '223.5.5.5', '2408:8207::1', 'IPv4 / IPv6 · same APN', 'MTU', '1500', 'CID 1 · cmnet' ] },
	{ key: 'NDIS 空应答（退回地址判定）', facts: EMPTY_NDIS, expect: [ '10.6.172.152', 'Connected' ] },
	{
		key: '只有 IPv4（无 v6 租约/无能力码）',
		// NDIS 也不可用（否则满字段的 NDIS 会把 v6 判成 connected）：
		// 只有 v4 地址 → v4 已连接、v6 未分配。
		facts: { ndis: false, ipv6: '', ipv6Dns: [], capability: null, maximumDown: null, maximumUp: null, sessions: [] },
		expect: [ '10.6.172.152' ],
		// 连接页的空值就是 '--'（「Not assigned」是概览页卡片的文案）：v6 两栏为空
		absent: [ '2408:8207::1' ]
	},
	{
		key: '独立 APN 的双栈（能力码 0B → 11）',
		facts: { capability: 11 },
		expect: [ 'IPv4 / IPv6 · separate APNs' ]
	},
	{ key: '会话数据不可用（路由返回 null / CLI 帧读取失败）', facts: null, expect: [ '--', '0 B' ] }
];

/* ------------------------------------------------------------ 装载 */
function answers(kind, facts) {
	if (kind === 'old') {
		return {
			// facts === null = 旧侧那条 CLI 帧整体读不到（stdout 为空）
			'at:advanced': { stdout: facts === null ? '' : fixtures.advancedSessionFrame(facts), stderr: '' },
			'at:connection-settings': { stdout: '', stderr: '' }
		};
	}
	return {
		'route:network.session': facts === null ? null : fixtures.sessionPayload(facts),
		'at:connection-settings': { stdout: '', stderr: '' }
	};
}

function side(kind, facts) {
	const api = lib.makeApi(answers(kind, facts));
	// 拨号设置仍走 CLI 动词（那一刀未动），两侧都要有；会话帧只有旧侧用。
	api.atConnectionSettings = () => api.at([ 'advanced', 'connection-settings' ]);
	api.atSession = () => api.at([ 'advanced', 'session' ]);
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

async function render(kind, facts) {
	const s = side(kind, facts);
	s.view.load();
	const holder = s.view.render();
	await s.view.contentReady;
	for (let i = 0; i < 20; i++) await lib.tick();
	return { side: s, holder: holder, text: lib.textOf(holder) };
}

/* ------------------------------------------------------------ 自检 */
if (oldSource(VIEW).indexOf('api.atSession()') === -1) {
	console.error('基线 ' + BASELINE + ' 已经包含这一刀（connection.js 里没有 api.atSession()）——没有旧管线可比较。\n'
		+ '本刀的基线是 c0268e0：\n  node scripts/prove-connection-parity.js c0268e0');
	process.exit(2);
}
check('基线里旧侧确实读 CLI 会话帧（api.atSession + parser.parseSession）',
	oldSource(VIEW).indexOf('api.atSession()') !== -1
	&& oldSource(RES + '/mt5700m/parser.js').indexOf('function parseSession') !== -1);
check('新侧不再有前端会话解码（parseSession / csvValues / hexIPv4 都已删除）',
	newSource(VIEW).indexOf('atSession') === -1
	&& newSource(RES + '/mt5700m/parser.js').indexOf('function parseSession') === -1
	&& newSource(RES + '/mt5700m/parser.js').indexOf('function csvValues') === -1
	&& newSource(RES + '/mt5700m/parser.js').indexOf('function hexIPv4') === -1);
check('新侧 api.js 删除了 atSession 动词', newSource(RES + '/mt5700m/api.js').indexOf('atSession') === -1);
check('新侧「清空模组计数」按钮走 network.flow_clear 路由（不再是 CLI flow-clear）',
	newSource(VIEW).indexOf("'network.flow_clear'") !== -1 && newSource(VIEW).indexOf("'flow-clear'") === -1);

(async function () {
	console.log('渲染：整页 DOM 与文案（CLI 文本帧 vs network.session 路由）');
	for (const shape of SHAPES) {
		const o = await render('old', shape.facts);
		const n = await render('new', shape.facts);
		const oShape = NUM(lib.serialize(o.holder, { attrs: true, keepValues: true }));
		const nShape = NUM(lib.serialize(n.holder, { attrs: true, keepValues: true }));
		check(shape.key + '：结构与文案逐字一致（数字归一化后）',
			oShape === nShape && NUM(o.text) === NUM(n.text),
			diffText(oShape, nShape) + '\n       ' + diffText(NUM(o.text), NUM(n.text)));
		shape.expect.forEach((needle) => {
			check(shape.key + '：两边都渲染出「' + needle + '」',
				o.text.indexOf(needle) !== -1 && n.text.indexOf(needle) !== -1,
				JSON.stringify([ o.text.indexOf(needle), n.text.indexOf(needle) ]));
		});
		(shape.absent || []).forEach((needle) => {
			check(shape.key + '：两边都没有「' + needle + '」（该栏为空）',
				o.text.indexOf(needle) === -1 && n.text.indexOf(needle) === -1,
				JSON.stringify([ o.text.indexOf(needle), n.text.indexOf(needle) ]));
		});
	}

	/* 数据源：新侧一条路由，旧侧一条 CLI 帧；且新侧不再有 CLI 调用 */
	{
		const shape = SHAPES[0];
		const o = await render('old', shape.facts);
		const n = await render('new', shape.facts);
		const calls = (r) => r.side.api.calls.map((c) => c.kind + ':' + (c.name || (c.args || []).join(' ') || ''));
		const oldCalls = calls(o), newCalls = calls(n);
		check('旧侧调 CLI（at:advanced session）', oldCalls.indexOf('at:advanced session') !== -1, JSON.stringify(oldCalls));
		check('新侧读 network.session 路由，且不再有这条 CLI 帧',
			newCalls.indexOf('route:network.session') !== -1 && newCalls.indexOf('at:advanced session') === -1,
			JSON.stringify(newCalls));
		check('连接页仍保留它的拨号设置读取（advanced connection-settings 那一刀未动）',
			oldCalls.some((c) => c.indexOf('connection-settings') !== -1)
			&& newCalls.some((c) => c.indexOf('connection-settings') !== -1),
			JSON.stringify([ oldCalls, newCalls ]));
	}

	/* 写路径：清空计数走路由；旧侧走 CLI 动词 */
	{
		const shape = SHAPES[0];
		const o = await render('old', shape.facts);
		const n = await render('new', shape.facts);
		o.side.api.calls.length = 0;
		n.side.api.calls.length = 0;
		lib.pressButton(o.holder, 'Clear counters');
		lib.modalButton(o.side.scope, 'Apply');
		for (let i = 0; i < 4; i++) await lib.tick();
		check('旧侧「清空计数」确认后调用 CLI flow-clear',
			o.side.api.calls.some((c) => c.kind === 'at' && (c.args || [])[0] === 'flow-clear'),
			JSON.stringify(o.side.api.calls));

		lib.pressButton(n.holder, 'Clear counters');
		lib.modalButton(n.side.scope, 'Apply');
		for (let i = 0; i < 4; i++) await lib.tick();
		check('新侧「清空计数」确认后调用 network.flow_clear 路由（零 CLI）',
			n.side.api.calls.some((c) => c.kind === 'routeCall' && c.name === 'network.flow_clear')
			&& !n.side.api.calls.some((c) => c.kind === 'at'),
			JSON.stringify(n.side.api.calls));
		const delays = n.side.scope.window.pending.map((p) => p.delay);
		check('新侧确认后仍然刷新页面（900 ms 重载，与旧侧同一节奏）',
			sameJson(delays, o.side.scope.window.pending.map((p) => p.delay)) && delays.indexOf(900) !== -1,
			JSON.stringify(delays));
	}

	/* 会话不可用时页面不崩、也不加新横幅（与旧版 CLI 帧读不到时同一观感） */
	{
		const o = await render('old', null);
		const n = await render('new', null);
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
	console.log('PASS：连接页与概览页的新旧渲染逐字一致（会话改由 network.session 路由提供，'
		+ '计数清零走 network.flow_clear，前端不再有第二份 parseSession 解码）');
})();
