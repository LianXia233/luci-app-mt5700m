#!/usr/bin/env node
'use strict';

/*
 * 压缩后的 LuCI 前端冒烟（minified tree smoke）
 *
 * 为什么需要它：压缩后 esbuild 会重命名一切、合并/删掉死路径，对压缩产物 grep
 * 源码是没意义的（`api.route('signal.get')` 之类字符串可能被内联改写），唯一的
 * 办法是把压缩后的文件真的装载、渲染一遍。同时它也证明压缩没有破坏页面里的
 * 路由调用与取值映射。
 *
 * 用法：
 *   bash scripts/minify-luci-frontend.sh /tmp/htdocs-min     # 先压缩一份
 *   node scripts/smoke-minified-luci.js [/tmp/htdocs-min]
 *
 * 覆盖已迁完详情帧的三个页面：
 *   - 无线页（零 CLI）、短信页（零 CLI）；
 *   - 概览页：零 CLI，详情读 qos.get / sim.number / network.pdp_contexts /
 *     network.session；
 *   - 连接页：拨号设置仍走 CLI 动词（未迁移），会话面板读 network.session，
 *     「清空计数」走 network.flow_clear 路由；
 *   - 系统页：22 段读帧仍是 `mt5700m-at system`（下一刀才切），6 条**写入**
 *     已走 network.radio_set / sim.slot_set / modem.reset / system.factory_reset /
 *     sim.pin_apply / modem.imei_set，保留的 7 条（LED / SIM 激活 / 温控表 /
 *     FOTA 三步）仍走动词。
 * 断言路由调用面、页面上的关键文字（盘面读数、无线偏好下拉、会话侧栏、概览页的
 * APN/QCI/速率/号码、系统页的身份/温度/运营商）与写路径（短信发送、清空计数、
 * 系统页四条写入）。退出码 0 = 通过。
 */

const fs = require('fs');
const path = require('path');
const lib = require('./lib/luci-stub');
const fixtures = require('./lib/at-fixtures');

const ROOT = process.argv[2] || '/tmp/htdocs-min';
const PREFIX = 'luci-app-mt5700m/htdocs/';
const RES = 'luci-static/resources';
const read = (rel) => fs.readFileSync(path.join(ROOT, rel.indexOf(PREFIX) === 0 ? rel.slice(PREFIX.length) : rel), 'utf8');

let failures = 0;
function check(name, ok, detail) {
	if (ok) { console.log('  ok   ' + name); return; }
	failures++;
	console.log('  FAIL ' + name + (detail ? '\n       ' + detail : ''));
}
const sameJson = (a, b) => JSON.stringify(a) === JSON.stringify(b);

/* ------------------------------------------------------------ 无线页 */
const MODEM = {
	hcsq: '^HCSQ: "NR",58,165,22',
	monsc: '^MONSC: NR,460,00,636648,0,10321,1FA,2F01,-83,-9,12.8',
	rrc: '^RRCSTAT: 1,1,98',
	cereg: '+CEREG: 2,1,"2F01","10321",7',
	cops: '+COPS: 0,0,"CHN-UNICOM",7',
	chiptemp: '^CHIPTEMP: 432,445,451,398,401,407,441,442,449,0,65535,448',
	syscfgex: '^SYSCFGEX: 080302,3FFFFFFF,1,2,7FFFFFFFFFFFFFFF',
	c5g: '^C5GOPTION: 1,1,1',
	ca: '^NRRCCAPQRY: 3,1', vonr: '^NRRCCAPQRY: 2,1', dss: '^NRRCCAPQRY: 5,0,1'
};

async function networkPage() {
	const api = lib.makeApi({
		'route:signal.get': fixtures.decodeSignal(MODEM.hcsq),
		'route:cell.get': Object.assign({ band: '78' }, fixtures.decodeCell(MODEM.monsc)),
		'route:registration.get': fixtures.decodeRegistration(MODEM.cereg),
		'route:network.get': fixtures.decodeOperator(MODEM.cops),
		'route:network.rrc': fixtures.decodeRrc(MODEM.rrc),
		'route:system.temperature': fixtures.decodeTemps(MODEM.chiptemp),
		'route:network.syscfg': fixtures.decodeSyscfg(MODEM.syscfgex),
		'route:network.c5goption': fixtures.decodeC5gOption(MODEM.c5g),
		'route:modem.nr_capability': fixtures.decodeNrCapability({ ca: MODEM.ca, vonr: MODEM.vonr, dss: MODEM.dss })
	});
	const side = lib.loadSide(read, api);
	side.view.load();
	const holder = side.view.render();
	await side.view.contentReady;
	const at = api.calls.filter(c => c.kind === 'at').map(c => c.args.join(' '));
	const routes = api.calls.filter(c => c.kind === 'route').map(c => c.name + (c.params && c.params.rat ? '/' + c.params.rat : ''));
	const text = lib.textOf(holder);
	const card = lib.collect(holder, n => n.tagName === 'SECTION').filter(s =>
		lib.collect(s, x => x.tagName === 'H3').some(h => lib.textOf(h) === 'Radio preferences'))[0];
	const selects = lib.collect(card, n => n.tagName === 'SELECT').map(n => String(n.value));
	check('无线页：零 CLI 调用', at.length === 0, JSON.stringify(at));
	check('无线页：11 条读路由（状态区块 6 + 无线偏好 3 + 两个锁）',
		routes.length === 11 && routes.indexOf('modem.nr_capability') !== -1, JSON.stringify(routes));
	check('无线页：仪表盘读数来自路由载荷（-83 dBm / 12.8 dB / 45.1°C 峰值）',
		text.indexOf('-83') !== -1 && text.indexOf('12.8') !== -1 && text.indexOf('45.1') !== -1);
	check('无线页：无线偏好下拉取值（080302 / 1 / 2 / option23 / CA / VoNR / DSS）',
		sameJson(selects, [ '080302', '1', '2', 'option23', '1', '1', '0', '1' ]), JSON.stringify(selects));
	check('无线页：当前无线电模式文案', text.indexOf('5G NR / LTE / WCDMA · 080302') !== -1);
}

/* ------------------------------------------------------------ 短信页 */
const RECEIVED = [
	{ index: 1, number: '8613800138000', time: '26/08/26,08:01:00', text: '流量统计已于今日零点自动刷新。' },
	{ index: 2, number: '8613800138000', time: '26/08/26,09:15:00', text: '设备已成功接入 5G 网络。' },
	{ index: 4, number: '8613912345678', time: '26/08/24,17:28:00', text: '巡检清单已经发到群里，现场重点看 5G 信号和温度。' },
	{ index: 5, number: '8613912345678', time: '26/08/24,18:06:00', text: '今晚 7 点到机房巡检，记得带测试电脑。' }
];

async function smsPage() {
	const api = lib.makeApi({
		'call:sms.list': { messages: RECEIVED.map(m => ({ index: m.index, content: m.text, number: m.number, time: m.time, type: 'received' })) },
		'call:sms.status': {
			enabled: true, imsOn: true, center: '+8613800138000',
			storage: {
				read: { name: 'SM', used: 0, total: 30 },
				write: { name: 'ME', used: 4, total: 50 },
				receive: { name: 'SM', used: 0, total: 30 }
			}
		}
	});
	const side = lib.loadSide(read, api, RES + '/view/mt5700m/sms.js');
	side.view.load();
	const holder = side.view.render();
	await side.view.contentReady;
	const at = api.calls.filter(c => c.kind === 'at').map(c => c.args.join(' '));
	const routes = api.calls.filter(c => c.kind === 'routeCall').map(c => c.name);
	const text = lib.textOf(holder);
	check('短信页：零 CLI 调用', at.length === 0, JSON.stringify(at));
	check('短信页：读 sms.list + sms.status', sameJson(routes, [ 'sms.list', 'sms.status' ]), JSON.stringify(routes));
	check('短信页：槽位角标取自 +CPMS 读取面（0 of 30）', text.indexOf('0 of 30 message slots used') !== -1);
	check('短信页：侧栏会话与最新会话正文/时间（号码为后端领域值，无 + 前缀）',
		text.indexOf('8613912345678') !== -1 && text.indexOf('8613800138000') !== -1 && text.indexOf('+8613') === -1
		&& text.indexOf('2026-08-24 18:06 · 2') !== -1 && text.indexOf('巡检清单已经发到群里') !== -1
		&& text.indexOf('今晚 7 点到机房巡检') !== -1);
	/* 写路径：发送 */
	const inputs = lib.collect(holder, n => n.tagName === 'INPUT' && String(n.attrs['class'] || '') === 'mt-sms-compose-input');
	inputs[0].value = '13800138000';
	inputs[1].value = 'hi';
	lib.pressButton(holder, 'Send');
	lib.modalButton(side.scope, 'Send');
	for (let i = 0; i < 4; i++) await lib.tick();
	const send = api.calls.filter(c => c.kind === 'routeCall' && c.name === 'sms.send')[0];
	check('短信页：发送走 sms.send 路由（参数取表单）',
		!!send && send.params.number === '13800138000' && send.params.text === 'hi', JSON.stringify(send));
	check('短信页：发送成功提示', side.scope.ui.notifications.some(n => n.text === 'Message sent.'));
}

/* ------------------------------------------------------------ 概览页 */
async function statusPage() {
	const snapshot = {
		signal: { value: { sysmode: 'NR5G', rsrp: -83, rsrq: -12, sinr: 12.8, rssi: -55 } },
		network: { value: { operator: 'CHN-UNICOM', sysmode: 'NR5G', sysmode_detail: '5G SA' } },
		temperature: { value: { average: 45.1 } },
		modem: { value: { manufacturer: 'Quectel', model: 'MT5700M', revision: 'MT5700M-2.5.0', imei: '862853030012345' } },
		sim: { value: { status: 'READY', iccid: '89860312345678901234', imsi: '460011234567890' } },
		cell: { value: { band: 41, channel: 504990, dlBandwidth: 100, sysmode: 'NR' } },
		endc: { value: { established: 1 } },
		usb: { value: { present: true, state: 'normal', path: '/dev/ttyUSB3', available: true } }
	};
	const api = lib.makeApi({
		'route:qos.get': { active_cid: 1, qci: '9', ambr_down_kbps: 102400, ambr_up_kbps: 51200, ambr_apn: 'cmnet' },
		'route:sim.number': { status: 'READY', number: '8613800138000' },
		'route:network.pdp_contexts': { contexts: [ { cid: 1, type: 'IP', apn: 'cmnet', pdp_addr: '10.6.172.152', active: true } ] },
		'route:network.session': fixtures.sessionPayload({})
	});
	api.managerStatus = () => Promise.resolve({ connected: true, at_port: '/dev/ttyUSB3', network: 'eth2' });
	api.trafficSummary = () => Promise.resolve({ interfaces: [] });
	api.cachedSnapshot = () => Promise.resolve(snapshot);
	const side = lib.loadSide(read, api, RES + '/view/mt5700m/status.js');
	side.view.load();
	const holder = side.view.render();
	side.scope.document.body.replaceChildren(holder);
	for (let i = 0; i < 200; i++) {
		if (lib.collect(holder, x => x.attrs && x.attrs['data-live-region'] === 'facts').length) break;
		await lib.tick();
	}
	for (let i = 0; i < 8; i++) await lib.tick();
	const at = api.calls.filter(c => c.kind === 'at').map(c => c.args.join(' '));
	const routes = api.calls.filter(c => c.kind === 'route').map(c => c.name);
	const text = lib.textOf(holder);
	check('概览页：零 CLI 调用（mt5700m-at 的任何动词都不再出现在概览页）',
		at.length === 0, JSON.stringify(at));
	check('概览页：详情读 qos.get + sim.number + network.pdp_contexts + network.session',
		sameJson(routes, [ 'qos.get', 'sim.number', 'network.pdp_contexts', 'network.session' ]), JSON.stringify(routes));
	check('概览页：盘面读数来自快照主题（-83 / 12.8 / 45.1 / n41 / 504990）',
		text.indexOf('-83') !== -1 && text.indexOf('12.8') !== -1 && text.indexOf('45.1') !== -1
		&& text.indexOf('B41') !== -1 && text.indexOf('504990') !== -1);
	check('概览页：详情行来自路由（cmnet / QCI 9 / Down 102 Mbps / 纯数字号码 / Not stored 不出现）',
		text.indexOf('cmnet') !== -1 && text.indexOf('QCI 9') !== -1
		&& text.indexOf('Down 102 Mbps / Up 51 Mbps') !== -1 && text.indexOf('8613800138000') !== -1
		&& text.indexOf('Not stored') === -1);
	check('概览页：没有任何告警横幅（详情帧不再因 CLI 类型错误挂横幅）', text.indexOf('could not be refreshed') === -1);
}

/* ------------------------------------------------------------ 连接页 */
async function connectionPage() {
	const api = lib.makeApi({
		'route:network.session': fixtures.sessionPayload({}),
		'at:advanced connection-settings': { stdout: '', stderr: '' }
	});
	api.atConnectionSettings = () => api.at([ 'advanced', 'connection-settings' ]);
	api.managerStatus = () => Promise.resolve({ connected: true, network: 'eth2', at_port: '/dev/ttyUSB3' });
	api.deviceStatus = () => Promise.resolve({ up: true, carrier: true });
	api.dialLog = () => Promise.resolve({ log: '' });
	const uci = {
		load: () => Promise.resolve(),
		get: (pkg, sec, opt) => ({ 'mt5700m.connection.enabled': '1', 'mt5700m.connection.apn': 'cmnet',
			'mt5700m.connection.pdp_type': 'ipv4v6' })[pkg + '.' + sec + '.' + opt],
		set: () => {}, save: () => Promise.resolve(), apply: () => Promise.resolve()
	};
	const option = () => { const o = { value: () => o, depends: () => o }; return o; };
	const section = { anonymous: false, option: () => option() };
	const form = {
		Map: function () { return { section: () => section, render: () => Promise.resolve({ tagName: 'DIV', attrs: { 'class': 'cbi-map' }, children: [] }) }; },
		NamedSection: function () {}, Flag: 'flag', Value: 'value', ListValue: 'list',
		DynamicList: 'dynamic', Section: function () {}
	};
	const side = lib.loadSide(read, api, RES + '/view/mt5700m/connection.js', { uci: uci, form: form });
	side.view.load();
	const holder = side.view.render();
	await side.view.contentReady;
	for (let i = 0; i < 20; i++) await lib.tick();
	const at = api.calls.filter(c => c.kind === 'at').map(c => c.args.join(' '));
	const routes = api.calls.filter(c => c.kind === 'route').map(c => c.name);
	const text = lib.textOf(holder);
	check('连接页：CLI 只剩拨号设置一条（会话面板已走路由）',
		sameJson(at, [ 'advanced connection-settings' ]), JSON.stringify(at));
	check('连接页：会话面板读 network.session', sameJson(routes, [ 'network.session' ]), JSON.stringify(routes));
	check('连接页：地址 / 网关 / DNS / MTU / 会话行来自路由载荷',
		text.indexOf('10.6.172.152') !== -1 && text.indexOf('223.5.5.5') !== -1
		&& text.indexOf('2408:8207::1') !== -1 && text.indexOf('IPv4 / IPv6 · same APN') !== -1
		&& text.indexOf('1500') !== -1 && text.indexOf('CID 1 · cmnet') !== -1);
	check('连接页：固件计数（十六进制→十进制后由页面按 1024 进制显示）',
		text.indexOf('303.7 KiB') !== -1 && text.indexOf('143.1 MiB') !== -1 && text.indexOf('1min') !== -1);
	/* 写路径：「清空计数」确认后走 network.flow_clear */
	api.calls.length = 0;
	lib.pressButton(holder, 'Clear counters');
	lib.modalButton(side.scope, 'Apply');
	for (let i = 0; i < 4; i++) await lib.tick();
	const write = api.calls.filter(c => c.kind === 'routeCall').map(c => c.name);
	check('连接页：清空计数走 network.flow_clear 路由（零 CLI）',
		sameJson(write, [ 'network.flow_clear' ]) && api.calls.every(c => c.kind !== 'at'), JSON.stringify(api.calls));
	check('连接页：确认后 900 ms 重载页面', side.scope.window.pending.some(p => p.delay === 900));
}

/* ------------------------------------------------------------ 系统页 */
async function systemPage() {
	const api = lib.makeApi({
		'at:system': { stdout: fixtures.systemCliFrame(fixtures.systemFacts({})), stderr: '' }
	});
	api.atSystem = () => api.at([ 'system' ]);
	const side = lib.loadSide(read, api, RES + '/view/mt5700m/system.js');
	side.view.load();
	const holder = side.view.render();
	await side.view.contentReady;
	for (let i = 0; i < 20; i++) await lib.tick();
	const text = lib.textOf(holder);
	const at = api.calls.filter((c) => c.kind === 'at').map((c) => c.args.join(' '));
	check('系统页：22 段读帧仍是 mt5700m-at system（下一刀才切）',
		sameJson(at, [ 'system' ]), JSON.stringify(at));
	/*
	 * 不断言 `_('%d%% complete')` 的字面形式：luci-stub 的 format 是
	 * `replace(/%[sd]/g)`，不处理 `%%` 转义，会得到 `50%% complete`。
	 * 真实 LuCI 是否转义待核（与本次迁移无关），因此这里只钉住不依赖该
	 * 转义的读数。
	 */
	check('系统页：盘面读数来自读帧（IMEI / ICCID / 峰值温度 / 运营商 / 固件版本）',
		text.indexOf('862853030012345') !== -1 && text.indexOf('89860312345678901234') !== -1
		&& text.indexOf('45.1') !== -1 && text.indexOf('CHN-UNICOM') !== -1
		&& text.indexOf('MT5700M-2.5.0') !== -1);
	/* 四条已迁完成功的写入：按按钮 → 确认 → 只应发出一条 routeCall */
	const fire = async function (buttonText, applyText, prepare) {
		api.calls.length = 0;
		lib.pressButton(holder, buttonText);
		if (prepare) prepare(lib.lastModal(side.scope).children);
		lib.modalButton(side.scope, applyText);
		for (let i = 0; i < 4; i++) await lib.tick();
		const calls = api.calls.filter((c) => c.kind === 'routeCall');
		return { name: calls.length === 1 ? calls[0].name : null,
			pure: api.calls.length > 0 && api.calls.every((c) => c.kind === 'routeCall'),
			calls: api.calls };
	};
	const cases = [
		[ 'Enter airplane mode', 'Continue', null, 'network.radio_set', '飞行模式切换' ],
		[ 'Switch SIM slot', 'Apply', null, 'sim.slot_set', '切换卡槽' ],
		[ 'Restart Module', 'Continue', null, 'modem.reset', '重启模块' ],
		[ 'Restore factory settings', 'Restore factory settings',
			(nodes) => { lib.inputsIn(nodes)[0].value = 'RESET'; },
			'system.factory_reset', '工厂复位' ]
	];
	for (const c of cases) {
		const r = await fire(c[0], c[1], c[2]);
		check('系统页：' + c[4] + '走 ' + c[3] + '（零 CLI）',
			r.name === c[3] && r.pure, JSON.stringify(r.calls));
	}
}

(async function () {
	if (!fs.existsSync(path.join(ROOT, RES))) {
		console.error('压缩树不存在：' + ROOT + '\n先跑 bash scripts/minify-luci-frontend.sh ' + ROOT);
		process.exit(1);
	}
	console.log('压缩树：' + ROOT);
	await networkPage();
	await smsPage();
	await statusPage();
	await connectionPage();
	await systemPage();
	console.log(failures === 0 ? '\nSMOKE OK：压缩后的无线页 / 短信页 / 概览页 / 连接页 / 系统页照常读路由、照常发送' : '\nSMOKE FAIL：' + failures + ' 项');
	process.exit(failures === 0 ? 0 : 1);
})().catch(function (err) {
	console.error('渲染失败：' + (err && err.stack || err));
	process.exit(1);
});
