'use strict';
'require view';
'require mt5700m.api as api';
'require mt5700m.parser as parser';
'require mt5700m.components as c';

/*
 * MT5700M LuCI — 概览（status）
 * ---------------------------------------------
 * 数据：mt5700m status（manager）+ fs.exec status / advanced session + mt5700m-traffic summary
 * 无内联样式；信号/载波/地址/模块/SIM/流量/快捷入口全部由组件拼装。
 */

return view.extend({
	load: function() {
		return api.managerStatus().catch(function() { return {}; }).then(function(manager) {
			return Promise.all([
				api.atStatus(),
				api.atSession(),
				api.trafficSummary().catch(function() { return { interfaces: [] }; })
			]).then(function(results) {
				return { native: results[0], session: results[1], traffic: results[2], manager: manager };
			});
		});
	},

	/* ---------- 信号卡 ---------- */

	signalCard: function(data) {
		var rsrp = parseFloat(data.rsrp), quality = parser.signalQuality('rsrp', rsrp);
		return c.card(_('Signal'), _('Current radio quality at a glance'), [
			E('div', { 'class': 'mt-gauge-head' }, [
				E('span', { 'class': 'mt-gauge-label' }, 'RSRP'),
				c.signalBadge(quality)
			]),
			E('div', { 'class': 'mt-gauge-value', 'style': 'font-size:31px;margin:6px 0 10px' }, isNaN(rsrp) ? '--' : String(data.rsrp)),
			c.signalBars(quality.percentage, quality.cls),
			E('div', { 'class': 'mt-details-body' }, [
				c.gauge('RSRQ', 'rsrq', data.rsrq, ' dB', '-25', '-3'),
				c.gauge('SINR', 'sinr', data.sinr, ' dB', '-10', '30'),
				c.gauge(_('Temperature'), 'temp', data.temperature, '°C', '20', '80')
			])
		]);
	},

	/* ---------- 载波聚合卡 ---------- */

	carrierCard: function(info) {
		var active = info.active || info.dual;
		var badgeCls = !info.available ? 'slate' : info.active || info.dual ? 'active' : '';
		var badgeText = !info.available ? _('Unavailable') : info.active ? _('Aggregating') : info.dual ? _('Dual connectivity') : _('Single carrier');
		var headline = !info.available ? '--' : info.active ? info.count + 'CA' : info.dual ? (info.mode || 'EN-DC') : (info.carriers[0] ? info.carriers[0].band : _('Single carrier'));

		var ccList = null;
		if (info.carriers.length > 1) {
			ccList = E('div', { 'class': 'mt-details-body' }, info.carriers.map(function(item, idx) {
				var role = idx === 0 ? _('PCell') : _('SCell %d').format(idx);
				return c.row(role + ' · ' + item.radio + ' · ' + item.band,
					'ARFCN ' + item.arfcn + ' · ' + _('DL') + ' ' + (item.dlFreq ? item.dlFreq + ' MHz' : '--') + ' / ' + _('UL') + ' ' + (item.ulFreq ? item.ulFreq + ' MHz' : '--'));
			}));
		}

		return c.card(_('Carrier status'), _('Carrier aggregation and bandwidth'), [
			E('div', { 'class': 'mt-gauge-head' }, [
				E('span', {}, [ E('span', { 'class': 'mt-gauge-value', 'style': 'font-size:24px' }, headline), E('div', { 'class': 'mt-muted' }, info.mode || _('Mobile network')) ]),
				c.badge(badgeText, badgeCls)
			]),
			info.carriers.length ? E('div', { 'class': 'mt-carrier-grid', 'style': 'margin:10px 0 12px' }, info.carriers.map(function(item) {
				return E('div', { 'class': 'mt-carrier-cell' }, [
					E('span', { 'class': 'mt-carrier-logo' }, item.radio),
					E('div', { 'style': 'min-width:0' }, [
						E('strong', {}, item.band),
						E('div', { 'class': 'mt-muted' }, 'ARFCN ' + (item.arfcn || '--'))
					])
				]);
			})) : E('div', { 'class': 'mt-scan-note' }, _('Current carrier information is unavailable.')),
			ccList,
			E('div', { 'class': 'mt-facts-grid', 'style': 'margin-top:12px' }, [
				c.fact(_('Downlink bandwidth'), info.dlBandwidth ? info.dlBandwidth + ' MHz' : '--'),
				c.fact(_('Uplink bandwidth'), info.ulBandwidth ? info.ulBandwidth + ' MHz' : '--')
			]),
			E('div', { 'class': 'mt-advanced-actions', 'style': 'justify-content:flex-start;margin-top:12px' },
				c.btnLink(_('View radio and cell details'), L.url('admin/modem/mt5700m/network')))
		]);
	},

	/* ---------- 地址卡 ---------- */

	addressCard: function(session) {
		var active = session.ipv4Connected || session.ipv6Connected;
		return c.card(_('Mobile IP'), _('Addresses assigned by the mobile network'), [
			E('div', { 'class': 'mt-gauge-head', 'style': 'margin-bottom:8px' }, [
				E('span', { 'class': 'mt-gauge-label' }, _('Assigned addresses')),
				c.badge(active ? _('Active') : _('Disconnected'), active ? 'active' : 'slate')
			]),
			c.row('IPv4', [ E('div', { 'class': 'mt-muted', 'style': 'text-align:right' }, session.ipv4Connected ? _('Connected') : _('Not assigned')), E('strong', {}, session.ipv4Address || '--') ]),
			c.row('IPv6', [ E('div', { 'class': 'mt-muted', 'style': 'text-align:right' }, session.ipv6Connected ? _('Connected') : _('Not assigned')), E('strong', {}, session.ipv6Address || '--') ]),
			E('div', { 'class': 'mt-session-note' }, (session.capability || '--') + ' · MTU ' + (session.mtu || '--')),
			E('div', { 'class': 'mt-advanced-actions', 'style': 'justify-content:flex-start;margin-top:12px' },
				c.btnLink(_('View connection details'), L.url('admin/modem/mt5700m/connection')))
		]);
	},

	/* ---------- 模块 / SIM 卡 ---------- */

	moduleCard: function(data) {
		return c.card(_('Module'), _('Identity and firmware'), [
			E('div', { 'class': 'mt-gauge-head', 'style': 'margin-bottom:6px' }, [
				E('span', { 'class': 'mt-gauge-label' }, _('Module information')),
				c.badge(data.product_name || 'MT5700M', 'primary')
			]),
			c.row(_('Manufacturer'), data.manufacturer),
			c.row(_('Model'), data.model),
			c.row(_('Firmware'), data.revision),
			c.row('IMEI', data.imei),
			c.row(_('AT port'), data.at_port || data.network_interface)
		]);
	},

	simCard: function(data) {
		var simState = data.sim || '';
		var simOk = /READY/i.test(simState);
		var downRate = data.ambr_down_mbps ? parseFloat(data.ambr_down_mbps) : NaN;
		var upRate = data.ambr_up_mbps ? parseFloat(data.ambr_up_mbps) : NaN;
		var rateText = (!isNaN(downRate) && !isNaN(upRate))
			? _('Down %s / Up %s').format(
				downRate >= 1 ? downRate.toFixed(0) + ' Mbps' : (downRate * 1000).toFixed(0) + ' Kbps',
				upRate >= 1 ? upRate.toFixed(0) + ' Mbps' : (upRate * 1000).toFixed(0) + ' Kbps')
			: '';
		return c.card(_('SIM & Subscription'), _('Subscriber identity and service plan'), [
			E('div', { 'class': 'mt-gauge-head', 'style': 'margin-bottom:6px' }, [
				E('span', { 'class': 'mt-gauge-label' }, _('SIM Status')),
				c.badge(simOk ? _('Ready') : (simState || _('Unknown')), simOk ? 'active' : 'slate')
			]),
			c.row(_('Operator'), data.operator),
			c.row(_('Access technology'), data.sysmode_detail || data.sysmode),
			c.row(_('APN'), data.active_apn || _('Carrier default')),
			c.row(_('QCI'), data.qci ? 'QCI ' + data.qci : ''),
			c.row('ICCID', data.iccid),
			c.row('IMSI', data.imsi),
			c.row(_('Phone number'), data.phone_number_state === 'not_stored' ? _('Not stored') : data.phone_number),
			rateText ? c.row(_('Subscription rate'), rateText) : null
		].filter(Boolean));
	},

	/* ---------- 流量面板 ---------- */

	trafficPanel: function(report, interfaceName) {
		var iface = (report.interfaces || []).filter(function(item) { return item.name === interfaceName; })[0] ||
			(report.interfaces || []).filter(function(item) { return item.name === 'eth2'; })[0] || { traffic: {} };
		var traffic = iface.traffic || {}, days = parser.sortedTraffic(traffic.day, false), months = parser.sortedTraffic(traffic.month, true);
		var today = parser.currentTraffic(days, false), month = parser.currentTraffic(months, true), lifetime = traffic.total || {};
		var recentDays = days.slice(-7).reverse(), maximum = Math.max.apply(Math, recentDays.map(parser.trafficTotal).concat([ 1 ]));
		var dayRows = recentDays.length ? recentDays.map(function(item) {
			var rx = Number(item.rx) || 0, tx = Number(item.tx) || 0;
			var h = Math.max(1, Math.round(Math.max(rx, tx) / maximum * 64));
			return E('div', { 'class': 'mt-traffic-bar' + (parser.trafficDateKey(item, false) === parser.trafficDateKey(today, false) ? ' active' : '') }, [
				E('span', { 'class': 'mt-traffic-bar-value' }, parser.formatBytes(parser.trafficTotal(item))),
				E('div', { 'class': 'mt-traffic-bar-fill', 'style': 'height:%dpx'.format(h) }),
				E('span', { 'class': 'mt-traffic-bar-note' }, parser.trafficDateKey(item, false).substring(5))
			]);
		}) : [ E('div', { 'class': 'mt-scan-note' }, _('Statistics appear after the MT5700M data interface has carried traffic for a few minutes.')) ];

		function stat(label, item) {
			return E('div', { 'class': 'mt-traffic-stat' }, [
				E('div', { 'class': 'mt-traffic-stat-value' }, parser.formatBytes(parser.trafficTotal(item))),
				E('div', { 'class': 'mt-traffic-stat-label' }, label),
				E('div', { 'class': 'mt-scan-note' }, _('Download %s · Upload %s').format(parser.formatBytes(item.rx), parser.formatBytes(item.tx)))
			]);
		}

		return c.card(_('Traffic Statistics'), _('Local usage recorded only for the MT5700M data interface'), [
			E('div', { 'class': 'mt-gauge-head', 'style': 'margin-bottom:10px' }, [
				E('span', { 'class': 'mt-scan-note' }, _('Last updated') + ' · ' + parser.trafficUpdated(iface)),
				E('span', { 'class': 'mt-traffic-legend' }, [
					E('span', {}, [ E('i', { 'class': 'mt-traffic-legend-dot' }), _('Download') ]),
					E('span', {}, [ E('i', { 'class': 'mt-traffic-legend-dot active' }), _('Upload') ])
				])
			]),
			E('div', { 'class': 'mt-traffic-stats' }, [ stat(_('Today'), today), stat(_('This month'), month), stat(_('All-time total'), lifetime) ]),
			E('div', { 'class': 'mt-traffic-days' }, dayRows)
		]);
	},

	/* ---------- 渲染 ---------- */

	render: function(res) {
		var data = parser.parseStatus(res), session = parser.parseSession(res.session && res.session.stdout || '');
		var reachable = data.reachable === '1', connected = data.connected === '1', carrierInfo = parser.carrierInfo(data);
		var opInfo = parser.operatorInfo(data.operator);
		var operator = opInfo.name;
		if (!/[A-Za-z0-9\u4e00-\u9fff]/.test(operator)) operator = '';
		var usbNames = { upgrade: _('Upgrade mode'), dump: _('Dump mode'), unknown: _('Unknown USB mode') };
		var abnormalUsb = data.usb_state === 'upgrade' || data.usb_state === 'dump' || data.usb_state === 'unknown';
		data.operator = operator;

		return E('div', { 'class': 'mt-page' }, [
			c.cssLink(),
			data.error ? E('div', { 'class': 'alert-message warning' }, data.error) : null,
			res.session && res.session.stderr ? E('div', { 'class': 'alert-message warning' }, res.session.stderr) : null,
			abnormalUsb ? E('div', { 'class': 'alert-message warning' }, _('The MT5700M is in %s. Mobile data and AT management are unavailable until normal mode returns.').format(usbNames[data.usb_state])) : null,
			c.hero(null, _('MT5700M Module'),
				!reachable ? _('The modem did not respond. Check the module connection.') : connected ? _('Mobile network is connected and ready.') : _('The module is online, but mobile data is not connected.'),
				[
					E('div', { 'class': 'mt-conn-state' }, [
						E('span', { 'class': 'mt-conn-state-dot' + (connected ? ' on' : '') }),
						E('span', { 'class': 'mt-conn-state-text' }, connected ? _('Connected') : reachable ? _('Module online') : _('Unavailable'))
					]),
					E('a', { 'class': 'mt-hero-btn', 'href': '/5700/', 'target': '_blank', 'rel': 'noopener' }, [ _('WebUI'), ' ↗' ]),
					E('button', { 'class': 'mt-hero-btn mt-hero-refresh', 'click': function() { window.location.reload(); } }, _('Refresh'))
				]),
			E('div', { 'class': 'mt-facts-grid', 'style': 'margin-bottom:14px' }, [
				E('div', { 'class': 'mt-facts-cell' }, [
					E('div', { 'class': 'mt-facts-label' }, _('Network Mode')),
					E('div', { 'class': 'mt-facts-value' }, data.sysmode_detail || data.sysmode || '--')
				]),
				E('div', { 'class': 'mt-facts-cell' }, [
					E('div', { 'class': 'mt-facts-label' }, _('Network interface')),
					E('div', { 'class': 'mt-facts-value' }, data.network_interface || '--')
				]),
				E('div', { 'class': 'mt-facts-cell' }, [
					E('div', { 'class': 'mt-facts-label' }, _('Operator')),
					E('div', { 'class': 'mt-facts-value' }, [ opInfo.logo ? E('img', { 'src': opInfo.logo, 'alt': operator, 'style': 'width:20px;height:20px;vertical-align:-4px;margin-right:6px' }) : null, operator || '--' ])
				]),
				E('div', { 'class': 'mt-facts-cell' }, [
					E('div', { 'class': 'mt-facts-label' }, _('AT port')),
					E('div', { 'class': 'mt-facts-value' }, data.at_port || '--')
				])
			]),
			E('div', { 'class': 'mt-grid' }, [
				this.signalCard(data),
				this.carrierCard(carrierInfo),
				this.addressCard(session)
			]),
			E('div', { 'class': 'mt-grid' }, [
				this.moduleCard(data),
				this.simCard(data)
			]),
			this.trafficPanel(res.traffic || {}, data.network_interface || 'eth2'),
			E('div', { 'class': 'mt-shortcuts' }, [
				c.btnLink(_('Mobile data'), L.url('admin/modem/mt5700m/connection'), { 'cls': 'mt-shortcuts-card' }),
				c.btnLink(_('Radio and Cells'), L.url('admin/modem/mt5700m/network'), { 'cls': 'mt-shortcuts-card' }),
				c.btnLink(_('Module and SIM'), L.url('admin/modem/mt5700m/system'), { 'cls': 'mt-shortcuts-card' })
			])
		]);
	},

	handleSave: null,
	handleSaveApply: null,
	handleReset: null
});
