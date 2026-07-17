'use strict';
'require view';
'require fs';
'require rpc';
'require ui';
'require mt5700m.controls as controls';

var callManagerStatus = rpc.declare({ object: 'mt5700m', method: 'status', expect: { } });
var callTraffic = rpc.declare({ object: 'mt5700m-traffic', method: 'summary', expect: { } });

function csvValues(text, prefix) {
	var line = (text || '').split(/\n/).filter(function(item) { return item.indexOf(prefix) === 0; })[0] || '';
	return line.substring(prefix.length).replace(/^[ :]+/, '').replace(/"/g, '').split(',').map(function(value) { return value.trim(); });
}

function hexIPv4(value) {
	if (!/^[0-9a-f]{8}$/i.test(value || ''))
		return '';
	return [ 6, 4, 2, 0 ].map(function(offset) { return parseInt(value.substr(offset, 2), 16); }).join('.');
}

function hexNumber(value) {
	value = String(value || '').replace(/^0x/i, '');
	if (!/^[0-9a-f]+$/i.test(value))
		return 0;
	if (value.length <= 8)
		return parseInt(value, 16);
	return parseInt(value.slice(0, -8), 16) * 4294967296 + parseInt(value.slice(-8), 16);
}

function formatBytes(value) {
	var units = [ 'B', 'KiB', 'MiB', 'GiB', 'TiB' ], index = 0;
	value = Math.max(0, Number(value) || 0);
	while (value >= 1024 && index < units.length - 1) { value /= 1024; index++; }
	return (index ? value.toFixed(value >= 10 ? 1 : 2) : String(Math.round(value))) + ' ' + units[index];
}

function formatDuration(seconds) {
	seconds = Math.max(0, Number(seconds) || 0);
	var days = Math.floor(seconds / 86400), hours = Math.floor(seconds % 86400 / 3600), minutes = Math.floor(seconds % 3600 / 60);
	return (days ? days + _('d') + ' ' : '') + (hours ? hours + _('h') + ' ' : '') + minutes + _('min');
}

function formatRate(value) {
	value = Number(value) || 0;
	if (value >= 1000000000) return (value / 1000000000).toFixed(2) + ' Gbps';
	if (value >= 1000000) return (value / 1000000).toFixed(1) + ' Mbps';
	return value ? Math.round(value / 1000) + ' Kbps' : '--';
}

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

return view.extend({
	load: function() {
		return callManagerStatus().catch(function() { return {}; }).then(function(manager) {
			return Promise.all([
				fs.exec('/usr/sbin/mt5700m-at', [ 'status' ]).catch(function(err) { return { stdout:'', stderr:err.message || String(err) }; }),
				fs.exec('/usr/sbin/mt5700m-at', [ 'advanced', 'session' ]).catch(function(err) { return { stdout:'', stderr:err.message || String(err) }; }),
				callTraffic().catch(function() { return { interfaces:[] }; })
			]).then(function(results) {
				return { native:results[0], session:results[1], traffic:results[2], manager:manager };
			});
		});
	},

	parseStatus: function(res) {
		var data = {};
		var parseOutput = function(output) {
			(output || '').trim().split(/\n/).forEach(function(line) {
				var pos = line.indexOf('=');

				if (pos > -1)
					data[line.substring(0, pos)] = line.substring(pos + 1);
			});
		};
		parseOutput(res.native && res.native.stdout);

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
		data.dial_running = res.manager.running === true ? '1' : '0';
		data.network_interface = res.manager.network || '';
		data.error = (res.native && res.native.stderr) || '';
		data.backend = _('Integrated MT5700M service');
		return data;
	},

	styleNode: function() {
		return E('style', {}, [
			'.mt5700m-page{max-width:1120px;margin:0 auto;color:var(--text-color-high,#20242a)}',
			'.mt5700m-hero{position:relative;overflow:hidden;padding:24px 26px;margin:0 0 18px;border-radius:16px;background:linear-gradient(135deg,#1264d8 0%,#087eae 58%,#07988e 100%);color:#fff;box-shadow:0 10px 30px rgba(14,92,155,.18)}',
			'.mt5700m-hero:after{content:"";position:absolute;width:220px;height:220px;right:-72px;top:-108px;border:44px solid rgba(255,255,255,.08);border-radius:50%}',
			'.mt5700m-hero-top{position:relative;z-index:1;display:flex;justify-content:space-between;align-items:flex-start;gap:18px}',
			'.mt5700m-eyebrow{font-size:12px;font-weight:700;letter-spacing:.08em;text-transform:uppercase;opacity:.78;margin-bottom:5px}',
			'.mt5700m-title{font-size:28px;line-height:1.18;font-weight:720;margin:0 0 7px;color:#fff}',
			'.mt5700m-summary{font-size:14px;line-height:1.5;opacity:.88}',
			'.mt5700m-status{display:inline-flex;align-items:center;gap:8px;padding:8px 12px;border-radius:999px;background:rgba(255,255,255,.16);font-size:13px;font-weight:650;white-space:nowrap;backdrop-filter:blur(4px)}',
			'.mt5700m-dot{width:8px;height:8px;border-radius:50%;background:#ffcd57;box-shadow:0 0 0 4px rgba(255,205,87,.18)}',
			'.mt5700m-status.is-reachable .mt5700m-dot{background:#ffcd57;box-shadow:0 0 0 4px rgba(255,205,87,.18)}',
			'.mt5700m-status.is-online .mt5700m-dot{background:#78f2b0;box-shadow:0 0 0 4px rgba(120,242,176,.18)}',
			'.mt5700m-hero-meta{position:relative;z-index:1;display:flex;flex-wrap:wrap;gap:8px 22px;margin-top:22px;font-size:13px}',
			'.mt5700m-meta{display:flex;gap:7px;align-items:center}',
			'.mt5700m-meta-label{opacity:.7}',
			'.mt5700m-meta-value{font-weight:650}',
			'.mt5700m-section-title{font-size:16px;font-weight:700;margin:22px 0 11px}',
			'.mt5700m-metrics{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:12px}',
			'.mt5700m-metric{padding:16px;border:1px solid var(--border-color-medium,#d9dde4);border-radius:13px;background:var(--background-color-high,#fff);box-shadow:0 3px 12px rgba(20,32,50,.04)}',
			'.mt5700m-metric.is-signal{padding:11px 14px 9px}',
			'.mt5700m-metric-head{display:flex;align-items:center;justify-content:space-between;gap:8px;margin-bottom:8px}',
			'.mt5700m-metric.is-signal .mt5700m-metric-head{margin-bottom:3px}',
			'.mt5700m-metric-label{font-size:12px;color:var(--text-color-medium,#69717d)}',
			'.mt5700m-quality{display:inline-flex;align-items:center;gap:5px;padding:3px 7px;border-radius:999px;background:#eef2f6;color:#6b7480;font-size:10px;font-weight:700}',
			'.mt5700m-quality:before{content:"";width:6px;height:6px;border-radius:50%;background:currentColor}',
			'.mt5700m-quality.is-excellent{background:#e8f8f1;color:#087c60}.mt5700m-quality.is-good{background:#eaf3ff;color:#176bc1}.mt5700m-quality.is-fair{background:#fff5df;color:#9b6500}.mt5700m-quality.is-weak,.mt5700m-quality.is-hot{background:#fff0ee;color:#b84035}.mt5700m-quality.is-warm{background:#fff5df;color:#9b6500}',
			'.mt5700m-metric-line{display:flex;align-items:baseline;gap:5px}',
			'.mt5700m-metric-value{font-size:23px;line-height:1.15;font-weight:720;letter-spacing:-.02em}',
			'.mt5700m-metric-unit{font-size:12px;color:var(--text-color-medium,#69717d)}',
			'.mt5700m-meter{position:relative;height:6px;margin-top:14px;border-radius:999px;background:linear-gradient(90deg,#db5b52 0%,#e4a23a 34%,#4b94df 67%,#13a979 100%);box-shadow:inset 0 0 0 1px rgba(30,42,56,.06)}',
			'.mt5700m-meter-marker{position:absolute;top:50%;width:11px;height:11px;border:2px solid #fff;border-radius:50%;background:#263746;box-shadow:0 1px 4px rgba(20,35,50,.34);transform:translate(-50%,-50%)}',
			'.mt5700m-signal-bars{display:flex;align-items:flex-end;gap:3px;height:44px;margin-top:2px;padding:0 1px}',
			'.mt5700m-signal-bar{flex:1;min-width:2px;border-radius:2px 2px 1px 1px;background:var(--border-color-medium,#d9dde4);opacity:.58;transition:background-color .2s,opacity .2s}',
			'.mt5700m-signal-bar.is-active{background:#4d94db;opacity:1}.mt5700m-signal-bars.is-excellent .mt5700m-signal-bar.is-active{background:#13a979}.mt5700m-signal-bars.is-good .mt5700m-signal-bar.is-active{background:#4b94df}.mt5700m-signal-bars.is-fair .mt5700m-signal-bar.is-active{background:#e4a23a}.mt5700m-signal-bars.is-weak .mt5700m-signal-bar.is-active{background:#db5b52}',
			'.mt5700m-ambr{display:flex;align-items:center;justify-content:space-between;gap:22px;padding:14px 16px;margin-bottom:10px;border:1px solid #d5e5f5;border-radius:13px;background:linear-gradient(135deg,#f4f9ff,#f3faf8)}',
			'.mt5700m-ambr-copy{min-width:205px}.mt5700m-ambr-title{display:flex;align-items:center;gap:8px;font-size:14px;font-weight:700}.mt5700m-ambr-tag{padding:3px 7px;border-radius:999px;background:#e6f1ff;color:#176bc1;font-size:10px;font-weight:700}.mt5700m-ambr-desc{margin-top:4px;color:var(--text-color-medium,#69717d);font-size:11px;line-height:1.45}',
			'.mt5700m-ambr-data{display:grid;grid-template-columns:1.25fr 1.4fr 1fr 1fr;align-items:center;gap:20px;flex:1;max-width:700px}.mt5700m-ambr-item{min-width:0}.mt5700m-ambr-item span{display:block;margin-bottom:3px;color:var(--text-color-medium,#69717d);font-size:10px}.mt5700m-ambr-item strong{display:block;overflow:hidden;text-overflow:ellipsis;font-size:17px;font-weight:720;white-space:nowrap}.mt5700m-ambr-item.is-id strong{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:14px;letter-spacing:.01em}',
			'.mt5700m-ca{border:1px solid var(--border-color-medium,#d9dde4);border-radius:13px;background:var(--background-color-high,#fff);overflow:hidden}',
			'.mt5700m-ca-head{display:flex;align-items:center;justify-content:space-between;gap:12px;padding:15px 17px;border-bottom:1px solid var(--border-color-low,#edf0f4)}.mt5700m-ca-title{font-size:14px;font-weight:700}.mt5700m-ca-summary{margin-top:3px;color:var(--text-color-medium,#69717d);font-size:12px}',
			'.mt5700m-ca-badge{padding:5px 9px;border-radius:999px;background:#eef2f6;color:#6b7480;font-size:11px;font-weight:700;white-space:nowrap}.mt5700m-ca-badge.active{background:#e8f8f1;color:#087c60}',
			'.mt5700m-ca-body{padding:14px 17px}.mt5700m-ca-stats{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:10px;margin-bottom:12px}.mt5700m-ca-stat{padding:10px 12px;border-radius:10px;background:var(--background-color-low,#f5f7f9)}.mt5700m-ca-stat-label{font-size:10px;color:var(--text-color-medium,#727a86);margin-bottom:3px}.mt5700m-ca-stat-value{font-size:15px;font-weight:700}',
			'.mt5700m-carriers{display:grid;gap:8px}.mt5700m-carrier{display:flex;align-items:center;justify-content:space-between;gap:14px;padding:10px 12px;border:1px solid var(--border-color-low,#edf0f4);border-radius:10px}.mt5700m-carrier-name{font-size:13px;font-weight:700;white-space:nowrap}.mt5700m-carrier-detail{text-align:right;color:var(--text-color-medium,#69717d);font-size:11px;line-height:1.55}.mt5700m-ca-note{margin:11px 0 0;color:var(--text-color-medium,#727a86);font-size:11px}',
			'.mt5700m-content-grid{display:grid;grid-template-columns:minmax(0,1.45fr) minmax(260px,.75fr);gap:14px}',
			'.mt5700m-panel{border:1px solid var(--border-color-medium,#d9dde4);border-radius:13px;background:var(--background-color-high,#fff);overflow:hidden}',
			'.mt5700m-panel-head{padding:14px 16px;border-bottom:1px solid var(--border-color-low,#edf0f4);font-size:14px;font-weight:700}',
			'.mt5700m-detail-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));padding:4px 16px 12px}',
			'.mt5700m-detail{padding:12px 0;border-bottom:1px solid var(--border-color-low,#edf0f4)}',
			'.mt5700m-detail:nth-last-child(-n+2){border-bottom:0}',
			'.mt5700m-detail:nth-child(odd){padding-right:18px}',
			'.mt5700m-detail-label{font-size:11px;color:var(--text-color-medium,#727a86);margin-bottom:4px}',
			'.mt5700m-detail-value{font-size:14px;font-weight:600;word-break:break-word}',
			'.mt5700m-locks{padding:5px 16px 12px}',
			'.mt5700m-lock{display:flex;align-items:center;justify-content:space-between;gap:12px;padding:12px 0;border-bottom:1px solid var(--border-color-low,#edf0f4)}',
			'.mt5700m-lock:last-child{border-bottom:0}',
			'.mt5700m-lock-name{font-size:13px;font-weight:600}',
			'.mt5700m-lock-state{padding:4px 9px;border-radius:999px;background:#eaf8f3;color:#087b62;font-size:11px;font-weight:700}',
			'.mt5700m-lock-state.is-locked{background:#fff1df;color:#a45d00}',
			'.mt5700m-actions{display:flex;flex-wrap:wrap;align-items:center;gap:9px;margin-top:16px}',
			'.mt5700m-actions .btn{border-radius:9px;padding:7px 14px}',
			'.mt5700m-alert{margin:0 0 14px}',
			'.mt5700m-session-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:13px}.mt5700m-overview-card{padding:17px;border:1px solid var(--border-color-medium,#d9dde4);border-radius:13px;background:var(--background-color-high,#fff);box-shadow:0 3px 12px rgba(20,32,50,.04)}',
			'.mt5700m-card-head{display:flex;align-items:flex-start;justify-content:space-between;gap:12px;margin-bottom:11px}.mt5700m-card-head h3{margin:0 0 4px;font-size:15px}.mt5700m-card-head p{margin:0;color:var(--text-color-medium,#69717d);font-size:11px;line-height:1.45}.mt5700m-card-badge{padding:4px 8px;border-radius:999px;background:#eef2f6;color:#6b7480;font-size:10px;font-weight:700;white-space:nowrap}.mt5700m-card-badge.on{background:#e8f8f1;color:#087c60}',
			'.mt5700m-session-columns{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:0 18px}.mt5700m-session-row{display:flex;justify-content:space-between;align-items:flex-start;gap:12px;padding:8px 0;border-bottom:1px solid var(--border-color-low,#edf0f4);font-size:11px}.mt5700m-session-row span{color:var(--text-color-medium,#69717d)}.mt5700m-session-row strong{text-align:right;word-break:break-all}.mt5700m-session-actions{display:flex;justify-content:flex-end;margin-top:11px}',
			'.mt5700m-traffic-summary{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:10px;margin-bottom:12px}.mt5700m-traffic-stat{position:relative;overflow:hidden;padding:14px 15px;border:1px solid var(--border-color-medium,#d9dde4);border-radius:12px;background:var(--background-color-high,#fff)}.mt5700m-traffic-stat:before{content:"";position:absolute;left:0;top:0;bottom:0;width:3px;background:#0fa17d}.mt5700m-traffic-stat.blue:before{background:#337de8}.mt5700m-traffic-stat.violet:before{background:#7b68d9}.mt5700m-traffic-stat.slate:before{background:#738091}.mt5700m-traffic-label{font-size:10px;color:var(--text-color-medium,#69717d);margin-bottom:6px}.mt5700m-traffic-value{font-size:19px;font-weight:740;letter-spacing:-.02em}.mt5700m-traffic-meta{margin-top:5px;font-size:9px;line-height:1.45;color:var(--text-color-medium,#7b8490)}',
			'.mt5700m-traffic-grid{display:grid;grid-template-columns:minmax(0,1.45fr) minmax(270px,.75fr);gap:12px}.mt5700m-traffic-days{display:flex;flex-direction:column;gap:8px}.mt5700m-traffic-day{display:grid;grid-template-columns:56px minmax(80px,1fr) 132px;align-items:center;gap:10px;font-size:10px}.mt5700m-traffic-date{font-weight:650;color:var(--text-color-medium,#66717e)}.mt5700m-traffic-bars{display:flex;flex-direction:column;gap:3px}.mt5700m-traffic-bar{height:5px;border-radius:999px;background:var(--background-color-low,#eef1f5);overflow:hidden}.mt5700m-traffic-bar i{display:block;height:100%;min-width:2px;border-radius:inherit;background:#337de8}.mt5700m-traffic-bar.tx i{background:#16a085}.mt5700m-traffic-values{display:flex;justify-content:flex-end;gap:8px;font-variant-numeric:tabular-nums}.mt5700m-traffic-values span:first-child{color:#2f75d5}.mt5700m-traffic-values span:last-child{color:#0c9478}',
			'.mt5700m-month-row{display:grid;grid-template-columns:62px 1fr auto;gap:8px;padding:8px 0;border-bottom:1px solid var(--border-color-low,#edf0f4);font-size:10px}.mt5700m-month-row:last-child{border-bottom:0}.mt5700m-month-row strong{text-align:right}.mt5700m-month-row small{display:block;margin-top:2px;color:var(--text-color-medium,#79828e)}.mt5700m-empty{padding:25px 14px;text-align:center;border:1px dashed var(--border-color-medium,#d6dce4);border-radius:10px;color:var(--text-color-medium,#727b87);font-size:11px}.mt5700m-privacy-note{margin:10px 0 0;color:var(--text-color-medium,#727b87);font-size:10px;line-height:1.5}',
			'@media(max-width:760px){.mt5700m-hero{padding:20px}.mt5700m-title{font-size:23px}.mt5700m-hero-top{display:block}.mt5700m-status{margin-top:14px}.mt5700m-metrics{grid-template-columns:repeat(2,minmax(0,1fr))}.mt5700m-ambr{display:block}.mt5700m-ambr-data{grid-template-columns:repeat(2,minmax(0,1fr));max-width:none;margin-top:13px}.mt5700m-content-grid,.mt5700m-session-grid,.mt5700m-traffic-grid{grid-template-columns:1fr}.mt5700m-traffic-summary{grid-template-columns:repeat(2,minmax(0,1fr))}}',
			'@media(max-width:430px){.mt5700m-metrics{grid-template-columns:1fr}.mt5700m-ambr-data{gap:12px}.mt5700m-ca-stats,.mt5700m-session-columns{grid-template-columns:1fr}.mt5700m-carrier{display:block}.mt5700m-carrier-detail{text-align:left;margin-top:5px}.mt5700m-detail-grid{grid-template-columns:1fr}.mt5700m-detail:nth-child(odd){padding-right:0}.mt5700m-detail:nth-last-child(2){border-bottom:1px solid var(--border-color-low,#edf0f4)}.mt5700m-traffic-day{grid-template-columns:48px 1fr}.mt5700m-traffic-values{grid-column:2;justify-content:flex-start}.mt5700m-traffic-value{font-size:16px}}'
		].join(''));
	},

	metric: function(label, value, unit, quality) {
		var missing = value == null || value === '' || value === 'null';
		var width = Math.max(0, Math.min(100, Number(quality.percentage) || 0));
		var marker = Math.max(2, Math.min(98, width));
		var bars = [];
		var activeBars = missing ? 0 : Math.max(1, Math.round(width / 100 * 12));
		var i;

		for (i = 0; quality.signal && i < 12; i++)
			bars.push(E('span', {
				'class': 'mt5700m-signal-bar' + (i < activeBars ? ' is-active' : ''),
				'style': 'height:%dpx'.format(8 + i * 3)
			}));

		return E('div', { 'class': 'mt5700m-metric mt-ui-card' + (quality.signal ? ' is-signal' : '') }, [
			E('div', { 'class': 'mt5700m-metric-head' }, [
				E('div', { 'class': 'mt5700m-metric-label' }, label),
				E('span', { 'class': 'mt5700m-quality ' + quality.cls }, missing ? _('No data') : quality.label)
			]),
			E('div', { 'class': 'mt5700m-metric-line' }, [
				E('span', { 'class': 'mt5700m-metric-value' }, missing ? '--' : String(value)),
				!missing && unit ? E('span', { 'class': 'mt5700m-metric-unit' }, unit) : null
			]),
			quality.signal
				? E('div', { 'class': 'mt5700m-signal-bars ' + quality.cls, 'aria-hidden': 'true' }, bars)
				: E('div', { 'class': 'mt5700m-meter', 'aria-hidden': 'true' }, missing ? null : E('span', { 'class': 'mt5700m-meter-marker', 'style': 'left:%d%%'.format(marker) }))
		]);
	},

	signalQuality: function(kind, value) {
		var percentage = 0;
		var levels;
		var index;
		var quality;

		if (isNaN(value))
			return { label: _('No data'), cls: 'is-unknown', percentage: 0, signal: true };

		if (kind === 'rsrp') {
			percentage = (value + 120) * 2.5;
			levels = [ -80, -90, -100 ];
		} else if (kind === 'rsrq') {
			percentage = (value + 25) * 4;
			levels = [ -10, -15, -20 ];
		} else {
			percentage = (value + 10) * 2.5;
			levels = [ 20, 13, 0 ];
		}

		index = value >= levels[0] ? 0 : value >= levels[1] ? 1 : value >= levels[2] ? 2 : 3;
		quality = [
			{ label: _('Excellent'), cls: 'is-excellent' },
			{ label: _('Good'), cls: 'is-good' },
			{ label: _('Fair'), cls: 'is-fair' },
			{ label: _('Weak'), cls: 'is-weak' }
		][index];
		quality.percentage = percentage;
		quality.signal = true;
		return quality;
	},

	temperatureQuality: function(value) {
		if (isNaN(value))
			return { label: _('No data'), cls: 'is-unknown', percentage: 0 };
		if (value <= 55)
			return { label: _('Normal'), cls: 'is-good', percentage: Math.max(20, 100 - Math.max(0, value - 35) * 2) };
		if (value <= 70)
			return { label: _('Warm'), cls: 'is-warm', percentage: Math.max(20, 100 - (value - 35) * 2) };
		return { label: _('Hot'), cls: 'is-hot', percentage: Math.max(12, 100 - (value - 35) * 2) };
	},

	carrierInfo: function(data) {
		var count = parseInt(data.carrier_count || '0', 10) || 0;
		var carriers = [];
		var parts;
		var i;

		for (i = 1; i <= count; i++) {
			parts = String(data['carrier_' + i] || '').split('|');
			if (parts.length < 8)
				continue;
			carriers.push({
				radio: parts[0], band: parts[1], dlChannel: parts[2], dlFrequency: parts[3], dlBandwidth: parts[4],
				ulChannel: parts[5], ulFrequency: parts[6], ulBandwidth: parts[7]
			});
		}

		return {
			available: carriers.length > 0,
			active: data.ca_active === '1' && carriers.length > 1,
			dual: data.dc_active === '1',
			mode: data.ca_mode || '',
			count: carriers.length,
			dlBandwidth: data.ca_dl_bandwidth || '',
			ulBandwidth: data.ca_ul_bandwidth || '',
			carriers: carriers
		};
	},

	formatSubscriptionRate: function(value) {
		var rate = parseFloat(value);

		if (isNaN(rate) || rate <= 0)
			return '--';

		return (rate % 1 ? rate.toFixed(1) : rate.toFixed(0)) + ' Mbps';
	},

	subscriptionPanel: function(data) {
		var down = this.formatSubscriptionRate(data.ambr_down_mbps);
		var up = this.formatSubscriptionRate(data.ambr_up_mbps);
		var phone = data.phone_number || (data.phone_number_state === 'not_stored' ? _('Not stored on SIM') : '--');

		return E('section', { 'class': 'mt5700m-ambr mt-ui-card' }, [
			E('div', { 'class': 'mt5700m-ambr-copy' }, [
				E('div', { 'class': 'mt5700m-ambr-title' }, [
					E('span', {}, _('SIM and subscription')),
					E('span', { 'class': 'mt5700m-ambr-tag' }, _('Local only'))
				]),
				E('div', { 'class': 'mt5700m-ambr-desc' }, _('The phone number and device ID are displayed locally; subscription rates are network-authorized limits, not a live speed test.'))
			]),
			E('div', { 'class': 'mt5700m-ambr-data' }, [
				E('div', { 'class': 'mt5700m-ambr-item is-id' }, [ E('span', {}, _('Phone Number')), E('strong', { 'title': phone }, phone) ]),
				E('div', { 'class': 'mt5700m-ambr-item is-id' }, [ E('span', {}, 'IMEI'), E('strong', { 'title': data.imei || '--' }, data.imei || '--') ]),
				E('div', { 'class': 'mt5700m-ambr-item' }, [ E('span', {}, _('Subscription downlink')), E('strong', {}, down) ]),
				E('div', { 'class': 'mt5700m-ambr-item' }, [ E('span', {}, _('Subscription uplink')), E('strong', {}, up) ])
			])
		]);
	},

	carrierPanel: function(info) {
		var badge = !info.available ? _('Unavailable') : info.active ? _('Aggregating') : info.dual ? _('Dual connectivity') : _('Single carrier');
		var summary = !info.available
			? _('Current carrier information is unavailable.')
			: info.active
				? _('%s is active with %d carriers.').format(info.mode, info.count)
				: info.dual
					? _('%s dual connectivity is active.').format(info.mode || 'EN-DC')
					: _('%s is currently using one carrier.').format(info.mode || _('Mobile network'));

		return E('section', { 'class': 'mt5700m-ca mt-ui-card' }, [
			E('div', { 'class': 'mt5700m-ca-head' }, [
				E('div', {}, [ E('div', { 'class': 'mt5700m-ca-title' }, _('Carrier status')), E('div', { 'class': 'mt5700m-ca-summary' }, summary) ]),
				E('span', { 'class': 'mt5700m-ca-badge' + (info.active || info.dual ? ' active' : '') }, badge)
			]),
			info.available ? E('div', { 'class': 'mt5700m-ca-body' }, [
				E('div', { 'class': 'mt5700m-ca-stats' }, [
					E('div', { 'class': 'mt5700m-ca-stat' }, [ E('div', { 'class': 'mt5700m-ca-stat-label' }, _('Current mode')), E('div', { 'class': 'mt5700m-ca-stat-value' }, info.mode) ]),
					E('div', { 'class': 'mt5700m-ca-stat' }, [ E('div', { 'class': 'mt5700m-ca-stat-label' }, _('Downlink bandwidth')), E('div', { 'class': 'mt5700m-ca-stat-value' }, (info.dlBandwidth || '--') + ' MHz') ]),
					E('div', { 'class': 'mt5700m-ca-stat' }, [ E('div', { 'class': 'mt5700m-ca-stat-label' }, _('Uplink bandwidth')), E('div', { 'class': 'mt5700m-ca-stat-value' }, (info.ulBandwidth || '--') + ' MHz') ])
				]),
				E('div', { 'class': 'mt5700m-carriers' }, info.carriers.map(function(carrier) {
					return E('div', { 'class': 'mt5700m-carrier' }, [
						E('div', { 'class': 'mt5700m-carrier-name' }, carrier.radio + ' · ' + carrier.band),
						E('div', { 'class': 'mt5700m-carrier-detail' }, [
							E('div', {}, 'DL ' + carrier.dlFrequency + ' MHz · ' + carrier.dlBandwidth + ' MHz'),
							E('div', {}, 'UL ' + carrier.ulFrequency + ' MHz · ' + carrier.ulBandwidth + ' MHz')
						])
					]);
				})),
				E('p', { 'class': 'mt5700m-ca-note' }, _('Carrier aggregation is scheduled by the mobile network on demand. Only the primary carrier may appear while idle.'))
			]) : null
		]);
	},

	parseSession: function(raw) {
		var ndis = csvValues(controls.section(raw, 'Data session'), '^NDISSTATQRY');
		var dhcp4 = csvValues(controls.section(raw, 'IPv4 lease'), '^DHCP');
		var dhcp6 = csvValues(controls.section(raw, 'IPv6 lease'), '^DHCPV6');
		var flow = csvValues(controls.section(raw, 'Data flow'), '^DSFLOWQRY');
		var mtu = csvValues(controls.section(raw, 'MTU'), '^CGMTU');
		var pdpAddress = csvValues(controls.section(raw, 'PDP address'), '+CGPADDR');
		var capability = controls.pick(controls.section(raw, 'IP capability'), /\^IPV6CAP:\s*(\w+)/, '');
		var capabilityNames = { '1':_('IPv4 only'), '2':_('IPv6 only'), '7':_('IPv4 / IPv6 · same APN'), '0B':_('IPv4 / IPv6 · separate APNs'), '0b':_('IPv4 / IPv6 · separate APNs') };
		var detailed = (controls.section(raw, 'Detailed sessions') || '').split(/\n/).map(function(line) {
			var match = line.match(/^\^DCONNSTAT:\s*(\d+)(?:[,，]["“”]?([^,"“”]*)["“”]?[,，](\d+)[,，](\d+)[,，](\d+)(?:[,，](\d+))?)?/);
			return match ? { cid:match[1], apn:match[2] || '', ipv4:match[3] === '1', ipv6:match[4] === '1', type:match[5] || '', ethernet:match[6] === '1' } : null;
		}).filter(function(item) { return item && item.apn; });

		return {
			ipv4Connected:ndis[0] === '1' && ndis[4] === 'IPV4',
			ipv6Connected:ndis[5] === '1' && ndis[8] === 'IPV6',
			ipv4Address:hexIPv4(dhcp4[0]) || pdpAddress[1] || '',
			ipv4Gateway:hexIPv4(dhcp4[2]),
			ipv4Dns:[ hexIPv4(dhcp4[4]), hexIPv4(dhcp4[5]) ].filter(Boolean).join(' · '),
			ipv6Address:dhcp6[0] && dhcp6[0] !== '::' ? dhcp6[0] : '',
			ipv6Dns:[ dhcp6[4], dhcp6[5] ].filter(function(value) { return value && value !== '::'; }).join(' · '),
			capability:capabilityNames[capability] || capability,
			mtu:mtu[1] && mtu[1] !== '0' ? mtu[1] : _('Network default'),
			currentDuration:hexNumber(flow[0]), currentTx:hexNumber(flow[1]), currentRx:hexNumber(flow[2]),
			totalDuration:hexNumber(flow[3]), totalTx:hexNumber(flow[4]), totalRx:hexNumber(flow[5]),
			maximumDown:dhcp4[6] || dhcp6[6], maximumUp:dhcp4[7] || dhcp6[7], detailed:detailed
		};
	},

	sessionRow: function(label, value) {
		return E('div', { 'class':'mt5700m-session-row' }, [ E('span', {}, label), E('strong', {}, value || '--') ]);
	},

	connectionOverview: function(session) {
		var self = this;
		var rows = [
			self.sessionRow('IPv4', session.ipv4Connected ? _('Connected') : _('Disconnected')),
			self.sessionRow(_('IPv4 address'), session.ipv4Address),
			self.sessionRow(_('IPv4 gateway'), session.ipv4Gateway),
			self.sessionRow(_('IPv4 DNS'), session.ipv4Dns),
			self.sessionRow('IPv6', session.ipv6Connected ? _('Connected') : _('Disconnected')),
			self.sessionRow(_('IPv6 address'), session.ipv6Address),
			self.sessionRow(_('IPv6 DNS'), session.ipv6Dns),
			self.sessionRow(_('IP capability'), session.capability),
			self.sessionRow('MTU', session.mtu)
		];

		if (session.detailed.length)
			session.detailed.forEach(function(item) {
				var type = item.type === '1' ? _('Module application') : item.type === '2' ? _('Host NDIS') : _('Other');
				rows.push(self.sessionRow('CID ' + item.cid + ' · ' + (item.apn || _('Carrier default')), [ item.ipv4 ? 'IPv4' : '', item.ipv6 ? 'IPv6' : '', item.ethernet ? _('Ethernet') : '', type ].filter(Boolean).join(' · ')));
			});

		return E('div', { 'class':'mt5700m-session-grid' }, [
			E('section', { 'class':'mt5700m-overview-card mt-ui-card' }, [
				E('div', { 'class':'mt5700m-card-head' }, [
					E('div', {}, [ E('h3', {}, _('Network session')), E('p', {}, _('Addresses and DNS supplied directly by the MT5700M mobile network session.')) ]),
					E('span', { 'class':'mt5700m-card-badge' + (session.ipv4Connected || session.ipv6Connected ? ' on' : '') }, session.ipv4Connected || session.ipv6Connected ? _('Active') : _('Disconnected'))
				]),
				E('div', { 'class':'mt5700m-session-columns' }, rows)
			]),
			E('section', { 'class':'mt5700m-overview-card mt-ui-card' }, [
				E('div', { 'class':'mt5700m-card-head' }, [ E('div', {}, [ E('h3', {}, _('Module traffic counters')), E('p', {}, _('Session counters reported by MT5700M firmware, independent of the local history below.')) ]), E('span', { 'class':'mt5700m-card-badge' }, _('Module')) ]),
				E('div', { 'class':'mt5700m-session-columns' }, [
					self.sessionRow(_('Current duration'), formatDuration(session.currentDuration)),
					self.sessionRow(_('Current total'), formatBytes(session.currentRx + session.currentTx)),
					self.sessionRow(_('Current received'), formatBytes(session.currentRx)),
					self.sessionRow(_('Current transmitted'), formatBytes(session.currentTx)),
					self.sessionRow(_('Accumulated duration'), formatDuration(session.totalDuration)),
					self.sessionRow(_('Accumulated total'), formatBytes(session.totalRx + session.totalTx)),
					self.sessionRow(_('Network maximum downlink'), formatRate(session.maximumDown)),
					self.sessionRow(_('Network maximum uplink'), formatRate(session.maximumUp))
				]),
				E('div', { 'class':'mt5700m-session-actions' }, E('button', { 'class':'btn', 'click':function() { controls.confirmRun(_('Clear module traffic counters'), _('This permanently clears current and accumulated MT5700M data-flow counters.'), [ 'flow-clear' ]); } }, _('Clear counters')))
			])
		]);
	},

	trafficStat: function(label, value, meta, style) {
		return E('div', { 'class':'mt5700m-traffic-stat ' + (style || '') }, [
			E('div', { 'class':'mt5700m-traffic-label' }, label),
			E('div', { 'class':'mt5700m-traffic-value' }, value),
			E('div', { 'class':'mt5700m-traffic-meta' }, meta)
		]);
	},

	trafficPanel: function(report, interfaceName) {
		var iface = (report.interfaces || []).filter(function(item) { return item.name === interfaceName; })[0] ||
			(report.interfaces || []).filter(function(item) { return item.name === 'eth2'; })[0] || { traffic:{} };
		var traffic = iface.traffic || {};
		var days = sortedTraffic(traffic.day, false), months = sortedTraffic(traffic.month, true);
		var today = currentTraffic(days, false), month = currentTraffic(months, true), lifetime = traffic.total || {};
		var recentDays = days.slice(-7).reverse(), recentMonths = months.slice(-6).reverse();
		var maximum = Math.max.apply(Math, recentDays.map(trafficTotal).concat([ 1 ]));
		var empty = !days.length && !months.length && !trafficTotal(lifetime);
		var dayContent = empty ? E('div', { 'class':'mt5700m-empty' }, _('Statistics appear after the MT5700M data interface has carried traffic for a few minutes.')) : E('div', { 'class':'mt5700m-traffic-days' }, recentDays.map(function(item) {
			var rx = Number(item.rx) || 0, tx = Number(item.tx) || 0;
			return E('div', { 'class':'mt5700m-traffic-day' }, [
				E('span', { 'class':'mt5700m-traffic-date' }, trafficDateKey(item, false).substring(5)),
				E('div', { 'class':'mt5700m-traffic-bars' }, [ E('div', { 'class':'mt5700m-traffic-bar' }, E('i', { 'style':'width:' + Math.max(1, rx / maximum * 100).toFixed(1) + '%' })), E('div', { 'class':'mt5700m-traffic-bar tx' }, E('i', { 'style':'width:' + Math.max(1, tx / maximum * 100).toFixed(1) + '%' })) ]),
				E('div', { 'class':'mt5700m-traffic-values' }, [ E('span', {}, formatBytes(rx)), E('span', {}, formatBytes(tx)) ])
			]);
		}));
		var monthContent = recentMonths.length ? E('div', {}, recentMonths.map(function(item) {
			return E('div', { 'class':'mt5700m-month-row' }, [ E('span', {}, trafficDateKey(item, true)), E('span', {}, [ _('Download') + ' ' + formatBytes(item.rx), E('small', {}, _('Upload') + ' ' + formatBytes(item.tx)) ]), E('strong', {}, formatBytes(trafficTotal(item))) ]);
		})) : E('div', { 'class':'mt5700m-empty' }, _('No monthly history yet'));

		return [
			E('div', { 'class':'mt5700m-traffic-summary' }, [
				this.trafficStat(_('Today'), formatBytes(trafficTotal(today)), _('Download %s · Upload %s').format(formatBytes(today.rx), formatBytes(today.tx)), ''),
				this.trafficStat(_('This month'), formatBytes(trafficTotal(month)), _('Download %s · Upload %s').format(formatBytes(month.rx), formatBytes(month.tx)), 'blue'),
				this.trafficStat(_('All-time total'), formatBytes(trafficTotal(lifetime)), _('Download %s · Upload %s').format(formatBytes(lifetime.rx), formatBytes(lifetime.tx)), 'violet'),
				this.trafficStat(_('Last updated'), trafficUpdated(iface), _('Tracked interface: %s').format(iface.name || interfaceName || 'eth2'), 'slate')
			]),
			E('div', { 'class':'mt5700m-traffic-grid' }, [
				E('section', { 'class':'mt5700m-overview-card mt-ui-card' }, [ E('div', { 'class':'mt5700m-card-head' }, [ E('div', {}, [ E('h3', {}, _('Recent 7 days')), E('p', {}, _('Download and upload recorded on the MT5700M data interface.')) ]), E('span', { 'class':'mt5700m-card-badge' }, _('Local history')) ]), dayContent ]),
				E('section', { 'class':'mt5700m-overview-card mt-ui-card' }, [ E('div', { 'class':'mt5700m-card-head' }, E('div', {}, [ E('h3', {}, _('Monthly usage')), E('p', {}, _('Recent monthly totals retained automatically.')) ])), monthContent ])
			]),
			E('p', { 'class':'mt5700m-privacy-note' }, _('Traffic is counted on the MT5700M physical data interface, includes IPv4 and IPv6, and stays only on this router.'))
		];
	},

	detail: function(label, value) {
		return E('div', { 'class': 'mt5700m-detail' }, [
			E('div', { 'class': 'mt5700m-detail-label' }, label),
			E('div', { 'class': 'mt5700m-detail-value' }, value || _('Unknown'))
		]);
	},

	lockRow: function(label, value) {
		var unlocked = value === '0' || value === '' || value == null;

		return E('div', { 'class': 'mt5700m-lock' }, [
			E('span', { 'class': 'mt5700m-lock-name' }, label),
			E('span', { 'class': 'mt5700m-lock-state' + (unlocked ? '' : ' is-locked') }, unlocked ? _('Not locked') : _('Locked'))
		]);
	},

	actionButton: function(label, command, cssClass, confirmText, recoveryDelay) {
		return E('button', {
			'class': 'btn ' + (cssClass || 'cbi-button'),
			'click': function(ev) {
				var button = ev.currentTarget;
				var run = function() {
					button.disabled = true;
					return fs.exec('/usr/sbin/mt5700m-at', command).then(function(res) {
						if (recoveryDelay) {
							ui.addNotification(null, E('p', {}, _('Restart accepted. The USB interface normally returns in about 22 seconds.')));
							window.setTimeout(function() { window.location.reload(); }, recoveryDelay);
						} else {
							ui.addNotification(null, E('p', {}, _('Command completed.')));
						}
					}, function(err) {
						ui.addNotification(null, E('p', {}, err.message || String(err)), 'danger');
					}).finally(function() {
						button.disabled = false;
					});
				};

				if (!confirmText)
					return run();

				return ui.showModal(_('Confirm Action'), [
					E('p', {}, confirmText),
					E('div', { 'class': 'right' }, [
						E('button', { 'class': 'btn', 'click': ui.hideModal }, _('Cancel')),
						' ',
						E('button', { 'class': 'btn cbi-button-negative', 'click': function() { ui.hideModal(); run(); } }, _('Continue'))
					])
				]);
			}
		}, label);
	},

	render: function(res) {
		var data = this.parseStatus(res);
		var session = this.parseSession(res.session && res.session.stdout || '');
		var reachable = data.reachable === '1';
		var connected = data.connected === '1';
		var rsrp = parseFloat(data.rsrp);
		var rsrq = parseFloat(data.rsrq);
		var sinr = parseFloat(data.sinr);
		var temp = parseFloat(data.temperature);
		var carrierInfo = this.carrierInfo(data);
		var usbStateNames = { normal: _('Normal mode'), upgrade: _('Upgrade mode'), dump: _('Dump mode'), unknown: _('Unknown USB mode'), absent: _('Not detected') };
		var usbState = usbStateNames[data.usb_state] || data.usb_state || _('Not detected');
		var abnormalUsb = data.usb_state === 'upgrade' || data.usb_state === 'dump' || data.usb_state === 'unknown';
		var operator = data.operator || '';
		if (!/[A-Za-z0-9\u4e00-\u9fff]/.test(operator))
			operator = '';
		var channel = data.channel === 'serial'
			? '%s · %s'.format(_('Serial Port'), data.at_port || _('Auto'))
			: '%s · %s:%s'.format(_('Network TCP'), data.host || '192.168.8.1', data.port || '20249');

		return E('div', { 'class': 'mt5700m-page mt-ui-page' }, [
			this.styleNode(),
			controls.styleNode(),
			data.error ? E('div', { 'class': 'alert-message warning mt5700m-alert' }, data.error) : null,
			res.session && res.session.stderr ? E('div', { 'class':'alert-message warning mt5700m-alert' }, res.session.stderr) : null,
			abnormalUsb ? E('div', { 'class': 'alert-message warning mt5700m-alert' }, _('The MT5700M is in %s (USB PID %s). Mobile data and AT management remain unavailable until normal mode returns.').format(usbState, data.usb_pid || '--')) : null,
			E('section', { 'class': 'mt5700m-hero mt-ui-hero' }, [
				E('div', { 'class': 'mt5700m-hero-top' }, [
					E('div', {}, [
						E('div', { 'class': 'mt5700m-eyebrow' }, _('MT5700M dedicated management')),
						E('h2', { 'class': 'mt5700m-title' }, _('MT5700M Module')),
						E('div', { 'class': 'mt5700m-summary' }, !reachable
							? _('The modem did not respond. Check the AT channel and module connection.')
							: connected
								? _('%s network is available and the modem is operating normally.').format(data.sysmode_detail || data.sysmode || '5G')
								: _('The modem is online, but mobile data is not connected.'))
					]),
					E('div', { 'class': 'mt5700m-status' + (connected ? ' is-online' : reachable ? ' is-reachable' : '') }, [
						E('span', { 'class': 'mt5700m-dot' }),
						connected ? _('Connected') : reachable ? _('Module online') : _('Unavailable')
					])
				]),
				E('div', { 'class': 'mt5700m-hero-meta' }, [
					E('div', { 'class': 'mt5700m-meta' }, [ E('span', { 'class': 'mt5700m-meta-label' }, _('Operator')), E('span', { 'class': 'mt5700m-meta-value' }, operator || '--') ]),
					E('div', { 'class': 'mt5700m-meta' }, [ E('span', { 'class': 'mt5700m-meta-label' }, _('Network Mode')), E('span', { 'class': 'mt5700m-meta-value' }, data.sysmode_detail || data.sysmode || '--') ]),
					E('div', { 'class': 'mt5700m-meta' }, [ E('span', { 'class': 'mt5700m-meta-label' }, _('Radio link')), E('span', { 'class': 'mt5700m-meta-value' }, !carrierInfo.available ? '--' : carrierInfo.active ? carrierInfo.mode + ' · ' + carrierInfo.count : carrierInfo.dual ? carrierInfo.mode : _('Single carrier')) ]),
					E('div', { 'class': 'mt5700m-meta' }, [ E('span', { 'class': 'mt5700m-meta-label' }, _('Dialing process')), E('span', { 'class': 'mt5700m-meta-value' }, data.dial_running === '1' ? _('Running') : _('Stopped')) ]),
					E('div', { 'class': 'mt5700m-meta' }, [ E('span', { 'class': 'mt5700m-meta-label' }, _('Network interface')), E('span', { 'class': 'mt5700m-meta-value' }, data.network_interface || '--') ])
				])
			]),
			E('div', { 'class': 'mt5700m-section-title' }, _('Signal and temperature')),
			E('div', { 'class': 'mt5700m-metrics' }, [
				this.metric('RSRP', data.rsrp, 'dBm', this.signalQuality('rsrp', rsrp)),
				this.metric('RSRQ', data.rsrq, 'dB', this.signalQuality('rsrq', rsrq)),
				this.metric('SINR', data.sinr, 'dB', this.signalQuality('sinr', sinr)),
				this.metric(_('Peak sensor temperature'), data.temperature, '°C', this.temperatureQuality(temp))
			]),
			E('div', { 'class':'mt5700m-section-title' }, _('Connection and usage')),
			this.connectionOverview(session),
			E('div', { 'class':'mt5700m-section-title' }, _('Traffic Statistics')),
			E('div', {}, this.trafficPanel(res.traffic || {}, data.network_interface || 'eth2')),
			E('div', { 'class': 'mt5700m-section-title' }, _('Radio link')),
			this.subscriptionPanel(data),
			this.carrierPanel(carrierInfo),
			E('div', { 'class': 'mt5700m-section-title' }, _('Device details')),
			E('div', { 'class': 'mt5700m-content-grid' }, [
				E('section', { 'class': 'mt5700m-panel mt-ui-card' }, [
					E('div', { 'class': 'mt5700m-panel-head' }, _('Module information')),
					E('div', { 'class': 'mt5700m-detail-grid' }, [
						this.detail(_('Model'), data.model || data.manufacturer),
						this.detail(_('Firmware version'), data.revision),
						this.detail(_('USB state'), usbState),
						this.detail(_('USB identity'), data.usb_pid ? '3466:' + data.usb_pid : '--'),
						this.detail(_('Network interface'), data.network_interface),
						this.detail(_('Management backend'), data.backend)
					])
				]),
				E('section', { 'class': 'mt5700m-panel mt-ui-card' }, [
					E('div', { 'class': 'mt5700m-panel-head' }, _('Connection')),
					E('div', { 'class': 'mt5700m-detail-grid' }, [
						this.detail(_('Mobile data'), connected ? _('Connected') : _('Disconnected')),
						this.detail(_('Dialing service'), data.dial_running === '1' ? _('Running') : _('Stopped')),
						this.detail(_('AT Channel'), channel),
						this.detail(_('Configuration'), _('MT5700M dedicated'))
					])
				])
			]),
			E('div', { 'class': 'mt5700m-actions' }, [
				E('button', { 'class': 'btn cbi-button-action', 'click': function() { window.location.reload(); } }, _('Refresh status')),
				this.actionButton(_('Restart Module'), [ 'restart' ], 'cbi-button-negative', _('This will restart the MT5700M module and temporarily interrupt 5G connectivity.'), 24000)
			])
		]);
	},

	handleSave: null,
	handleSaveApply: null,
	handleReset: null
});
