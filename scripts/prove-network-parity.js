#!/usr/bin/env node
'use strict';

/*
 * 网络页一致性证明（LuCI 无线页状态区块 / network 帧切片）
 *
 * 这一刀把无线页状态区块从「切 `mt5700m-at network` 文本帧」改成「读统一路由」：
 *   - 信号仪表数值：^MONSC 第 8..10 字段（NR）/ 第 7..9 字段（LTE）→ `signal.get`
 *     （modules::signal 解码 ^HCSQ，索引 → dBm/dB 只有一份换算）
 *   - 服务小区行（RAT/ARFCN/PCI/CID/TAC/SCS）：^MONSC 文本 → `cell.get`
 *   - 注册状态：+CEREG 文本 → `registration.get`
 *   - 运营商：+COPS 文本 → `network.get`
 *   - RRC 状态：^RRCSTAT 文本 → `network.rrc`（这刀新加的路由）
 *   - 温度：帧里的 `temperature=` 行 → `system.temperature` 的 `peak`
 *   - 每页 `network` 帧调用（api.atNetwork）与 parser.js 的 parseServingCell
 *     （MONSC 字段布局的第二份 JS 实现）一起删除
 *
 * 用法：
 *   node scripts/prove-network-parity.js [基线]    # 基线默认 HEAD，本刀之前是 24ed5ef
 * 退出码 0 = 通过，1 = 不一致，2 = 基线选错了（基线里已经没有旧实现）。
 *
 * 脚本做五件事：
 *   1. 用 Rust 单测样本钉住脚本里的 mini 解码器（modules/{signal,cell,network,system}）；
 *   2. 用同一份「调制解调器读数」造两套输入：旧 = `mt5700m-at network` 文本帧，
 *      新 = 六个路由的载荷（载荷由 mini 解码器按后端规则从同一份读数推出）；
 *   3. 整个页面两边都真的 load()/render() 一遍，逐行比较 `mt-row` 的标签与取值，
 *      断言差异恰好是三行十六进制归一化（PCI/CID/TAC/LAC）；
 *   4. 断言其余结构/文案逐字一致（仪表盘取值、注册文案、仪表量程、卡片标题…）；
 *   5. 断言调用面：新版不再调 `at network`，状态区块只走六个路由。
 */

const fs = require('fs');
const path = require('path');
const cp = require('child_process');
const lib = require('./lib/luci-stub');

const REPO = path.resolve(__dirname, '..');
const RES = 'luci-app-mt5700m/htdocs/luci-static/resources';
const BASELINE = process.argv[2] || 'HEAD';

function oldSource(rel) {
	return cp.execSync('git show ' + BASELINE + ':' + rel, { cwd: REPO, maxBuffer: 64 * 1024 * 1024 }).toString();
}
function newSource(rel) { return fs.readFileSync(path.join(REPO, rel), 'utf8'); }

let failures = 0;
function check(name, ok, detail) {
	if (ok) { console.log('  ok   ' + name); return; }
	failures++;
	console.log('  FAIL ' + name + (detail ? '\n       ' + detail : ''));
}
const sameJson = (a, b) => JSON.stringify(a) === JSON.stringify(b);
function diffText(a, b) {
	const la = String(a).split('\n'), lb = String(b).split('\n');
	for (let i = 0; i < Math.max(la.length, lb.length); i++)
		if (la[i] !== lb[i]) return 'line ' + (i + 1) + ':\n       old: ' + la[i] + '\n       new: ' + lb[i];
	return '';
}

const fixtures = require('./lib/at-fixtures');
const {
	round1,
	decodeSignal, decodeCell, decodeRegistration, decodeRrc, decodeOperator, decodeTemps,
	decodeSyscfg, decodeC5gOption, decodeNrCapability,
	temperatureText,
} = fixtures;

/* ------------------------------------------------------------ 钉子 */

console.log('Rust 契约钉子（mini 解码器 vs modules/*/parser.rs 的单测样本）');
check('signal：^HCSQ NR 索引 → dBm/dB（58→-83 / 165→12.8 / 22→-9）',
	sameJson(decodeSignal('^HCSQ: "NR",58,165,22'), { sysmode: 'NR', rsrp: -83, sinr: 12.8, rsrq: -9 }),
	JSON.stringify(decodeSignal('^HCSQ: "NR",58,165,22')));
check('signal：^HCSQ LTE（rssi 54→-67 / rsrp 24→-117 / rsrq 155 上限 -3）',
	sameJson(decodeSignal('^HCSQ: "LTE",54,24,45,155'),
		{ sysmode: 'LTE', rssi: 54 - 121, rsrp: 24 - 141, sinr: round1(-20.2 + 45 * 0.2), rsrq: -3 }),
	JSON.stringify(decodeSignal('^HCSQ: "LTE",54,24,45,155')));
check('cell：NR scs 是十进制、cid/pci/lac 是十六进制（0x10321/0x1FA/0x2F01）',
	sameJson(decodeCell('^MONSC: NR,460,00,636648,1,10321,1FA,2F01,-82,-9'),
		{ sysmode: 'NR', mcc: '460', mnc: '00', channel: '636648', scs: 1, cid: '66337', pci: 506, lac: '12033' }),
	JSON.stringify(decodeCell('^MONSC: NR,460,00,636648,1,10321,1FA,2F01,-82,-9')));
check('cell：LTE 没有 scs 字段（第 4 位起就是 cid/pci/lac）',
	sameJson(decodeCell('^MONSC: LTE,460,00,1650,10321,64,2F01,-82,-9,-70'),
		{ sysmode: 'LTE', mcc: '460', mnc: '00', channel: '1650', cid: '66337', pci: 100, lac: '12033' }),
	JSON.stringify(decodeCell('^MONSC: LTE,460,00,1650,10321,64,2F01,-82,-9,-70')));
const rrc3 = decodeRrc('^RRCSTAT: 1,2,99');
check('rrc：三字段形态 1,2,99 → state=2 / camped=99',
	rrc3.state === 2 && rrc3.camped === 99, JSON.stringify(rrc3));
const rrc2 = decodeRrc('^RRCSTAT: 1,98');
check('rrc：两字段形态 1,98 → state=1 / camped=98（旧页面固定下标会把它读成 state=98）',
	rrc2.state === 1 && rrc2.camped === 98, JSON.stringify(rrc2));
check('temperature：^CHIPTEMP 十分之一度 + peak 取最大（45.1 / mimo_pa）',
	sameJson(decodeTemps('^CHIPTEMP: 432,445,451,398,401,407,441,442,449,0,65535,448'),
		{ sub3GPA: 43.2, sub6GPA: 44.5, mimoPa: 45.1, tcxo: 39.8, peri1: 40.1, peri2: 40.7,
		  ap1: 44.1, ap2: 44.2, modem1: 44.9, modem2: 0, bbp1: 0, bbp2: 44.8,
		  average: round1((43.2 + 44.5 + 45.1 + 39.8 + 40.1 + 40.7 + 44.1 + 44.2 + 44.9 + 44.8) / 10),
		  peak: 45.1, peak_sensor: 'mimo_pa' }),
	JSON.stringify(decodeTemps('^CHIPTEMP: 432,445,451,398,401,407,441,442,449,0,65535,448')));

/* ------------------------------------------------------------ 桩

 * 两边都要装 api.js 里的那几个取数函数（视图按名调用），桩只记录调用。
 * 旧版还会调 api.atNetwork()（这一刀要删掉的那次 CLI 帧调用）。
 */
function apiFor(answers) {
	const api = lib.makeApi(answers);
	api.atRadio = () => api.at([ 'radio' ]);
	api.atNetwork = () => api.at([ 'network' ]);
	api.atCellscan = () => api.at([ 'cellscan' ]);
	api.atCellscanResult = () => api.at([ 'cellscan-result' ]);
	return api;
}

const MODEM = {
	hcsq: '^HCSQ: "NR",58,165,22',
	monsc: '^MONSC: NR,460,00,636648,0,10321,1FA,2F01,-83,-9,12.8',
	rrc: '^RRCSTAT: 1,2,99',
	cereg: '+CEREG: 2,1,"2F01","10321",7',
	cops: '+COPS: 0,0,"CHN-UNICOM",7',
	chiptemp: '^CHIPTEMP: 432,445,451,398,401,407,441,442,449,0,65535,448',
	syscfgex: '^SYSCFGEX: 0803,3FFFFFFF,1,2,7FFFFFFFFFFFFFFF',
	c5g: '^C5GOPTION: 1,0,1',
	nrrccap: '^NRRCCAPQRY: 3,1\n^NRRCCAPQRY: 2,1\n^NRRCCAPQRY: 5,0,0'
};

/* 旧输入：`mt5700m-at network` 的帧（api/cli.rs::dump_section 的格式） */
function oldFrame() {
	return fixtures.textFrame([
		[ 'Signal', 'AT^HCSQ?', MODEM.hcsq ],
		[ 'Serving cell', 'AT^MONSC', MODEM.monsc ],
		[ 'RRC state', 'AT^RRCSTAT?', MODEM.rrc ],
		[ 'Network registration', 'AT+CEREG?', MODEM.cereg ],
		[ 'Operator', 'AT+COPS?', MODEM.cops ]
	]) + temperatureText(decodeTemps(MODEM.chiptemp));
}
/* 新输入：无线偏好区块仍是文本帧（这一刀没动它），其余走路由 */
function radioFrame() {
	return fixtures.textFrame([
		[ 'Radio mode', 'AT^SYSCFGEX?', MODEM.syscfgex ],
		[ '5G access mode', 'AT^C5GOPTION?', MODEM.c5g ],
		[ 'NR carrier aggregation', 'AT^NRRRCCAPQRY=3', undefined ],
		[ 'VoNR', 'AT^NRRCCAPQRY=2', undefined ],
		[ 'DSS', 'AT^NRRCCAPQRY=5', undefined ]
	]).replace(/undefined\nOK/g, MODEM.nrrccap + '\nOK');
}

const ANSWERS = {
	'at:network': { stdout: oldFrame(), stderr: '' },
	'at:radio': { stdout: radioFrame(), stderr: '' },
	'route:signal.get': decodeSignal(MODEM.hcsq),
	'route:cell.get': decodeCell(MODEM.monsc),
	'route:registration.get': decodeRegistration(MODEM.cereg),
	'route:network.get': decodeOperator(MODEM.cops),
	'route:network.rrc': decodeRrc(MODEM.rrc),
	'route:system.temperature': decodeTemps(MODEM.chiptemp),
	// radio-preference 切片：无线偏好区块也改读路由（^SYSCFGEX / ^C5GOPTION /
	// ^NRRCCAPQRY 3/2/5），载荷由共享夹具层按 modules/{network,modem} 的规则解码
	// —— 同一批应答，页面看到的取值与旧版切帧一致，所以整页比较仍然是逐字相同。
	'route:network.syscfg': decodeSyscfg(MODEM.syscfgex),
	'route:network.c5goption': decodeC5gOption(MODEM.c5g),
	'route:modem.nr_capability': decodeNrCapability({ ca: MODEM.nrrccap, vonr: MODEM.nrrccap, dss: MODEM.nrrccap })
};

/* ------------------------------------------------------------ 装载 */

const apiOld = apiFor(ANSWERS);
const apiNew = apiFor(ANSWERS);
const old = lib.loadSide(oldSource, apiOld);
const neu = lib.loadSide(newSource, apiNew);

if (typeof old.parser.parseServingCell !== 'function') {
	console.error('基线 ' + BASELINE + ' 已经包含这一刀（parser.js 里没有 parseServingCell）——没有旧管线可比较。\n'
		+ '本刀的基线是 24ed5ef：\n  node scripts/prove-network-parity.js 24ed5ef');
	process.exit(2);
}

/* 整页 load()/render()：两边都按 LuCI 的调用方式跑一遍 */
function renderView(side) {
	const view = side.view;
	view.load();
	const holder = view.render();
	return view.contentReady.then(() => holder);
}

/* 页面上 mt-row 的「标签 / 取值」对（顺序即 DOM 顺序） */
function rowsOf(holder) {
	return lib.collect(holder, n => n.tagName === 'DIV' && String((n.attrs && n.attrs['class']) || '') === 'mt-row')
		.map(n => ({
			label: lib.textOf(n.children[0]),
			value: n.children.slice(1).map(lib.textOf).join('')
		}));
}
/* 细节块的内容是有意差异（下面单独断言），抹平后其余结构必须逐字一致 */
function neutralizeDetails(holder) {
	const pre = lib.collect(holder, n => n.tagName === 'PRE' && String(n.attrs && n.attrs['class'] || '').indexOf('mt-raw') !== -1)[0];
	if (!pre) return 0;
	pre.children = [ '<technical-details>' ];
	return 1;
}

/* 三行已知的十六进制归一化：取值抹成同一个占位符，剩下的必须逐字一致 */
const HEX_ROWS = [ 'PCI', 'Cell ID', 'TAC / LAC' ];
function neutralizeHexRows(holder) {
	let n = 0;
	lib.collect(holder, x => x.tagName === 'DIV' && String((x.attrs && x.attrs['class']) || '') === 'mt-row')
		.forEach(row => {
			if (HEX_ROWS.indexOf(lib.textOf(row.children[0])) !== -1) { row.children[1].children = [ '<hex>' ]; n++; }
		});
	return n;
}

Promise.all([ renderView(old), renderView(neu) ]).then(function (holders) {
	const oldHolder = holders[0], newHolder = holders[1];

	/* ------------------------------------------------ 1) 逐行比较 */
	console.log('\n服务小区 / 无线电状态区块：逐行比较（标签 + 取值）');
	const oldRows = rowsOf(oldHolder), newRows = rowsOf(newHolder);
	const expected = [
		{ label: 'PCI', old: '1FA', new: '506' },
		{ label: 'Cell ID', old: '10321', new: '66337' },
		{ label: 'TAC / LAC', old: '2F01', new: '12033' }
	];
	const diffs = [];
	for (let i = 0; i < Math.max(oldRows.length, newRows.length); i++) {
		const a = oldRows[i], b = newRows[i];
		if (!a || !b) { diffs.push({ label: '(row ' + i + ')', old: a && a.label + '=' + a.value, new: b && b.label + '=' + b.value }); continue; }
		if (a.label !== b.label || a.value !== b.value) diffs.push({ label: a.label, old: a.value, new: b.value });
	}
	check('行数一致（' + oldRows.length + ' 行）', oldRows.length === newRows.length,
		oldRows.length + ' vs ' + newRows.length);
	check('差异恰好三行：PCI / Cell ID / TAC / LAC 的十六进制 → 十进制',
		sameJson(diffs, expected), JSON.stringify(diffs));
	const hexLabels = oldRows.map(r => r.label).filter(l => HEX_ROWS.indexOf(l) !== -1);
	check('hex→dec 归一化证明：' + expected.map(e => e.label + ' ' + e.old + '→' + e.new).join('，'),
		hexLabels.length === 3 && expected.every(e => {
			const a = oldRows.filter(r => r.label === e.label)[0], b = newRows.filter(r => r.label === e.label)[0];
			return a && b && a.value === e.old && b.value === e.new;
		}), JSON.stringify(hexLabels));

	/* ------------------------------------ 2) 结构 / 文案 / 其余取值逐字一致 */
	console.log('\n其余结构、文案与取值逐字一致');
	// 细节块的内容是有意差异（第 3 节单独断言），先把两边原文留档，再抹平比较其余结构
	const preOf = (holder) => lib.collect(holder, n => n.tagName === 'PRE' && String(n.attrs && n.attrs['class'] || '').indexOf('mt-raw') !== -1)[0];
	const oldPreText = lib.textOf(preOf(oldHolder)), newPreText = lib.textOf(preOf(newHolder));
	const neutral = [ neutralizeHexRows(oldHolder), neutralizeHexRows(newHolder),
		neutralizeDetails(oldHolder), neutralizeDetails(newHolder) ];
	check('两边各抹掉三行取值 + 细节块内容（证明差异只在取值与细节块）',
		neutral[0] === 3 && neutral[1] === 3 && neutral[2] === 1 && neutral[3] === 1,
		JSON.stringify(neutral));
	const shapeOpts = { attrs: true, skipAttrs: [ 'value' ] };
	const oldShape = lib.serialize(oldHolder, shapeOpts), newShape = lib.serialize(newHolder, shapeOpts);
	check('整页结构 + 文案 + 取值逐字一致（含仪表盘读数、量程、卡片标题、按钮）',
		oldShape === newShape, diffText(oldShape, newShape));

	const gauge = (holder) => lib.slotValues(holder, /mt-circular-gauge-val/);
	check('仪表盘读数：RSRP/RSRQ/SINR/温度四处逐字一致（' + gauge(newHolder).map(v => v.replace(/\s+/g, ' ').trim()).join(' | ') + '）',
		sameJson(gauge(oldHolder), gauge(newHolder)), JSON.stringify({ old: gauge(oldHolder), new: gauge(newHolder) }));
	check('温度读数是 peak（45.1，^CHIPTEMP 里最大的那个传感器），不是第一颗传感器',
		gauge(newHolder).some(v => v.indexOf('45.1') === 0));

	/* ------------------------------------- 3) Technical details 的既有形态 */
	check('「Technical details」折叠块仍在（同一 class、同一标题）',
		!!preOf(oldHolder) && !!preOf(newHolder) && oldPreText.length > 0 && newPreText.length > 0);
	check('旧版这块倾倒的是 AT 文本帧（含 ^HCSQ/^MONSC 原文）——本刀删掉的耦合',
		oldPreText.indexOf('^HCSQ') !== -1 && oldPreText.indexOf('^MONSC') !== -1);
	check('新版不再出现任何 AT 应答原文；改倒路由载荷（api.<route> + JSON）',
		newPreText.indexOf('^HCSQ') === -1 && newPreText.indexOf('api.signal.get') !== -1
		&& newPreText.indexOf('"peak": 45.1') !== -1);

	/* ------------------------------------------------ 4) 调用面 */
	console.log('\n调用面（哪些后端入口被用到）');
	const kind = (calls, k, name) => calls.filter(c => c.kind === k && (name === undefined
		|| (k === 'route' && c.name === name) || (k === 'at' && c.args[0] === name)));
	const oldAt = apiOld.calls.filter(c => c.kind === 'at').map(c => c.args[0]);
	const newAt = apiNew.calls.filter(c => c.kind === 'at').map(c => c.args[0]);
	check('旧版：at network（状态区块的文本帧）+ at radio', sameJson(oldAt, [ 'network', 'radio' ]), JSON.stringify(oldAt));
	// radio-preference 切片把最后一块 CLI 调用（advanced radio）也迁到路由，
	// 无线页到此零 CLI 调用 —— 下面两条断言随之收紧。
	check('新版：零 CLI 调用（at network 与 at radio 都已迁成路由）', sameJson(newAt, []), JSON.stringify(newAt));
	const newRoutes = apiNew.calls.filter(c => c.kind === 'route').map(c => c.name + (c.params && c.params.rat ? '/' + c.params.rat : ''));
	check('新版：状态区块六个路由 + 无线偏好三条 + 两个锁路由（顺序即 load() 的 Promise.all）',
		sameJson(newRoutes, [ 'signal.get', 'cell.get', 'registration.get', 'network.get',
			'network.rrc', 'system.temperature', 'network.syscfg', 'network.c5goption',
			'modem.nr_capability', 'network.lock_get/lte', 'network.lock_get/nr' ]),
		JSON.stringify(newRoutes));
	check('旧版：没有 RRC 路由（^RRCSTAT 只能从帧里切），锁路由已在',
		kind(apiOld.calls, 'route', 'network.rrc').length === 0
		&& kind(apiOld.calls, 'route', 'network.lock_get').length === 2);
	check('删掉的旧实现：parser.parseServingCell（MONSC 布局的第二份 JS 解码）',
		typeof old.parser.parseServingCell === 'function' && neu.parser.parseServingCell === undefined);
	check('删掉的旧实现：api.js 的 atNetwork（没人再调 mt5700m-at network）',
		typeof old.view.load === 'function' && typeof apiOld.atNetwork === 'function'
		&& newSource(RES + '/mt5700m/api.js').indexOf('function atNetwork') === -1);

	console.log(failures === 0 ? '\nPASS：' + '网络页状态区块的新旧渲染一致（仅三行十六进制归一化 + 细节块内容）' : '\nFAIL：' + failures + ' 项不一致');
	process.exit(failures === 0 ? 0 : 1);
}).catch(function (err) {
	console.error('渲染失败：' + (err && err.stack || err));
	process.exit(1);
});
