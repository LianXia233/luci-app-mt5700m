'use strict';
'require view';
'require mt5700m.api as api';
'require mt5700m.parser as parser';
'require mt5700m.components as c';

/*
 * MT5700M LuCI — 概览（status）
 * ---------------------------------------------
 * 数据：mt5700m status（manager）+ fs.exec status / advanced session + mt5700m-traffic summary
 * + mt5700m-at cached（StateCache 快照，SWR 首屏）。
 * 无内联样式；信号/载波/地址/模块/SIM/流量/快捷入口全部由组件拼装。
 *
 * 渲染策略（Async Architecture）：
 *   1) 骨架屏立即出（不等待任何后端）；
 *   2) 快照帧：只走非阻塞数据源（daemon StateCache + rpcd），毫秒级到达并
 *      立即完成首屏渲染 —— 这一帧就是页面的"完成态"，不依赖任何 AT 查询；
 *   3) 详情帧：AT 查询在**后台**异步发起（带硬超时），到达后增量替换；
 *      超时或失败时保留快照帧，绝不让页面停在骨架屏。
 *   4) 快照轮询：定期刷新 StateCache，页面持续自更新（无需手动刷新）。
 *
 * 为什么必须这么做：
 *   AT 后端 daemon 以 TIOCEXCL 独占串口，异常情况下 AT 查询会长时间挂起。
 *   rpcd 的 fs.exec 没有超时概念，一旦把 AT 查询放进渲染关键路径，
 *   Promise 永不 settle，页面会永久停在骨架屏；挂起的请求还会占满
 *   uhttpd 的并发进程（本机 -n 3），导致整个 LuCI 无响应。
 *   异步化之后 AT 查询永远不在关键路径上，页面一定能渲染出来。
 */

return view.extend({
	/* 快照轮询间隔（毫秒）：StateCache 由后台采集器持续写入，轮询零 AT 流量 */
	pollIntervalMs: 15000,

	load: function() {
		this.stopPolling();
		// 异步化：首屏只依赖非阻塞数据源。AT 查询不在 load() 里发起。
		//
		// traffic 缓存在 this.trafficCache：轮询刷新时（frameFromSnapshot）
		// 必须复用**上一帧**的流量数据，不能重置为空，否则「IP 流量统计」
		// 会在第一次轮询（15s 后）被清成 0 B。
		var self = this;
		this.managerError = '';
		this.trafficCache = { interfaces: [] };
		this.pending = Promise.all([
			api.managerStatus().catch(function(err) {
				self.managerError = err && err.message || String(err);
				return {};
			}),
			api.trafficSummary().catch(function() { return { interfaces: [] }; }),
			api.cachedSnapshot().catch(function() { return null; })
		]).then(function(results) {
			if (results[1] && results[1].interfaces) self.trafficCache = results[1];
			return { manager: results[0], traffic: results[1], snapshot: results[2] };
		});
		return Promise.resolve();
	},

	/* 快照 / 详情合并成 parseStatus 可消费的 key=value 行。 */
	frameFromSnapshot: function(snapshot, manager, nativeDetail, sessionDetail) {
		var lines = this.mergeStatusLines(nativeDetail || '', this.snapshotLines(snapshot, nativeDetail));
		if (manager) {
			if (manager.at_port) lines.push('at_port=' + manager.at_port);
			if (manager.network) lines.push('network_interface=' + manager.network);
			if (manager.mode) lines.push('mode=' + manager.mode);
			if (manager.connected !== undefined) lines.push('connected=' + (manager.connected ? '1' : '0'));
			if (manager.ipv4_address) lines.push('ipv4_address=' + manager.ipv4_address);
		}
		return {
			manager: manager || {},
			native: { stdout: lines.join('\n'), stderr: '' },
			session: sessionDetail || { stdout: '', stderr: '' },
			traffic: this.trafficCache || { interfaces: [] }
		};
	},

	/*
	 * 以「有值者優先」合併 key=value 行：详情补充快照没有的字段，
	 * 快照则覆盖同名详情值；空字段不会抹掉上一次成功读取的值。
	 */
	mergeStatusLines: function(base, overrides) {
		var values = Object.create(null), order = [];
		function add(line) {
			var pos = line.indexOf('=');
			if (pos < 1) return;
			var key = line.substring(0, pos).trim();
			var value = line.substring(pos + 1);
			if (!key || !value.trim()) return;
			if (!(key in values)) order.push(key);
			values[key] = value;
		}
		String(base || '').split(/\r?\n/).forEach(add);
		(overrides || []).forEach(add);
		return order.map(function(key) { return key + '=' + values[key]; });
	},

	/* StateCache 的不完整 / 空条目不覆盖最近一次有效值。 */
	mergeSnapshot: function(previous, incoming) {
		var merged = Object.assign({}, previous || {});
		if (!incoming || typeof incoming !== 'object') return merged;
		Object.keys(incoming).forEach(function(topic) {
			var entry = incoming[topic];
			if (!entry || typeof entry !== 'object') return;
			var value = entry.value;
			var oldEntry = merged[topic];
			var oldValue = oldEntry && oldEntry.value;
			if (!value || typeof value !== 'object' || Array.isArray(value)) {
				if (!oldEntry && value !== undefined && value !== null && value !== '')
					merged[topic] = entry;
				return;
			}
			var nextValue = Object.assign({}, oldValue && typeof oldValue === 'object' ? oldValue : {});
			Object.keys(value).forEach(function(key) {
				var field = value[key];
				if (field !== undefined && field !== null && field !== '')
					nextValue[key] = field;
			});
			merged[topic] = Object.assign({}, oldEntry || {}, entry, { value: nextValue });
		});
		return merged;
	},

	snapshotDiff: function(previous, next) {
		var changed = [];
		Object.keys(next || {}).forEach(function(topic) {
			var before = previous && previous[topic] ? previous[topic].value : undefined;
			var after = next[topic] ? next[topic].value : undefined;
			if (JSON.stringify(before) === JSON.stringify(after)) return;
			var fields = [];
			if (before && typeof before === 'object' && after && typeof after === 'object') {
				var keys = Object.keys(before).concat(Object.keys(after));
				keys.forEach(function(key) {
					if (fields.indexOf(key) === -1 && JSON.stringify(before[key]) !== JSON.stringify(after[key]))
						fields.push(key);
				});
			} else {
				fields.push('*');
			}
			changed.push({ topic: topic, fields: fields });
		});
		return changed;
	},

	stateWarnings: function(state) {
		return [ state.detailError, state.snapshotError, state.trafficError, state.managerError ].filter(Boolean);
	},

	frameFromState: function(state) {
		var frame = this.frameFromSnapshot(
			state.snapshot, state.manager, state.nativeDetail, state.sessionDetail);
		frame.traffic = state.traffic || { interfaces: [] };
		frame.uiErrors = [];
		if (state.detailError)
			frame.uiErrors.push({ message: state.detailError, retry: state.retryDetails });
		if (state.snapshotError)
			frame.uiErrors.push({ message: state.snapshotError, retry: state.retryPoll });
		if (state.trafficError)
			frame.uiErrors.push({ message: state.trafficError, retry: state.retryPoll });
		if (state.managerError)
			frame.uiErrors.push({
				message: _('Dial manager status is temporarily unavailable; connection details may be incomplete.') + ' ' + state.managerError,
				retry: state.retryManager
			});
		frame.onRetry = state.retryPoll || state.onRefresh || null;
		frame.onRefresh = state.onRefresh || null;
		return frame;
	},

	stopPolling: function(poll) {
		var current = poll || this._pollState;
		if (!current) return;
		current.stopped = true;
		if (current.timer) clearTimeout(current.timer);
		if (current.mountTimer) clearTimeout(current.mountTimer);
		if (current.observer) current.observer.disconnect();
		current.timer = null;
		current.mountTimer = null;
		current.observer = null;
		if (this._pollState === current) this._pollState = null;
	},

	schedulePolling: function(poll) {
		var self = this;
		if (!poll || poll.stopped || poll.timer || poll.inFlight) return;
		poll.timer = setTimeout(function() {
			poll.timer = null;
			self.runPoll(poll.holder, poll.state);
		}, this.pollIntervalMs);
	},

	/* 快照与流量只读轮询：串行调度，上一轮未完成时不会再发新请求。 */
	runPoll: function(holder, state) {
		var self = this;
		var poll = state && state.polling;
		if (!poll || poll.stopped) return Promise.resolve();
		if (poll.timer) {
			clearTimeout(poll.timer);
			poll.timer = null;
		}
		if (poll.inFlight) return poll.inFlight;
		if (!document.body.contains(holder)) {
			if (poll.seenMounted) this.stopPolling(poll);
			else this.schedulePolling(poll);
			return Promise.resolve();
		}
		poll.seenMounted = true;

		var beforeWarnings = this.stateWarnings(state).join('\n');
		var request = Promise.all([
			api.cachedSnapshot().catch(function() { return null; }),
			api.trafficSummary().catch(function() { return null; })
		]).then(function(result) {
			if (poll.stopped || !document.body.contains(holder)) {
				if (poll.seenMounted) self.stopPolling(poll);
				return;
			}

			var oldSnapshot = state.snapshot || {};
			if (result[0] && typeof result[0] === 'object') {
				state.snapshot = self.mergeSnapshot(oldSnapshot, result[0]);
				state.snapshotError = '';
			} else {
				state.snapshotError = _('Shared modem status could not be refreshed; any available cached values are retained.');
			}

			var changedTopics = self.snapshotDiff(oldSnapshot, state.snapshot || {});
			var trafficChanged = false;
			var report = result[1];
			var hasReport = report && Array.isArray(report.interfaces);
			var hadTraffic = state.traffic && Array.isArray(state.traffic.interfaces) && state.traffic.interfaces.length > 0;
			if (hasReport && (report.interfaces.length > 0 || !hadTraffic)) {
				trafficChanged = JSON.stringify(state.traffic || {}) !== JSON.stringify(report);
				state.traffic = report;
				state.trafficError = '';
				self.trafficCache = report;
			} else if (!hasReport || hadTraffic) {
				state.trafficError = _('Traffic statistics could not be refreshed; any previously displayed values are retained.');
			}

			var regions = [];
			function addRegions(names) {
				(names || []).forEach(function(name) {
					if (regions.indexOf(name) === -1) regions.push(name);
				});
			}
			changedTopics.forEach(function(change) {
				var fields = change.fields || [];
				var has = function(name) { return fields.indexOf('*') !== -1 || fields.indexOf(name) !== -1; };
				if (change.topic === 'signal') {
					if ([ 'rsrp', 'rsrq', 'sinr', 'rssi' ].some(has)) addRegions([ 'signal' ]);
					if (has('sysmode')) addRegions([ 'hero', 'facts', 'carrier', 'sim' ]);
				} else if (change.topic === 'network') {
					if ([ 'operator', 'sysmode', 'sysmode_detail' ].some(has)) addRegions([ 'facts', 'sim' ]);
					if (has('sysmode') || has('sysmode_detail')) addRegions([ 'hero' ]);
				} else if (change.topic === 'temperature' && has('average')) {
					addRegions([ 'signal' ]);
				} else if (change.topic === 'modem') {
					addRegions([ 'module' ]);
				} else if (change.topic === 'sim') {
					addRegions([ 'sim' ]);
				} else if (change.topic === 'cell' || change.topic === 'endc') {
					addRegions([ 'carrier' ]);
				}
			});
			if (trafficChanged) regions.push('traffic');
			if (beforeWarnings !== self.stateWarnings(state).join('\n')) regions.push('alerts');
			if (regions.length)
				self.updateRegions(holder, self.frameFromState(state), regions);
		}).catch(function() {
			if (poll.stopped || !document.body.contains(holder)) return;
			state.snapshotError = _('Shared modem status could not be refreshed; any available cached values are retained.');
			if (self.stateWarnings(state).join('\n') !== beforeWarnings)
				self.updateRegions(holder, self.frameFromState(state), [ 'alerts' ]);
		});

		var settled = request.then(function() {}, function() {});
		var final = settled.then(function() {
			if (poll.inFlight === final) poll.inFlight = null;
			if (!poll.stopped) self.schedulePolling(poll);
		});
		poll.inFlight = final;
		return final;
	},

	startPolling: function(holder, state) {
		var self = this;
		this.stopPolling();
		var poll = { holder: holder, state: state, timer: null, mountTimer: null,
			observer: null, inFlight: null, stopped: false, seenMounted: false };
		this._pollState = poll;
		state.polling = poll;

		function begin() {
			if (poll.stopped) return;
			if (!document.body.contains(holder)) {
				self.stopPolling(poll);
				return;
			}
			poll.seenMounted = true;
			if (typeof MutationObserver !== 'undefined') {
				poll.observer = new MutationObserver(function() {
					if (!document.body.contains(holder)) self.stopPolling(poll);
				});
				poll.observer.observe(document.body, { childList: true, subtree: true });
			}
			self.schedulePolling(poll);
		}

		if (document.body.contains(holder)) begin();
		else poll.mountTimer = setTimeout(begin, 0);
	},

	retryManager: function(holder, state) {
		var self = this;
		if (state.managerPending) return state.managerPending;
		var pending = api.managerStatus().then(function(manager) {
			if (!document.body.contains(holder)) return;
			state.manager = manager || {};
			state.managerError = '';
			self.updateRegions(holder, self.frameFromState(state), [ 'hero', 'facts', 'module', 'traffic', 'alerts' ]);
		}, function(err) {
			if (!document.body.contains(holder)) return;
			state.managerError = err && err.message || String(err);
			self.updateRegions(holder, self.frameFromState(state), [ 'alerts' ]);
		}).then(function() {
			state.managerPending = null;
		});
		state.managerPending = pending;
		return pending;
	},

	refreshDetail: function(holder, state) {
		var self = this;
		if (state.detailPending) return state.detailPending;
		var beforeWarnings = this.stateWarnings(state).join('\n');
		var oldNative = state.nativeDetail || '';
		var oldSession = state.sessionDetail && state.sessionDetail.stdout || '';
		function failedResponse(err) {
			return { stdout: '', stderr: err && err.message || String(err) };
		}
		var pending = Promise.all([
			api.atStatus().catch(failedResponse),
			api.atSession().catch(failedResponse)
		]).then(function(result) {
			if (!document.body.contains(holder)) return;
			var native = result[0] || { stdout: '', stderr: '' };
			var session = result[1] || { stdout: '', stderr: '' };
			var errors = [];
			var nativeChanged = false, sessionChanged = false;

			if (native.stderr) errors.push(native.stderr);
			else if (native.stdout) {
				state.nativeDetail = self.mergeStatusLines(oldNative, native.stdout).join('\n');
				nativeChanged = state.nativeDetail !== oldNative;
			}
			if (session.stderr) errors.push(session.stderr);
			else if (session.stdout) {
				state.sessionDetail = { stdout: session.stdout, stderr: '' };
				sessionChanged = session.stdout !== oldSession;
			}

			state.detailError = errors.length
				? _('Some modem details could not be refreshed. Existing values are retained.') + ' ' + errors.join(' · ')
				: '';
			var regions = [];
			if (nativeChanged)
				regions = regions.concat([ 'hero', 'facts', 'signal', 'carrier', 'module', 'sim' ]);
			if (sessionChanged) regions.push('address');
			if (beforeWarnings !== self.stateWarnings(state).join('\n')) regions.push('alerts');
			if (regions.length)
				self.updateRegions(holder, self.frameFromState(state), regions);
		}).catch(function(err) {
			if (!document.body.contains(holder)) return;
			state.detailError = _('Some modem details could not be refreshed. Existing values are retained.') + ' ' + (err && err.message || String(err));
			if (beforeWarnings !== self.stateWarnings(state).join('\n'))
				self.updateRegions(holder, self.frameFromState(state), [ 'alerts' ]);
		}).then(function() {
			state.detailPending = null;
		});
		state.detailPending = pending;
		return pending;
	},

		/*
	 * 把 StateCache 快照折叠成 parseStatus 可消费的 key=value 行。
	 * 仅映射既有 UI 字段，且跳过空值 —— parseStatus 之后照常做
	 * temperature 清洗、connected 推导等，行为与完整帧一致。
	 */
	snapshotLines: function(snapshot, nativeDetail) {
		snapshot = snapshot || {};
		var map = {
			signal: { sysmode: 'sysmode', rsrp: 'rsrp', rsrq: 'rsrq', sinr: 'sinr', rssi: 'rssi' },
			network: { operator: 'operator', sysmode: 'sysmode', sysmode_detail: 'sysmode_detail' },
			temperature: { average: 'temperature' },
			modem: { manufacturer: 'manufacturer', model: 'product_name', revision: 'revision', imei: 'imei' },
			sim: { status: 'sim_state', iccid: 'iccid', imsi: 'imsi' }
		};
		var lines = [];
		Object.keys(map).forEach(function(topic) {
			var value = snapshot[topic] && snapshot[topic].value;
			if (!value || typeof value !== 'object') return;
			var fields = map[topic];
			Object.keys(fields).forEach(function(src) {
				var val = value[src];
				if (val === undefined || val === null || val === '') return;
				lines.push(fields[src] + '=' + val);
			});
		});

		/*
		 * The cell collector already stores the serving band/channel in the
		 * shared daemon cache. Mirror the existing parser's carrier_N shape so
		 * polling can update that card without issuing another AT query.
		 */
		var cell = snapshot.cell && snapshot.cell.value;
		if (cell && typeof cell === 'object') {
			var band = cell.band == null ? '' : String(cell.band);
			var channel = cell.channel == null ? '' : String(cell.channel);
			var radio = cell.sysmode || (snapshot.signal && snapshot.signal.value && snapshot.signal.value.sysmode) || 'NR';
			var bandwidth = cell.dlBandwidth == null ? '' : String(cell.dlBandwidth);
			var previousCarrier = null;
			String(nativeDetail || '').split(/\r?\n/).some(function(line) {
				if (line.indexOf('carrier_1=') !== 0) return false;
				previousCarrier = line.substring('carrier_1='.length).split('|');
				return true;
			});
			if (band || channel) {
				// Keep the last known frequency only for the same radio, band, and
				// ARFCN; never carry it over to a different serving cell.
				var sameCell = previousCarrier && previousCarrier[0] === radio &&
					previousCarrier[1] === 'B' + band && previousCarrier[2] === channel;
				var dlFrequency = sameCell ? previousCarrier[3] || '' : '';
				var ulFrequency = sameCell ? previousCarrier[5] || '' : '';
				lines.push('carrier_count=1');
				lines.push('carrier_1=' + radio + '|B' + band + '|' + channel + '|' + dlFrequency + '|' + bandwidth + '|' + ulFrequency + '|0|' + bandwidth);
				lines.push('ca_active=0');
				lines.push('ca_mode=' + radio);
				if (bandwidth) {
					lines.push('ca_dl_bandwidth=' + bandwidth);
					lines.push('ca_ul_bandwidth=' + bandwidth);
				}
			}
		}

		var endc = snapshot.endc && snapshot.endc.value;
		if (endc && typeof endc === 'object' && endc.established !== undefined && endc.established !== null)
			lines.push('dc_active=' + (Number(endc.established) === 1 ? '1' : '0'));
		return lines;
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
			E('div', { 'class': 'mt-circular-gauges-grid' }, [
				c.svgCircularGauge(data.rsrq, -25, -3, ' dB', 'RSRQ', quality.cls),
				c.svgCircularGauge(data.sinr, -10, 30, ' dB', 'SINR', quality.cls),
				c.svgCircularGauge(data.temperature, 20, 80, '°C', _('Temperature'), 'accent')
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

		var primaryBand = info.carriers[0] ? info.carriers[0].band : '5G';
		var carrierSvg = c.svgCarrier(primaryBand, info.carriers.length);

		return c.card(_('Carrier status'), _('Carrier aggregation and bandwidth'), [
			E('div', { 'class': 'mt-gauge-head', 'style': 'align-items:center' }, [
				E('div', { 'style': 'display:flex;align-items:center;gap:12px' }, [
					carrierSvg,
					E('div', {}, [
						E('span', { 'class': 'mt-gauge-value', 'style': 'font-size:24px' }, headline),
						E('div', { 'class': 'mt-muted' }, info.mode || _('Mobile network'))
					])
				]),
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
			/*
			 * IPv6 是 39 字符无空格长串，窄容器下默认只在 `:` 处断行，
			 * 实测会把末位（如 `...a74:16d` + `8`）孤立到下一行。
			 * `anywhere` 允许在任意字符间断，代价是可能把一个 hextet 劈开；
			 * 对「是否已连接」这个判断无影响，可读性明显更好。
			 */
			c.row('IPv6', [ E('div', { 'class': 'mt-muted', 'style': 'text-align:right' }, session.ipv6Connected ? _('Connected') : _('Not assigned')), E('strong', { 'style': 'word-break:break-all' }, session.ipv6Address || '--') ]),
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
			E('div', { 'class': 'mt-gauge-head', 'style': 'margin-bottom:10px;align-items:center' }, [
				E('div', { 'style': 'display:flex;align-items:center;gap:10px' }, [
					c.svgTrafficArrows(),
					E('span', { 'class': 'mt-scan-note' }, _('Last updated') + ' · ' + parser.trafficUpdated(iface))
				]),
				E('span', { 'class': 'mt-traffic-legend' }, [
					E('span', {}, [ E('i', { 'class': 'mt-traffic-legend-dot' }), _('Download') ]),
					E('span', {}, [ E('i', { 'class': 'mt-traffic-legend-dot active' }), _('Upload') ])
				])
			]),
			E('div', { 'class': 'mt-traffic-stats' }, [ stat(_('Today'), today), stat(_('This month'), month), stat(_('All-time total'), lifetime) ]),
			E('div', { 'class': 'mt-traffic-days' }, dayRows)
		]);
	},

	/* ---------- 状态视图与局部渲染 ---------- */

	statusViewData: function(res) {
		var data = parser.parseStatus(res);
		var session = parser.parseSession(res.session && res.session.stdout || '');
		var opInfo = parser.operatorInfo(data.operator);
		var operator = opInfo.name;
		if (!/[A-Za-z0-9\u4e00-\u9fff]/.test(operator)) operator = '';
		data.operator = operator;
		return {
			data: data,
			session: session,
			reachable: data.reachable === '1',
			connected: data.connected === '1',
			carrierInfo: parser.carrierInfo(data),
			opInfo: opInfo,
			operator: operator,
			usbNames: { upgrade: _('Upgrade mode'), dump: _('Dump mode'), unknown: _('Unknown USB mode') },
			abnormalUsb: data.usb_state === 'upgrade' || data.usb_state === 'dump' || data.usb_state === 'unknown'
		};
	},

	liveRegion: function(node, name) {
		if (node) node.setAttribute('data-live-region', name);
		return node;
	},

	alertsRegion: function(res, viewData) {
		var notices = [];
		if (viewData.data.error)
			notices.push(E('div', { 'class': 'alert-message warning' }, viewData.data.error));
		if (res.session && res.session.stderr)
			notices.push(E('div', { 'class': 'alert-message warning' }, res.session.stderr));
		(res.uiErrors || []).forEach(function(error) {
			var message = typeof error === 'string' ? error : error.message;
			var retry = typeof error === 'string' ? res.onRetry : error.retry;
			var content = [ E('span', {}, message) ];
			if (retry)
				content.push(' ', c.btn(_('Retry'), retry, { 'cls': 'mt-session-action' }));
			notices.push(E('div', { 'class': 'alert-message warning' }, content));
		});
		if (viewData.abnormalUsb) {
			notices.push(E('div', { 'class': 'alert-message warning' },
				_('The MT5700M is in %s. Mobile data and AT management are unavailable until normal mode returns.')
					.format(viewData.usbNames[viewData.data.usb_state])));
		}
		return this.liveRegion(E('div', { 'class': 'mt-live-alerts' }, notices), 'alerts');
	},

	heroRegion: function(res, viewData) {
		var self = this;
		var connected = viewData.connected, reachable = viewData.reachable;
		return this.liveRegion(c.hero(_('OVERVIEW'), _('MT5700M Module'),
			!reachable ? _('The modem did not respond. Check the module connection.') : connected ? _('Mobile network is connected and ready.') : _('The module is online, but mobile data is not connected.'),
			[
				E('div', { 'class': 'mt-conn-state' }, [
					c.svgStatusPulse(connected ? 'ok' : reachable ? 'warn' : 'bad', 18),
					E('span', { 'class': 'mt-conn-state-text' }, connected ? _('Connected') : reachable ? _('Module online') : _('Unavailable'))
				]),
				E('a', { 'class': 'mt-hero-btn', 'href': '/5700/', 'target': '_blank', 'rel': 'noopener' }, [ c.svgWebUiIcon(), _('WebUI') ]),
				E('button', { 'class': 'mt-hero-btn mt-hero-refresh', 'click': function() {
					if (res.onRefresh) return res.onRefresh();
				} }, [ c.svgRefreshIcon(), _('Refresh') ])
			], null, c.svgTower({ active: reachable, status: connected ? 'ok' : reachable ? 'warn' : 'bad' })), 'hero');
	},

	factsRegion: function(viewData) {
		var data = viewData.data, opInfo = viewData.opInfo, operator = viewData.operator;
		return this.liveRegion(E('div', { 'class': 'mt-facts-grid', 'style': 'margin-bottom:14px' }, [
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
				E('div', { 'class': 'mt-facts-value mt-facts-value--inline' }, [
					opInfo.logo ? E('img', { 'src': opInfo.logo, 'alt': operator, 'class': 'mt-facts-logo' }) : null,
					E('span', {}, operator || '--')
				])
			]),
			E('div', { 'class': 'mt-facts-cell' }, [
				E('div', { 'class': 'mt-facts-label' }, _('AT port')),
				E('div', { 'class': 'mt-facts-value' }, data.at_port || '--')
			])
		]), 'facts');
	},

	createRegion: function(res, viewData, name) {
		if (name === 'alerts') return this.alertsRegion(res, viewData);
		if (name === 'hero') return this.heroRegion(res, viewData);
		if (name === 'facts') return this.factsRegion(viewData);
		if (name === 'signal') return this.liveRegion(this.signalCard(viewData.data), 'signal');
		if (name === 'carrier') return this.liveRegion(this.carrierCard(viewData.carrierInfo), 'carrier');
		if (name === 'address') return this.liveRegion(this.addressCard(viewData.session), 'address');
		if (name === 'module') return this.liveRegion(this.moduleCard(viewData.data), 'module');
		if (name === 'sim') return this.liveRegion(this.simCard(viewData.data), 'sim');
		if (name === 'traffic')
			return this.liveRegion(this.trafficPanel(res.traffic || {}, viewData.data.network_interface || 'eth2'), 'traffic');
		return null;
	},

	/* 只更新指定区域节点；不重建 mt-page，其余区域与表单状态保持不动。 */
	updateRegions: function(holder, res, regions) {
		if (!document.body.contains(holder)) return;
		var self = this;
		var viewData = this.statusViewData(res);
		var updated = [];
		(regions || []).forEach(function(name) {
			if (updated.indexOf(name) !== -1) return;
			updated.push(name);
			var current = holder.querySelector('[data-live-region="' + name + '"]');
			if (!current || !current.parentNode) return;
			var next = self.createRegion(res, viewData, name);
			if (next) current.parentNode.replaceChild(next, current);
		});
	},

	/* ---------- 渲染 ---------- */

	// 骨架屏 → StateCache 首帧（不等 AT）→ 详情与轮询只补对应区域。
	render: function() {
		var self = this;
		var holder = E('div', { 'class': 'mt-view' });
		holder.appendChild(c.skeletonPage(6));

		this.pending.then(function(data) {
			var state = {
				manager: data.manager || {},
				managerError: self.managerError || '',
				managerPending: null,
				traffic: data.traffic || { interfaces: [] },
				snapshot: data.snapshot || {},
				nativeDetail: '',
				sessionDetail: null,
				detailError: '',
				snapshotError: data.snapshot ? '' : _('Shared modem status is temporarily unavailable. Showing available data.'),
				trafficError: '',
				detailPending: null,
				polling: null,
				onRefresh: null
			};
			state.retryDetails = function() { return self.refreshDetail(holder, state); };
			state.retryPoll = function() { return self.runPoll(holder, state); };
			state.retryManager = function() { return self.retryManager(holder, state); };
			state.onRefresh = function() {
				self.refreshDetail(holder, state);
				if (state.managerError) self.retryManager(holder, state);
				return self.runPoll(holder, state);
			};
			holder.replaceChildren(self.renderPage(self.frameFromState(state)));
			self.startPolling(holder, state);
			self.refreshDetail(holder, state);
		}, function(err) {
			holder.replaceChildren(E('div', { 'class': 'mt-page' }, [
				E('div', { 'class': 'alert-message error' }, String(err && err.message || err))
			]));
		});

		return holder;
	},

	renderPage: function(res) {
		var viewData = this.statusViewData(res);
		var statusGrid = E('div', { 'class': 'mt-grid' }, [
			this.createRegion(res, viewData, 'signal'),
			this.createRegion(res, viewData, 'carrier'),
			this.createRegion(res, viewData, 'address')
		]);
		var moduleGrid = E('div', { 'class': 'mt-grid' }, [
			this.createRegion(res, viewData, 'module'),
			this.createRegion(res, viewData, 'sim')
		]);
		return E('div', { 'class': 'mt-page' }, [
			c.cssLink(),
			this.createRegion(res, viewData, 'alerts'),
			this.createRegion(res, viewData, 'hero'),
			this.createRegion(res, viewData, 'facts'),
			statusGrid,
			moduleGrid,
			this.createRegion(res, viewData, 'traffic'),
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
