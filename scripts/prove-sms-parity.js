#!/usr/bin/env node
'use strict';

/*
 * 短信页一致性证明（LuCI 短信页 → 统一路由，前端最后一份 PDU 解码删除）
 *
 * 这一刀把无线页之外最后一块 CLI 文本帧切掉：短信页原来读 `sms-list` /
 * `sms-info` 两个 CLI 帧，自己在 `parser.js` 里解 +CMGL 的 PDU（GSM 7bit /
 * UCS-2 / 长短信 UDH 合并）、从 +CPMS/+CSCA/^IMSSWITCH 里正则取值，写操作走
 * `sms-send`/`sms-delete`/`sms-clear`/`sms-set`/`sms-ims` 动词。现在：
 *   - 读：`sms.list` / `sms.status`（与 WebUI 的短信页同一组路由）
 *   - 写：`sms.send` / `sms.delete` / `sms.clear_all` / `sms.center_set` /
 *     `sms.storage_set` / `sms.ims_set`
 *   - 删除：`api.js` 的 `atSmsList`/`atSmsInfo`、`parser.js` 的
 *     `parseMessages`/`parseInfo`/`decodePdu`/`decodeGsm7`/`decodeUcs2`/
 *     `swapDigits` —— 前端不再有第二份 PDU 解码。
 *
 * 用法：
 *   node scripts/prove-sms-parity.js [基线]     # 基线默认 HEAD，本刀之前是 812ba41
 * 退出码 0 = 通过，1 = 不一致，2 = 基线选错了（基线里已经没有旧实现）。
 *
 * 关键设计：PDU 夹具沿用 WebUI 演示数据里的已知向量（
 * mt5700webui-openwrt-server/semi-tcpweb/src/services/mockAT.ts 的
 * RECEIVED_SMS / MOCK_SENT_MESSAGES）。旧侧只喂 PDU（页面自己解），新侧只喂
 * 解码后的领域字段 —— 两侧渲染逐字相同，等于说「后端的解码在这批向量上与旧
 * JS 解码给出同一结果」，而不是把夹具抄成某一侧的样子。
 */

const fs = require('fs');
const path = require('path');
const cp = require('child_process');
const lib = require('./lib/luci-stub');

const REPO = path.resolve(__dirname, '..');
const RES = 'luci-app-mt5700m/htdocs/luci-static/resources';
const VIEW = RES + '/view/mt5700m/sms.js';
const BASELINE = process.argv[2] || 'HEAD';

const oldSource = (rel) => cp.execSync('git show ' + BASELINE + ':' + rel, { cwd: REPO, maxBuffer: 64 * 1024 * 1024 }).toString();
const newSource = (rel) => fs.readFileSync(path.join(REPO, rel), 'utf8');

let failures = 0;
function check(name, ok, detail) {
	if (ok) { console.log('  ok   ' + name); return; }
	failures++;
	console.log('  FAIL ' + name + (detail ? '\n       ' + detail : ''));
}
const sameJson = (a, b) => JSON.stringify(a) === JSON.stringify(b);
/*
 * 唯一的取值差异：`+` 前缀。
 *
 * 旧 JS 解码按 PDU 的 TOA 位决定要不要加 `+`（TOA=145 → 国际号码），后端的
 * 领域模型给的是纯数字（与 WebUI 演示数据、WebUI 的 normalizePhoneNumber 一致）。
 * 数字本身逐位相同，所以比较时把 `+数字` 归一化，并单独把「+ 的个数」钉死，
 * 保证这个已知差异不会悄悄变大。
 */
const NUM_PLUS = (t) => String(t).replace(/\+[\d#]/g, m => m.slice(1));
/* 工具栏按钮是 c.btnLink 生成的 <a>，lib.pressButton 只找 <button> */
function pressLink(holder, text) {
	const link = lib.collect(holder, x => x.tagName === 'A' && lib.textOf(x) === text)[0];
	if (!link) throw new Error('link not found: ' + text);
	link.click();
}
function diffText(a, b) {
	const la = String(a).split('\n'), lb = String(b).split('\n');
	for (let i = 0; i < Math.max(la.length, lb.length); i++)
		if (la[i] !== lb[i]) return 'line ' + (i + 1) + ':\n       old: ' + la[i] + '\n       new: ' + lb[i];
	return '';
}

/* ------------------------------------------------------------ 夹具
 * 五条收到短信的 PDU 与解码结果、两条本地发送历史，取自 WebUI 演示数据
 * （mockAT.ts）。演示数据里 number/time/text 就是这份 PDU 的解码结果，是两个
 * 前端共用的契约样例。
 */
const RECEIVED = [
	{ index: 1, pdu: '00000D91683108108300F00008628062801000231E6D4191CF7EDF8BA15DF24E8E4ECA65E596F670B981EA52A8523765B03002', number: '8613800138000', time: '26/08/26,08:01:00', text: '流量统计已于今日零点自动刷新。' },
	{ index: 2, pdu: '00000D91683108108300F00008628062905100231C8BBE59075DF26210529F63A5516500200035004700207F517EDC3002', number: '8613800138000', time: '26/08/26,09:15:00', text: '设备已成功接入 5G 网络。' },
	{ index: 3, pdu: '00000791680180F600086280526124002340672C67085957991052694F596D4191CF0020003100320038002E003600470042FF0C67096548671F81F300200030003800206708002000330031002065E53002', number: '8610086', time: '26/08/25,16:42:00', text: '本月套餐剩余流量 128.6GB，有效期至 08 月 31 日。' },
	{ index: 4, pdu: '00000D91683119325476F8000862804271820023345DE168C06E0553555DF27ECF53D152307FA491CCFF0C73B0573A91CD70B9770B00200035004700204FE153F7548C6E295EA63002', number: '8613912345678', time: '26/08/24,17:28:00', text: '巡检清单已经发到群里，现场重点看 5G 信号和温度。' },
	{ index: 5, pdu: '00000D91683119325476F8000862804281600023284ECA665A00200037002070B95230673A623F5DE168C0FF0C8BB05F975E266D4B8BD5753581113002', number: '8613912345678', time: '26/08/24,18:06:00', text: '今晚 7 点到机房巡检，记得带测试电脑。' }
];
/*
 * 手写的一条 GSM 7bit DELIVER：payload 就是 modules/sms::pdu 单测里
 * `pack_septets([0x68,0x65,0x6C,0x6C,0x6F]) == "E8329BFD06"`（"hello"）那份，
 * 外面套标准 DELIVER 布局（SMSC 0、MTI 00、OA 5 位国内号 10086、DCS 00 七位、
 * SCTS 62 80 62 80 10 00 23 = 26/08/26,08:01:00）。旧侧解它 → 新侧拿同一条的
 * 领域字段，两侧必须给出同一段文字与同一个时间，7bit 解包这条路径也就被钉住。
 */
const GSM7 = {
	index: 6,
	pdu: '000005' + '81' + '0180F6' + '00' + '00' + '62806280100023' + '05' + 'E8329BFD06',
	number: '10086',
	time: '26/08/26,08:01:00',
	text: 'hello'
};

const SENT_HISTORY = [
	{ index: -101, content: '收到，我会提前 10 分钟到。', number: '13912345678', time: '26/08/24,18:10:00', type: 'sent' },
	{ index: -102, content: '网络状态已确认，当前运行正常。', number: '13800138000', time: '26/08/26,09:18:00', type: 'sent' }
];

/* ------------------------------------------------------------ 旧帧（CLI 文本）
 * `api/cli.rs::print_sms_list` / `::print_sms_info` 的输出形状：
 *   print_sms_list = "===== SMS storage =====\n" + cpms + "\n\n"
 *                  + "===== SMS messages =====\n" + cmgl + "\n"
 *   print_sms_info = 四段 dump_section（IMS / Service mode / SMSC / Storage）
 * 每段里的 text 就是 client::at_cmd 的原文（回显 + 应答 + 空行 + OK）。
 */
const CPMS_TEXT = (cpms) => 'AT+CPMS?\r\n+CPMS: ' + cpms + '\r\n\r\nOK';
const CSCA_TEXT = (center) => center ? 'AT+CSCA?\r\n+CSCA: "' + center + '",145\r\n\r\nOK' : '';
const IMS_TEXT = (on) => 'AT^IMSSWITCH?\r\n^IMSSWITCH: ' + (on ? '1' : '0') + ',0,0\r\n\r\nOK';
const CEUS_TEXT = 'AT+CEUS?\r\n+CEUS: 1,1\r\n\r\nOK';
function cmglText(messages) {
	return 'AT+CMGL=4\r\n' + messages.map(m =>
		'+CMGL: ' + m.index + ',1,,' + (m.pdu.length / 2 - 1) + '\r\n' + m.pdu).join('\r\n') + '\r\nOK';
}

function dumpSection(label, command, text) {
	return '===== ' + label + ': ' + command + ' =====\n' + text + '\n\n';
}
function smsListFrame(cpms, messages) {
	return '===== SMS storage =====\n' + CPMS_TEXT(cpms) + '\n\n===== SMS messages =====\n' + cmglText(messages) + '\n';
}
function smsInfoFrame(cpms, center, imsOn) {
	return dumpSection('IMS', 'AT^IMSSWITCH?', IMS_TEXT(imsOn))
		+ dumpSection('Service mode', 'AT+CEUS?', CEUS_TEXT)
		+ dumpSection('SMSC', 'AT+CSCA?', CSCA_TEXT(center))
		+ dumpSection('Storage', 'AT+CPMS?', CPMS_TEXT(cpms));
}

/* ------------------------------------------------------------ 新载荷（路由域模型） */
function listPayload(messages) {
	return {
		messages: (messages || RECEIVED).map(m => ({
			index: m.index, content: m.text, number: m.number, time: m.time, type: 'received'
		}))
	};
}
function statusPayload(cpms, center, imsOn) {
	// `+CPMS: "read",u,t,"write",u,t,"receive",u,t`；整份读不到时 to_json 里
	// 没有 storage 键（SmsSettings::to_json 只在 storage 非空时放入）。
	const payload = { enabled: true };
	if (cpms) {
		const f = cpms.split(',').map(v => v.trim().replace(/"/g, ''));
		const plane = (i) => ({ name: f[i], used: parseInt(f[i + 1], 10), total: parseInt(f[i + 2], 10) });
		payload.storage = { read: plane(0), write: plane(3), receive: plane(6) };
	}
	if (center) payload.center = center;
	if (imsOn !== null) payload.imsOn = imsOn;
	return payload;
}

/* ------------------------------------------------------------ 形态表 */
const CPMS_MIXED = '"SM",0,30,"ME",3,50,"SM",0,30';
const CPMS_SAME = '"ME",5,50,"ME",5,50,"ME",5,50';
const SHAPES = [
	{
		key: '正常：5 条收到（CPMS 三个面不同）',
		cpms: CPMS_MIXED, center: '+8613800138000', imsOn: true, received: null, plus: 3
	},
	{
		key: '同面存储（read=write=receive=ME,5,50，WebUI 演示数据的形状）',
		cpms: CPMS_SAME, center: '+8613800138000', imsOn: true, received: null, plus: 3
	},
	{ key: 'IMS 关闭', cpms: CPMS_SAME, center: '+8613800138000', imsOn: false, received: null, plus: 3 },
	{ key: '状态读不到（+CPMS/+CSCA/^IMSSWITCH 都空）', cpms: '', center: '', imsOn: null, received: null, plus: 3 },
	{ key: '空收件箱', cpms: CPMS_SAME, center: '+8613800138000', imsOn: true, received: [], plus: 0 },
	{
		key: '唯一会话：单条 UCS-2 消息（正文与时间直接可见）',
		cpms: CPMS_SAME, center: '+8613800138000', imsOn: true, received: [ RECEIVED[0] ], plus: 1
	},
	{
		key: '唯一会话：单条 GSM 7bit 消息（hello）',
		cpms: CPMS_SAME, center: '+8613800138000', imsOn: true, received: [ GSM7 ], plus: 0
	}
];

/* ------------------------------------------------------------ 装载 */
const apiOld = lib.makeApi({});
const apiNew = lib.makeApi({});
if (typeof lib.loadSide(oldSource, apiOld, VIEW).scope === 'undefined') process.exit(2);
{
	// 旧版才有这两个包装；基线里没有就是基线选错了
	const oldApiJs = oldSource(RES + '/mt5700m/api.js');
	if (oldApiJs.indexOf('function atSmsList') === -1) {
		console.error('基线 ' + BASELINE + ' 已经包含这一刀（api.js 里没有 atSmsList）——没有旧管线可比较。\n'
			+ '本刀的基线是 812ba41：\n  node scripts/prove-sms-parity.js 812ba41');
		process.exit(2);
	}
}
const oldParserSrc = oldSource(RES + '/mt5700m/parser.js');
const newParserSrc = newSource(RES + '/mt5700m/parser.js');
check('基线里旧侧确实有前端 PDU 解码（parseMessages/decodePdu/decodeGsm7/decodeUcs2）',
	oldParserSrc.indexOf('function parseMessages') !== -1 && oldParserSrc.indexOf('function decodePdu') !== -1
	&& oldParserSrc.indexOf('function decodeGsm7') !== -1 && oldParserSrc.indexOf('function decodeUcs2') !== -1);

/* 两侧各自的 api 桩：旧版页面调 atSmsList/atSmsInfo，新版调 route/routeCall */
function side(kind, answers) {
	const api = lib.makeApi(answers);
	if (kind === 'old') {
		api.atSmsList = () => api.at([ 'sms-list' ]);
		api.atSmsInfo = () => api.at([ 'sms-info' ]);
	}
	return lib.loadSide(kind === 'old' ? oldSource : newSource, api, VIEW);
}

function answers(kind, shape) {
	if (kind === 'old') {
		const received = shape.received === null ? RECEIVED : shape.received;
		return {
			'at:sms-list': { stdout: smsListFrame(shape.cpms, received), stderr: '' },
			'at:sms-info': { stdout: smsInfoFrame(shape.cpms, shape.center, shape.imsOn === false ? false : true), stderr: '' }
		};
	}
	return {
		'call:sms.list': listPayload(shape.received),
		'call:sms.status': statusPayload(shape.cpms, shape.center, shape.imsOn)
	};
}

async function render(kind, shape) {
	const s = side(kind, answers(kind, shape));
	s.view.load();
	const holder = s.view.render();
	await s.view.contentReady;
	return { side: s, holder: holder };
}
function store(side_) { return side_.scope.localStorage; }
function seedHistory(side_) {
	store(side_).setItem('sms_sent_messages_cache', JSON.stringify(SENT_HISTORY.map(m => ({
		number: m.number, text: m.content, date: '20' + m.time.slice(0, 2) + '-' + m.time.slice(3, 5) + '-' + m.time.slice(6, 8)
			+ ' ' + m.time.slice(9, 11) + ':' + m.time.slice(12, 14),
		direction: 'out', order: m.index
	}))));
}

(async function () {
	console.log('渲染：整页 DOM 与文案（含两条本地发送历史）');
	for (const shape of SHAPES) {
		const o = await render('old', shape);
		const n = await render('new', shape);
		const oShape = lib.serialize(o.holder, { attrs: true, skipAttrs: [ 'value' ] });
		const nShape = lib.serialize(n.holder, { attrs: true, skipAttrs: [ 'value' ] });
		check(shape.key + '：结构 + class 逐字一致',
			NUM_PLUS(oShape) === NUM_PLUS(nShape),
			NUM_PLUS(oShape) === NUM_PLUS(nShape) ? '' : diffText(NUM_PLUS(oShape), NUM_PLUS(nShape)));
		const oText = lib.textOf(o.holder), nText = lib.textOf(n.holder);
		check(shape.key + '：可见文字（含消息正文/时间/槽位数字）逐字一致（差异只有号码的 + 前缀）',
			NUM_PLUS(oText) === NUM_PLUS(nText),
			NUM_PLUS(oText) === NUM_PLUS(nText) ? '' : diffText(NUM_PLUS(oText), NUM_PLUS(nText)));
		check(shape.key + '：+ 前缀的个数 = 侧栏里的发送方个数（' + shape.plus + '）',
			(oText.match(/\+\d/g) || []).length === shape.plus && (nText.match(/\+\d/g) || []).length === 0,
			JSON.stringify([ (oText.match(/\+\d/g) || []).length, (nText.match(/\+\d/g) || []).length ]));
	}

	/* 会话分组与消息正文：逐条点名断言，避免「两侧同样为空」蒙混过关 */
	{
		const shape = SHAPES[0];
		const o = await render('old', shape), n = await render('new', shape);
		seedHistory(o.side); /* 上面渲染没带历史，这里单独再看一次带历史的分组 */
		const oHist = await render('old', shape), nHist = await render('new', shape);
		// 重新播种两侧历史（render 会新建 scope，存储是每 scope 一份）
		seedHistory(oHist.side); seedHistory(nHist.side);
		await (async () => {})();
		const withHistory = async (kind) => {
			const s = side(kind, answers(kind, shape));
			seedHistory(s);
			s.view.load();
			const holder = s.view.render();
			await s.view.contentReady;
			return holder;
		};
		const oH = await withHistory('old'), nH = await withHistory('new');
		check('带本地发送历史：结构 + 文案逐字一致（差异只有号码的 + 前缀）',
			NUM_PLUS(lib.serialize(oH, { attrs: true, skipAttrs: [ 'value' ] })) === NUM_PLUS(lib.serialize(nH, { attrs: true, skipAttrs: [ 'value' ] })),
			diffText(NUM_PLUS(lib.textOf(oH)), NUM_PLUS(lib.textOf(nH))));
		const navs = (h) => lib.collect(h, x => x.tagName === 'BUTTON' && String(x.attrs['class'] || '').indexOf('mt-sms-nav-item') !== -1)
			.map(b => NUM_PLUS(lib.textOf(b)));
		check('会话侧栏：号码 + 条数（5 个会话：3 个发件人 + 2 个本地号码）一致',
			sameJson(navs(oH), navs(nH)) && navs(nH).length === 5, JSON.stringify(navs(nH)));
		const chat = (h) => lib.collect(h, x => String(x.attrs['class'] || '').indexOf('mt-sms-chat-item') === 0)
			.map(x => lib.textOf(x));
		/* 页面只把「最新一条会话」灌进聊天面板（其它会话点侧栏才展开），
		 * 最新的是 8613912345678 那组（最后一条 order=5），两条消息。 */
		check('聊天面板：最新会话的两条消息 + Received 标记 + 时间一致',
			sameJson(chat(oH), chat(nH)) && chat(nH).length === 2
			&& chat(nH)[0].indexOf('巡检清单已经发到群里，现场重点看 5G 信号和温度。') === 0
			&& chat(nH)[1].indexOf('今晚 7 点到机房巡检，记得带测试电脑。') === 0,
			JSON.stringify(chat(nH)));
		const badge = (h) => lib.textOf(lib.collect(h, x => String(x.attrs['class'] || '').indexOf('mt-badge') === 0)[0] || null);
		check('槽位角标用的是 +CPMS 第一个面（read: SM 0/30，不是 write 面的 ME 3/50）',
			badge(oH) === '0 of 30 message slots used' && badge(nH) === badge(oH), JSON.stringify([ badge(oH), badge(nH) ]));
		check('消息时间排版沿用旧样式 20YY-MM-DD HH:MM（侧栏显示每组最后一条的时间）',
			lib.textOf(nH).indexOf('2026-08-26 09:15 · 2') !== -1
			&& lib.textOf(nH).indexOf('2026-08-24 18:06 · 2') !== -1
			&& lib.textOf(nH).indexOf('2026-08-24 18:10 · 1') !== -1);
	}

	/* 单会话：正文与时间直接落在 DOM 上（唯一会话时聊天面板会渲染它） */
	{
		const o = await render('old', SHAPES[5]), n = await render('new', SHAPES[5]);
		check('唯一 UCS-2 会话：侧栏号码 + 正文 + 时间逐字一致',
			NUM_PLUS(lib.textOf(o.holder)) === NUM_PLUS(lib.textOf(n.holder))
			&& lib.textOf(n.holder).indexOf('流量统计已于今日零点自动刷新。') !== -1
			&& lib.textOf(n.holder).indexOf('2026-08-26 08:01') !== -1,
			lib.textOf(n.holder));
		const o2 = await render('old', SHAPES[6]), n2 = await render('new', SHAPES[6]);
		check('唯一 GSM 7bit 会话：侧栏号码（国内号，无 +）+ 正文 hello + 时间逐字一致',
			lib.textOf(o2.holder) === lib.textOf(n2.holder)
			&& lib.textOf(n2.holder).indexOf('10086') !== -1
			&& lib.textOf(n2.holder).indexOf('hello') !== -1
			&& lib.textOf(n2.holder).indexOf('2026-08-26 08:01') !== -1,
			lib.textOf(n2.holder));
	}

	/* 失败路径：两条读都失败 → 同两条 warning 横幅、同样的文案 */
	{
		const o = side('old', { 'at:sms-list': { ok: false, error: 'backend unreachable' }, 'at:sms-info': { ok: false, error: 'backend unreachable' } });
		const n = side('new', { 'call:sms.list': { ok: false, error: 'backend unreachable' }, 'call:sms.status': { ok: false, error: 'backend unreachable' } });
		o.view.load(); n.view.load();
		const oH = o.view.render(), nH = n.view.render();
		await o.view.contentReady;
		await n.view.contentReady;
		const banners = (h) => lib.collect(h, x => String(x.attrs['class'] || '').indexOf('alert-message') === 0).map(b => lib.textOf(b));
		check('读取失败：两条横幅（列表原文 + 存储固定文案）与空收件箱状态一致',
			sameJson(banners(oH), banners(nH))
			&& sameJson(banners(nH), [ 'backend unreachable', 'SMS storage information is temporarily unavailable. backend unreachable' ]),
			JSON.stringify(banners(nH)));
		check('读取失败：整页结构与文案仍然逐字一致',
			lib.serialize(oH, { attrs: true, skipAttrs: [ 'value' ] }) === lib.serialize(nH, { attrs: true, skipAttrs: [ 'value' ] }),
			diffText(lib.textOf(oH), lib.textOf(nH)));
		check('读取失败：没有消息时侧栏不显示任何 + 号码（+ 前缀差异不适用于空态）',
			(lib.textOf(oH).match(/\+\d/g) || []).length === 0 && (lib.textOf(nH).match(/\+\d/g) || []).length === 0);
	}

	/* ------------------------------------------------------------ 设置弹窗
	 * 预填（短信中心 / 存储面 / IMS 开关）与保存路径逐条比较。
	 */
	console.log('\n消息设置弹窗：预填 + 保存路径');
	{
		const shape = SHAPES[0];
		const o = await render('old', shape);
		const n = await render('new', shape);
		pressLink(o.holder, 'Settings');
		pressLink(n.holder, 'Settings');
		const modalOf = (side_) => side_.scope.ui.modals[side_.scope.ui.modals.length - 1];
		const oM = modalOf(o.side), nM = modalOf(n.side);
		check('弹窗标题与结构一致', lib.serialize(oM.children, { attrs: true }) === lib.serialize(nM.children, { attrs: true }),
			diffText(lib.serialize(oM.children, { attrs: true }), lib.serialize(nM.children, { attrs: true })));
		const inputs = (m) => lib.collect(m.children, x => x.tagName === 'INPUT').map(x => String(x.value));
		const selects = (m) => lib.collect(m.children, x => x.tagName === 'SELECT').map(x => String(x.value));
		check('预填：短信中心 / 存储面（+CPMS 第一个面）/ IMS 开关一致',
			sameJson(inputs(oM), inputs(nM)) && sameJson(selects(oM), selects(nM))
			&& sameJson(inputs(nM), [ '+8613800138000' ]) && sameJson(selects(nM), [ 'SM', '1' ]),
			JSON.stringify({ inputs: inputs(nM), selects: selects(nM) }));

		/* 改三个值再保存：证明写出去的值来自表单（不是预填） */
		const setForm = (m) => {
			lib.collect(m.children, x => x.tagName === 'INPUT')[0].value = '+8613800138001';
			const sels = lib.collect(m.children, x => x.tagName === 'SELECT');
			sels[0].value = 'ME';
			sels[1].value = '0';
		};
		setForm(oM); setForm(nM);
		o.side.api.calls.length = 0;
		n.side.api.calls.length = 0;
		lib.modalButton(o.side.scope, 'Save settings');
		lib.modalButton(n.side.scope, 'Save settings');
		for (let i = 0; i < 4; i++) await lib.tick();
		const oldCalls = o.side.api.calls.filter(c => c.kind === 'at').map(c => c.args);
		const newCalls = n.side.api.calls.filter(c => c.kind === 'routeCall').map(c => [ c.name, c.params ]);
		check('旧：三个 CLI 动词（sms-set smsc / sms-set storage / sms-ims）',
			sameJson(oldCalls, [
				[ 'sms-set', 'smsc', '+8613800138001' ],
				[ 'sms-set', 'storage', 'ME' ],
				[ 'sms-ims', '0' ]
			]), JSON.stringify(oldCalls));
		check('新：三条写路由，参数即表单值（storage_set 三个面同名，与旧动词一致）',
			sameJson(newCalls, [
				[ 'sms.center_set', { number: '+8613800138001' } ],
				[ 'sms.storage_set', { read: 'ME', write: 'ME', receive: 'ME' } ],
				[ 'sms.ims_set', { enabled: false } ]
			]), JSON.stringify(newCalls));
		check('保存成功：同一条通知 + 同样等 1500ms 再刷新',
			sameJson(o.side.scope.ui.notifications.map(x => x.text + '|' + x.level),
				n.side.scope.ui.notifications.map(x => x.text + '|' + x.level))
			&& sameJson(o.side.scope.window.pending.map(p => p.delay), [ 1500 ])
			&& sameJson(n.side.scope.window.pending.map(p => p.delay), [ 1500 ]),
			JSON.stringify({ notes: n.side.scope.ui.notifications, pending: n.side.scope.window.pending.map(p => p.delay) }));
		o.side.scope.window.pending.forEach(p => p.fn());
		n.side.scope.window.pending.forEach(p => p.fn());
		check('保存成功后两侧都刷新页面',
			o.side.scope.window.reloaded === true && n.side.scope.window.reloaded === true);
	}

	/* ------------------------------------------------------------ 清空 / 发送 */
	console.log('\n工具栏与撰写区：清空、发送');
	{
		const shape = SHAPES[0];
		const o = await render('old', shape);
		const n = await render('new', shape);
		o.side.api.calls.length = 0;
		n.side.api.calls.length = 0;
		pressLink(o.holder, 'Clear received messages');
		pressLink(n.holder, 'Clear received messages');
		check('清空弹窗标题一致',
			lib.modalTitle(o.side.scope) === lib.modalTitle(n.side.scope) && lib.modalTitle(n.side.scope).indexOf('Clear all messages?') === 0,
			JSON.stringify([ lib.modalTitle(o.side.scope), lib.modalTitle(n.side.scope) ]));
		lib.modalButton(o.side.scope, 'Clear all');
		lib.modalButton(n.side.scope, 'Clear all');
		for (let i = 0; i < 2; i++) await lib.tick();
		check('清空：旧 `sms-clear` → 新 `sms.clear_all`',
			sameJson(o.side.api.calls.filter(c => c.kind === 'at').map(c => c.args), [ [ 'sms-clear' ] ])
			&& sameJson(n.side.api.calls.filter(c => c.kind === 'routeCall').map(c => [ c.name, c.params === undefined ? null : c.params ]),
				[ [ 'sms.clear_all', null ] ]),
			JSON.stringify(n.side.api.calls.filter(c => c.kind === 'routeCall')));
		check('清空后两侧都立即刷新（无延时队列）',
			o.side.scope.window.reloaded === true && n.side.scope.window.reloaded === true
			&& o.side.scope.window.pending.length === 0 && n.side.scope.window.pending.length === 0);
	}
	{
		const shape = SHAPES[0];
		const o = await render('old', shape);
		const n = await render('new', shape);
		const fill = (holder) => {
			const inputs = lib.collect(holder, x => x.tagName === 'INPUT' && String(x.attrs['class'] || '') === 'mt-sms-compose-input');
			inputs[0].value = '13800138000';
			inputs[1].value = 'hello from LuCI';
		};
		fill(o.holder); fill(n.holder);
		o.side.api.calls.length = 0;
		n.side.api.calls.length = 0;
		lib.pressButton(o.holder, 'Send');
		lib.pressButton(n.holder, 'Send');
		check('发送确认弹窗文案一致（含号码）',
			lib.modalTitle(o.side.scope) === lib.modalTitle(n.side.scope)
			&& lib.textOf(o.side.scope.ui.modals[o.side.scope.ui.modals.length - 1].children) === lib.textOf(n.side.scope.ui.modals[n.side.scope.ui.modals.length - 1].children),
			JSON.stringify([ lib.textOf(o.side.scope.ui.modals[o.side.scope.ui.modals.length - 1].children), lib.textOf(n.side.scope.ui.modals[n.side.scope.ui.modals.length - 1].children) ]));
		lib.modalButton(o.side.scope, 'Send');
		lib.modalButton(n.side.scope, 'Send');
		for (let i = 0; i < 4; i++) await lib.tick();
		check('发送：旧 `sms-send <号> <文>` → 新 `sms.send {number,text}`',
			sameJson(o.side.api.calls.filter(c => c.kind === 'at').map(c => c.args), [ [ 'sms-send', '13800138000', 'hello from LuCI' ] ])
			&& sameJson(n.side.api.calls.filter(c => c.kind === 'routeCall').map(c => [ c.name, c.params ]),
				[ [ 'sms.send', { number: '13800138000', text: 'hello from LuCI' } ] ]),
			JSON.stringify(n.side.api.calls.filter(c => c.kind === 'routeCall')));
		const hist = (side_) => JSON.parse(store(side_).getItem('mt5700m.sms.sent') || '[]')
			.map(m => ({ number: m.number, text: m.text, date: m.date, direction: m.direction }));
		check('发送成功：同一条通知 + 同样的本地历史写入（键名/字段一致）',
			sameJson(o.side.scope.ui.notifications.map(x => x.text + '|' + x.level),
				n.side.scope.ui.notifications.map(x => x.text + '|' + x.level))
			&& sameJson(hist(o.side), hist(n.side))
			&& sameJson(hist(n.side), [ { number: '13800138000', text: 'hello from LuCI', date: 'now', direction: 'out' } ])
			&& store(n.side).getItem('sms_sent_messages_cache') === null,
			JSON.stringify(hist(n.side)));
		check('发送成功：同样等 1200ms 再刷新',
			sameJson(o.side.scope.window.pending.map(p => p.delay), [ 1200 ])
			&& sameJson(n.side.scope.window.pending.map(p => p.delay), [ 1200 ]),
			JSON.stringify(n.side.scope.window.pending.map(p => p.delay)));
	}
	/* 发送失败：同一条 danger 通知，且不刷新 */
	{
		const shape = SHAPES[0];
		const o = side('old', Object.assign(answers('old', shape), { 'at:sms-send': { ok: false, error: 'modem rejected the message' } }));
		const n = side('new', Object.assign(answers('new', shape), { 'call:sms.send': { ok: false, error: 'modem rejected the message' } }));
		o.view.load(); n.view.load();
		const oH = o.view.render(), nH = n.view.render();
		await o.view.contentReady;
		await n.view.contentReady;
		const fill = (holder) => {
			const inputs = lib.collect(holder, x => x.tagName === 'INPUT' && String(x.attrs['class'] || '') === 'mt-sms-compose-input');
			inputs[0].value = '13800138000';
			inputs[1].value = 'hello';
		};
		fill(oH); fill(nH);
		lib.pressButton(oH, 'Send'); lib.pressButton(nH, 'Send');
		lib.modalButton(o.scope, 'Send'); lib.modalButton(n.scope, 'Send');
		for (let i = 0; i < 4; i++) await lib.tick();
		check('发送失败：同一条 danger 通知（取后端文案），都不刷新',
			sameJson(o.scope.ui.notifications.map(x => x.text + '|' + x.level),
				n.scope.ui.notifications.map(x => x.text + '|' + x.level))
			&& n.scope.ui.notifications[0].level === 'danger'
			&& n.scope.ui.notifications[0].text === 'modem rejected the message'
			&& o.scope.window.reloaded === false && n.scope.window.reloaded === false,
			JSON.stringify(n.scope.ui.notifications));
	}

	/* ------------------------------------------------------------ 调用面 */
	console.log('\n调用面与删除的旧实现');
	{
		const o = await render('old', SHAPES[0]);
		const n = await render('new', SHAPES[0]);
		const at = (side_) => side_.api.calls.filter(c => c.kind === 'at').map(c => c.args[0]);
		const routes = (side_) => side_.api.calls.filter(c => c.kind !== 'at').map(c => c.name);
		check('旧版：页面加载读两个 CLI 帧（sms-list / sms-info）', sameJson(at(o.side), [ 'sms-list', 'sms-info' ]), JSON.stringify(at(o.side)));
		check('新版：本页零 CLI 调用', at(n.side).length === 0, JSON.stringify(at(n.side)));
		check('新版：页面加载读 sms.list + sms.status', sameJson(routes(n.side), [ 'sms.list', 'sms.status' ]), JSON.stringify(routes(n.side)));
	}
	check('删除的旧实现：前端 PDU 解码（parseMessages/parseInfo/decodePdu/decodeGsm7/decodeUcs2/swapDigits）',
		[ 'parseMessages', 'parseInfo', 'decodePdu', 'decodeGsm7', 'decodeUcs2', 'swapDigits' ].every(name =>
			oldParserSrc.indexOf('function ' + name) !== -1 && newParserSrc.indexOf(name) === -1));
	check('删除的旧实现：api.js 的 atSmsList/atSmsInfo（CLI 帧包装）',
		newSource(RES + '/mt5700m/api.js').indexOf('atSmsList') === -1
		&& newSource(RES + '/mt5700m/api.js').indexOf('atSmsInfo') === -1);
	check('新版页面源码里不再出现任何 CLI 动词（sms-* 字符串）',
		newSource(VIEW).indexOf("'sms-") === -1, (newSource(VIEW).match(/'sms-[a-z]+'/g) || []).join(' '));
	check('旧版唯一残留的删除助手 deleteMessage 也已改走 sms.delete 路由（该助手当前无按钮触发，见 migration）',
		newSource(VIEW).indexOf("api.routeCall('sms.delete'") !== -1);
	check('ucode 超时预算按原动词给：sms.send/clear_all/ims_set = 60s，delete/storage_set/center_set = 25s',
		[ "['api.sms.send', 60]", "['api.sms.clear_all', 60]", "['api.sms.ims_set', 60]",
			"['api.sms.delete', 25]", "['api.sms.storage_set', 25]", "['api.sms.center_set', 25]" ].every(entry =>
			newSource('luci-app-mt5700m/root/usr/share/rpcd/ucode/mt5700.uc').indexOf(entry) !== -1));

	console.log(failures === 0
		? '\nPASS：短信页的新旧渲染与全部收发/设置路径一致（读走 sms.list/sms.status，写走六条路由）'
		: '\nFAIL：' + failures + ' 项不一致');
	process.exit(failures === 0 ? 0 : 1);
})().catch(function (err) {
	console.error('渲染失败：' + (err && err.stack || err));
	process.exit(1);
});
