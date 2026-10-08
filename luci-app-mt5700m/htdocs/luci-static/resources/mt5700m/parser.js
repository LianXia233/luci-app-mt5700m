'use strict';
'require baseclass';

/*
 * MT5700M LuCI — parser.js
 * ---------------------------------------------
 * 纯数据层：AT 应答 → 结构化对象，以及数值格式化 / 运营商标识 / 短信 PDU 解码。
 * 不依赖 DOM、不依赖 ui/fs/rpc，可在任何页面安全复用（node --check 可直接校验）。
 * 行为与 v2.4.8 逐项等价；所有 _() 键保持原文，po 文件零改动。
 */

/* ---------- 文本抽取 ---------- */

// 以 prefix 开头首行的取值（去前导冒号/空格）
function lineValue(text, prefix) {
	var line = (text || '').split(/\n/).filter(function(item) { return item.indexOf(prefix) === 0; })[0] || '';
	return line.substring(prefix.length).replace(/^[ :]+/, '').trim();
}

/* ---------- 数值 / 地址格式化 ---------- */

// 1024 进制字节数 → B / KiB / MiB / GiB / TiB
function formatBytes(value) {
	var units = [ 'B', 'KiB', 'MiB', 'GiB', 'TiB' ], index = 0;
	value = Math.max(0, Number(value) || 0);
	while (value >= 1024 && index < units.length - 1) { value /= 1024; index++; }
	return (index ? value.toFixed(value >= 10 ? 1 : 2) : String(Math.round(value))) + ' ' + units[index];
}

// 秒 → "d h min" 拼接
function formatDuration(seconds) {
	seconds = Math.max(0, Number(seconds) || 0);
	var days = Math.floor(seconds / 86400), hours = Math.floor(seconds % 86400 / 3600), minutes = Math.floor(seconds % 3600 / 60);
	return (days ? days + _('d') + ' ' : '') + (hours ? hours + _('h') + ' ' : '') + minutes + _('min');
}

// 比特率 → Gbps / Mbps / Kbps
function formatRate(value) {
	value = Number(value) || 0;
	if (value >= 1000000000) return (value / 1000000000).toFixed(2) + ' Gbps';
	if (value >= 1000000) return (value / 1000000).toFixed(1) + ' Mbps';
	return value ? Math.round(value / 1000) + ' Kbps' : '--';
}

// 订阅速率（kbps）→ Mbps
function subscriptionRate(value) {
	value = Number(value) || 0;
	return value > 0 ? (value / 1000).toFixed(value % 1000 ? 1 : 0) + ' Mbps' : '--';
}

/* ---------- 会话（network.session 路由） ---------- */

/*
 * `network.session` 的领域载荷 → 页面用的会话视图字段。
 *
 * 字段名沿用旧 `parseSession()`（从 `advanced session` 文本帧里正则出来的那张
 * 对象）的产物，所以概览页的「移动 IP」卡与连接页的会话面板的渲染代码没动：
 * 换掉的只是数据来源 —— 八个 AT 命令的解码现在只有后端一份
 * （modules/network/{parser,state,service}.rs）。
 *
 * 三个纯展示的映射留在这里（各自的语言文案）：DSL 能力码 → 文案、MTU 缺省 →
 * 「Network default」、DNS 列表 → 「 · 」拼接。
 */
function sessionInfo(payload) {
	payload = payload || {};
	var v4 = payload.ipv4 || {}, v6 = payload.ipv6 || {}, flow = payload.flow || {};
	var capabilityNames = {
		1: _('IPv4 only'), 2: _('IPv6 only'), 7: _('IPv4 / IPv6 · same APN'),
		11: _('IPv4 / IPv6 · separate APNs')
	};
	var joinDns = function(list) {
		return (list || []).filter(function(value) { return value && value !== '::'; }).join(' · ');
	};
	var capability = payload.capability;
	return {
		ipv4Connected: v4.connected === true,
		ipv6Connected: v6.connected === true,
		ipv4Address: v4.address || '',
		ipv4Gateway: v4.gateway || '',
		ipv4Dns: joinDns(v4.dns),
		ipv6Address: v6.address || '',
		ipv6Dns: joinDns(v6.dns),
		capability: capabilityNames[capability] || (capability != null ? String(capability) : ''),
		mtu: payload.mtu ? String(payload.mtu) : _('Network default'),
		currentDuration: Number(flow.current_duration) || 0,
		currentTx: Number(flow.current_tx) || 0,
		currentRx: Number(flow.current_rx) || 0,
		totalDuration: Number(flow.total_duration) || 0,
		totalTx: Number(flow.total_tx) || 0,
		totalRx: Number(flow.total_rx) || 0,
		maximumDown: payload.maximum_down || '',
		maximumUp: payload.maximum_up || '',
		detailed: (payload.sessions || []).map(function(item) {
			return {
				cid: String(item.cid), apn: item.apn || '',
				ipv4: item.ipv4 === true, ipv6: item.ipv6 === true,
				type: item.type || '', ethernet: item.ethernet === true
			};
		})
	};
}

/* ---------- 运营商标识 ---------- */

// 英文运营商名 / MCC-MNC → 中文名 + base64 logo（四家运营商统一映射）
function operatorInfo(name) {
	var n = (name || '').toUpperCase().replace(/\s+/g, ' ').trim();
	var mccMnc = n.match(/(\d{5,6})/);
	if (mccMnc && n.indexOf(',') !== -1) n = mccMnc[1];
	if (/^4600[02478]$/.test(n)) n = 'CHINA MOBILE';
	else if (/^4600[169]$/.test(n)) n = 'CHINA UNICOM';
	else if (/^460(03|05|11)$/.test(n)) n = 'CHINA TELECOM';
	else if (/^46015$/.test(n)) n = 'CHINA BROADNET';
	if (n.indexOf('CHINA MOBILE') !== -1 || n.indexOf('CMCC') !== -1)
		return { name: '中国移动', logo: 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAABu0lEQVR4nG1TO07DQBB9jiA0RCQI0ca5wdIADSE5gckJIELUELfmE6S4daBOESQOAD5BfgUolTkB0CNkEAWfBj17NiyBaVaa2ffm82Ys/Gd+dABgH4AtnhjAOTzVnP5qTQHzAK4AVCYgoA9ACSF9VXiKb2IZA8xsPQETVBLACQBHyKLkT5rIqMCPlIAZuADQMCqBABl7EFJbV5KRD9NgXQmtDk+twFMlIbDlZQJYwajYdofhnQTOBKwkUyMoO5F8ZrzfGncGz+/LjsTDzOvnYj4oO5syYRNcDcpOLD6thjpc3XPyc099qdjJNG+7DKpgVOwagyJYSWY9sLq78Vhg/Hhtt5LLxmwDFvyol8vG8en6ti1gzmALQNfYgYY7DBlLkiQVA+ro5jIlYBaWRWZpQVvSijsMzUFHHCorfvtasKkCo3H8sVRpjTuhDFJLR7ApceKDHzXdYTiYn30Z6D3oGfoyQ138O0YrKRhoA6A/hqcKMxKsSZZUZz+6F0LdznWyDyY4JTNuIV1PkjAT94Ja09gWF0xLzHgNnhIV/h6T7ldLysHykOij/gRPjuk3wQ8Rl0qDaMzGc9YDntg3CCWtdUcoEzEAAAAASUVORK5CYII=' };
	if (n.indexOf('CHINA UNICOM') !== -1 || n.indexOf('UNICOM') !== -1)
		return { name: '中国联通', logo: 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAYAAAAf8/9hAAABU0lEQVR4nK2TQU7DQAxFnyddt2kr9iyA5Dj0KJyEo8BxkhYJ9og26j5j9CeTttlVFEtRxo79/f2dMYDWymfgFbjnOvsCXirv3i0Xv/E32wjg87Kzu/MUD5g5bVinWBV/cDe2YYmZTZhYa6WPXnSnjh3bYpl8AclUmPz+QBNKwgXIbELIYmbRA0GQJ1bjecgpziUtC3cMgSpRqbV36KBuMrHCoLFSrzSGMA1npoLKhxl3xWocBoGGxEJY8jwVCuCx3w8a2Qpzd2+tTMmPvk8F6lQQeIjfyf8Id/TEgRmwsxWRSOVdbnGLNdLAe48xiok3Vp59lunRWTF9U87JZ+EzzdTaeiKiNqC4aCalB+nyNkhaDSJqjVmYJJZF6v5Ia/MEUmdNpJHg6nikKeYYRWooiNlkHh8kMRv3PPrKzjHlXPyM//Ir33SZgq6kDvmKXmvK3aj2F3NoxdeFZE2BAAAAAElFTkSuQmCC' };
	if (n.indexOf('CHINA TELECOM') !== -1 || n.indexOf('TELECOM') !== -1)
		return { name: '中国电信', logo: 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAACAAAAAgCAYAAABzenr0AAAEeElEQVR4nMVWbUyTVxR++vJWavkqBSuDyqfTwKSdKIx9CVExcSYDDCYywshkkizGZdkWp2FjMehYRsxCtjESt7n5Rx0O4kIQM5ygbAoKMpAqfvIpUmQFFIqU0uVeae3b20KLLD5J2/uee997np773HOOSJFdZsIzBE++tMPjz8S5wkcC7mk2kIjdoPSTghOJBHZ/L3dkr41AQpQCcs8Fs0fAVRB/uZtfwO7UKHi48+gcGMV7By/i5OU+Ov9g3IDi7auxgOcwZTKh8ZYOv57vxMHqWxgeMwj24px1ul4VAE/JY74Fb6mRv1VFnROELPJAxZ4EpMYp6fMjwxRSvjqLI3Wd9Dl2qRyFmSvRVZKMrMQw4Z9RZJeZZtLAm6uDkLdlBVaFy7Fxfw0ejk/iXP56u2tH9AZEf1iJrvtjFlta/BKUfvSaYF3WtxdwuPbOzBpYHuiN6ry1OPHJGuo0ZlcVqpr7UJj5ot31PYNj8JKI8WWGcP74hW5G5AeyVoJ3EznWwLvrIlD0ziqIeRF2/tiI705dh8kEGoX4Zf7M+s+OtmDfb22IDPJm5pb4SeHv7c6IVCmXYmzCyBLYl66iAjNOmbC58Bx+v9RrmXs7QXh+BFd7R/BFmcYytoYbJ8L3ObHMLSF/hohRzHPCI9iVHEmdE3xd0S5wTpAcG8QQ+KXmDlW6LdzFHH7eEY9NMYHM3N/tA9CNTtAxbzaqQmQoyFBbFpWe7xK8RJROPraobLrL2KKDZdR5TLgv7CH3SItlzJsHeWkrBKGKUvqg4eag5fmV5ezZE3G2dQ8LSO5JjaJJyCwyW+wtvYJajZYlsEEdIFhYtC0GBuMUvcskxOoQGbPZtd4RKtQN6kBkJYQhJU5Jz90RCso12Fvaaj8PTB7bavflPp0ef17pR0y4nFH50OgEFZI5ITkCidSOHy7Ru28NkgcsbzZ36Og1s8VzvguR8Xqo3Y1lHjPnebNGdv7UiNv9D+3Oc+ZB3jFhaJ4WddcGkJR/BpsKah06J+CtmW4rrkdJTiwtInMBScVl9d0oPnUDF2/+69Q7vPXDoTO3UdOmxe7USGx5ORi+ToSYpODq1nuobOpDRWMv9BNGl0iLHBUjco3IVSR3erGPBO+/sYzJA0WV7fjgUBPmCoEIbTFpNKGlc4h+zHnAloBMOnuEZgPv7MK7Oj1jC/aXMralAV5IiQtCz6AeR/963A/MBM5ZAh3aUcYWqmBT8/50FW0+xA4y4ZwJ/NOpY2xhCk/4eQlLbdzzfvS3uWNongl02N/wpWmHBKRBDV3kQTOkpmd4fgncf/AIrV0siSTVkxqS/moI/f2j5R7tJ+aVAIG567VG5ppQeC0U0/qfkxRBbeUNPXAWvNMrp5sP0rRYg2igKjeRJiByA0ijUV7vPAHOFQLkXE+39jN2kiPWRS+m429OXse4wflsyMFFfHz4st0WzJyWC09cdWk/zlUCpGx/atVSmUGOIO1AHa39/ysBTHc220saaJklwTir0SLx89Oov/GkhXMWtBjNhcR84T/wVJ3gylHtAwAAAABJRU5ErkJggg==' };
	if (n.indexOf('BROADNET') !== -1 || n.indexOf('GBA') !== -1 || n.indexOf('CHINA BROADCASTING') !== -1 || n.indexOf('CBN') !== -1)
		return { name: '中国广电', logo: 'data:image/png;base64,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' };
	return { name: name || _('Mobile Network'), logo: null };
}

/* ---------- 状态（manager status + AT status） ---------- */

// 归一化 manager.status + fs.exec('status') 为页面数据
function parseStatus(res) {
	var data = {};
	(res.native && res.native.stdout || '').trim().split(/\n/).forEach(function(line) {
		var pos = line.indexOf('=');
		if (pos > -1)
			data[line.substring(0, pos)] = line.substring(pos + 1);
	});

	data.reachable = data.connected === '1' ? '1' : '0';
	data.model = data.product_name || 'MT5700M';
	data.temperature = String(data.temperature || '').replace(/[^0-9.-]/g, '');
	data.sysmode_detail = data.network_mode || data.sysmode_detail || data.sysmode || '';
	data.at_port = data.at_port || res.manager.at_port || '';
	data.connected = res.manager.connected === true && data.reachable === '1' && !/^(|NOSERVICE|NO SERVICE|UNKNOWN)$/i.test(data.sysmode || data.sysmode_detail || '') ? '1' : '0';
	if (/^(upgrade|dump|unknown)$/.test(data.usb_state || '')) {
		data.reachable = '0';
		data.connected = '0';
	}
	data.network_interface = res.manager.network || '';
	data.error = res.native && res.native.stderr || '';
	// 后端输出的键名是 sim_state（见 at-webserver print_cached_status），
	// 而 status.js 读的是 data.sim。两边不一致会让 SIM 状态恒为 Unknown。
	data.sim = data.sim || data.sim_state || '';
	return data;
}

// 信号质量分级：rsrp/rsrq/sinr → { label, cls, percentage }
function signalQuality(kind, value) {
	var percentage, levels, index;
	if (isNaN(value))
		return { label:_('No data'), cls:'unknown', percentage:0 };
	if (kind === 'rsrp') { percentage = (value + 120) * 2.5; levels = [ -80, -90, -100 ]; }
	else if (kind === 'rsrq') { percentage = (value + 25) * 4; levels = [ -10, -15, -20 ]; }
	else { percentage = (value + 10) * 2.5; levels = [ 20, 13, 0 ]; }
	index = value >= levels[0] ? 0 : value >= levels[1] ? 1 : value >= levels[2] ? 2 : 3;
	return {
		label:[ _('Excellent'), _('Good'), _('Fair'), _('Weak') ][index],
		cls:[ 'excellent', 'good', 'fair', 'weak' ][index],
		percentage:Math.max(0, Math.min(100, percentage))
	};
}

// carrier_N 字段（'|' 分隔）→ 载波聚合信息
function carrierInfo(data) {
	var count = parseInt(data.carrier_count || '0', 10) || 0;
	var carriers = [], parts, i;
	for (i = 1; i <= count; i++) {
		parts = String(data['carrier_' + i] || '').split('|');
		if (parts.length < 8) continue;
		carriers.push({ radio:parts[0], band:parts[1], arfcn:parts[2], dlFreq:parts[3], dlBandwidth:parts[4], ulFreq:parts[6], ulBandwidth:parts[7] });
	}
	return {
		available:carriers.length > 0,
		active:data.ca_active === '1' && carriers.length > 1,
		dual:data.dc_active === '1', mode:data.ca_mode || '', count:carriers.length,
		dlBandwidth:data.ca_dl_bandwidth || '', ulBandwidth:data.ca_ul_bandwidth || '', carriers:carriers
	};
}

/* ---------- 流量统计 ---------- */

function trafficTotal(item) {
	return (Number(item && item.rx) || 0) + (Number(item && item.tx) || 0);
}

function trafficDateKey(item, monthly) {
	var date = item && item.date || {};
	var month = String(date.month || 0).padStart(2, '0');
	var day = String(date.day || 0).padStart(2, '0');
	return monthly ? [ date.year || 0, month ].join('-') : [ date.year || 0, month, day ].join('-');
}

function sortedTraffic(items, monthly) {
	return (items || []).slice().sort(function(a, b) { return trafficDateKey(a, monthly).localeCompare(trafficDateKey(b, monthly)); });
}

function currentTraffic(items, monthly) {
	var now = new Date();
	var key = monthly
		? [ now.getFullYear(), String(now.getMonth() + 1).padStart(2, '0') ].join('-')
		: [ now.getFullYear(), String(now.getMonth() + 1).padStart(2, '0'), String(now.getDate()).padStart(2, '0') ].join('-');
	return (items || []).filter(function(item) { return trafficDateKey(item, monthly) === key; })[0] || {};
}

function trafficUpdated(iface) {
	var value = iface && iface.updated;
	if (!value || !value.date || value.date.year < 2024)
		return _('Waiting for data');
	return '%04d-%02d-%02d %02d:%02d'.format(value.date.year || 0, value.date.month || 0,
		value.date.day || 0, value.time && value.time.hour || 0, value.time && value.time.minute || 0);
}

/* ---------- 无线与小区（network / radio-diagnostics） ---------- */

// 无效测量哨兵 → ''（NR 无效值为真实值×8：RSRP -1256 / RSRQ -348 / SINR -188）
function cleanSignal(value) {
	if (value === undefined || value === null || value === '')
		return '';
	var v = parseFloat(value);
	if (isNaN(v)) return '';
	if (v === -1256 || v === -348 || v === -188 || v === 32767 || v === 255)
		return '';
	return String(value).trim();
}

// 频段名 → 后端锁频命令所需的数字（n41→41, B3→3）
function bandNameToNumber(bandName) {
	if (!bandName) return '';
	var m = String(bandName).match(/^n(\d+)$/i);
	if (m) return m[1];
	m = String(bandName).match(/^B(\d+)$/i);
	if (m) return m[1];
	return '';
}

// SSB 字段专用哨兵过滤（ARFCN 0xFFFFFFFF / PCI 0xFFFF / RSRP·SINR·TA 0x7FFF 与 -1）
function ssbValue(value, invalid) {
	if (value === undefined || value === null || value === '')
		return '';
	if ((invalid || []).some(function(item) { return String(value) === String(item); }))
		return '';
	return value;
}

// CSV 清洗：去空白、去首尾与重复逗号
function cleanCsv(value) {
	return (value || '').replace(/\s+/g, '').replace(/^,+|,+$/g, '').replace(/,+/g, ',');
}

function validCsv(value) {
	return /^[0-9]+(,[0-9]+)*$/.test(value);
}

function csvInRange(value, minimum, maximum) {
	return validCsv(value) && value.split(',').every(function(item) {
		var number = Number(item);
		return number >= minimum && number <= maximum;
	});
}

// MCS 索引 → 调制方式（3GPP 表；LTE 用 36.213 表 7.1.7.1-1，NR 用 38.214 表 4/5/3 与默认 1/2）
function mcsModulation(mcs, table, rat) {
	if (mcs === undefined || mcs === null || mcs === '' || mcs === '255')
		return '';
	var m = parseInt(mcs, 10);
	if (!(m >= 0 && m <= 31))
		return '';
	var t = parseInt(table, 10);
	if (rat === '0') {
		if (m <= 6) return 'QPSK';
		if (m <= 15) return '16QAM';
		if (m <= 27) return '64QAM';
		return '256QAM';
	}
	if (t === 4) {
		if (m <= 10) return 'QPSK';
		if (m <= 20) return '16QAM';
		return '64QAM';
	}
	if (t === 5) {
		if (m <= 10) return 'QPSK';
		if (m <= 20) return '16QAM';
		if (m <= 26) return '64QAM';
		return '256QAM';
	}
	if (t === 3) {
		if (m <= 9) return 'QPSK';
		if (m <= 16) return '16QAM';
		return '64QAM';
	}
	if (m <= 9) return 'QPSK';
	if (m <= 16) return '16QAM';
	if (m <= 28) return '64QAM';
	return '256QAM';
}

// 完整 MCS 段 → [{ rat, carriers:[{table, code0, code1}] }]（多载波 / EN-DC 每 RAT 多组）
/* ---------- 短信会话分组（纯展示聚合，不含任何 AT/PDU 解码） ---------- */

// 按号码分组会话，组内按时间升序、组间按最新消息倒序
function groupMessages(messages) {
	var groups = {};
	messages.forEach(function(msg) { (groups[msg.number] || (groups[msg.number] = [])).push(msg); });
	return Object.keys(groups).map(function(number) { return { number:number, messages:groups[number].sort(function(a,b){return (a.order||0)-(b.order||0);}) }; }).sort(function(a,b){return (b.messages[b.messages.length-1].order||0)-(a.messages[a.messages.length-1].order||0);});
}

/* ---------- 系统（FOTA 状态机） ---------- */

var FOTA_STATE_NAMES = {
	'10': _('Waiting to download'), '11': _('Checking for updates'), '12': _('Update available'),
	'13': _('Update check failed'), '14': _('No update available'), '20': _('Download failed'),
	'30': _('Downloading'), '31': _('Download paused'), '40': _('Download complete'),
	'50': _('Preparing installation')
};

return baseclass.extend({
	lineValue: lineValue,
	formatBytes: formatBytes,
	formatDuration: formatDuration,
	formatRate: formatRate,
	subscriptionRate: subscriptionRate,
	sessionInfo: sessionInfo,
	operatorInfo: operatorInfo,
	parseStatus: parseStatus,
	signalQuality: signalQuality,
	carrierInfo: carrierInfo,
	trafficTotal: trafficTotal,
	trafficDateKey: trafficDateKey,
	sortedTraffic: sortedTraffic,
	currentTraffic: currentTraffic,
	trafficUpdated: trafficUpdated,
	cleanSignal: cleanSignal,
	bandNameToNumber: bandNameToNumber,
	ssbValue: ssbValue,
	cleanCsv: cleanCsv,
	validCsv: validCsv,
	csvInRange: csvInRange,
	mcsModulation: mcsModulation,
	groupMessages: groupMessages,
	FOTA_STATE_NAMES: FOTA_STATE_NAMES
});
