#!/usr/bin/env node
'use strict';

/*
 * 邻区卡片一致性证明（LuCI 无线页，item 9 的邻区切片）
 *
 * 本批把「邻区」从 LuCI 自己切 AT 文本帧，改成读后端路由：
 *   - 诊断区块的邻区卡片：`advanced radio-diagnostics` 帧的 ^MONNC 段 → `cell.neighbors`
 *   - SSB 面板的邻区卡片：载荷里的 arfcn 前端自己查表 → 用载荷自带的 band
 *   - 扫频弹窗的邻区卡片：cellscan 帧的 ^MONNC 段 → `cell.neighbors`
 * 频段表（ARFCN→频段号）同时从 parser.js 消失，移到后端 core::radio，
 * 前端只剩「频段号 → B3/n78 标签」（bandLabel）。
 *
 * 验证方式：把 HEAD（迁移前）与当前工作区的 parser.js / components.js /
 * network.js 装进同一个桩 DOM，用**同一份调制解调器应答**渲染，然后比较：
 *   1. 结构 + 文案：除下面 4 个取值槽（VALUE_CLASS）外，整块诊断 DOM
 *      逐字一致（数字归一化后）；
 *   2. 取值槽：差异必须完全等于 EXPECTED_VALUE_DIFFS —— 每一条都是
 *      「显示真值」的修正（十六进制 PCI → 十进制；n78 以前落空显示 'NR'），
 *      没有样式/文案/布局变化；
 *   3. 旧管线的取值由 HEAD 的 parser.js 亲自跑出来（parseMonnc / arfcnToBand），
 *      新管线的取值由当前 network.js 渲染出来；后端解码规则另有一组
 *      Rust 单元测试样本（RUST_PINS）钉住脚本里的 mini 解码器。
 *
 * 用法：
 *   node scripts/prove-neighbors-parity.js            # 基线 = HEAD（提交前跑）
 *   node scripts/prove-neighbors-parity.js 663f989    # 基线 = 某一批之前的提交
 * 退出码 0 = 通过，1 = 不一致，2 = 基线选错了（基线里已经有本批）。
 *
 * 迁移收官后仍作为恒真回归断言纳入 CI（frontend-proofs job，显式传基线
 * `663f989`——邻区切片之前，远端原生提交）。本地复跑同参数即可。
 */

const fs = require('fs');
const path = require('path');
const cp = require('child_process');

const REPO = path.resolve(__dirname, '..');
const RES = 'luci-app-mt5700m/htdocs/luci-static/resources';

const BASELINE = process.argv[2] || 'HEAD';

function oldSource(rel) {
	return cp.execSync('git show ' + BASELINE + ':' + rel, { cwd: REPO, maxBuffer: 64 * 1024 * 1024 }).toString();
}
function newSource(rel) {
	return fs.readFileSync(path.join(REPO, rel), 'utf8');
}

/* ------------------------------------------------------------------ 公共桩 */
// DOM 桩、序列化与装载逻辑在 scripts/lib/luci-stub.js（三个证明脚本共用一份）
const lib = require('./lib/luci-stub');
const fixtures = require('./lib/at-fixtures');

const VALUE_CLASS = /mt-lock-cell-name|mt-lock-cell-desc|mt-signal-value|mt-ssb-serving-title/;

// 兼容本脚本原有的调用签名（第二个参数是 keepValues 布尔）
function serialize(node, keepValues) { return lib.serialize(node, { keepValues: keepValues, valueClass: VALUE_CLASS }); }
function slotValues(node) { return lib.slotValues(node, VALUE_CLASS); }
function slotsOf(node, cls) { return lib.slotsOf(node, VALUE_CLASS, cls); }
const textOf = lib.textOf;
const lines = lib.lines;
const countTag = lib.countTag;

/* ------------------------------------------------------------ 模块装载 */

function loadSide(read) {
	return lib.loadSide(read, lib.makeApi({}));
}

/* ------------------------------------------ 后端解码契约（mini 解码器）
 *
 * 只复刻本批用到的路径：^MONNC → 领域值。规则逐条对应
 * modules/cell/parser.rs（hex PCI、core::radio 频段表、NR 1/8 换算、
 * 哨兵值置空、空字段缺省），并由 RUST_PINS 的样本钉住。
 */
const NO_VALUE = [-1256, -348, -188, 32767, 255];

function measurement(text, limit) {
	if (text === undefined || text === null || text === '') return undefined;
	const n = Number(text);
	if (!isFinite(n)) return undefined;
	if (NO_VALUE.indexOf(n) !== -1) return undefined;
	return (limit !== undefined && Math.abs(n) > limit) ? (n / 8).toFixed(1) : String(text);
}

function arfcnToBand(rat, arfcn) {
	const LTE = [[0, 599, 1], [1200, 1949, 3], [2400, 2649, 5], [3450, 3799, 8], [36200, 36349, 34], [37750, 38249, 38], [38250, 38649, 39], [38650, 39649, 40], [39650, 41589, 41]];
	const NR = [[422000, 434000, 1], [361000, 376000, 3], [173800, 178800, 5], [185000, 192000, 8], [151600, 160600, 28], [499200, 537999, 41], [620000, 653333, 78], [653334, 680000, 77], [693334, 733333, 79]];
	const table = rat === 'LTE' ? LTE : rat === 'NR' ? NR : null;
	if (!table) return undefined;
	for (const [lo, hi, band] of table) if (arfcn >= lo && arfcn <= hi) return band;
	return undefined;
}

function decodeMonnc(lines) {
	const cells = [];
	for (const line of lines) {
		const i = line.indexOf('^MONNC:');
		if (i < 0) continue;
		const parts = line.slice(i + 7).trim().split(',').map(s => s.trim());
		const rat = (parts[0] || '').toUpperCase();
		if (rat !== 'LTE' && rat !== 'NR') continue;
		const arfcn = parts[1] !== undefined && parts[1] !== '' ? Number(parts[1]) : undefined;
		const pci = parts[2] !== undefined && parts[2] !== '' ? parseInt(parts[2], 16) : undefined;
		const cell = { type: rat };
		if (arfcn !== undefined) cell.arfcn = arfcn;
		if (pci !== undefined) cell.pci = pci;
		const set = (key, value) => { if (value !== undefined) cell[key] = value; };
		if (rat === 'LTE') {
			set('rsrp', measurement(parts[3]));
			set('rsrq', measurement(parts[4]));
			set('rxlev', measurement(parts[5]));
		} else {
			set('rsrp', measurement(parts[3], 157));
			set('rsrq', measurement(parts[4], 43.5));
			set('sinr', measurement(parts[5], 40));
		}
		set('band', arfcn === undefined ? undefined : arfcnToBand(rat, arfcn));
		cells.push(cell);
	}
	return { cells };
}

/* 与 Rust 单元测试逐字对应的样本 */
const RUST_PINS = [
	// modules/cell/parser.rs::monnc_parses_both_rasters_and_skips_none
	{ raw: ['^MONNC: NONE', '^MONNC: LTE,1850,64,-85,-12,30', '^MONNC: NR,643456,1A,-95,-11,-5'],
	  expect: [{ type: 'LTE', arfcn: 1850, pci: 100, rsrp: '-85', rsrq: '-12', rxlev: '30', band: 3 },
	           { type: 'NR', arfcn: 643456, pci: 26, rsrp: '-95', rsrq: '-11', sinr: '-5', band: 78 }] },
	// modules/cell/parser.rs::monnc_scales_oversized_nr_values
	{ raw: ['^MONNC: NR,643456,1A,-760,-96,-320'], expect: [{ type: 'NR', arfcn: 643456, pci: 26, rsrp: '-95.0', rsrq: '-12.0', sinr: '-40.0', band: 78 }] },
	// modules/cell/parser.rs::monnc_drops_sentinel_measurements
	{ raw: ['^MONNC: NR,643456,1A,-1256,-348,-188', '^MONNC: LTE,1850,64,255,32767,-12'],
	  expect: [{ type: 'NR', arfcn: 643456, pci: 26, band: 78 },
	           { type: 'LTE', arfcn: 1850, pci: 100, rxlev: '-12', band: 3 }] },
];

/* -------------------------------------------------------------- 场景数据 */

// 帧里的 ^MONNC 段与同一批邻区的领域值（= mockAT.ts 的 api.cell.neighbors 契约）
const MONNC_LINES = [
	'^MONNC: LTE,1650,1DC,-85,-12,30',
	'^MONNC: NR,636648,40,-70,-10,20',
];
const NEIGHBORS_PAYLOAD = decodeMonnc(MONNC_LINES);

// beam.ssb 载荷：mockAT.ts 的 api.beam.ssb（与同一份 ^NRSSBID 应答一致）。
// 迁移前载荷里没有 band；core::radio 现在给出 n78（两处）。
const SSB_PAYLOAD_OLD = {
	servingCell: { arfcn: '636648', cid: '1A2B3C', pci: '506', rsrp: 85, sinr: 50, ta: 1,
		ssbs: [ { ssbId: 0, rsrp: 90 }, { ssbId: 1, rsrp: 80 }, { ssbId: 2, rsrp: 70 }, { ssbId: 3, rsrp: 60 } ] },
	neighborCells: [ { pci: '506', arfcn: '632448', rsrp: 88, sinr: 45,
		ssbs: [ { ssbId: 0, rsrp: 90 }, { ssbId: 1, rsrp: 80 }, { ssbId: 2, rsrp: 70 } ] } ],
};
const SSB_PAYLOAD_NEW = JSON.parse(JSON.stringify(SSB_PAYLOAD_OLD));
SSB_PAYLOAD_NEW.servingCell.band = arfcnToBand('NR', 636648);
SSB_PAYLOAD_NEW.neighborCells[0].band = arfcnToBand('NR', 632448);

// 诊断区块其余各行：两侧收到同一份路由载荷（本批未改它们）
const SHARED_PAYLOADS = {
	mcs: { uplink: { rat: 1, carriers: [ { index: 1, group: 1, rat: 'NR', mcs_table_index: 0, code0: 25, code1: 23 }, { index: 2, group: 1, rat: 'NR', mcs_table_index: 0, code0: 21, code1: 19 } ], avg_mcs: 22 },
	       downlink: { rat: 1, carriers: [ { index: 1, group: 1, rat: 'NR', mcs_table_index: 0, code0: 18, code1: 16 } ], avg_mcs: 17 } },
	txPower: { carriers: [ { pusch: 23, pucch: 22, srs: 21, prach: 20, freq: 3549720 } ] },
	qos: { active_cid: 8, qci: 9 },
	endc: { available: 1, plmnAvailable: 1, restricted: 1, established: 1 },
	ca: { lte_secondary_count: 1, secondary_connection_count: 1 },
	registration: { state: 1, act: 11 },
	ims: { enabled: 1, registered: 1 },
};

// 迁移前的文本帧（诊断区块里已迁走的行就是从这些段切出来的）
const OLD_FRAME = [
	'===== Neighbour cells: AT^MONNC =====',
	...MONNC_LINES,
	'OK',
	'',
].join('\n');

/* ------------------------------------------------------------------ 检查 */

let failures = 0;
function check(name, ok, detail) {
	if (ok) { console.log('  ok   ' + name); return; }
	failures++;
	console.log('  FAIL ' + name + (detail ? '\n       ' + detail : ''));
}
function firstDifference(a, b) {
	const la = a.split('\n'), lb = b.split('\n');
	for (let i = 0; i < Math.max(la.length, lb.length); i++)
		if (la[i] !== lb[i]) return 'line ' + (i + 1) + ':\n       old: ' + la[i] + '\n       new: ' + lb[i];
	return '';
}
function sameJson(a, b) { return JSON.stringify(a) === JSON.stringify(b); }

console.log('Rust 契约钉子（mini 解码器 vs modules/cell/parser.rs 的单测样本）');
for (const pin of RUST_PINS) {
	const got = decodeMonnc(pin.raw).cells, want = pin.expect;
	let ok = got.length === want.length;
	if (ok) for (let i = 0; i < want.length && ok; i++) {
		ok = sameJson(got[i], want[i]);
		if (!ok) console.log('       ' + pin.raw[0].slice(0, 34) + '…\n       got  ' + JSON.stringify(got[i]) + '\n       want ' + JSON.stringify(want[i]));
	}
	check(pin.raw.join(' | ').slice(0, 44) + '…', ok);
}

const before = loadSide(oldSource);
const after = loadSide(newSource);
if (typeof before.parser.parseMonnc !== 'function') {
	console.error('基线 ' + BASELINE + ' 已经包含本批（parser.js 里没有 parseMonnc 了）——'
		+ '没有可比较的旧管线。\n本批的基线是 663f989：\n'
		+ '  node scripts/prove-neighbors-parity.js 663f989');
	process.exit(2);
}

/* 旧管线对同一份应答给出的取值（跑 HEAD 的 parser.js） */
console.log('\n旧管线取值（HEAD 的 parser.js 亲自跑）');
const oldMonnc = before.parser.parseMonnc(MONNC_LINES.join('\n'));
check('parseMonnc 的 PCI 是帧里的十六进制原文', oldMonnc[0].pci === '1DC' && oldMonnc[1].pci === '40',
	JSON.stringify(oldMonnc.map(n => n.pci)));
check('arfcnToBand(636648, NR) 落空 → 显示 RAT 名', before.parser.arfcnToBand(636648, 'NR') === 'NR');
check('arfcnToBand(632448, NR) 落空 → 显示 RAT 名', before.parser.arfcnToBand(632448, 'NR') === 'NR');
check('arfcnToBand(1650, LTE) = B3（两侧一致）', before.parser.arfcnToBand(1650, 'LTE') === 'B3');
check('新管线不再有 parseMonnc / arfcnToBand 之外的邻区表', after.parser.parseMonnc === undefined);

/* 1) 结构与文案：四个取值槽以外必须逐字一致 */
console.log('\n诊断区块 DOM（取值槽只比较有无，其余逐字比较）');
const oldDiag = before.view.radioDiagnostics(OLD_FRAME, Object.assign({}, SHARED_PAYLOADS, { ssb: SSB_PAYLOAD_OLD, neighbors: NEIGHBORS_PAYLOAD }));
const newDiag = after.view.radioDiagnostics(Object.assign({}, SHARED_PAYLOADS, { ssb: SSB_PAYLOAD_NEW, neighbors: NEIGHBORS_PAYLOAD }));
const oldShape = serialize(oldDiag), newShape = serialize(newDiag);
check('结构/文案逐字一致（数字归一化，取值槽只比有无）', oldShape === newShape,
	oldShape === newShape ? '' : firstDifference(oldShape, newShape));

/* 2) 取值槽差异 == 预期清单 */
console.log('\n邻区取值（每条差异都必须是「显示真值」的修正）');
const oldValues = slotValues(oldDiag), newValues = slotValues(newDiag);
const diffs = [];
for (let i = 0; i < Math.max(oldValues.length, newValues.length); i++)
	if (oldValues[i] !== newValues[i]) diffs.push([oldValues[i], newValues[i]]);
const EXPECTED_VALUE_DIFFS = [
	// SSB 服务小区：旧 JS 频段表只覆盖到 3 GHz，636648（n78）落空显示 RAT 名
	['Serving cell · NR', 'Serving cell · n78'],
	// SSB 邻区 632448：同样由 core::radio 给出 n78
	['NR · 632448', 'n78 · 632448'],
	// 诊断邻区 NR 636648：n78，且 PCI 由十六进制 `40` 变成十进制 64
	['NR · 636648', 'n78 · 636648'],
	['PCI 40', 'PCI 64'],
];
check('取值差异 == 预期清单（' + EXPECTED_VALUE_DIFFS.length + ' 条）', sameJson(diffs, EXPECTED_VALUE_DIFFS),
	sameJson(diffs, EXPECTED_VALUE_DIFFS) ? '' : 'got  ' + JSON.stringify(diffs) + '\n       want ' + JSON.stringify(EXPECTED_VALUE_DIFFS));
check('诊断邻区节只显示 NR（NR 优先，与迁移前相同）',
	slotValues(newDiag).indexOf('B3 · 1650') === -1 && countTag(serialize(newDiag, true), 'div.mt-lock-cell-card') === 2);

/* 3) 扫频弹窗：邻区段读路由，标题分组逻辑不变
 *
 * 663f989 的 `renderCellScan(raw)` 从帧里的 ^MONNC 段切邻区，没有载荷入口，
 * 所以这一节只渲染**当前**实现（模块级函数由视图导出），断言的是「同一份
 * `cell.neighbors` 领域值渲染出与迁移前相同的邻区小节」。弹窗本身的新旧一致性
 * 由 `scripts/prove-cellscan-parity.js fceb6ea` 负责（它能在两边都驱动真实路径）。
 */
const scanFrame = [
	'===== Serving cell: AT^MONSC =====', '^MONSC: NR,460,00,636648,0,10321,1FA,2F01,-82,-9', 'OK',
	'===== Frequency scan: AT^CELLSCAN =====', '^CELLSCAN: 636648,506,-82', 'OK', '',
].join('\n');
const CELL_PAYLOAD = Object.assign({ band: '78' }, fixtures.decodeCell('^MONSC: NR,460,00,636648,0,10321,1FA,2F01,-82,-9'));
const SIGNAL_PAYLOAD = fixtures.decodeSignal('^HCSQ: "NR",58,165,22');
const scan = after.view.renderCellScan(CELL_PAYLOAD, SIGNAL_PAYLOAD, NEIGHBORS_PAYLOAD);
const scanShape = serialize(scan, true);
check('NR 邻区优先成节：NR neighbour cells (#)', scanShape.indexOf('text=NR neighbour cells (#)') !== -1);
check('其余 LTE 邻区成节：LTE neighbour cells (#)', scanShape.indexOf('text=LTE neighbour cells (#)') !== -1);
check('两张邻区卡片', countTag(scanShape, 'div.mt-lock-cell-card') === 2);
const scanCards = [];
lib.slotEntries(scan, VALUE_CLASS).filter(e => e.cls.split(' ').indexOf('mt-lock-cell-name') !== -1).forEach(e => scanCards.push(e.text));
const scanPcis = slotsOf(scan, 'mt-lock-cell-desc');
check('弹窗邻区卡片取值 = 领域值（PCI 十进制 / 频段标签）',
	sameJson(scanCards, [ 'n78 · 636648', 'B3 · 1650' ]) && sameJson(scanPcis, [ 'PCI 64', 'PCI 476' ]),
	JSON.stringify({ cards: scanCards, pcis: scanPcis }));
check('服务小区现在也读载荷（ARFCN:636648 / 十进制 PCI:506 / 模块频段 n78）',
	textOf(scan).indexOf('ARFCN:636648') !== -1 && textOf(scan).indexOf('PCI:506') !== -1 && textOf(scan).indexOf('n78') !== -1,
	textOf(scan).slice(0, 100));
// 扫频原文卡片：迁移前后都取不到（`section()` 的前缀规则要求 '===== <label>:' 前缀，
// cli.rs 打的是 '===== Frequency scan: AT^CELLSCAN ====='）——既有缺陷，不在本批范围。
check('扫频原文段迁移前后同样解析为空（既有缺陷，未改动）',
	before.parser.section(scanFrame, 'Frequency scan: AT^CELLSCAN') === '' && after.parser.section(scanFrame, 'Frequency scan: AT^CELLSCAN') === '');

/* 4) 回退与 NONE：与旧管线逐字一致 */
console.log('\n回退与 NONE 场景');
const nonePayload = { cells: [] };
const noneDiag = after.view.radioDiagnostics(Object.assign({}, SHARED_PAYLOADS, { ssb: null, neighbors: nonePayload }));
const noneOld = before.view.radioDiagnostics(OLD_FRAME.replace(MONNC_LINES.join('\n'), '^MONNC: NONE'), Object.assign({}, SHARED_PAYLOADS, { ssb: null }));
check('^MONNC: NONE → 无邻区节，DOM 与迁移前一致（整块逐字）', serialize(noneDiag, true) === serialize(noneOld, true),
	serialize(noneDiag, true) === serialize(noneOld, true) ? '' : firstDifference(serialize(noneOld, true), serialize(noneDiag, true)));
const unknownOld = before.view.radioDiagnostics(OLD_FRAME.replace(MONNC_LINES.join('\n'), '^MONNC: NR,1,1,-70,-10,20'), Object.assign({}, SHARED_PAYLOADS, { ssb: null }));
const unknownNew = after.view.radioDiagnostics(Object.assign({}, SHARED_PAYLOADS, { ssb: null, neighbors: decodeMonnc(['^MONNC: NR,1,1,-70,-10,20']) }));
check('频段查不到 → 卡片标题仍显示 RAT 名（与旧 arfcnToBand 回退一致）',
	sameJson(slotValues(unknownOld), slotValues(unknownNew)), JSON.stringify(slotValues(unknownNew)));

console.log('\n' + (failures ? failures + ' 项不一致' : '结构/文案一致；取值差异全部为预期修正')
	+ '（' + EXPECTED_VALUE_DIFFS.length + ' 处：十六进制 PCI → 十进制、n78 由后端频段表给出）');
process.exit(failures ? 1 : 0);
