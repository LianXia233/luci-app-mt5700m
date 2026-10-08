#!/usr/bin/env node
'use strict';

/*
 * 系统页（system）一致性证明（LuCI 系统页 → 统一路由，6 条写入动词消失）
 *
 * 这一刀切掉的是系统页最后 6 条 CLI **写入**动词。读帧（22 段）
 * `mt5700m-at system` 本刀未动 —— 它没有评估手段，切换它需要先补齐后端的
 * LED 读 / 网络时间 / SIM 激活读脚本，因此留在下一刀。
 *
 *   - airplane      `mt5700m-at airplane <0|1>`      → network.radio_set {airplane}
 *   - sim-slot      `advanced-set sim-slot <v>`      → sim.slot_set {slot}
 *   - set-imei      `mt5700m-at set-imei <v>`        → modem.imei_set {imei}
 *   - restart       `mt5700m-at restart`             → modem.reset
 *   - sim-pin       `mt5700m-at sim-pin <op> a1 a2`  → sim.pin_apply {operation,pin,newPin}
 *   - factory-reset `mt5700m-at factory-reset`       → system.factory_reset
 *
 * 用法：
 *   node scripts/prove-system-parity.js [基线]   # 基线默认 pre-system-route
 * 退出码 0 = 通过，1 = 不一致，2 = 基线选错了（基线里已经没有旧实现）。
 *
 * 关键设计：两侧喂的是**同一份 CLI 文本帧**（病态用例见下），A 组因此要求
 * 渲染结果逐字相同 —— 本刀没有碰解析，任何一行不一样都说明改坏了 UI。
 * C 组同时钉住**旧侧发出什么**与**新侧发出什么**，两者逐字对照；只看新侧
 * 是不够的：新侧参数写错而旧侧对不上时也必须失败。
 *
 * 两处**有意**的语义差异（与 docs/architecture-v2/migration.md 一致）：
 *
 * 1. `sim.slot_set` 走完整的厂商切换序列（含 ^HVSST 去激活/激活 + 等待
 *    注册），CLI 的 `advanced-set sim-slot` 只发一条 `AT^SCICHG`。厂商手册
 *    要求的前置/后置步骤不能省，因此以模块侧为准。
 * 2. `system.factory_reset` 发 `AT&F`，CLI 发 `AT&F0`。两者是同一条工厂
 *    复位命令（&F 默认即 profile 0），不是行为差异。
 *
 * 保留 7 条 CLI（前端仍走动词），本脚本 D 组断言它们**没有**被迁走 ——
 * 后端尚无对应写的能力（LED / SIM 激活 / 温控阈值 / 温控日志），或迁移
 * 会改变交互形态、需要产品决策（FOTA 下载 / 续传 / 安装）。
 */

const fs = require('fs');
const path = require('path');
const cp = require('child_process');
const lib = require('./lib/luci-stub');
const fixtures = require('./lib/at-fixtures');

const REPO = path.resolve(__dirname, '..');
const RES = 'luci-app-mt5700m/htdocs/luci-static/resources';
const VIEW = RES + '/view/mt5700m/system.js';
const COMPONENTS = RES + '/mt5700m/components.js';
const BASELINE = process.argv[2] || 'pre-system-route';

function gitShow(rel) {
	return cp.execSync('git show ' + BASELINE + ':' + rel, {
		cwd: REPO, maxBuffer: 64 * 1024 * 1024
	}).toString();
}
const newSource = (rel) => fs.readFileSync(path.join(REPO, rel), 'utf8');

let failures = 0;
function check(name, ok, detail) {
	if (ok) { console.log('  ok   ' + name); return; }
	failures++;
	console.log('  FAIL ' + name + (detail ? '\n       ' + detail : ''));
}

function diffText(a, b) {
	const la = String(a).split('\n'), lb = String(b).split('\n');
	for (let i = 0; i < Math.max(la.length, lb.length); i++)
		if (la[i] !== lb[i])
			return 'line ' + (i + 1) + ':\n       old: ' + la[i] + '\n       new: ' + lb[i];
	return '';
}

/* ------------------------------------------------------------ 共享事实
 * `mt5700m-at system` 的读帧夹具在 scripts/lib/at-fixtures.js（与 smoke 共用），
 * 两侧喂的是**同一份 CLI 文本帧**，渲染出来的每一行因此必须逐字相同。
 */
const systemFrame = fixtures.systemCliFrame;
const shape = fixtures.systemFacts;
const FULL = fixtures.SYSTEM_FACTS;

/* 病态用例：覆盖缺失段、空值、NO CARRIER 类应答 */
const CASES = [
	{ name: '完整帧', s: FULL },
	{ name: 'SIM 需 PIN（sim_state 非 READY）', s: shape({ sim: 'SIM PIN', iccid: '', imsi: '' }) },
	{ name: '号码未存储（+CNUM 报错）', s: shape({ number: '' }) },
	{ name: 'FOTA 安装完成（state=40）', s: shape({ fotastate: '40', fotadlq: '"update.bin",100,100' }) },
	{ name: '温控段全缺', s: shape({ thermalStatus: '', thermalPara: '', thermalLogSw: '', chiptemp: '' }) },
	{ name: '飞行模式（CFUN=0）', s: shape({ cfun: '0', fotastate: '13' }) }
];

/* ------------------------------------------------------------ 装载两侧 */
function buildSide(read) {
	const api = lib.makeApi({});
	const side = lib.loadSide(read, api, VIEW);
	return side;
}

function renderPage(side, s) {
	return lib.lines(side.view.renderPage({ stdout: systemFrame(s), stderr: '' }));
}

/* 弹窗控件定位用 luci-stub 的共享选择器（collect 支持数组入参，modal.children
 * 可以直接喂进去）。 */
const modalNodes = (side) => lib.lastModal(side.scope).children;

function eq(a, b) { return JSON.stringify(a) === JSON.stringify(b); }

function callsOf(side) { return side.scope.api.calls; }
function lastCall(side) { const c = callsOf(side); return c[c.length - 1] || null; }

(async function main() {
	let oldSource;
	try {
		oldSource = (rel) => gitShow(rel);
		gitShow(VIEW);
	} catch (e) {
		console.error('基线不可用（git show ' + BASELINE + ' 失败）: ' + e.message);
		process.exit(2);
	}
	const newRead = (rel) => newSource(rel);
	const oldRead = (rel) => oldSource(rel);

	/* 基线自检：基线必须还是 CLI 版本，否则这一刀已经做过了 */
	const oldView = oldSource(VIEW);
	if (!/sim-pin/.test(oldView) || !/factory-reset/.test(oldView)) {
		console.error('基线 ' + BASELINE + ' 里已经没有旧 CLI 实现了（选错基线？）');
		process.exit(2);
	}
	const newView = newSource(VIEW);

	console.log('A. 渲染逐字相同（读帧未动，任何差异都是改坏了 UI）');
	for (const c of CASES) {
		const oldSide = buildSide(oldRead), newSide = buildSide(newRead);
		const a = renderPage(oldSide, c.s), b = renderPage(newSide, c.s);
		check(c.name, a === b, diffText(a, b));
	}

	console.log('B. 6 条 CLI 动词已消失（静态）');
	check('airplane 动词消失', !/\[\s*'airplane'/.test(newView));
	check("advanced-set sim-slot 动词消失", !/'advanced-set',\s*'sim-slot'/.test(newView));
	check('set-imei 动词消失', !/\[\s*'set-imei'/.test(newView));
	check('restart 动词消失', !/\[\s*'restart'\]/.test(newView));
	check('sim-pin 动词消失', !/\[\s*'sim-pin'/.test(newView));
	check('factory-reset 动词消失', !/\[\s*'factory-reset'\]/.test(newView));

	console.log('C. 迁移映射（旧侧发什么 vs 新侧发什么，逐字对照）');

	/* 1/2. airplane：functionLevel 决定按钮文案与语义 */
	for (const cfun of [ '0', '1' ]) {
		const s = shape({ cfun: cfun });
		const oldSide = buildSide(oldRead), newSide = buildSide(newRead);
		const label = cfun === '0' ? 'Resume mobile radio' : 'Enter airplane mode';
		lib.pressButton(oldSide.view.renderPage({ stdout: systemFrame(s), stderr: '' }), label);
		lib.pressButton(newSide.view.renderPage({ stdout: systemFrame(s), stderr: '' }), label);
		lib.modalButton(oldSide.scope, 'Continue');
		lib.modalButton(newSide.scope, 'Continue');
		await lib.tick();
		const o = lastCall(oldSide), n = lastCall(newSide);
		const ok = o && o.kind === 'at' && eq(o.args, [ 'airplane', cfun === '0' ? '1' : '0' ])
			&& n && n.kind === 'routeCall' && n.name === 'network.radio_set'
			&& eq(n.params, { airplane: cfun !== '0' });
		check('airplane CFUN=' + cfun + ' → radio_set {airplane:' + (cfun !== '0') + '}', ok,
			'old=' + JSON.stringify(o) + '\n       new=' + JSON.stringify(n));
	}

	/* 3/4. sim-slot：初始卡槽来自 ^SCICHG 首值 */
	for (const slot of [ '0', '1' ]) {
		const s = shape({ scichg: slot + ',0' });
		const oldSide = buildSide(oldRead), newSide = buildSide(newRead);
		lib.pressButton(oldSide.view.renderPage({ stdout: systemFrame(s), stderr: '' }), 'Switch SIM slot');
		lib.pressButton(newSide.view.renderPage({ stdout: systemFrame(s), stderr: '' }), 'Switch SIM slot');
		lib.modalButton(oldSide.scope, 'Apply');
		lib.modalButton(newSide.scope, 'Apply');
		await lib.tick();
		const o = lastCall(oldSide), n = lastCall(newSide);
		const ok = o && eq(o.args, [ 'advanced-set', 'sim-slot', slot ])
			&& n && n.name === 'sim.slot_set' && eq(n.params, { slot: Number(slot) });
		check('sim-slot=' + slot + ' → slot_set {slot:' + slot + '}', ok,
			'old=' + JSON.stringify(o) + '\n       new=' + JSON.stringify(n));
	}

	/* 5. set-imei */
	{
		const s = FULL;
		const oldSide = buildSide(oldRead), newSide = buildSide(newRead);
		const expect = '862853030099999';
		for (const side of [ oldSide, newSide ]) {
			const page = side.view.renderPage({ stdout: systemFrame(s), stderr: '' });
			lib.pressButton(page, 'Device identity laboratory');
			lib.inputsIn(modalNodes(side))[0].value = expect;
			lib.modalButton(side.scope, 'Review change');
			lib.modalButton(side.scope, 'Apply');
		}
		await lib.tick();
		const o = lastCall(oldSide), n = lastCall(newSide);
		const ok = o && eq(o.args, [ 'set-imei', expect ])
			&& n && n.name === 'modem.imei_set' && eq(n.params, { imei: expect });
		check('set-imei → imei_set {imei}', ok,
			'old=' + JSON.stringify(o) + '\n       new=' + JSON.stringify(n));
	}

	/* 6. restart */
	{
		const s = FULL;
		const oldSide = buildSide(oldRead), newSide = buildSide(newRead);
		for (const side of [ oldSide, newSide ]) {
			lib.pressButton(side.view.renderPage({ stdout: systemFrame(s), stderr: '' }), 'Restart Module');
			lib.modalButton(side.scope, 'Continue');
		}
		await lib.tick();
		const o = lastCall(oldSide), n = lastCall(newSide);
		const ok = o && eq(o.args, [ 'restart' ])
			&& n && n.name === 'modem.reset' && (n.params === null || n.params === undefined);
		check('restart → modem.reset（无参）', ok,
			'old=' + JSON.stringify(o) + '\n       new=' + JSON.stringify(n));
	}

	/* 7. factory-reset */
	{
		const s = FULL;
		const oldSide = buildSide(oldRead), newSide = buildSide(newRead);
		for (const side of [ oldSide, newSide ]) {
			lib.pressButton(side.view.renderPage({ stdout: systemFrame(s), stderr: '' }), 'Restore factory settings');
			lib.inputsIn(modalNodes(side))[0].value = 'RESET';
			lib.modalButton(side.scope, 'Restore factory settings');
		}
		await lib.tick();
		const o = lastCall(oldSide), n = lastCall(newSide);
		const ok = o && eq(o.args, [ 'factory-reset' ])
			&& n && n.name === 'system.factory_reset';
		check('factory-reset → system.factory_reset（有意差异：AT&F vs AT&F0）', ok,
			'old=' + JSON.stringify(o) + '\n       new=' + JSON.stringify(n));
	}

	/* 8-12. sim-pin 五种操作 */
	const PIN_CASES = [
		{ op: 'verify', first: '1234', second: '', old: [ 'sim-pin', 'verify', '1234', '' ],
			params: { operation: 'verify', pin: '1234', newPin: '' } },
		{ op: 'enable', first: '1234', second: '', old: [ 'sim-pin', 'enable', '1234', '' ],
			params: { operation: 'enable', pin: '1234', newPin: '' } },
		{ op: 'disable', first: '1234', second: '', old: [ 'sim-pin', 'disable', '1234', '' ],
			params: { operation: 'disable', pin: '1234', newPin: '' } },
		{ op: 'change', first: '1234', second: '5678', old: [ 'sim-pin', 'change', '1234', '5678' ],
			params: { operation: 'change', pin: '1234', newPin: '5678' } },
		{ op: 'unblock', first: '12345678', second: '5678', old: [ 'sim-pin', 'unblock', '12345678', '5678' ],
			params: { operation: 'unblock', pin: '12345678', newPin: '5678' } }
	];
	for (const pc of PIN_CASES) {
		const oldSide = buildSide(oldRead), newSide = buildSide(newRead);
		for (const side of [ oldSide, newSide ]) {
			lib.pressButton(side.view.renderPage({ stdout: systemFrame(FULL), stderr: '' }), 'Manage SIM PIN');
			lib.selectsIn(modalNodes(side))[0].value = pc.op;
			const ins = lib.inputsIn(modalNodes(side));
			ins[0].value = pc.first;
			ins[1].value = pc.second;
			lib.modalButton(side.scope, 'Apply');
		}
		await lib.tick();
		const o = lastCall(oldSide), n = lastCall(newSide);
		const ok = o && eq(o.args, pc.old)
			&& n && n.name === 'sim.pin_apply' && eq(n.params, pc.params);
		check('sim-pin ' + pc.op + ' → pin_apply', ok,
			'old=' + JSON.stringify(o) + '\n       new=' + JSON.stringify(n));
	}

	console.log('D. 保留 7 条 CLI（无写路由 / 需产品决策，不得被迁走）');
	for (const token of [ 'thermal-thresholds', 'thermal-log' ]) {
		check('保留 advanced-set ' + token, newView.indexOf(token) >= 0);
	}
	check('保留 advanced-set led', /'advanced-set',\s*'led'/.test(newView));
	check('保留 advanced-set sim-activation', /'advanced-set',\s*'sim-activation'/.test(newView));
	check('保留 fota-start', /'fota-start'/.test(newView));
	check('保留 fota-resume', /'fota-resume'/.test(newView));
	check('保留 fota-upgrade', /'fota-upgrade'/.test(newView));

	console.log('E. 结构（写入确认只有一份实现）');
	const comp = newSource(COMPONENTS);
	check('components 导出 runConfirmedRoute', /runConfirmedRoute:/.test(comp));
	check('runConfirmed / runConfirmedRoute 共用 runConfirmedAction',
		(function () {
			const cli = comp.match(/function runConfirmed\([\s\S]*?\n}/);
			const rt = comp.match(/function runConfirmedRoute\([\s\S]*?\n}/);
			return cli && rt && /runConfirmedAction/.test(cli[0]) && /runConfirmedAction/.test(rt[0]);
		})());
	check('读帧入口 atSystem 仍在（下一刀才切）', /api\.atSystem\(\)/.test(newView));

	console.log('');
	if (failures) {
		console.log(failures + ' 项不一致');
		process.exit(1);
	}
	console.log('全部通过：系统页 6 条写入已从 CLI 迁到路由，UI 逐字未变。');
})();
