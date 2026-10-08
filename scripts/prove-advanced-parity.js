#!/usr/bin/env node
'use strict';

/*
 * 高级设置页一致性证明（高级设置批：8 段 CLI 读帧 + 6 条 CLI 写 → 5 条读路由 + 6 条写路由）
 *
 * 本批切掉的是高级设置页的全部 CLI：`mt5700m-at advanced hardware` 的八段
 * 读帧（USB mode / Interface mode / NIC speed / PCIe controller / LED / SIM
 * hotplug / SIM slot / Thermal control）与六条 advanced-set 写入（usb-mode /
 * pcie-controller / nic-speed / interface-mode / sim-hotplug / thermal）：
 *   - 读：5 条路由 network.usb_mode / network.interface_cfg /
 *     system.device_control / sim.slot / system.thermal（LED 段页面不读，
 *     不需要路由；device_control 一条覆盖 NIC + PCIe 两段）；
 *   - 写：六处 c.confirmRun([…argv]) 改 c.confirmRoute(name, params)，其中
 *     pcie / nic / sim-hotplug / thermal 四条复用系统页批次已有的路由
 *     （system.power_control_set / system.nic_rate_set / sim.hotplug_set /
 *     system.thermal_set），usb-mode / interface-mode 两条是本批新增
 *     （network.usb_mode_set / network.interface_mode_set）；
 *   - 删除：api.js 的 atHardware 速记。
 *
 * 用法：
 *   node scripts/prove-advanced-parity.js [基线]   # 基线默认 pre-advanced-route tag
 * 退出码 0 = 通过，1 = 不一致，2 = 基线选错了（基线里已经没有旧实现）。
 *
 * 关键设计：夹具用**一份 ADVANCED_FACTS** 同时生成 8 段 CLI 帧与 5 条路由
 * 载荷，两侧渲染逐字相同就等于「后端从同一批 AT 应答解出来的值，与旧前端
 * 自己正则出来的完全一致」。病态形态钉住旧正则的怪癖在载荷层被原样仿制：
 *   - ^SETMODE? 应答垃圾：旧两级 fallback（^SETMODE: 正则 → 单独数字行 →
 *     默认 '4'）—— Rust parse_usb_mode 同样认无前缀数字行，载荷缺席走默认；
 *   - ^THERMAUTOFUN? 短应答（不足三字段）：旧 match 失败 → 温控默认
 *     开/2 s —— 载荷层 enabled/interval 缺席走进同一分支；
 *   - ^TDPCIELANCFG: 0,0（姊妹项目 mock 形态）：首字段 0 不在下拉选项集，
 *     两侧同样显示无选中。
 *
 * 有意差异（不是漂移），单独点名断言：
 *   1. 真实抓包 TDCFG 应答 `Mode : 2` 冒号前带空格 —— 旧正则 /Mode:\s*(\d+)/
 *      不识别，接口模式控件在真实设备上永远无选中；新链路
 *      （parse_interface_cfg 宽松解析）把值解出来，控件恢复可用 —— 与连接页
 *      PostRoute 同款、与姊妹项目 luci-app-mt5700 的 dial.js 行为对齐；
 *   2. PCIe 写命令形态：旧 CLI 动词实发 `AT^TDPMCFG=<v>,0,0,0`（四字段），
 *      新路由 system.power_control_set 实发 `AT^TDPMCFG=<v>`（短形态，
 *      system::commands::tdpmcfg 的权威注释：modem 两者都收，页面统一短形态
 *      —— 与姊妹项目一致）。AT 串形态由 cargo test 钉住，这里钉 argv/params。
 *   3. Technical details 折叠块：旧倾倒 AT 文本帧原文，新倾倒路由载荷
 *      （api.<route> + JSON）—— 渲染组把它抹成占位符，A2 组单独断言。
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
const VIEW = RES + '/view/mt5700m/advanced.js';
const BASELINE = process.argv[2] || 'pre-advanced-route';

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

/* ------------------------------------------------------------ 装载 */
function answers(kind, shape) {
	const f = fixtures.advancedFacts(shape.facts);
	if (kind === 'old')
		return { 'at:advanced': { stdout: fixtures.advancedFrame(f), stderr: '' } };
	return fixtures.advancedRouteAnswers(f);
}

function side(kind, shape) {
	const api = lib.makeApi(answers(kind, shape));
	if (kind === 'old')
		api.atHardware = () => api.at([ 'advanced', 'hardware' ]);
	// 新侧刻意**不挂** atHardware：若新页面残留调用，这里直接崩。
	return lib.loadSide(kind === 'old' ? oldSource : newSource, api, VIEW, {});
}

/* 整页渲染（渐进骨架 → 内容替换） */
async function render(kind, shape) {
	const s = side(kind, shape);
	s.view.load();
	const holder = s.view.render();
	await s.view.contentReady;
	for (let i = 0; i < 20; i++) await lib.tick();
	return { side: s, holder: holder, text: lib.textOf(holder) };
}

/* Technical details 折叠块（<pre class="mt-raw">）：内容是有意差异，比较时
 * 两边都抹成占位符；块本身（class、标题、非空）在 A2 组单独断言。 */
function neutralizeDetails(node) {
	const pre = lib.collect(node, (n) => n.tagName === 'PRE'
		&& String((n.attrs && n.attrs['class']) || '').indexOf('mt-raw') !== -1)[0];
	if (!pre) return false;
	pre.children = [ '<technical-details>' ];
	return true;
}

/* 页面上的 select 取值（DOM 顺序：USB / PCIe / PHY / 接口模式 / 热插拔 / 温控开 / 温控周期） */
const selectsOf = (r) => lib.collect(r.holder, (n) => n.tagName === 'SELECT').map((n) => String(n.value));

/* ------------------------------------------------------------ 形态表 */
/* facts 是 ADVANCED_FACTS 的字段覆盖，缺席即默认。 */
const BASE_SELECTS = [ '4', '1', '2', '1', '1', '1', '30' ];
const SHAPES = [
	{
		key: '正常形态（8 段满字段）', facts: {},
		selects: BASE_SELECTS,
		expect: [ 'External SIM', 'Linux NCM', 'RTL8125 · 2.5 Gbps' ]
	},
	{
		/* ^SETMODE? 应答垃圾：旧两级 fallback 全部落空 → 默认 '4'；
		 * 新载荷 usb_mode={} → mode 缺席 → 同一默认。 */
		key: 'USB mode 应答垃圾（两侧回落默认 Linux NCM）', facts: { setmode: 'x' },
		selects: BASE_SELECTS
	},
	{
		/* ^THERMAUTOFUN? 短应答（不足三字段）：旧 match 失败 → 温控默认
		 * 开 / 2 s；新载荷 thermal={} → enabled/interval 缺席 → 同一分支。 */
		key: '温控应答过短（两侧回落默认 开/2s）', facts: { thermautofun: '1' },
		selects: [ '4', '1', '2', '1', '1', '1', '2' ]
	},
	{
		/* ^TDPCIELANCFG: 0,0（姊妹项目 mock 同款形态）：首字段 0 不在下拉
		 * 选项集（1|2），两侧同样原样保真 '0'（真实 DOM 会回落空/首项，
		 * 两侧同走一条路）。 */
		key: 'PHY profile 应答首字段 0（两侧原样保真）', facts: { tdpcielancfg: '0,0' },
		selects: [ '4', '1', '0', '1', '1', '1', '30' ]
	},
	{
		/* ^TDPMCFG: 首字段垃圾：旧 pick '' → 控件回落 Enabled；
		 * 新载荷 power_control 缺席 → 同一回落。 */
		key: 'PCIe 控制器应答垃圾（两侧回落 Enabled）', facts: { tdpmcfg: 'x,0' },
		selects: BASE_SELECTS
	}
];

/* ------------------------------------------------------------ 自检 */
if (oldSource(VIEW).indexOf('api.atHardware()') === -1) {
	console.error('基线 ' + BASELINE + ' 已经包含本批（advanced.js 里没有 api.atHardware()）——没有旧管线可比较。\n'
		+ '本批的基线是 pre-advanced-route tag（提交 cf7b688）：\n  node scripts/prove-advanced-parity.js pre-advanced-route');
	process.exit(2);
}
check('基线里旧侧确实读 hardware CLI 帧 + 六条 CLI 写（atHardware / confirmRun / advanced-set）',
	oldSource(VIEW).indexOf('api.atHardware()') !== -1
	&& oldSource(VIEW).indexOf('c.confirmRun') !== -1
	&& oldSource(VIEW).indexOf("'advanced-set'") !== -1);
check('新侧高级设置页不再有 CLI 帧解析（atHardware / confirmRun / parser.section / parser.pick 全部离开）',
	newSource(VIEW).indexOf('atHardware') === -1
	&& newSource(VIEW).indexOf('confirmRun(') === -1
	&& newSource(VIEW).indexOf('parser.section(') === -1
	&& newSource(VIEW).indexOf('parser.pick(') === -1);
check('新侧六条写入全部走 confirmRoute（四条复用系统页/已迁路由 + 两条本批新增）',
	[ 'network.usb_mode_set', 'system.power_control_set', 'system.nic_rate_set',
		'network.interface_mode_set', 'sim.hotplug_set', 'system.thermal_set' ]
		.every((name) => newSource(VIEW).indexOf("'" + name + "'") !== -1));
check('新侧 api.js 删除了 atHardware 速记', newSource(RES + '/mt5700m/api.js').indexOf('atHardware') === -1);

/* ------------------------------------------------------------ 写路径表 */
/* 每条写入两侧分别驱动：按压按钮 → 确认弹窗按钮。控件取值即载荷解析产物，
 * 弹窗确认后旧侧预期 argv（api.at 的 args）、新侧预期 routeCall 的名字 + params。 */
const WRITES = [
	{
		key: 'USB 模式（Apply USB mode）', button: 'Apply USB mode',
		oldArgs: [ 'advanced-set', 'usb-mode', '4' ],
		newCall: { name: 'network.usb_mode_set', params: { mode: 4 } }
	},
	{
		/* 有意变更（见头部注释 2）：旧 CLI 实发 AT^TDPMCFG=1,0,0,0，新路由实发
		 * 短形态 AT^TDPMCFG=1 —— argv/params 层面钉住，AT 串由 cargo test 钉住。 */
		key: 'PCIe 控制器（Apply PCIe controller）', button: 'Apply PCIe controller',
		oldArgs: [ 'advanced-set', 'pcie-controller', '1' ],
		newCall: { name: 'system.power_control_set', params: { enabled: true } }
	},
	{
		key: 'PHY profile（Apply PHY profile）', button: 'Apply PHY profile',
		oldArgs: [ 'advanced-set', 'nic-speed', '2' ],
		newCall: { name: 'system.nic_rate_set', params: { rate: 2 } }
	},
	{
		key: '接口模式（Apply interface mode）', button: 'Apply interface mode',
		oldArgs: [ 'advanced-set', 'interface-mode', '1' ],
		newCall: { name: 'network.interface_mode_set', params: { mode: 1 } }
	},
	{
		key: 'SIM 热插拔（Apply SIM hotplug）', button: 'Apply SIM hotplug',
		oldArgs: [ 'advanced-set', 'sim-hotplug', '1' ],
		newCall: { name: 'sim.hotplug_set', params: { hotplug: true } }
	},
	{
		key: '温控（Apply thermal settings）', button: 'Apply thermal settings',
		oldArgs: [ 'advanced-set', 'thermal', '1', '30' ],
		newCall: { name: 'system.thermal_set', params: { enabled: true, interval: 30 } }
	}
];

async function fireWrite(r, w) {
	r.side.api.calls.length = 0;
	lib.pressButton(r.holder, w.button);
	lib.modalButton(r.side.scope, 'Apply');
	for (let i = 0; i < 6; i++) await lib.tick();
	return r.side.api.calls;
}

(async function () {
	console.log('渲染：整页 DOM 与文案（8 段 CLI 帧 vs 5 条读路由）');
	for (const shape of SHAPES) {
		const o = await render('old', shape);
		const n = await render('new', shape);
		const oNeutral = neutralizeDetails(o.holder);
		const nNeutral = neutralizeDetails(n.holder);
		const oShape = NUM(lib.serialize(o.holder, { attrs: true, keepValues: true }));
		const nShape = NUM(lib.serialize(n.holder, { attrs: true, keepValues: true }));
		check(shape.key + '：结构与文案逐字一致（数字归一化后，Technical details 抹平）',
			oNeutral && nNeutral && oShape === nShape,
			diffText(oShape, nShape));
		(shape.expect || []).forEach((needle) => {
			check(shape.key + '：两边都渲染出「' + needle + '」',
				o.text.indexOf(needle) !== -1 && n.text.indexOf(needle) !== -1,
				JSON.stringify([ o.text.indexOf(needle), n.text.indexOf(needle) ]));
		});
		if (shape.selects) {
			check(shape.key + '：七个控件取值两侧一致且等于期望（USB/PCIe/PHY/接口模式/热插拔/温控开/温控周期）',
				sameJson(selectsOf(o), selectsOf(n)) && sameJson(selectsOf(n), shape.selects),
				JSON.stringify([ selectsOf(o), selectsOf(n), shape.selects ]));
		}
	}

	/* 有意差异 1（不是漂移）：真实抓包 TDCFG 应答 `Mode : 2` 冒号前带空格，
	 * 旧正则 /Mode:\s*(\d+)/ 不识别 → 接口模式控件在真实设备上永远无选中；
	 * 新链路（parse_interface_cfg 的宽松解析）把值解出来，控件恢复可用 ——
	 * 与连接页 PostRoute 同款、与姊妹项目 luci-app-mt5700 的 dial.js 对齐。 */
	{
		console.log('有意差异：真实抓包 TDCFG（Mode 冒号前带空格 → 旧正则盲区）');
		const shape = { facts: { tdcfg: 'Mode : 2\r\nPostRoute : 1\r\nDmz: 192.168.8.100' } };
		const o = await render('old', shape);
		const n = await render('new', shape);
		const oSel = selectsOf(o), nSel = selectsOf(n);
		check('盲区形态：旧侧接口模式无选中（正则不识别），新侧解出 mode=2（有意修复）',
			oSel[3] === '' && nSel[3] === '2',
			JSON.stringify([ oSel, nSel ]));
		check('盲区形态：其余六个控件两侧不受影响（逐位一致）',
			sameJson(oSel.slice(0, 3).concat(oSel.slice(4)), nSel.slice(0, 3).concat(nSel.slice(4))),
			JSON.stringify([ oSel, nSel ]));
	}

	console.log('调用面：整页 load()/render() 的 API 调用序');
	{
		const o = await render('old', { facts: {} });
		const n = await render('new', { facts: {} });
		const oAt = o.side.api.calls.filter((c) => c.kind === 'at').map((c) => c.args.join(' '));
		const oRoute = o.side.api.calls.filter((c) => c.kind === 'route').map((c) => c.name);
		const nAt = n.side.api.calls.filter((c) => c.kind === 'at');
		const nRoute = n.side.api.calls.filter((c) => c.kind === 'route').map((c) => c.name);
		check('旧侧：恰一条 advanced hardware CLI 帧、零路由',
			sameJson(oAt, [ 'advanced hardware' ]) && oRoute.length === 0,
			JSON.stringify([ oAt, oRoute ]));
		check('新侧：零 CLI、恰 5 条读路由且顺序即 load() 的 Promise.all',
			nAt.length === 0 && sameJson(nRoute, [ 'network.usb_mode', 'network.interface_cfg',
				'system.device_control', 'sim.slot', 'system.thermal' ]),
			JSON.stringify([ nAt, nRoute ]));
	}

	console.log('A2. Technical details 折叠块（有意差异：AT 原文 → 路由载荷）');
	{
		const o = await render('old', { facts: {} });
		const n = await render('new', { facts: {} });
		const oPre = lib.collect(o.holder, (x) => x.tagName === 'PRE'
			&& String((x.attrs && x.attrs['class']) || '').indexOf('mt-raw') !== -1)[0];
		const nPre = lib.collect(n.holder, (x) => x.tagName === 'PRE'
			&& String((x.attrs && x.attrs['class']) || '').indexOf('mt-raw') !== -1)[0];
		check('「Technical details」折叠块仍在（同一 class、非空）',
			!!oPre && !!nPre && lib.textOf(oPre).length > 0 && lib.textOf(nPre).length > 0);
		check('旧侧倾倒 8 段 AT 帧原文（USB mode 段标题在内）',
			lib.textOf(oPre).indexOf('===== USB mode: AT^SETMODE? =====') !== -1
			&& lib.textOf(oPre).indexOf('===== Thermal control: AT^THERMAUTOFUN? =====') !== -1);
		check('新侧倾倒 5 条路由载荷（api.<route> + JSON）',
			lib.textOf(nPre).indexOf('===== api.network.usb_mode =====') !== -1
			&& lib.textOf(nPre).indexOf('"mode": 4') !== -1
			&& lib.textOf(nPre).indexOf('===== api.system.thermal =====') !== -1);
	}

	console.log('写路径：六条写入逐条驱动（旧 argv vs 新路由 + params）');
	{
		const r = await render('old', { facts: {} });
		for (const w of WRITES) {
			const calls = await fireWrite(r, w);
			const at = calls.filter((c) => c.kind === 'at').map((c) => c.args);
			check(w.key + '：旧侧 CLI argv', sameJson(at, [ w.oldArgs ]),
				JSON.stringify(at));
		}
	}
	{
		const r = await render('new', { facts: {} });
		for (const w of WRITES) {
			const calls = await fireWrite(r, w);
			const routeCalls = calls.filter((c) => c.kind === 'routeCall');
			check(w.key + '：新侧路由名 + params 逐键一致（键序一并钉住）',
				routeCalls.length === 1 && routeCalls[0].name === w.newCall.name
				&& sameJson(routeCalls[0].params, w.newCall.params),
				JSON.stringify(calls));
		}
		const anyAt = r.side.api.calls.filter((c) => c.kind === 'at');
		check('新侧六条写入全程零 CLI', anyAt.length === 0, JSON.stringify(anyAt));
	}

	if (failures) {
		console.log('FAIL：' + failures + ' 项不一致（见上）');
		process.exit(1);
	}
	console.log('PASS：高级设置页的新旧渲染逐字一致（8 段 hardware 读帧已切成'
		+ ' 5 条读路由 + 6 条写路由；USB mode 两级 fallback / 温控短应答回落 /'
		+ ' PHY 首字段 0 三种怪癖形态在载荷层原样仿制；Mode 冒号空格盲区与'
		+ ' PCIe 写命令短形态作为有意修复单独点名 —— 对齐姊妹项目 luci-app-mt5700）');
	process.exit(0);
})();
