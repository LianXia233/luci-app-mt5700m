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
 * 覆盖目前零 CLI 调用的两个页面（无线页、短信页）：断言路由调用面、页面上的
 * 关键文字（仪表盘读数、无线偏好下拉、会话侧栏）与一条写路径（短信发送）。
 * 退出码 0 = 通过。
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

(async function () {
	if (!fs.existsSync(path.join(ROOT, RES))) {
		console.error('压缩树不存在：' + ROOT + '\n先跑 bash scripts/minify-luci-frontend.sh ' + ROOT);
		process.exit(1);
	}
	console.log('压缩树：' + ROOT);
	await networkPage();
	await smsPage();
	console.log(failures === 0 ? '\nSMOKE OK：压缩后的无线页与短信页照常读路由、照常发送' : '\nSMOKE FAIL：' + failures + ' 项');
	process.exit(failures === 0 ? 0 : 1);
})().catch(function (err) {
	console.error('渲染失败：' + (err && err.stack || err));
	process.exit(1);
});
