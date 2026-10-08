#!/usr/bin/env node
'use strict';

/*
 * 锁频面板一致性证明（LuCI 无线页，item 9 的锁频切片）
 *
 * 这一刀把锁频从「页面切 ^LTEFREQLOCK/^NRFREQLOCK 文本 + 提交 CLI 位置参数」
 * 改成「读 network.lock_get / 写 network.lock_apply」：
 *   - 面板回填与两行锁状态：帧里的文本段 → `network.lock_get` 载荷
 *   - 面板「Review and apply」与邻区卡片「Lock」：`['lock', rat, …]` CLI 位置
 *     参数 → `network.lock_apply` 的 typed items（+ verify，模块侧轮询校验）
 *   - parser.js 的 `collectFreqLock` / `parseLockData`（^LTEFREQLOCK 行布局的第二份
 *     JS 实现）随最后调用者一起删除
 *
 * 用法：
 *   node scripts/prove-lock-parity.js [基线]      # 基线默认 HEAD，本刀之前是 dfb4810
 * 退出码 0 = 通过，1 = 不一致，2 = 基线选错了（基线里已经没有旧实现）。
 *
 * 这个脚本做四件事：
 *   1. 用 Rust 单元测试的样本（modules/network/parser.rs 的
 *      freq_lock_lte_rows_and_hex_pci / freq_lock_nr_rows_carry_scs_before_pci）
 *      钉住脚本里的 mini 解码器；
 *   2. 跑一遍 HEAD 的 parser.js，**记录**旧回填为什么是空的（
 *      collectFreqLock 要求每行都带前缀，而固件只在首行带）；
 *   3. 对新实现渲染面板，断言回填值等于领域值，且面板结构/文案与旧版逐字一致；
 *   4. 对一组表单输入，比较新旧两条写路径：旧 = CLI 位置参数（含两处已知填错
 *      的参数位置），新 = items；断言 items 与用户输入一一对应，并把结果处理
 *      （成功提示 + 2.5s 刷新、失败提示）与旧版逐字对齐。
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

/* ------------------------------------------------ 后端契约（mini 解码器）
 * modules/network/parser.rs::parse_freq_lock 的规则：首行 `^…FREQLOCK: <type>`，
 * 次行 `<mobility>,<num>`，随后 num 行数据；LTE 行是 band,arfcn,pci(hex)，
 * NR 行是 band,arfcn,scs,pci(hex)。type 0 没有数据行；没有锁行 = null。
 */
function decodeLockReply(raw, rat) {
	const prefix = rat === 'nr' ? '^NRFREQLOCK:' : '^LTEFREQLOCK:';
	const rows = raw.split('\n').map(l => l.trim()).filter(l => l && !l.includes('OK') && !l.startsWith('AT'));
	const head = rows.findIndex(l => l.startsWith(prefix));
	if (head < 0) return null;
	const lock_type = parseInt(rows[head].slice(prefix.length).trim(), 10);
	if (!(lock_type >= 0)) return null;
	if (lock_type === 0) return { lock_type: 0, mobility: 0, items: [] };
	const meta = (rows[head + 1] || '').split(',');
	const mobility = parseInt(meta[0], 10) || 0;
	const num = parseInt(meta[1], 10) || 0;
	const items = [];
	for (let i = 0; i < num; i++) {
		const parts = (rows[head + i + 2] || '').split(',').map(p => p.trim().replace(/"/g, ''));
		const numAt = (idx, radix) => (parts[idx] === undefined || parts[idx] === '') ? undefined : parseInt(parts[idx], radix);
		const item = {};
		const band = numAt(0, 10), arfcn = numAt(1, 10);
		if (band !== undefined) item.band = band;
		if (arfcn !== undefined) item.arfcn = arfcn;
		if (rat === 'nr') {
			const scs = numAt(2, 10), pci = numAt(3, 16);
			if (scs !== undefined) item.scs = scs;
			if (pci !== undefined) item.pci = pci;
		} else {
			const pci = numAt(2, 16);
			if (pci !== undefined) item.pci = pci;
		}
		items.push(item);
	}
	return { lock_type, mobility, items };
}

/* 与 modules/network/parser.rs 的单测样本逐字对应 */
const RUST_PINS = [
	{ rat: 'lte', raw: '^LTEFREQLOCK: 2\n0,1\n3,1850,64\nOK',
	  expect: { lock_type: 2, mobility: 0, items: [ { band: 3, arfcn: 1850, pci: 100 } ] } },   // PCI 0x64
	{ rat: 'nr', raw: '^NRFREQLOCK: 2\n0,2\n78,643456,1,10\n41,504990,0,1A\nOK',
	  expect: { lock_type: 2, mobility: 0, items: [ { band: 78, arfcn: 643456, scs: 1, pci: 16 },
	                                                { band: 41, arfcn: 504990, scs: 0, pci: 26 } ] } },
	{ rat: 'lte', raw: '^LTEFREQLOCK: 0\nOK', expect: { lock_type: 0, mobility: 0, items: [] } },
	{ rat: 'lte', raw: 'OK', expect: null },
	{ rat: 'lte', raw: '^NRFREQLOCK: 2\n0,1\n78,1,1,2\nOK', expect: null },
];

console.log('Rust 契约钉子（mini 解码器 vs modules/network/parser.rs 的单测样本）');
for (const pin of RUST_PINS) {
	check(pin.rat + ': ' + pin.raw.split('\n')[0], sameJson(decodeLockReply(pin.raw, pin.rat), pin.expect),
		'got ' + JSON.stringify(decodeLockReply(pin.raw, pin.rat)));
}

/* ------------------------------------------------------------ 旧管线取值 */

const old = lib.loadSide(oldSource, lib.makeApi({}));
const neu = lib.loadSide(newSource, lib.makeApi({}));

if (typeof old.parser.collectFreqLock !== 'function') {
	console.error('基线 ' + BASELINE + ' 已经包含这一刀（parser.js 里没有 collectFreqLock）——没有旧管线可比较。\n'
		+ '本刀的基线是 dfb4810：\n  node scripts/prove-lock-parity.js dfb4810');
	process.exit(2);
}

/* 帧里的锁段：dump_section 输出 `===== LTE lock: AT^LTEFREQLOCK? =====` + 原文 */
function frameSection(label, reply) {
	return [ '===== ' + label + ': AT^' + (label === 'LTE lock' ? 'LTEFREQLOCK' : 'NRFREQLOCK') + '? =====', reply, '' ].join('\n');
}
function oldPanelFields(label, rat, reply) {
	const section = old.parser.section(frameSection(label, reply), label);
	const raw = old.parser.collectFreqLock(section, rat === 'nr' ? '^NRFREQLOCK' : '^LTEFREQLOCK');
	return { raw: raw, parsed: old.parser.parseLockData(raw, rat) };
}

console.log('\n旧管线（HEAD 的 parser.js 亲自跑）：回填的既有行为');
const lteReply = '^LTEFREQLOCK: 2\n0,1\n3,1850,64\nOK';
const oldLte = oldPanelFields('LTE lock', 'lte', lteReply);
check('固件原文形态 → collectFreqLock 只拿到首行（' + JSON.stringify(oldLte.raw) + '）',
	sameJson(oldLte.raw, [ '2' ]));
check('→ 回填四列全空（旧版缺陷：面板永远看不到已生效的锁）',
	sameJson(oldLte.parsed, { type: '2', bands: '', arfcns: '', scs: '', pcis: '' }),
	JSON.stringify(oldLte.parsed));
// collectFreqLock 假设的形态（每行都带前缀）：旧版这时是对的，用它对表
const prefixedReply = '^LTEFREQLOCK: 2\n^LTEFREQLOCK: 0,1\n^LTEFREQLOCK: 3,1850,64\nOK';
const oldPrefixed = oldPanelFields('LTE lock', 'lte', prefixedReply).parsed;
check('「每行带前缀」的假想形态旧版可解析 → type=2 / bands=3 / arfcns=1850 / pcis=64',
	sameJson(oldPrefixed, { type: '2', bands: '3', arfcns: '1850', scs: '', pcis: '64' }),
	JSON.stringify(oldPrefixed));

/* ------------------------------------------------------------ 新管线取值 */

const VALUE_CLASS = /mt-band-field|mt-card-title/;
const PAYLOAD_LTE = decodeLockReply(lteReply, 'lte');
const PAYLOAD_NR = decodeLockReply('^NRFREQLOCK: 2\n0,2\n78,643456,1,10\n41,504990,0,1A\nOK', 'nr');

function panel(rat, lock) {
	const scope = neu.scope;
	const node = neu.c.lockPanel(rat === 'nr' ? '5G NR network' : 'LTE network', rat, lock);
	return { node: node, scope: scope };
}
function inputValues(node) {
	return lib.collect(node, n => n.tagName === 'INPUT').map(n => n.attrs.value);
}
function selectValue(node) {
	const sel = lib.collect(node, n => n.tagName === 'SELECT')[0];
	return sel ? String(sel.value) : '';
}

console.log('\n新管线：network.lock_get 载荷 → 面板字段');
const panelLte = panel('lte', PAYLOAD_LTE).node;
check('LTE 锁回填 type/bands/arfcns/pcis（PCI 0x64 已还原成十进制 100）',
	sameJson({ type: selectValue(panelLte), values: inputValues(panelLte) }, { type: '2', values: [ '3', '1850', '100' ] }),
	JSON.stringify({ type: selectValue(panelLte), values: inputValues(panelLte) }));
const panelNr = panel('nr', PAYLOAD_NR).node;
check('NR 锁回填 type/bands/arfcns/scs/pcis（两条 item 按列对齐，PCI 0x10/0x1A → 16/26）',
	sameJson({ type: selectValue(panelNr), values: inputValues(panelNr) },
		{ type: '2', values: [ '78,41', '643456,504990', '1,0', '16,26' ] }),
	JSON.stringify({ type: selectValue(panelNr), values: inputValues(panelNr) }));
const panelUnlocked = panel('lte', PAYLOAD_LTE && decodeLockReply('^LTEFREQLOCK: 0\nOK', 'lte')).node;
check('未上锁（type 0）→ 下拉是 Remove Lock、四列为空（与旧版同形）',
	selectValue(panelUnlocked) === '0' && sameJson(inputValues(panelUnlocked), [ '', '', '' ]),
	JSON.stringify({ type: selectValue(panelUnlocked), values: inputValues(panelUnlocked) }));
const panelNone = panel('lte', null).node;
check('取不到（后端不答）→ 与旧版空数组同形（type 0 / 四列空）',
	selectValue(panelNone) === '0' && sameJson(inputValues(panelNone), [ '', '', '' ]));

console.log('\n面板结构/文案（label、placeholder、帮助文字、按钮）');
const oldPanelNode = old.c.lockPanel('LTE network', 'lte', old.parser.parseLockData([], 'lte'));
const newPanelNode = panel('lte', null).node;
const shapeOpts = { attrs: true, skipAttrs: [ 'value' ], valueClass: VALUE_CLASS };
const oldShape = lib.serialize(oldPanelNode, shapeOpts), newShape = lib.serialize(newPanelNode, shapeOpts);
check('LTE 面板逐字一致（input 的 value 单独比较）', oldShape === newShape, oldShape === newShape ? '' : diffText(oldShape, newShape));
check('input 的 value 是唯一差异项（旧版空、新版有值）',
	JSON.stringify(old.parser.parseLockData([], 'lte')) === JSON.stringify({ type: '0', bands: '', arfcns: '', scs: '', pcis: '' }));
const oldNrNode = old.c.lockPanel('5G NR network', 'nr', old.parser.parseLockData([], 'nr'));
const newNrNode = panel('nr', null).node;
check('NR 面板逐字一致（多一个 SCS 字段）',
	lib.serialize(oldNrNode, shapeOpts) === lib.serialize(newNrNode, shapeOpts),
	diffText(lib.serialize(oldNrNode, shapeOpts), lib.serialize(newNrNode, shapeOpts)));

/* ------------------------------------------------------------ 写路径比较 */

/*
 * 逐个输入用例跑一遍「点 Review and apply → 点 Apply/Remove Lock」，记录
 *   旧：api.at(['lock', rat, type, bands, arfcns, scs, pcis])（CLI 位置式）
 *   新：api.routeCall('network.lock_apply', {rat, lock_type, items, verify})
 * 并断言新参数与用户填的四列一一对应。
 *
 * 面板的字段校验（哪些列必填、范围、条数上限）是 UI 行为，这一刀没有改：
 * 例如 NR 的 ARFCN/小区锁必须填 SCS，所以「不填 SCS」的用例只能从邻区卡片
 * 的 Lock 按钮走到（卡片不带 SCS，正是下面要证明的修复点）。
 */
const CASES = [
	{ key: 'lte/0', rat: 'lte', type: '0', fields: [ '', '', '', '' ], note: 'LTE 解锁',
	  oldArgs: [ 'lock', 'lte', '0', '', '', '' ], items: undefined },
	{ key: 'lte/3', rat: 'lte', type: '3', fields: [ '3,8', '', '', '' ], note: 'LTE 频段锁',
	  oldArgs: [ 'lock', 'lte', '3', '3,8', '', '' ], items: [ { band: 3 }, { band: 8 } ] },
	{ key: 'lte/1', rat: 'lte', type: '1', fields: [ '3', '1850', '', '' ], note: 'LTE ARFCN 锁',
	  oldArgs: [ 'lock', 'lte', '1', '3', '1850', '' ], items: [ { band: 3, arfcn: 1850 } ] },
	{ key: 'lte/2', rat: 'lte', type: '2', fields: [ '3', '1850', '', '100' ], note: 'LTE 小区锁',
	  oldArgs: [ 'lock', 'lte', '2', '3', '1850', '100' ], items: [ { band: 3, arfcn: 1850, pci: 100 } ] },
	{ key: 'nr/0', rat: 'nr', type: '0', fields: [ '', '', '', '' ], note: 'NR 解锁',
	  oldArgs: [ 'lock', 'nr', '0', '', '', '', '' ], items: undefined },
	{ key: 'nr/3', rat: 'nr', type: '3', fields: [ '78,41', '', '', '' ], note: 'NR 频段锁',
	  oldArgs: [ 'lock', 'nr', '3', '78,41', '', '', '' ], items: [ { band: 78 }, { band: 41 } ] },
	{ key: 'nr/1', rat: 'nr', type: '1', fields: [ '78', '643456', '1', '' ], note: 'NR ARFCN 锁（SCS 必填）',
	  oldArgs: [ 'lock', 'nr', '1', '78', '643456', '1', '' ], items: [ { band: 78, arfcn: 643456, scs: 1 } ] },
	{ key: 'nr/2', rat: 'nr', type: '2', fields: [ '78', '643456', '1', '16' ], note: 'NR 小区锁',
	  oldArgs: [ 'lock', 'nr', '2', '78', '643456', '1', '16' ], items: [ { band: 78, arfcn: 643456, scs: 1, pci: 16 } ] },
];

function driveRenderedPanel(node, scope, rat, type, fields) {
	const selects = lib.collect(node, n => n.tagName === 'SELECT');
	const inputs = lib.collect(node, n => n.tagName === 'INPUT');
	selects[0].value = type;
	// 字段顺序按 placeholder 定位（bands / arfcns / scs(仅 NR) / pcis）
	const byPlaceholder = (ph) => inputs.filter(n => n.attrs.placeholder === ph)[0];
	byPlaceholder(rat === 'nr' ? '78,41' : '3,8').value = fields[0];
	byPlaceholder(rat === 'nr' ? '630000,520000' : '1850,3450').value = fields[1];
	if (rat === 'nr')
		byPlaceholder('1,1').value = fields[2];
	byPlaceholder('100,200').value = fields[3];
	lib.pressButton(node, 'Review and apply');
	lib.modalButton(scope, type === '0' ? 'Remove Lock' : 'Apply Lock');
}

function drivePanel(read, api, c) {
	const side = lib.loadSide(read, api);
	const node = side.c.lockPanel(c.rat === 'nr' ? '5G NR network' : 'LTE network', c.rat, null);
	driveRenderedPanel(node, side.scope, c.rat, c.type, c.fields);
	return api.calls[api.calls.length - 1];
}

console.log('\n写路径：CLI 位置参数（旧） vs network.lock_apply items（新）');
for (const c of CASES) {
	const apiOld = lib.makeApi({}), apiNew = lib.makeApi({});
	const oldCall = drivePanel(oldSource, apiOld, c);
	const newCall = drivePanel(newSource, apiNew, c);
	const params = newCall && newCall.kind === 'routeCall' ? newCall.params : null;
	const itemsOk = c.items === undefined ? (params && params.items === undefined) : sameJson(params && params.items, c.items);
	check(c.note + ' → items ' + (c.items === undefined ? '（不带 items）' : JSON.stringify(c.items)),
		!!params && params.rat === c.rat && params.lock_type === Number(c.type) && params.verify === true && itemsOk,
		'params ' + JSON.stringify(params));
	check('  （旧参数逐字记录：[' + c.oldArgs.join(', ') + ']）',
		!!oldCall && oldCall.kind === 'at' && sameJson(oldCall.args, c.oldArgs), JSON.stringify(oldCall && oldCall.args));
}

/* 邻区卡片「Lock」按钮：单条 item；两个 RAT 的既有缺陷都在这里 */
console.log('\n邻区卡片 Lock 按钮（旧参数里的两处槽位缺陷）');
const NB_NR = { rat: 'NR', arfcn: '636648', pci: '64', rsrp: '-70', rsrq: '-10', sinr: '20', rxlev: '', band: 'n78' };
const NB_NR_NOPCI = { rat: 'NR', arfcn: '636648', pci: '', rsrp: '-70', rsrq: '-10', sinr: '20', rxlev: '', band: 'n78' };
const NB_LTE = { rat: 'LTE', arfcn: '1650', pci: '476', rsrp: '-85', rsrq: '-12', sinr: '', rxlev: '30', band: 'B3' };

function driveCard(read, api, nb, ratType) {
	const side = lib.loadSide(read, api);
	const node = side.c.cellLockCard(nb, 0, ratType, nb.band, ratType === 'nr');
	lib.pressButton(node, 'Lock');
	lib.modalButton(side.scope, 'Lock');
	return api.calls[api.calls.length - 1];
}

function cardCase(note, nb, ratType, expectedOldArgs, expectedItems) {
	const apiOld = lib.makeApi({}), apiNew = lib.makeApi({});
	const oldCall = driveCard(oldSource, apiOld, nb, ratType);
	const newCall = driveCard(newSource, apiNew, nb, ratType);
	check(note + ' → items ' + JSON.stringify(expectedItems),
		newCall.kind === 'routeCall' && newCall.params.verify === true && sameJson(newCall.params.items, expectedItems),
		JSON.stringify(newCall && newCall.params));
	check('  （旧参数：[' + oldCall.args.join(', ') + ']）', sameJson(oldCall.args, expectedOldArgs), JSON.stringify(oldCall.args));
}

cardCase('NR 小区卡片（有 PCI）', NB_NR, 'nr', [ 'lock', 'nr', '2', '78', '636648', '0', '64' ],
	[ { band: 78, arfcn: 636648, scs: 0, pci: 64 } ]);
cardCase('LTE 小区卡片（有 PCI）', NB_LTE, 'lte', [ 'lock', 'lte', '2', '3', '1650', '', '476' ],
	[ { band: 3, arfcn: 1650, pci: 476 } ]);
cardCase('NR ARFCN 卡片（无 PCI，旧版 SCS 槽是空串 → CLI 直接拒绝）', NB_NR_NOPCI, 'nr',
	[ 'lock', 'nr', '1', '78', '636648', '', '' ], [ { band: 78, arfcn: 636648 } ]);

/* ------------------------------------------------------------ 结果处理 */
/* ------------------------------------------------------------ 结果处理 */

const tick = lib.tick;

async function outcome(answers) {
	const fresh = lib.makeApi(answers);
	const scopeSide = lib.loadSide(newSource, fresh);
	const node = scopeSide.c.lockPanel('LTE network', 'lte', null);
	const selects = lib.collect(node, n => n.tagName === 'SELECT');
	const inputs = lib.collect(node, n => n.tagName === 'INPUT');
	selects[0].value = '2';
	inputs.filter(n => n.attrs.placeholder === '3,8')[0].value = '3';
	inputs.filter(n => n.attrs.placeholder === '1850,3450')[0].value = '1850';
	inputs.filter(n => n.attrs.placeholder === '100,200')[0].value = '64';
	lib.pressButton(node, 'Review and apply');
	lib.modalButton(scopeSide.scope, 'Apply Lock');
	await tick(); await tick(); await tick();
	return {
		notifications: scopeSide.scope.ui.notifications.map(n => ({ level: n.level, text: n.text })),
		reloadPending: scopeSide.scope.window.pending.map(p => p.delay),
		reloaded: scopeSide.scope.window.reloaded
	};
}

(async () => {
	console.log('\n结果处理（与旧版同样的提示与刷新）');
	const ok = await outcome({});
	check('成功 → 「Frequency lock updated.」+ 2.5s 后刷新',
		sameJson(ok.notifications, [ { level: '', text: 'Frequency lock updated.' } ]) && sameJson(ok.reloadPending, [ 2500 ]),
		JSON.stringify(ok));
	const applied = await outcome({ 'call:network.lock_apply': { results: [ { rat: 'lte', applied: false, error: 'AT busy' } ] } });
	check('模组拒绝（applied:false）→ danger 提示带后端消息',
		sameJson(applied.notifications, [ { level: 'danger', text: 'AT busy' } ]) && applied.reloadPending.length === 0,
		JSON.stringify(applied));
	const unverified = await outcome({ 'call:network.lock_apply': { results: [ { rat: 'lte', applied: true, verified: false, verify_error: 'frequency lock verification failed: expected 2, got 3' } ] } });
	check('写入成功但校验失败 → danger 提示（校验在后端，前端只显示）',
		sameJson(unverified.notifications, [ { level: 'danger', text: 'frequency lock verification failed: expected 2, got 3' } ]),
		JSON.stringify(unverified));
	const transport = await outcome({ 'call:network.lock_apply': { ok: false, error: 'backend down' } });
	check('传输/后端失败 → danger 提示（与旧版 await CLI 失败同一条路径）',
		sameJson(transport.notifications, [ { level: 'danger', text: 'backend down' } ]),
		JSON.stringify(transport));

	console.log('\n' + (failures ? failures + ' 项不一致'
		: '锁频面板：结构/文案一致，回填与写路径均由后端解码驱动（含两处旧参数缺陷的修复）'));
	process.exit(failures ? 1 : 0);
})();
