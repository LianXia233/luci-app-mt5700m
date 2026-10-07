#!/usr/bin/env node
'use strict';

/*
 * 小区扫描弹窗一致性证明（LuCI 无线页，扫描弹窗切片）
 *
 * 这一刀把弹窗从「切 `mt5700m-at cellscan` 文本帧」改成「读统一路由」：
 *   - 服务小区卡片：^MONSC 文本 + `parser.arfcnToBand` 猜频段 → `cell.get`
 *     （modules/cell 解码，PCI/CID 十进制、band 来自 core::radio）与
 *     `signal.get`（modules/signal 解码 ^HCSQ，与整页仪表同一份读数）
 *   - 扫描状态：`mt5700m-at cellscan-result` 的文本 JSON → `cell.scan_result`
 *   - 触发扫描：靠 `cellscan` 动词的副作用 → `cell.scan_start`
 *   - 删除 `parser.parseMonsc`/`parser.arfcnToBand`（^MONSC 布局的又一份 JS
 *     实现与那张只覆盖到 3 GHz 的频段表）与 `api.atCellscan`/
 *     `api.atCellscanResult` 两个动词包装；从此无线页只剩 `atRadio` 一次
 *     CLI 调用
 *   - 旧实现里从未渲染过的「Frequency scan」卡片（`parser.section()` 的前缀
 *     拼写对不上 CLI 的输出）随文本路径一起删除，弹窗可见内容不变
 *
 * 用法：
 *   node scripts/prove-cellscan-parity.js [基线]   # 基线默认 HEAD，本刀之前是 fceb6ea
 * 退出码 0 = 通过，1 = 不一致，2 = 基线选错了（基线里已经没有旧实现）。
 *
 * 脚本做四件事：
 *   1. 用 Rust 单测样本钉住夹具（scripts/lib/at-fixtures.js）；
 *   2. 两边都走真实路径「点 Cell Scan → Continue」，逐节点比较弹窗 DOM；
 *   3. 断言所有差异都落在服务小区卡片内（邻区卡片逐字一致），且就是四种已知
 *      的取值修正：PCI/CID 十六进制→十进制、频段由模块给出（旧版 3 GHz 以上
 *      的线性换算落在表外，显示成 "NR · NR"）、以及三条信号条的读数改由
 *      ^HCSQ 解码（旧版 ^MONSC 的 NR 后备里根本没读 SINR）；
 *   4. 断言调用面：不再有 at cellscan / at cellscan-result，弹窗只读四条路由。
 */

const fs = require('fs');
const path = require('path');
const cp = require('child_process');
const lib = require('./lib/luci-stub');
const fixtures = require('./lib/at-fixtures');
const {
	round1, decodeSignal, decodeCell, decodeTemps, temperatureText, textFrame,
} = fixtures;

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

/* ------------------------------------------------------------ 夹具钉子 */
console.log('Rust 契约钉子（夹具 vs modules/*/parser.rs 的单测样本）');
check('cell：NR 十六进制 cid/pci/lac 与十进制 scs（0x10321→66337 / 0x1FA→506）',
	sameJson(decodeCell('^MONSC: NR,460,00,636648,0,10321,1FA,2F01,-82,-9'),
		{ sysmode: 'NR', mcc: '460', mnc: '00', channel: '636648', scs: 0, cid: '66337', pci: 506, lac: '12033' }),
	JSON.stringify(decodeCell('^MONSC: NR,460,00,636648,0,10321,1FA,2F01,-82,-9')));
check('signal：^HCSQ NR 索引 → dBm/dB（58→-83 / 165→12.8 / 22→-9）',
	sameJson(decodeSignal('^HCSQ: "NR",58,165,22'), { sysmode: 'NR', rsrp: -83, sinr: 12.8, rsrq: -9 }),
	JSON.stringify(decodeSignal('^HCSQ: "NR",58,165,22')));
check('signal：^HCSQ LTE（rssi 54→-67 / rsrp 24→-117 / rsrq 155 上限 -3）',
	sameJson(decodeSignal('^HCSQ: "LTE",54,24,45,155'),
		{ sysmode: 'LTE', rssi: 54 - 121, rsrp: 24 - 141, sinr: round1(-20.2 + 45 * 0.2), rsrq: -3 }),
	JSON.stringify(decodeSignal('^HCSQ: "LTE",54,24,45,155')));
check('temperature：peak 取最大（45.1 / mimo_pa）',
	decodeTemps('^CHIPTEMP: 432,445,451,398,401,407,441,442,449,0,65535,448').peak === 45.1);

/* ------------------------------------------------------------ 调制解调器读数
 * 服务小区 NR：^MONSC 报的 PCI=0x1FA、CID=0x10321、RSRP=-82、RSRQ=-9（NR 的
 * ^MONSC 后备里没有 SINR），^HFREQINFO 报 band 78；^HCSQ 是同一批测量的索引
 * 形态（-83 / -9 / 12.8）。邻区两条（数据来自 cell.neighbors，两边同一份）。
 */
const MODEM = {
	hcsq: '^HCSQ: "NR",58,165,22',
	monsc: '^MONSC: NR,460,00,636648,0,10321,1FA,2F01,-82,-9',
	hfreq: '^HFREQINFO: 1,7,78,636648,3549720,100000,650048,3750720,100000',
	chiptemp: '^CHIPTEMP: 432,445,451,398,401,407,441,442,449,0,65535,448',
	syscfgex: '^SYSCFGEX: 0803,3FFFFFFF,1,2,7FFFFFFFFFFFFFFF'
};
const NEIGHBORS = {
	cells: [
		{ type: 'NR', arfcn: 636648, pci: 501, rsrp: -95, rsrq: -12, sinr: 8, band: 78 },
		{ type: 'NR', arfcn: 504990, pci: 22, band: 41 }
	]
};
const CELL = Object.assign({ band: '78' }, decodeCell(MODEM.monsc));
const SIGNAL = decodeSignal(MODEM.hcsq);

/* 旧输入：`mt5700m-at cellscan` 的帧（发布以来：服务小区 + Frequency scan 头） */
function oldScanFrame() {
	return textFrame([
		[ 'Serving cell', 'AT^MONSC', MODEM.monsc ],
		[ 'Frequency scan', 'AT^CELLSCAN', '^CELLSCAN: SCANNING' ]
	]) + temperatureText(decodeTemps(MODEM.chiptemp));
}

/* ------------------------------------------------------------ 桩 */
function apiFor(answers) {
	const api = lib.makeApi(answers);
	api.atRadio = () => api.at([ 'radio' ]);
	api.atCellscan = () => api.at([ 'cellscan' ]);
	api.atCellscanResult = () => api.at([ 'cellscan-result' ]);
	return api;
}
const ANSWERS = {
	'at:radio': { stdout: textFrame([ [ 'Radio mode', 'AT^SYSCFGEX?', MODEM.syscfgex ] ]), stderr: '' },
	'at:cellscan': { stdout: oldScanFrame(), stderr: '' },
	'at:cellscan-result': { stdout: '{"running":false,"state":"idle"}\n', stderr: '' },
	'route:cell.get': CELL,
	'route:signal.get': SIGNAL,
	'route:cell.neighbors': NEIGHBORS,
	'route:cell.scan_result': { running: false, state: 'idle', cells: [], count: 0 },
	'route:cell.scan_start': { started: true }
};

const apiOld = apiFor(ANSWERS);
const apiNew = apiFor(ANSWERS);
const old = lib.loadSide(oldSource, apiOld);
const neu = lib.loadSide(newSource, apiNew);

if (typeof old.parser.parseMonsc !== 'function' || typeof old.api.atCellscan !== 'function') {
	console.error('基线 ' + BASELINE + ' 已经包含这一刀（parser.js 里没有 parseMonsc / api.js 里没有 atCellscan）'
		+ '——没有旧管线可比较。\n本刀的基线是 fceb6ea：\n  node scripts/prove-cellscan-parity.js fceb6ea');
	process.exit(2);
}

/* ------------------------------------------------------------ 跑真实路径
 * 与用户操作一致：load()/render() → 点 Cell Scan → 点 Continue → 跑完微任务。
 */
function renderView(side) {
	side.view.load();
	const holder = side.view.render();
	return side.view.contentReady.then(() => holder);
}
async function openScanModal(side) {
	const holder = await renderView(side);
	lib.pressButton(holder, 'Cell Scan');
	lib.modalButton(side.scope, 'Continue');
	for (let i = 0; i < 6; i++) await lib.tick();
	const modals = side.scope.ui.modals;
	return modals[modals.length - 1];
}
const scanBody = (modal) => modal.children[0];
function diffLines(a, b) {
	const la = String(a).split('\n'), lb = String(b).split('\n');
	const out = [];
	for (let i = 0; i < Math.max(la.length, lb.length); i++)
		if (la[i] !== lb[i]) out.push({ line: i + 1, path: la[i] ? String(la[i]).split(' ')[0] : '', old: la[i], new: lb[i] });
	return out;
}

(async function () {
	const oldModal = await openScanModal(old);
	const newModal = await openScanModal(neu);

	console.log('\n弹窗（标题 + 服务小区卡片 + 邻区卡片）');
	check('标题一致（Cell Scan）', oldModal.title === newModal.title,
		JSON.stringify({ old: oldModal.title, new: newModal.title }));

	const opts = { attrs: true, skipAttrs: [ 'value' ] };
	const oldShape = lib.serialize(scanBody(oldModal), opts);
	const newShape = lib.serialize(scanBody(newModal), opts);
	// `lib.serialize` 把数字归一成 '#'（结构比较用不到具体数值），所以取值差异
	// 另外用 `lib.lines`（保留原文）逐行比较。
	const diffs = diffLines(oldShape, newShape);
	const textDiffs = diffLines(lib.lines(scanBody(oldModal)), lib.lines(scanBody(newModal)));

	/* 旧版那两条死分支：`parser.section(raw, 'Frequency scan: AT^CELLSCAN')` 找的是
	 * 带冒号的 `'===== Frequency scan:' + …`，而帧里是 `===== Frequency scan:
	 * AT^CELLSCAN =====` —— 前缀永不匹配，所以旧版从来没渲染过 ^CELLSCAN 原文，
	 * “+CME ERROR: 3” 的提示卡片也从未出现。新版把这段连同 parser.section 的调用
	 * 一起删除，弹窗可见内容因此不变。 */
	check('旧版从来没有渲染 ^CELLSCAN 原文 / Frequency scan 卡片（死分支）',
		oldShape.indexOf('mt-scan-raw') === -1 && lib.textOf(scanBody(oldModal)).indexOf('^CELLSCAN') === -1,
		oldShape.split('\n').filter(l => l.indexOf('scan-raw') !== -1).join(' | '));
	check('新版同样没有 ^CELLSCAN 卡片（弹窗可见内容不变）',
		newShape.indexOf('mt-scan-raw') === -1 && lib.textOf(scanBody(newModal)).indexOf('^CELLSCAN') === -1);

	console.log('\n差异必须全部落在服务小区卡片内（邻区卡片逐字一致）');
	const SERVING = /mt-ssb-serving|mt-signal-(bar|label|track|fill|value)/;
	const outside = diffs.filter(d => !SERVING.test(d.path));
	check('没有服务小区卡片以外的差异（共 ' + diffs.length + ' 行差异，全部在服务小区）',
		outside.length === 0, JSON.stringify(outside.slice(0, 4), null, 1));
	const tail = (shape) => {
		const lines = shape.split('\n');
		const at = lines.findIndex(l => l.indexOf('mt-lock-cell-grid') !== -1);
		return at === -1 ? '' : lines.slice(at).join('\n');
	};
	check('邻区小节逐字一致（2 张卡片，取值与结构全部相同）',
		tail(oldShape).length > 200 && tail(oldShape) === tail(newShape),
		tail(oldShape) === tail(newShape) ? '' : '长度 ' + tail(oldShape).length + ' vs ' + tail(newShape).length);

	console.log('\n取值差异清单（结构比较把数字归一成 #，所以这里按归一化后的形态断言）');
	check('共 4 行差异，全部在服务小区卡片内',
		textDiffs.length === 4 && textDiffs.every(d => SERVING.test(String(d.old).split(' ')[0])),
		JSON.stringify(textDiffs.map(d => ({ old: d.old.split(' text=')[1] || d.old, new: d.new.split(' text=')[1] || d.new })), null, 1).slice(0, 1600));
	check('① 标题：旧 "NR · NR · PCI:#FA · ARFCN:#"（3 GHz 以上的 ARFCN 用 0–3 GHz 的线性'
		+ '换算落在表外，回退成 RAT 名）→ 新 "NR · n# · PCI:# · ARFCN:#"（模块的频段号 + 十进制 PCI）',
		textDiffs.filter(d => d.old.indexOf('NR · NR · PCI:#FA · ARFCN:#') !== -1
			&& d.new.indexOf('NR · n# · PCI:# · ARFCN:#') !== -1).length === 1,
		JSON.stringify(textDiffs.map(d => d.old.split(' text=')[1]), null, 1).slice(0, 900));
	/* ②③④ 都是第三条信号条：旧版 ^MONSC 的 NR 布局里没有 SINR（第 11 个字段），
	 * 于是空读数走 unknown/`--`；^HCSQ 有该索引，解出 12.8 dB。前两条（RSRP/RSRQ）
	 * 数值同源，归一化后不产生差异。 */
	const nFill = textDiffs.filter(d => /i\.mt-signal-fill\.unknown$/.test(d.old) && /i\.mt-signal-fill\.fair$/.test(d.new)).length;
	const nValue = textDiffs.filter(d => /span\.mt-signal-value\.unknown$/.test(d.old) && /span\.mt-signal-value\.fair$/.test(d.new)).length;
	const nText = textDiffs.filter(d => /span\.mt-signal-value\.unknown text=--dB$/.test(d.old)
		&& /span\.mt-signal-value\.fair text=#\.#dB$/.test(d.new)).length;
	check('②③④ SINR 信号条：unknown 类 → fair 类（进度条与读数各一处）+ 读数 "--dB" → "#.#dB"',
		nFill === 1 && nValue === 1 && nText === 1,
		'fill=' + nFill + ' value=' + nValue + ' text=' + nText
		+ '\n       ' + JSON.stringify(textDiffs.map(d => [ d.old.split(' text=')[0], d.new.split(' text=')[0] ]), null, 1).slice(0, 900));
	check('结构差异行数在预期内（≤10）', diffs.length <= 10, 'got ' + diffs.length);

	console.log('\n读数（原文，取自 DOM 文本）');
	const bars = (modal) => lib.collect(scanBody(modal), n => n.tagName === 'SPAN'
		&& String(n.attrs && n.attrs['class'] || '').indexOf('mt-signal-value') !== -1).map(n => lib.textOf(n));
	const barsOld = bars(oldModal), barsNew = bars(newModal);
	check('服务小区三条信号条：旧 ' + JSON.stringify(barsOld.slice(0, 3)) + ' → 新 ' + JSON.stringify(barsNew.slice(0, 3)),
		sameJson(barsOld.slice(0, 3), [ '-82dBm', '-9dB', '--dB' ])
		&& sameJson(barsNew.slice(0, 3), [ '-83dBm', '-9dB', '12.8dB' ]),
		JSON.stringify({ old: barsOld.slice(0, 3), new: barsNew.slice(0, 3) }));
	check('邻区卡片的读数逐字一致（' + JSON.stringify(barsNew.slice(3)) + '）',
		sameJson(barsOld.slice(3), barsNew.slice(3)) && barsNew.slice(3).length === 6);
	check('服务小区元信息：旧含 10321 / 新含 66337（CID 十进制），SCS:15kHz 两边相同',
		lib.textOf(scanBody(oldModal)).indexOf('10321 · SCS:15kHz') !== -1
		&& lib.textOf(scanBody(newModal)).indexOf('66337 · SCS:15kHz') !== -1);

	console.log('\n文案与元信息（两版相同）');
	const text = (modal) => lib.textOf(scanBody(modal));
	check('服务小区卡片标题与 SCS 文案一致（SCS:15kHz 两边都有）',
		text(oldModal).indexOf('Serving cell') !== -1 && text(newModal).indexOf('Serving cell') !== -1
		&& text(oldModal).indexOf('SCS:15kHz') !== -1 && text(newModal).indexOf('SCS:15kHz') !== -1);
	check('邻区小节标题一致（NR neighbour cells）',
		text(oldModal).indexOf('NR neighbour cells') !== -1 && text(newModal).indexOf('NR neighbour cells') !== -1);

	/* ------------------------------------------------------------ 形态 B
	 * `^HFREQINFO` 不可达时 `cell.get` 没有 `band`。旧版靠 arfcnToBand 的表内
	 * 命中猜出频段（表外回退 RAT 名）；新版回退 RAT 名 —— 与 bandLabel 的既有
	 * 回退一致，也是邻居切片删掉前端频段表时定下的行为。
	 */
	console.log('\n形态 B（没有 band 字段）：回退 RAT 名，不抛错');
	const cellNoBand = decodeCell(MODEM.monsc);
	delete cellNoBand.band;
	const neuNoBand = lib.loadSide(newSource, apiFor(Object.assign({}, ANSWERS, { 'route:cell.get': cellNoBand })));
	const modalNoBand = await openScanModal(neuNoBand);
	check('标题回退成「NR · NR · PCI:506 · ARFCN:636648」——与旧版表外回退同形，不留空也不报错',
		text(modalNoBand).indexOf('NR · NR · PCI:506 · ARFCN:636648') !== -1,
		'got ' + text(modalNoBand).slice(0, 160));

	/* ------------------------------------------------------------ 调用面 */
	console.log('\n调用面');
	const routes = (api) => api.calls.filter(c => c.kind === 'route').map(c => c.name);
	const at = (api) => api.calls.filter(c => c.kind === 'at').map(c => c.args[0]);
	check('旧版：at cellscan（取帧）——扫描由 CLI 动词的副作用触发',
		at(apiOld).indexOf('cellscan') !== -1, JSON.stringify(at(apiOld)));
	check('新版：不再有 at cellscan / at cellscan-result；只剩 at radio',
		at(apiNew).every(a => a === 'radio'), JSON.stringify(at(apiNew)));
	check('新版弹窗读四条路由：cell.get / signal.get / cell.neighbors / cell.scan_result',
		[ 'cell.get', 'signal.get', 'cell.neighbors', 'cell.scan_result' ].every(n => routes(apiNew).indexOf(n) !== -1),
		JSON.stringify(routes(apiNew)));
	check('删除的旧实现：parser.parseMonsc / parser.arfcnToBand',
		typeof old.parser.parseMonsc === 'function' && old.parser.arfcnToBand !== undefined
		&& neu.parser.parseMonsc === undefined && neu.parser.arfcnToBand === undefined);
	check('删除的旧实现：api.js 的 atCellscan / atCellscanResult（源码里已无这两个包装）',
		oldSource(RES + '/mt5700m/api.js').indexOf('function atCellscan') !== -1
		&& newSource(RES + '/mt5700m/api.js').indexOf('function atCellscan') === -1
		&& newSource(RES + '/mt5700m/api.js').indexOf('atCellscanResult') === -1);

	console.log(failures === 0
		? '\nPASS：扫描弹窗的新旧渲染一致（差异全部是已知的取值修正，死分支删除后可见内容不变）'
		: '\nFAIL：' + failures + ' 项不一致');
	process.exit(failures === 0 ? 0 : 1);
})().catch(function (err) {
	console.error('渲染失败：' + (err && err.stack || err));
	process.exit(1);
});
