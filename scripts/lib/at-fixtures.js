'use strict';

/*
 * 迁移期一致性证明的公共 AT 桩（scripts/lib/at-fixtures.js）
 *
 * 这些 prove-*.js 脚本都要做同一件事：把「同一份调制解调器读数」写成两种形态 ——
 * 旧前端消费的 CLI 文本帧，和新前端消费的路由载荷 —— 然后比较两边的渲染。
 * mini 解码器（按 modules/<module>/parser.rs 的规则把 AT 应答解成领域值）与帧构造器
 * 因此集中在这里一份，避免每把刀各抄一遍。
 *
 * 这些解码器只是**测试夹具**：真正的解码在 Rust 里，脚本里的钉子（每个
 * prove-*.js 开头，样本取自对应的 Rust 单测）负责保证夹具与后端一致。
 */

/* ------------------------------------------------------- mini 解码器
 * 按后端 parser 的规则把同一份 AT 应答解成路由载荷。只实现测试用得到的
 * 分支，但字段索引/进制/换算与 Rust 逐条对齐，并由下面的钉子钉住。
 */
const round1 = (v) => Math.round(v * 10) / 10;
const ord = (v, radix) => {
	const m = String(v === undefined ? '' : v).trim();
	if (!/^[0-9a-fA-F]+$/.test(m)) return undefined;
	return parseInt(m, radix);
};

/* modules/signal/parser.rs：^HCSQ 索引 → dBm/dB（各 RAT 字段顺序不同） */
function decodeSignal(raw) {
	const line = (raw || '').split('\n').find(l => l.trim().indexOf('^HCSQ:') === 0) || '';
	const f = line.trim().slice('^HCSQ:'.length).split(',').map(s => s.trim().replace(/"/g, ''));
	const valid = (v) => /^\d+$/.test(v || '') && v !== '255';
	const n = (v) => parseInt(v, 10);
	const st = { sysmode: f[0] || '' };
	if (f[0] === 'NR') {
		if (valid(f[1])) st.rsrp = n(f[1]) >= 97 ? -44 : n(f[1]) - 141;
		if (valid(f[2])) st.sinr = n(f[2]) >= 251 ? 30.0 : round1(-20.2 + n(f[2]) * 0.2);
		if (valid(f[3])) st.rsrq = n(f[3]) >= 34 ? -3.0 : round1(-20.0 + n(f[3]) * 0.5);
	} else if (f[0] === 'LTE') {
		if (valid(f[1])) st.rssi = n(f[1]) - 121;
		if (valid(f[2])) st.rsrp = n(f[2]) >= 97 ? -44 : n(f[2]) - 141;
		if (valid(f[3])) st.sinr = n(f[3]) >= 251 ? 30.0 : round1(-20.2 + n(f[3]) * 0.2);
		if (valid(f[4])) st.rsrq = n(f[4]) >= 34 ? -3.0 : round1(-20.0 + n(f[4]) * 0.5);
	} else if (f[0] === 'WCDMA') {
		if (valid(f[1])) st.rssi = n(f[1]) - 121;
		if (valid(f[2])) st.rscp = n(f[2]) >= 96 ? -25 : n(f[2]) - 121;
		if (valid(f[3])) st.ecio = round1(-32.5 + n(f[3]) * 0.5);
	} else if (f[0] === 'GSM') {
		if (valid(f[1])) st.rssi = n(f[1]) - 121;
	}
	return st;
}

/* modules/cell/parser.rs::parse_monsc：LTE cid/pci/lac 十六进制；NR 多一个 scs 码 */
function decodeCell(raw) {
	const line = (raw || '').split('\n').find(l => l.trim().indexOf('^MONSC:') === 0) || '';
	const f = line.trim().slice('^MONSC:'.length).split(',').map(s => s.trim());
	const st = { sysmode: f[0] };
	[ 'mcc', 'mnc', 'channel' ].forEach((k, i) => { if (f[i + 1]) st[k] = f[i + 1]; });
	if (f[0] === 'LTE') {
		if (ord(f[4], 16) !== undefined) st.cid = String(ord(f[4], 16));
		if (ord(f[5], 16) !== undefined) st.pci = ord(f[5], 16);
		if (ord(f[6], 16) !== undefined) st.lac = String(ord(f[6], 16));
	} else if (f[0] === 'NR') {
		if (ord(f[4], 10) !== undefined) st.scs = ord(f[4], 10);
		if (ord(f[5], 16) !== undefined) st.cid = String(ord(f[5], 16));
		if (ord(f[6], 16) !== undefined) st.pci = ord(f[6], 16);
		if (ord(f[7], 16) !== undefined) st.lac = String(ord(f[7], 16));
	} else if (f[0] === 'WCDMA') {
		if (ord(f[4], 10) !== undefined) st.pci = ord(f[4], 10);
		if (ord(f[5], 16) !== undefined) st.cid = String(ord(f[5], 16));
		if (ord(f[6], 16) !== undefined) st.lac = String(ord(f[6], 16));
	}
	return st;
}

/* modules/network/parser.rs::parse_registration（CEREG：<n>,<stat>[,"<tac>","<ci>"[,<AcT>]]） */
function decodeRegistration(raw) {
	const line = (raw || '').split('\n').find(l => l.trim().indexOf('+CEREG:') === 0) || '';
	const f = line.trim().slice('+CEREG:'.length).split(',').map(s => s.trim().replace(/"/g, ''));
	const st = {};
	if (f[1] !== undefined) st.state = parseInt(f[1], 10);
	if (f[2]) st.tac = f[2];
	if (f[3]) st.ci = f[3];
	if (f[4]) st.act = f[4];
	return st;
}

/* modules/network/parser.rs::parse_rrcstat：末字段 98/99 是驻留标记，之前一位是状态；
 * 没有标记时末字段就是状态（两字段与三字段两种固件形态都要读对）。 */
function decodeRrc(raw) {
	const line = (raw || '').split('\n').find(l => l.trim().indexOf('^RRCSTAT:') === 0) || '';
	const f = line.trim().slice('^RRCSTAT:'.length).split(',').map(s => s.trim()).filter(s => s !== '');
	const st = {};
	let idx = f.length;
	if (f.length && (f[f.length - 1] === '98' || f[f.length - 1] === '99'))
		st.camped = parseInt(f[--idx], 10);
	if (idx > 0 && /^-?\d+$/.test(f[idx - 1])) st.state = parseInt(f[idx - 1], 10);
	return st;
}

/* modules/network/parser.rs::parse_cops_operator：优先引号里的运营商名 */
function decodeOperator(raw) {
	const line = (raw || '').split('\n').find(l => l.trim().indexOf('+COPS:') === 0) || '';
	const body = line.trim().slice('+COPS:'.length).trim();
	const q = body.indexOf('"'), q2 = body.indexOf('"', q + 1);
	if (q >= 0 && q2 > q) return { operator: body.slice(q + 1, q2) };
	return {};
}

/* modules/network/parser.rs::parse_syscfgex + state.rs::SysCfgState.to_json
 * `^SYSCFGEX: <acqorder>,<band>,<roam>,<srvdomain>,<lteband>,,`
 * 少于五个字段或 acqorder 为空 → 整份配置 None（不是逐字段缺省）；band/lteband
 * 永远是 Some，空串在 to_json 里被省略；roam/srvdomain 非数字 → 省略。 */
function decodeSyscfg(raw) {
	const line = String(raw || '').split('\n').map(l => l.trim()).find(l => l.indexOf('^SYSCFGEX:') === 0);
	if (!line) return {};
	const f = line.slice('^SYSCFGEX:'.length).trim().split(',').map(x => x.trim());
	if (f.length < 5) return {};
	const acqorder = f[0].replace(/"/g, '');
	if (!acqorder) return {};
	const st = { acqorder: acqorder };
	if (f[1]) st.band = f[1];
	if (/^-?\d+$/.test(f[2])) st.roam = parseInt(f[2], 10);
	if (/^-?\d+$/.test(f[3])) st.srvdomain = parseInt(f[3], 10);
	if (f[4]) st.lteband = f[4];
	return st;
}

/* modules/network/parser.rs::parse_c5goption + state.rs::C5gOptionState.to_json
 * `^C5GOPTION: <sa>,<dc>,<gc>` —— 三个字段缺一不可（少于三个就整行跳过），
 * 每个字段是 u8（0..=255），解析失败的那个省略。 */
function decodeC5gOption(raw) {
	for (const line of String(raw || '').split('\n')) {
		const t = line.trim();
		const idx = t.indexOf('^C5GOPTION:');
		if (idx === -1) continue;
		const f = t.slice(idx + '^C5GOPTION:'.length).trim().split(',').map(x => x.trim());
		if (f.length < 3) continue;
		const st = {};
		[ 'nr_sa_support_flag', 'nr_dc_mode', 'gc_access_mode' ].forEach((k, i) => {
			if (/^\d+$/.test(f[i]) && parseInt(f[i], 10) <= 255) st[k] = parseInt(f[i], 10);
		});
		return st;
	}
	return {};
}

/* modules/modem/parser.rs::parse_nrrccap：应答回显 kind（3 = CA，2 = VoNR，5 = DSS），
 * 所以「问哪个能力」只会拿到哪个能力的数字。kind 字段不是整数 → 整条 None；
 * 数字全部不可解析 → 也是 None。
 * modules/modem/state.rs::NrCapabilityState 的 JSON：ca 是布尔（== 1），vonr 是数字，
 * dss 只在两个数字都在时才出现。 */
const NRRCCAP_CA = 3, NRRCCAP_VONR = 2, NRRCCAP_DSS = 5;
function parseNrrccap(raw, kind) {
	for (const line of String(raw || '').split('\n')) {
		const t = line.trim();
		if (t.indexOf('^NRRCCAPQRY:') !== 0) continue;
		const f = t.slice('^NRRCCAPQRY:'.length).split(',').map(x => x.trim());
		if (!/^-?\d+$/.test(f[0] || '')) return null;
		if (parseInt(f[0], 10) !== kind) continue;
		const values = f.slice(1).filter(x => /^-?\d+$/.test(x)).map(x => parseInt(x, 10));
		return values.length ? values : null;
	}
	return null;
}
function decodeNrCapability(parts) {
	const st = {};
	const ca = parseNrrccap((parts || {}).ca, NRRCCAP_CA);
	if (ca) st.ca = ca[0] === 1;
	const vonr = parseNrrccap((parts || {}).vonr, NRRCCAP_VONR);
	if (vonr) st.vonr = vonr[0];
	const dss = parseNrrccap((parts || {}).dss, NRRCCAP_DSS);
	if (dss && dss.length >= 2) st.dss = { rateMatchingLTE: dss[0], additionalDMRS: dss[1] };
	return st;
}

/* modules/system/parser.rs::parse_chiptemp（十分之一度，65535/越界 = 未上报）
 * + state.rs::TemperatureState::peak（>0 且 ≤150 里取最大，取不到就没有） */
const SENSORS = [ 'sub3GPA', 'sub6GPA', 'mimoPa', 'tcxo', 'peri1', 'peri2', 'ap1', 'ap2', 'modem1', 'modem2', 'bbp1', 'bbp2' ];
const SENSOR_KEYS = [ 'sub3g_pa', 'sub6g_pa', 'mimo_pa', 'tcxo', 'peri1', 'peri2', 'ap1', 'ap2', 'modem1', 'modem2', 'bbp1', 'bbp2' ];
function decodeTemps(raw) {
	const line = (raw || '').split('\n').find(l => l.trim().indexOf('^CHIPTEMP:') === 0) || '';
	const f = line.trim().slice('^CHIPTEMP:'.length).split(',').map(s => s.trim());
	const st = {};
	let sum = 0, count = 0;
	SENSORS.forEach((name, i) => {
		const raw_v = /^\d+$/.test(f[i] || '') ? parseInt(f[i], 10) : 0;
		const v = (raw_v >= 65535 || raw_v > 1500) ? 0 : raw_v / 10;
		if (v > 0) { sum += v; count++; }
		st[name] = v;
	});
	if (count) st.average = round1(sum / count);
	SENSORS.forEach((name, i) => {
		const v = st[name];
		if (v > 0 && v <= 150 && (st.peak === undefined || v > st.peak)) { st.peak = v; st.peak_sensor = SENSOR_KEYS[i]; }
	});
	return st;
}
/* modules/system/state.rs::to_text：`temp_*` 逐行 + `temperature=` + `temperature_sensor=` */
function temperatureText(temps) {
	const out = [];
	SENSOR_KEYS.forEach((key, i) => { if (temps[SENSORS[i]] > 0) out.push('temp_' + key + '=' + temps[SENSORS[i]].toFixed(1)); });
	if (temps.peak !== undefined) {
		out.push('temperature=' + temps.peak.toFixed(1));
		out.push('temperature_sensor=' + temps.peak_sensor);
	}
	return out.join('\n') + '\n';
}

/*
 * `api/cli.rs::dump_section` 的帧格式：`===== <label>: <command> =====` + 原文。
 * sections: [ [label, command, reply], ... ]
 */
function textFrame(sections) {
	return sections.map(function (s) {
		return '===== ' + s[0] + ': ' + s[1] + ' =====\n' + s[2] + '\nOK\n\n';
	}).join('');
}

module.exports = {
	round1, ord,
	decodeSignal, decodeCell, decodeRegistration, decodeRrc, decodeOperator, decodeTemps,
	decodeSyscfg, decodeC5gOption, decodeNrCapability,
	NRRCCAP_CA, NRRCCAP_VONR, NRRCCAP_DSS,
	temperatureText, textFrame,
	SENSORS, SENSOR_KEYS,
};
