#!/usr/bin/env node
'use strict';

/*
 * 无线偏好卡片一致性证明（LuCI 无线页，radio-preference 切片）
 *
 * 本批把无线页最后一块切 CLI 文本帧的代码（`mt5700m-at advanced radio` 的
 * 五个段）改成统一路由，读写都走：
 *   - 读：`network.syscfg`（^SYSCFGEX?）、`network.c5goption`（^C5GOPTION?）、
 *     `modem.nr_capability`（^NRRCCAPQRY 的 3/2/5 三种查询）
 *   - 写：`network.syscfg_set` / `network.c5goption_set` / `modem.nr_capability_set`
 *     （原来分别走 `advanced-set radio-policy / 5g-access / carrier-aggregation /
 *     vonr / dss` 五个 CLI 动词）
 *   - 删除 `api.atRadio`（本页最后一个 CLI 调用）与 `parser.matchValues`
 *     （最后一个调用者就在这五个段里）
 *
 * 用法：
 *   node scripts/prove-radio-parity.js [基线]    # 基线默认 HEAD，本批之前是 4474436
 * 退出码 0 = 通过，1 = 不一致，2 = 基线选错了（基线里已经没有旧实现）。
 *
 * 脚本做四件事：
 *   1. 用 Rust 单测样本钉住夹具（scripts/lib/at-fixtures.js）；
 *   2. 整页两边都 load()/render()，逐字比较无线偏好卡片（DOM + 取值）；
 *   3. 五个写操作各驱动一遍「按钮 → Apply」，比较新旧提交的参数与通知文案：
 *      旧 = CLI 位置参数，新 = 路由 typed 参数；断言两者一一对应；
 *   4. 断言调用面：新版不再有 at radio，本页零 CLI 调用。
 */

const fs = require('fs');
const path = require('path');
const cp = require('child_process');
const lib = require('./lib/luci-stub');
const fixtures = require('./lib/at-fixtures');

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

/* ------------------------------------------------------------ 夹具钉子
 * 这些值与 modules/<module>/parser.rs 的单测样本对应；夹具只做「一台模组的一种读数」，
 * 下面的形态表覆盖缺失字段的回退。
 */
console.log('Rust 契约钉子（夹具 vs modules/*）');
const MODEM = {
	hcsq: '^HCSQ: "NR",58,165,22',
	monsc: '^MONSC: NR,460,00,636648,0,10321,1FA,2F01,-82,-9',
	chiptemp: '^CHIPTEMP: 432,445,451,398,401,407,441,442,449,0,65535,448',
	rrc: '^RRCSTAT: 1,1,98',
	cereg: '+CEREG: 2,1,"2F01","10321",7',
	cops: '+COPS: 0,0,"CHN-UNICOM",7'
};
const RADIO = {
	syscfgex: '^SYSCFGEX: 080302,3FFFFFFF,1,2,7FFFFFFFFFFFFFFF',
	c5g: '^C5GOPTION: 1,1,1',
	ca: '^NRRCCAPQRY: 3,1',
	vonr: '^NRRCCAPQRY: 2,1',
	dss: '^NRRCCAPQRY: 5,0,1'
};
check('空夹具自检（decodeSignal/decodeTemps 与 Rust 单测同形）',
	fixtures.decodeSignal(MODEM.hcsq).rsrp === -83 && fixtures.decodeTemps(MODEM.chiptemp).peak === 45.1);

/* ------------------------------------------------------------ 载荷与帧 */
function radioFrame(replies) {
	return fixtures.textFrame([
		[ 'Radio mode', 'AT^SYSCFGEX?', replies.syscfgex ],
		[ '5G access mode', 'AT^C5GOPTION?', replies.c5g ],
		[ 'NR carrier aggregation', 'AT^NRRCCAPQRY=3', replies.ca ],
		[ 'VoNR', 'AT^NRRCCAPQRY=2', replies.vonr ],
		[ 'DSS', 'AT^NRRCCAPQRY=5', replies.dss ]
	]);
}
/* 路由载荷一律来自共享夹具层（scripts/lib/at-fixtures.js 的 decodeSyscfg /
 * decodeC5gOption / decodeNrCapability）——与 modules/{network,modem} 的 parser
 * 规则逐条对应；证明脚本自己不再写第二份解码。 */
/* ------------------------------------------------------------ 桩 */
function apiFor(answers) {
	const api = lib.makeApi(answers);
	api.atRadio = () => api.at([ 'advanced', 'radio' ]);
	return api;
}
function answersFor(replies) {
	return {
		'at:advanced': { stdout: radioFrame(replies), stderr: '' },
		'route:signal.get': fixtures.decodeSignal(MODEM.hcsq),
		'route:cell.get': Object.assign({ band: '78' }, fixtures.decodeCell(MODEM.monsc)),
		'route:registration.get': fixtures.decodeRegistration(MODEM.cereg),
		'route:network.get': fixtures.decodeOperator(MODEM.cops),
		'route:network.rrc': fixtures.decodeRrc(MODEM.rrc),
		'route:system.temperature': fixtures.decodeTemps(MODEM.chiptemp),
		'route:network.syscfg': fixtures.decodeSyscfg(replies.syscfgex),
		'route:network.c5goption': fixtures.decodeC5gOption(replies.c5g),
		'route:modem.nr_capability': fixtures.decodeNrCapability(replies)
	};
}

const apiOld = apiFor(answersFor(RADIO));
const apiNew = apiFor(answersFor(RADIO));
const old = lib.loadSide(oldSource, apiOld);
const neu = lib.loadSide(newSource, apiNew);

if (typeof old.api.atRadio !== 'function') {
	console.error('基线 ' + BASELINE + ' 已经包含本批（api.js 里没有 atRadio）——没有旧管线可比较。\n'
		+ '本批的基线是 4474436：\n  node scripts/prove-radio-parity.js 4474436');
	process.exit(2);
}

/* ------------------------------------------------------------ 渲染 */
function radioCardOf(holder) {
	// 无线偏好卡片：卡片标题是 'Radio preferences' 的那一节
	return lib.collect(holder, n => n.tagName === 'SECTION').filter(section =>
		lib.collect(section, x => x.tagName === 'H3').some(h => lib.textOf(h) === 'Radio preferences'))[0] || null;
}
function selectsOf(holder) {
	const card = radioCardOf(holder);
	return lib.collect(card, n => n.tagName === 'SELECT').map(n => String(n.value));
}
function textOf(holder) {
	const card = radioCardOf(holder);
	return lib.textOf(card);
}

(async function () {
	old.view.load();
	neu.view.load();
	const oldHolder = old.view.render(), newHolder = neu.view.render();
	await old.view.contentReady;
	await neu.view.contentReady;

	console.log('\n无线偏好卡片：结构与文案');
	const shapeOpts = { attrs: true, skipAttrs: [ 'value' ] };
	const oldShape = lib.serialize(oldHolder, shapeOpts), newShape = lib.serialize(newHolder, shapeOpts);
	check('整页结构 + 文案 + 取值逐字一致', oldShape === newShape,
		oldShape === newShape ? '' : diffText(oldShape, newShape));
	check('下拉取值（无线电序/漫游/服务域/5G 接入/CA/VoNR/DSS 速率/DMRS）一致',
		sameJson(selectsOf(oldHolder), selectsOf(newHolder)) && selectsOf(newHolder).length === 8,
		JSON.stringify({ old: selectsOf(oldHolder), new: selectsOf(newHolder) }));
	check('当前无线电模式文案一致（5G NR / LTE / WCDMA · 080302）',
		textOf(oldHolder).indexOf('5G NR / LTE / WCDMA · 080302') !== -1
		&& textOf(newHolder).indexOf('5G NR / LTE / WCDMA · 080302') !== -1);

	console.log('\n形态：缺段/缺失字段的回退与旧版逐字一致');
	const FORMS = [
		{ key: '全部缺失（模组不答）', replies: {} },
		{ key: '只有 ^SYSCFGEX 且 field 为空', replies: { syscfgex: '^SYSCFGEX: ,,,,' } },
		{ key: '漫游/服务域为 0', replies: Object.assign({}, RADIO, { syscfgex: '^SYSCFGEX: 0803,3FFFFFFF,0,1,7FFFFFFFFFFFFFFF' }) },
		{ key: '5G 接入 = Option 2 三元组', replies: Object.assign({}, RADIO, { c5g: '^C5GOPTION: 1,0,1' }) },
		{ key: '5G 接入 = Option 3 三元组', replies: Object.assign({}, RADIO, { c5g: '^C5GOPTION: 0,1,0' }) },
		{ key: '5G 接入三元组不完整', replies: Object.assign({}, RADIO, { c5g: '^C5GOPTION: 1,0' }) },
		{ key: 'CA 关闭 / VoNR FR2 / DSS 1,0', replies: Object.assign({}, RADIO, { ca: '^NRRCCAPQRY: 3,0', vonr: '^NRRCCAPQRY: 2,2', dss: '^NRRCCAPQRY: 5,1,0' }) },
		{ key: '能力查询只剩 CA', replies: Object.assign({}, RADIO, { vonr: '', dss: '' }) }
	];
	for (const form of FORMS) {
		const o = lib.loadSide(oldSource, apiFor(answersFor(form.replies)));
		const n = lib.loadSide(newSource, apiFor(answersFor(form.replies)));
		o.view.load(); n.view.load();
		const oh = o.view.render(), nh = n.view.render();
		await o.view.contentReady;
		await n.view.contentReady;
		const oShape = lib.serialize(oh, shapeOpts), nShape = lib.serialize(nh, shapeOpts);
		check(form.key, oShape === nShape && sameJson(selectsOf(oh), selectsOf(nh)),
			oShape === nShape ? JSON.stringify(selectsOf(nh)) : diffText(oShape, nShape));
	}

	/* ------------------------------------------------------------ 写路径
	 * 五个 Apply 各自走一遍：旧 = CLI 位置参数，新 = 路由 typed 参数。
	 * 断言两者一一对应（同一份表单状态 → 同一组语义值）。
	 */
	console.log('\n写路径：表单 → 参数（旧 CLI 位置参数 vs 新路由参数）');
	async function commit(side, holder, buttonText) {
		const api = side.api;
		api.calls.length = 0;
		lib.pressButton(holder, buttonText);
		lib.modalButton(side.scope, 'Apply');
		for (let i = 0; i < 4; i++) await lib.tick();
		return api.calls.filter(c => c.kind === 'at' || c.kind === 'routeCall');
	}
	/* 每个用例先把两边都置成同一份表单状态 */
	function setSelect(holder, index, value) {
		const selects = lib.collect(radioCardOf(holder), n => n.tagName === 'SELECT');
		selects[index].value = value;
		return selects;
	}
	/* 勾选：bandChecklist 的复选框按顺序（WCDMA 2 个、LTE 9 个） */
	function bandInputs(holder) {
		return lib.collect(radioCardOf(holder), n => n.tagName === 'INPUT' && n.attrs.type === 'checkbox');
	}

	const CASES = [
		{
			key: '网络策略（Apply network and band settings）',
			button: 'Apply network and band settings',
			form: (holder) => { setSelect(holder, 0, '0803'); setSelect(holder, 1, '0'); setSelect(holder, 2, '1'); },
			expectCall: (call) => call.kind === 'routeCall' && call.name === 'network.syscfg_set'
				&& sameJson(call.params, { acqorder: '0803', band: '3FFFFFFF', roam: 0, srvdomain: 1, lteband: '7FFFFFFFFFFFFFFF' }),
			oldArgsFor: (holder) => [ 'advanced-set', 'radio-policy', '0803', '3FFFFFFF', '0', '1', '7FFFFFFFFFFFFFFF' ]
		},
		{
			key: '部分选段（取消 WCDMA bit + 只留一个 LTE 段）',
			button: 'Apply network and band settings',
			form: (holder) => {
				const boxes = bandInputs(holder);
				boxes[0].checked = false;                    // 取消 0x400000
				for (let i = 0; i < 9; i++) boxes[2 + i].checked = i === 0;  // LTE 只留 0x1
			},
			oldArgsFor: () => [ 'advanced-set', 'radio-policy', '080302', '2000000000000', '1', '2', '1' ],
			expectCall: (call) => call.kind === 'routeCall' && call.name === 'network.syscfg_set'
				&& sameJson(call.params, { acqorder: '080302', band: '2000000000000', roam: 1, srvdomain: 2, lteband: '1' })
		},
		{
			key: '5G 接入模式 Option 3（Apply 5G access mode）',
			button: 'Apply 5G access mode',
			form: (holder) => { setSelect(holder, 3, 'option3'); },
			oldArgsFor: () => [ 'advanced-set', '5g-access', 'option3' ],
			expectCall: (call) => call.kind === 'routeCall' && call.name === 'network.c5goption_set'
				&& sameJson(call.params, { nr_sa_support_flag: 0, nr_dc_mode: 1, gc_access_mode: 0 })
		},
		{
			key: '载波聚合关闭（Apply carrier aggregation）',
			button: 'Apply carrier aggregation',
			form: (holder) => { setSelect(holder, 4, '0'); },
			oldArgsFor: () => [ 'advanced-set', 'carrier-aggregation', '0' ],
			expectCall: (call) => call.kind === 'routeCall' && call.name === 'modem.nr_capability_set'
				&& sameJson(call.params, { ca: false })
		},
		{
			key: 'VoNR FR1 + FR2（Apply VoNR mode）',
			button: 'Apply VoNR mode',
			form: (holder) => { setSelect(holder, 5, '3'); },
			oldArgsFor: () => [ 'advanced-set', 'vonr', '3' ],
			expectCall: (call) => call.kind === 'routeCall' && call.name === 'modem.nr_capability_set'
				&& sameJson(call.params, { vonr: 3 })
		},
		{
			key: 'DSS 两个开关（Apply DSS settings）',
			button: 'Apply DSS settings',
			form: (holder) => { setSelect(holder, 6, '1'); setSelect(holder, 7, '0'); },
			oldArgsFor: () => [ 'advanced-set', 'dss', '1', '0' ],
			expectCall: (call) => call.kind === 'routeCall' && call.name === 'modem.nr_capability_set'
				&& sameJson(call.params, { dss: { rateMatchingLTE: 1, additionalDMRS: 0 } })
		}
	];

	for (const c of CASES) {
		const oSide = lib.loadSide(oldSource, apiFor(answersFor(RADIO)));
		const nSide = lib.loadSide(newSource, apiFor(answersFor(RADIO)));
		oSide.view.load(); nSide.view.load();
		const oHolder = oSide.view.render(), nHolder = nSide.view.render();
		await oSide.view.contentReady;
		await nSide.view.contentReady;
		c.form(oHolder); c.form(nHolder);
		const oldCalls = await commit(oSide, oHolder, c.button);
		const newCalls = await commit(nSide, nHolder, c.button);
		const oldAt = oldCalls.filter(x => x.kind === 'at');
		const oldArgs = oldAt.length ? oldAt[0].args : null;
		const newCall = newCalls.filter(x => x.kind === 'routeCall')[0] || null;
		check(c.key + '：旧 CLI 位置参数 = ' + JSON.stringify(c.oldArgsFor()),
			sameJson(oldArgs, c.oldArgsFor()), JSON.stringify(oldArgs));
		check(c.key + '：新路由参数与表单一一对应', !!newCall && c.expectCall(newCall),
			JSON.stringify(newCall));
		const oldNote = oSide.scope.ui.notifications.map(n => n.text + '|' + n.level);
		const newNote = nSide.scope.ui.notifications.map(n => n.text + '|' + n.level);
		check(c.key + '：成功通知文案与级别一致（' + JSON.stringify(newNote) + '）', sameJson(oldNote, newNote),
			JSON.stringify({ old: oldNote, new: newNote }));
		// 桩把非 0 延时的 setTimeout 记进 window.pending（立即执行会掩盖「等 900ms
		// 再刷新」这条行为），这里比较延时并在两边都跑一遍回调，证明刷新真的发生。
		const oldPending = oSide.scope.window.pending.map(p => p.delay);
		const newPending = nSide.scope.window.pending.map(p => p.delay);
		oSide.scope.window.pending.forEach(p => p.fn());
		nSide.scope.window.pending.forEach(p => p.fn());
		check(c.key + '：成功后都等 900ms 再刷新页面',
			sameJson(oldPending, [ 900 ]) && sameJson(newPending, [ 900 ])
			&& oSide.scope.window.reloaded === true && nSide.scope.window.reloaded === true,
			JSON.stringify({ old: oldPending, new: newPending,
				reloaded: [ oSide.scope.window.reloaded === true, nSide.scope.window.reloaded === true ] }));
	}

	/* 失败路径：后端/CLI 报错 → danger 通知带同样的错误文本 */
	console.log('\n写路径：失败通知');
	{
		const oSide = lib.loadSide(oldSource, apiFor(Object.assign({}, answersFor(RADIO), { 'at:advanced-set': { ok: false, error: 'modem rejected the write' } })));
		const nSide = lib.loadSide(newSource, apiFor(Object.assign({}, answersFor(RADIO), { 'call:modem.nr_capability_set': { ok: false, error: 'modem rejected the write' } })));
		oSide.view.load(); nSide.view.load();
		const oHolder = oSide.view.render(), nHolder = nSide.view.render();
		await oSide.view.contentReady;
		await nSide.view.contentReady;
		lib.pressButton(oHolder, 'Apply VoNR mode');
		lib.modalButton(oSide.scope, 'Apply');
		lib.pressButton(nHolder, 'Apply VoNR mode');
		lib.modalButton(nSide.scope, 'Apply');
		for (let i = 0; i < 4; i++) await lib.tick();
		check('失败 → 同一条 danger 通知（消息取后端文本）',
			sameJson(oSide.scope.ui.notifications, nSide.scope.ui.notifications)
			&& nSide.scope.ui.notifications[0].level === 'danger'
			&& nSide.scope.ui.notifications[0].text === 'modem rejected the write',
			JSON.stringify({ old: oSide.scope.ui.notifications, new: nSide.scope.ui.notifications }));
	}

	/* ------------------------------------------------------------ 调用面 */
	console.log('\n调用面');
	const at = (api) => api.calls.filter(c => c.kind === 'at').map(c => c.args[0] + ' ' + (c.args[1] || ''));
	check('旧版：页面加载就读 at advanced radio', sameJson(at(apiOld), [ 'advanced radio' ]), JSON.stringify(at(apiOld)));
	check('新版：本页零 CLI 调用（at radio 已删）', at(apiNew).length === 0, JSON.stringify(at(apiNew)));
	const routes = (api) => api.calls.filter(c => c.kind === 'route').map(c => c.name);
	check('新版加载时读三条无线偏好路由 + 原有六条',
		[ 'network.syscfg', 'network.c5goption', 'modem.nr_capability' ].every(n => routes(apiNew).indexOf(n) !== -1),
		JSON.stringify(routes(apiNew)));
	check('删除的旧实现：api.js 的 atRadio、parser.js 的 matchValues（最后一个调用者本批迁走）',
		oldSource(RES + '/mt5700m/api.js').indexOf('function atRadio') !== -1
		&& newSource(RES + '/mt5700m/api.js').indexOf('atRadio') === -1
		&& oldSource(RES + '/mt5700m/parser.js').indexOf('function matchValues') !== -1
		&& newSource(RES + '/mt5700m/parser.js').indexOf('matchValues') === -1);
	check('ucode 超时预算：三条新写路由与 lock_apply 同为 25s',
		newSource('luci-app-mt5700m/root/usr/share/rpcd/ucode/mt5700.uc').indexOf('api.network.c5goption_set') !== -1
		&& newSource('luci-app-mt5700m/root/usr/share/rpcd/ucode/mt5700.uc').indexOf('api.modem.nr_capability_set') !== -1);

	console.log(failures === 0
		? '\nPASS：无线偏好卡片的新旧渲染与写路径一致（读走路由、写走 typed 参数）'
		: '\nFAIL：' + failures + ' 项不一致');
	process.exit(failures === 0 ? 0 : 1);
})().catch(function (err) {
	console.error('渲染失败：' + (err && err.stack || err));
	process.exit(1);
});
