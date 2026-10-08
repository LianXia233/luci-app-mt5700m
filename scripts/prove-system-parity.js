#!/usr/bin/env node
'use strict';

/*
 * 系统页（system）一致性证明（LuCI 系统页 → 统一路由，10 条写入动词 + 22 段读帧全部消失）
 *
 * 第三批切掉最后的 22 段 CLI 读帧：`mt5700m-at system` → 15 条 display /
 * on-demand 路由（modem.get / system.version / sim.* / qos.get / network.* /
 * system.temperature / system.thermal / system.led / system.network_time /
 * system.fota_mode / system.fota）。旧 JS 里的第二份帧解析（22 段的切分规则）
 * 随之消失 —— 字段语义只剩后端一份。
 *
 *   第一批（6 条写入）
 *   - airplane      `mt5700m-at airplane <0|1>`      → network.radio_set {airplane}
 *   - sim-slot      `advanced-set sim-slot <v>`      → sim.slot_set {slot}
 *   - set-imei      `mt5700m-at set-imei <v>`        → modem.imei_set {imei}
 *   - restart       `mt5700m-at restart`             → modem.reset
 *   - sim-pin       `mt5700m-at sim-pin <op> a1 a2`  → sim.pin_apply {operation,pin,newPin}
 *   - factory-reset `mt5700m-at factory-reset`       → system.factory_reset
 *
 *   第二批（4 条写入，后端能力本批前已补齐）
 *   - led                `advanced-set led <0|1>`          → system.led_set {enabled}
 *   - sim-activation     `advanced-set sim-activation`     → sim.activation_set {active}
 *   - thermal-thresholds `advanced-set thermal-thresholds` → system.thermal_thresholds_set {thresholds}
 *   - thermal-log        `advanced-set thermal-log`        → system.thermal_log_set {serial,file}
 *
 *   第三批（本批，读帧）
 *   - `api.atSystem()`（`mt5700m-at system` 的 22 段文本帧）→ 15 条路由，
 *     api.js 的 atSystem 速记一并删除。渲染必须逐字不变。
 *
 *   保留 3 条写入：FOTA 下载 / 续传 / 安装 —— 模块是「任务 + 观察 + 中止」
 *   形态，迁移会改变交互，需产品决策。
 *
 * 用法：
 *   node scripts/prove-system-parity.js [基线]   # 基线默认 pre-system-route
 * 退出码 0 = 通过，1 = 不一致，2 = 基线选错了（基线里已经没有旧实现）。
 *
 * 关键设计：两侧出自**同一份 SYSTEM_FACTS**（病态用例见下）—— 旧侧把它排成
 * 22 段 CLI 文本帧，新侧排成 15 条路由载荷，两套排法都来自
 * scripts/lib/at-fixtures.js 的同一份事实对象。事实一改两边同时动，「渲染
 * 逐字相同」因此仍然成立。A 组比较时把 Technical details 折叠块抹平：块的
 * 内容是**有意**差异（AT 原文 → api.<route> + JSON），单独断言。
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
 * 保留 3 条 CLI（前端仍走动词），本脚本 D 组断言它们**没有**被迁走 ——
 * 迁移会改变交互形态、需要产品决策（FOTA 下载 / 续传 / 安装）。
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
 * SYSTEM_FACTS 在 scripts/lib/at-fixtures.js（与 smoke 共用）。旧侧把它排成
 * 22 段 CLI 文本帧（systemCliFrame），新侧排成 15 条路由载荷
 * （systemRouteResults / systemRouteAnswers）—— 两套排法出自同一份事实，
 * 渲染出来的每一行因此必须逐字相同（Technical details 块除外，单独断言）。
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

/* ------------------------------------------------------------ 装载
 * 旧版 load() 调 api.atSystem()（mt5700m/api.js 的速记），桩上要补挂同名
 * 入口才能整页跑起来；新版不再引用它。 */
function apiFor(answers) {
	const api = lib.makeApi(answers || {});
	api.atSystem = () => api.at([ 'system' ]);
	return api;
}
function buildSide(read, answers) {
	return lib.loadSide(read, apiFor(answers), VIEW);
}

function renderPageOld(side, s) {
	return side.view.renderPage({ stdout: systemFrame(s), stderr: '' });
}
function renderPageNew(side, s) {
	return side.view.renderPage(fixtures.systemRouteResults(s));
}

/* Technical details 折叠块（<pre class="mt-raw">）的内容是**有意**差异：
 * 旧侧倾倒 AT 文本帧原文，新侧倾倒路由载荷（api.<route> + JSON）。块本身
 * 仍在（同一 class、同一标题），比较时两边都抹成占位符。 */
function detailsPre(node) {
	return lib.collect(node, n => n.tagName === 'PRE'
		&& String((n.attrs && n.attrs['class']) || '').indexOf('mt-raw') !== -1)[0];
}
function neutralizeDetails(node) {
	const pre = detailsPre(node);
	if (!pre) return false;
	pre.children = [ '<technical-details>' ];
	return true;
}
function linesNeutralized(node) {
	neutralizeDetails(node);
	return lib.lines(node);
}

/* 整页 load()/render()（调用面组用）：按 LuCI 的调用方式真跑一遍 */
function renderView(side) {
	side.view.load();
	const holder = side.view.render();
	return side.view.contentReady.then(() => holder);
}

/* 弹窗控件定位用 luci-stub 的共享选择器（collect 支持数组入参，modal.children
 * 可以直接喂进去）。 */
const modalNodes = (side) => lib.lastModal(side.scope).children;

function eq(a, b) { return JSON.stringify(a) === JSON.stringify(b); }
const sameJson = eq;

function atSystemIn(src) { return /function atSystem|atSystem:/.test(src); }

function callsOf(side) { return side.scope.api.calls; }
function lastCall(side) { const c = callsOf(side); return c[c.length - 1] || null; }
/* 一笔弹窗可能同时发多笔请求（温控表 + 日志），按名字 / 参数反查。 */
function callNamed(side, name, args) {
	const c = callsOf(side);
	for (let i = c.length - 1; i >= 0; i--) {
		if (name && c[i].name !== name) continue;
		if (args && !eq(c[i].args, args)) continue;
		return c[i];
	}
	return null;
}

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

	/* 基线自检：基线必须还是 CLI 版本，否则本批已经做过了 */
	const oldView = oldSource(VIEW);
	if (!/sim-pin/.test(oldView) || !/factory-reset/.test(oldView)) {
		console.error('基线 ' + BASELINE + ' 里已经没有旧 CLI 实现了（选错基线？）');
		process.exit(2);
	}
	const newView = newSource(VIEW);

	console.log('A. 渲染逐字相同（同一份事实：旧 = 22 段文本帧，新 = 15 条路由载荷）');
	for (const c of CASES) {
		const oldSide = buildSide(oldRead), newSide = buildSide(newRead);
		const a = linesNeutralized(renderPageOld(oldSide, c.s));
		const b = linesNeutralized(renderPageNew(newSide, c.s));
		check(c.name, a === b, diffText(a, b));
	}

	console.log('A2. Technical details 折叠块（有意差异：AT 原文 → 路由载荷）');
	{
		const oldSide = buildSide(oldRead), newSide = buildSide(newRead);
		const oldNode = renderPageOld(oldSide, FULL), newNode = renderPageNew(newSide, FULL);
		const oldPre = detailsPre(oldNode), newPre = detailsPre(newNode);
		const oldText = lib.textOf(oldPre), newText = lib.textOf(newPre);
		check('「Technical details」折叠块仍在（同一 class、同一标题，非空）',
			!!oldPre && !!newPre && oldText.length > 0 && newText.length > 0);
		check('旧版倾倒的是整段 AT 文本帧（含 ^VERSION / ^CHIPTEMP 原文）',
			oldText.indexOf('^VERSION') !== -1 && oldText.indexOf('^CHIPTEMP') !== -1);
		check('新版不再出现任何 AT 应答原文；改倒路由载荷（api.<route> + JSON）',
			newText.indexOf('^VERSION') === -1 && newText.indexOf('^CHIPTEMP') === -1
			&& newText.indexOf('api.system.version') !== -1
			&& newText.indexOf('"buildDate"') !== -1);
		check('两侧各抹掉一处细节块内容（A 组剩下的差异必须是零）',
			neutralizeDetails(oldNode) && neutralizeDetails(newNode));
	}

	console.log('B. 10 条 CLI 动词已消失（静态）');
	check('airplane 动词消失', !/\[\s*'airplane'/.test(newView));
	check("advanced-set sim-slot 动词消失", !/'advanced-set',\s*'sim-slot'/.test(newView));
	check('set-imei 动词消失', !/\[\s*'set-imei'/.test(newView));
	check('restart 动词消失', !/\[\s*'restart'\]/.test(newView));
	check('sim-pin 动词消失', !/\[\s*'sim-pin'/.test(newView));
	check('factory-reset 动词消失', !/\[\s*'factory-reset'\]/.test(newView));
	check("advanced-set led 动词消失", !/'advanced-set',\s*'led'/.test(newView));
	check("advanced-set sim-activation 动词消失", !/'advanced-set',\s*'sim-activation'/.test(newView));
	check('thermal-thresholds 动词消失', !/'thermal-thresholds'/.test(newView));
	check('thermal-log 动词消失', !/'thermal-log'/.test(newView));

	console.log('C. 迁移映射（旧侧发什么 vs 新侧发什么，逐字对照）');

	/* 1/2. airplane：functionLevel 决定按钮文案与语义 */
	for (const cfun of [ '0', '1' ]) {
		const s = shape({ cfun: cfun });
		const oldSide = buildSide(oldRead), newSide = buildSide(newRead);
		const label = cfun === '0' ? 'Resume mobile radio' : 'Enter airplane mode';
		lib.pressButton(renderPageOld(oldSide, s), label);
		lib.pressButton(renderPageNew(newSide, s), label);
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
		lib.pressButton(renderPageOld(oldSide, s), 'Switch SIM slot');
		lib.pressButton(renderPageNew(newSide, s), 'Switch SIM slot');
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
		for (const [ side, page ] of [[ oldSide, renderPageOld(oldSide, s) ], [ newSide, renderPageNew(newSide, s) ]]) {
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
		for (const [ side, page ] of [[ oldSide, renderPageOld(oldSide, s) ], [ newSide, renderPageNew(newSide, s) ]]) {
			lib.pressButton(page, 'Restart Module');
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
		for (const [ side, page ] of [[ oldSide, renderPageOld(oldSide, s) ], [ newSide, renderPageNew(newSide, s) ]]) {
			lib.pressButton(page, 'Restore factory settings');
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
		for (const [ side, page ] of [[ oldSide, renderPageOld(oldSide, FULL) ], [ newSide, renderPageNew(newSide, FULL) ]]) {
			lib.pressButton(page, 'Manage SIM PIN');
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

	/* 13/14. LED：初始值来自 ^LEDSWITCH 段 */
	for (const led of [ '1', '0' ]) {
		const s = shape({ led: led });
		const oldSide = buildSide(oldRead), newSide = buildSide(newRead);
		for (const [ side, page ] of [[ oldSide, renderPageOld(oldSide, s) ], [ newSide, renderPageNew(newSide, s) ]]) {
			lib.pressButton(page, 'Apply LED setting');
			lib.modalButton(side.scope, 'Apply');
		}
		await lib.tick();
		const o = lastCall(oldSide), n = lastCall(newSide);
		const ok = o && eq(o.args, [ 'advanced-set', 'led', led ])
			&& n && n.name === 'system.led_set' && eq(n.params, { enabled: led === '1' });
		check('led=' + led + ' → led_set {enabled:' + (led === '1') + '}', ok,
			'old=' + JSON.stringify(o) + '\n       new=' + JSON.stringify(n));
	}

	/* 15/16. SIM 激活：初始值取 ^HVSST 第二字段 */
	for (const field of [ '1', '0' ]) {
		const s = shape({ hvsst: '1,' + field + ',0' });
		const oldSide = buildSide(oldRead), newSide = buildSide(newRead);
		for (const [ side, page ] of [[ oldSide, renderPageOld(oldSide, s) ], [ newSide, renderPageNew(newSide, s) ]]) {
			lib.pressButton(page, 'Apply SIM activation');
			lib.modalButton(side.scope, 'Apply');
		}
		await lib.tick();
		const o = lastCall(oldSide), n = lastCall(newSide);
		const ok = o && eq(o.args, [ 'advanced-set', 'sim-activation', field ])
			&& n && n.name === 'sim.activation_set' && eq(n.params, { active: field === '1' });
		check('sim-activation=' + field + ' → activation_set {active:' + (field === '1') + '}', ok,
			'old=' + JSON.stringify(o) + '\n       new=' + JSON.stringify(n));
	}

	/* 17. 温控表 + 温控日志：一条弹窗同时发两笔 */
	{
		const nine = [ 60, 70, 65, 80, 75, 90, 85, 100, 95 ];
		const oldSide = buildSide(oldRead), newSide = buildSide(newRead);
		for (const [ side, page ] of [[ oldSide, renderPageOld(oldSide, FULL) ], [ newSide, renderPageNew(newSide, FULL) ]]) {
			lib.pressButton(page, 'Configure thermal protection');
			lib.modalButton(side.scope, 'Apply');
		}
		await lib.tick();
		const oT = callNamed(oldSide, null, [ 'advanced-set', 'thermal-thresholds' ].concat(nine.map(String)));
		const oL = callNamed(oldSide, null, [ 'advanced-set', 'thermal-log', '1', '0' ]);
		const nT = callNamed(newSide, 'system.thermal_thresholds_set');
		const nL = callNamed(newSide, 'system.thermal_log_set');
		const ok = oT && oL && nT && nL
			&& eq(nT.params, { thresholds: nine })
			&& eq(nL.params, { serial: true, file: false });
		check('温控表 / 日志 → thresholds_set + thermal_log_set', ok,
			'old=' + JSON.stringify([ oT, oL ]) + '\n       new=' + JSON.stringify([ nT, nL ]));
	}

	console.log('D. 保留 3 条 CLI（FOTA 三步：迁移会改变交互，需产品决策）');
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
	check('读帧入口已删：system.js 不再引用 atSystem / api.at(', !/atSystem|api\.at\(/.test(newView));
	check('api.js 的 atSystem 速记一并删除（没有人再调 mt5700m-at system）',
		!atSystemIn(newSource(RES + '/mt5700m/api.js')));

	console.log('F. 调用面（22 段读帧 → 15 条路由，整页 load()/render() 真跑）');
	{
		const answers = Object.assign(
			{ 'at:system': { stdout: systemFrame(FULL), stderr: '' } },
			fixtures.systemRouteAnswers(FULL));
		const oldSide = buildSide(oldRead, answers), newSide = buildSide(newRead, answers);
		await Promise.all([ renderView(oldSide), renderView(newSide) ]);
		const oldAt = oldSide.api.calls.filter(c => c.kind === 'at').map(c => c.args[0]);
		const newAt = newSide.api.calls.filter(c => c.kind === 'at').map(c => c.args[0]);
		check('旧版：at system（22 段文本帧，load() 里的一次调用）',
			sameJson(oldAt, [ 'system' ]), JSON.stringify(oldAt));
		check('新版：零 CLI 调用（系统页不再走 mt5700m-at）',
			sameJson(newAt, []), JSON.stringify(newAt));
		const newRoutes = newSide.api.calls.filter(c => c.kind === 'route').map(c => c.name);
		check('新版：15 条路由（顺序即 load() 的 Promise.all）',
			sameJson(newRoutes, fixtures.SYSTEM_ROUTES), JSON.stringify(newRoutes));
		check('新版：路由数正确（15 条）', newRoutes.length === 15,
			'got ' + newRoutes.length);
	}

	console.log('');
	if (failures) {
		console.log(failures + ' 项不一致');
		process.exit(1);
	}
	console.log('全部通过：系统页 10 条写入 + 22 段读帧已从 CLI 迁到路由，UI 逐字未变。');
})();
