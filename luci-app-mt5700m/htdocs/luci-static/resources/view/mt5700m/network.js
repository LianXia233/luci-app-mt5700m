'use strict';
'require view';
'require dom';
'require ui';
'require mt5700m.api as api';
'require mt5700m.parser as parser';
'require mt5700m.components as c';

/*
 * MT5700M LuCI — 无线与小区（network）
 * ---------------------------------------------
 * 数据：fs.exec network（信号/小区/注册/锁频/MCS）+ advanced radio（频段/接入架构）
 *       + 后端路由（诊断区块/邻区：cell.neighbors、beam.ssb 等）+ cellscan（模态）
 * 分块：A) 渲染与英雄指标  B) 无线诊断（radioDiagnostics / SSB / 邻区）
 *       C) 频段勾选与锁频面板（bandChecklist / lockPanel）
 * 全部解析在 parser.js，全部 UI 原语在 components.js；本文件仅保留页面级组装。
 */

// 表格行（技术细节用）
function tr(label, value) {
	return E('tr', {}, [ E('td', {}, label), E('td', {}, value || '--') ]);
}

// SSB 波束卡片（design system .mt-beam-*）
function beamCard(beam) {
	var rsrp = parseFloat(beam.rsrp);
	var cls = isNaN(rsrp) ? 'unknown' : c.signalColorClass(beam.rsrp, 'rsrp');
	return E('div', { 'class': 'mt-beam-card ' + cls }, [
		E('div', { 'class': 'mt-beam-card-name' }, _('SSB-%s').format(beam.id)),
		E('div', { 'class': 'mt-beam-card-freq' }, beam.rsrp ? beam.rsrp + ' dBm' : '--')
	]);
}

/*
 * 频段号 → 页面标签（'B3' / 'n78'）。
 *
 * 频段知识（ARFCN → 频段号）在后端 modules/cell 与 modules/beam 共用的
 * core::radio 表里；前端只负责加前缀。取不到频段号时沿用旧版
 * arfcnToBand 的回退：显示 RAT 名（'LTE' / 'NR'）。
 */
function bandLabel(band, rat) {
	if (band === undefined || band === null || band === '')
		return rat || '';
	return (rat === 'NR' ? 'n' : 'B') + band;
}

/*
 * `api.signal.get` 载荷 → 仪表卡片的三条读数。
 *
 * 指标集合与旧版 parseServingCell 完全一致（NR: RSRP/RSRQ/SINR，LTE:
 * RSRP/RSRQ/RSSI，WCDMA: RSCP/RXLEV/ECIO，其他 RAT: RSRP/RSRQ/SINR），缺读数
 * 仍留空串由 svgCircularGauge 显示 `--`；数值改由 modules::signal 解码
 * `^HCSQ` 得到（索引 → dBm/dB 的换算只在后端一处），页面不再切 `^MONSC` 的
 * 第 8..10 个字段——那是同一批测量值的第二份 JS 解码。
 */
function signalMetrics(signal, rat) {
	var sig = signal || {};
	var pick = function(label, value, unit) {
		var v = (value === undefined || value === null) ? '' : String(value);
		return { label: label, value: v, unit: unit };
	};
	var mode = String(rat || sig.sysmode || '').toUpperCase();
	if (mode.indexOf('NR') === 0)
		return [ pick('RSRP', sig.rsrp, 'dBm'), pick('RSRQ', sig.rsrq, 'dB'), pick('SINR', sig.sinr, 'dB') ];
	if (mode.indexOf('LTE') === 0)
		return [ pick('RSRP', sig.rsrp, 'dBm'), pick('RSRQ', sig.rsrq, 'dB'), pick('RSSI', sig.rssi, 'dBm') ];
	if (mode.indexOf('WCDMA') === 0)
		return [ pick('RSCP', sig.rscp, 'dBm'), pick('RXLEV', sig.rssi, 'dBm'), pick('ECIO', sig.ecio, 'dB') ];
	return [ pick('RSRP', sig.rsrp, 'dBm'), pick('RSRQ', sig.rsrq, 'dB'), pick('SINR', sig.sinr, 'dB') ];
}

/*
 * `api.cell.neighbors` 的一行 → cellLockCard 的既有形状。
 *
 * 后端给的是领域值：PCI 十进制（十六进制解码在 modules/cell）、频段号、
 * 越界读数的 1/8 换算、空读数直接缺字段；这里只做「缺字段 → 空串」和
 * 「频段号 → 标签」，交给卡片的字段与旧版 parseMonnc 同名同型。
 */
function neighborCard(cell) {
	var rat = (cell.type === 'NR' || cell.rat === 'NR') ? 'NR' : 'LTE';
	function str(value) { return (value === undefined || value === null) ? '' : String(value); }
	return {
		rat: rat,
		arfcn: str(cell.arfcn),
		pci: str(cell.pci),
		rsrp: str(cell.rsrp),
		rsrq: str(cell.rsrq),
		sinr: str(cell.sinr),
		rxlev: str(cell.rxlev),
		band: bandLabel(cell.band, rat)
	};
}

/*
 * `api.beam.ssb`（modules/beam 的 Rust 解码）→ ssbPanel 的既有形状。
 *
 * 前端只做改名与展示清洗：`^NRSSBID` 的字段偏移、255/32767 空槽过滤、
 * 邻区计数探测都只在后端一处实现。signalBar/beamCard 拿到的仍是旧版
 * 同名字段（同名同型），所以面板 DOM 与迁移前逐字节一致。
 */
function ssbStateFromPayload(payload) {
	if (!payload || typeof payload !== 'object' || !payload.servingCell)
		return null;
	var cell = payload.servingCell;
	function str(value) { return (value === undefined || value === null) ? '' : String(value); }
	return {
		arfcn: str(cell.arfcn), cid: str(cell.cid), pci: str(cell.pci),
		rsrp: str(cell.rsrp), sinr: str(cell.sinr), ta: str(cell.ta),
		band: bandLabel(cell.band, 'NR'),
		beams: (cell.ssbs || []).map(function(b) {
			return { id: str(b.ssbId), rsrp: str(b.rsrp) };
		}),
		neighbours: (payload.neighborCells || []).map(function(nb) {
			return {
				pci: str(nb.pci), arfcn: str(nb.arfcn),
				rsrp: parser.cleanSignal(nb.rsrp), sinr: parser.cleanSignal(nb.sinr),
				band: bandLabel(nb.band, 'NR')
			};
		})
	};
}

// 小区扫描结果（模态）——服务小区 / 邻区 / 频段扫描
function renderCellScan(raw, payloads) {
	var sections = [];
	var monsc = parser.parseMonsc(parser.section(raw, 'Serving cell: AT^MONSC') || raw);
	if (monsc && monsc.rat) {
		var scsKhz = monsc.scs ? ({ '0':'15', '1':'30', '2':'60', '3':'120', '4':'240' }[monsc.scs] || '?') : '';
		var scRatLabel = monsc.rat;
		var scBand = parser.arfcnToBand(monsc.arfcn, scRatLabel);
		var scBars = [ c.signalBar(monsc.rsrp, 'rsrp', 'RSRP'), c.signalBar(monsc.rsrq, 'rsrq', 'RSRQ') ];
		if (monsc.rat === 'LTE')
			scBars.push(c.signalBar(monsc.rssi, 'rsrp', 'RSSI'));
		else
			scBars.push(c.signalBar(monsc.sinr, 'sinr', 'SINR'));
		sections.push(E('section', { 'class': 'mt-card' }, [
			E('h4', { 'class': 'mt-card-title', 'style': 'margin:0 0 12px' }, _('Serving cell')),
			E('div', { 'class': 'mt-ssb-serving' }, [
				E('div', { 'class': 'mt-ssb-serving-head' }, [
					E('span', { 'class': 'mt-ssb-serving-title' }, scRatLabel + (scBand ? ' · ' + scBand : '') + (monsc.pci ? ' · PCI:' + monsc.pci : '') + (monsc.arfcn ? ' · ARFCN:' + monsc.arfcn : '')),
					E('span', { 'class': 'mt-ssb-serving-meta' }, (monsc.cellId || '') + (scsKhz ? ' · SCS:' + scsKhz + 'kHz' : ''))
				])
			].concat(scBars))
		]));
	}
	var monnc = (((payloads || {}).neighbors || {}).cells || []).map(neighborCard);
	if (monnc.length) {
		var scRat = (monsc && monsc.rat) || '';
		var nrNbs = monnc.filter(function(nb) { return nb.rat === 'NR'; });
		var lteNbs = monnc.filter(function(nb) { return nb.rat === 'LTE'; });
		var activeNbs, activeLabel, otherNbs = [], otherLabel = '';
		if (nrNbs.length) {
			activeNbs = nrNbs;
			activeLabel = _('NR neighbour cells (%d)');
			if (lteNbs.length) { otherNbs = lteNbs; otherLabel = _('LTE neighbour cells (%d)'); }
		} else if (lteNbs.length) {
			activeNbs = lteNbs;
			activeLabel = _('LTE neighbour cells (%d)');
		} else {
			activeNbs = monnc;
			activeLabel = _('Neighbour cells (%d)');
		}
		if (activeNbs.length) {
			var nbCards = activeNbs.map(function(nb, i) {
				var ratType = nb.rat === 'NR' ? 'nr' : nb.rat === 'LTE' ? 'lte' : '';
				return c.cellLockCard(nb, i, ratType, nb.band);
			});
			sections.push(E('section', { 'class': 'mt-card' }, [
				E('h4', { 'class': 'mt-card-title', 'style': 'margin:0 0 12px' }, activeLabel.format(activeNbs.length)),
				E('div', { 'class': 'mt-lock-cell-grid' }, nbCards)
			]));
		}
		if (otherNbs.length && otherLabel) {
			var otherCards = otherNbs.map(function(nb, i) {
				var ratType = nb.rat === 'NR' ? 'nr' : nb.rat === 'LTE' ? 'lte' : '';
				return c.cellLockCard(nb, i, ratType, nb.band);
			});
			sections.push(E('section', { 'class': 'mt-card' }, [
				E('h4', { 'class': 'mt-card-title', 'style': 'margin:0 0 12px' }, otherLabel.format(otherNbs.length)),
				E('div', { 'class': 'mt-lock-cell-grid' }, otherCards)
			]));
		}
	}
	var cellscanSection = parser.section(raw, 'Frequency scan: AT^CELLSCAN');
	var cellscanLines = (cellscanSection || '').split(/\n/).filter(function(l) { return l.trim() && l.trim() !== 'OK'; });
	var hasCellScanData = cellscanLines.some(function(l) { return l.indexOf('^CELLSCAN:') === 0; });
	var hasCellScanError = cellscanLines.some(function(l) { return l.indexOf('ERROR') !== -1; });
	if (hasCellScanData) {
		sections.push(E('section', { 'class': 'mt-card' }, [
			E('h4', { 'class': 'mt-card-title', 'style': 'margin:0 0 12px' }, _('Frequency scan')),
			E('pre', { 'class': 'mt-raw mt-scan-raw' }, cellscanSection)
		]));
	} else if (hasCellScanError) {
		sections.push(E('section', { 'class': 'mt-card' }, [
			E('h4', { 'class': 'mt-card-title', 'style': 'margin:0 0 12px' }, _('Frequency scan')),
			E('div', { 'class': 'mt-scan-note' }, _('Frequency scan is not available while the module is camped on a cell (%s).').format('+CME ERROR: 3'))
		]));
	}
	if (!sections.length)
		return E('div', {}, E('div', { 'class': 'alert-message warning' }, _('No scan data received.')));
	return E('div', { 'class': 'mt-scan-results' }, sections);
}

return view.extend({
	load: function() {
		// 请求发起即返回，不阻塞首屏；render() 等 pending 填充。
		//
		// 状态区块（信号/服务小区/注册/运营商/RRC/温度）与两种锁模型全部走统一
		// 路由：后端 modules/* 解码 AT 应答，页面只做展示映射。旧版这里切
		// `fs.exec network` 的文本帧（^HCSQ/^MONSC/+CEREG/+COPS/^RRCSTAT/
		// temperature=），那是同一批 AT 应答的第二份 JS 解码。
		// `atRadio` 仍是文本帧：无线偏好/5G 能力几行还没迁完（见 migration）。
		this.pending = Promise.all([
			api.atRadio(),
			api.route('signal.get'),
			api.route('cell.get'),
			api.route('registration.get'),
			api.route('network.get'),
			api.route('network.rrc'),
			api.route('system.temperature'),
			api.route('network.lock_get', { rat: 'lte' }),
			api.route('network.lock_get', { rat: 'nr' })
		]);
		return Promise.resolve();
	},

	/* ---------- C) 频段勾选 / 锁频面板（组件已封装，页面仅做布局） ---------- */

	lockPanel: c.lockPanel,

	/* `network.rrc` 载荷 -> 既有文案（0..3 的名称 + 98/99 的驻留后缀）。 */
	rrcText: function(rrc) {
		if (!rrc)
			return '';
		var labels = [ _('Idle'), _('Connected'), _('Inactive'), _('Invalid') ];
		var text = (rrc.state === undefined || rrc.state === null)
			? '' : (labels[Number(rrc.state)] !== undefined ? labels[Number(rrc.state)] : String(rrc.state));
		if (rrc.camped === 98)
			text += ' · ' + _('Camped');
		else if (rrc.camped === 99)
			text += ' · ' + _('Not camped');
		return text;
	},

	/* `network.lock_get` 载荷 -> 锁状态文案（0 = 未锁；取不到保持旧的 '--'）。 */
	lockStateText: function(lock) {
		if (!lock)
			return '--';
		return lock.lock_type === 0 ? _('Not locked') : _('Locked');
	},

	/* ---------- B) 无线诊断 ---------- */

	/*
	 * 诊断区块全部走统一 API 路由（modules/* 的后端解码），页面只做展示映射，
	 * 不再按标签切 CLI 文本帧、也不再用正则取字段。每个 route() 都不会 reject，
	 * 失败返回 null，对应那一行显示空值 —— 与迁移前「取不到就留空」一致。
	 */
	fetchDiagnostics: function() {
		return Promise.all([
			api.route('cell.neighbors'),
			api.route('beam.ssb'),
			api.route('modem.mcs'),
			api.route('modem.nr_txpower'),
			api.route('qos.get'),
			api.route('modem.endc'),
			api.route('ca.get'),
			api.route('registration.get'),
			api.route('network.ims')
		]);
	},

	/* 后端 4 位登网状态 -> 页面文案（1/5 = 已注册，与旧版一致）。 */
	registrationText: function(payload) {
		if (!payload)
			return '';
		return (payload.state === 1 || payload.state === 5) ? _('Registered') : _('Not registered');
	},

	/* `modem.nr_txpower` 第一个载波的读数：999/-0 视为无读数（后端已置空）。 */
	nrTxPowerRows: function(payload) {
		var carrier = (payload && payload.carriers && payload.carriers[0]) || {};
		var dbm = function(value) {
			return (value === undefined || value === null) ? '' : value + ' dBm';
		};
		return {
			pusch: dbm(carrier.pusch),
			pucch: dbm(carrier.pucch),
			freq: (carrier.freq === undefined || carrier.freq === null || carrier.freq === 0)
				? '' : (Number(carrier.freq) / 1000).toFixed(1) + ' MHz'
		};
	},

	/* ---------- 行渲染 ---------- */

	radioDiagnostics: function(payloads) {
		var ssbInfo = ssbStateFromPayload(payloads.ssb);
		var mcs = payloads.mcs || {};
		var txPowerRows = this.nrTxPowerRows(payloads.txPower);
		var qos = payloads.qos || {};
		var endc = payloads.endc || {};
		var ca = payloads.ca || {};
		var monnc = ((payloads.neighbors || {}).cells || []).map(neighborCard);
		var nrMonnc = monnc.filter(function(nb) { return nb.rat === 'NR'; });
		var lteMonnc = monnc.filter(function(nb) { return nb.rat === 'LTE'; });
		var diagNb = nrMonnc.length ? nrMonnc : lteMonnc;
		var extra = [ this.ssbPanel(ssbInfo) ];
		if (diagNb.length)
			extra.push(this.lockNeighbourSection(
				(nrMonnc.length ? _('NR neighbour cells (%d)') : _('LTE neighbour cells (%d)')).format(diagNb.length),
				diagNb));
		return E('div', {}, [
			E('div', { 'class': 'mt-grid', 'style': 'margin-top:12px' }, [
				E('section', { 'class': 'mt-card' }, [
					E('h3', { 'class': 'mt-card-title' }, _('Radio link details')),
					this.row(_('Uplink modulation'), c.mcsDetailNode(mcs.uplink)), this.row(_('Downlink modulation'), c.mcsDetailNode(mcs.downlink)),
					this.row(_('QoS class'), qos.qci ? 'QCI ' + qos.qci : ''), this.row(_('NR PUSCH power'), txPowerRows.pusch),
					this.row(_('NR PUCCH power'), txPowerRows.pucch), this.row(_('NR transmit frequency'), txPowerRows.freq)
				]),
				E('section', { 'class': 'mt-card' }, [
					E('h3', { 'class': 'mt-card-title' }, _('5G beam and service')),
					this.row(_('LTE secondary carriers'), payloads.ca ? String(ca.lte_secondary_count) : ''), this.row(_('NSA secondary connections'), payloads.ca ? String(ca.secondary_connection_count) : ''),
					this.row(_('NR neighbour cells'), ssbInfo ? String(ssbInfo.neighbours.length) : ''),
					this.row(_('Data registration'), this.registrationText(payloads.registration)),
					this.row(_('IMS registration'), payloads.ims ? (payloads.ims.registered === 1 ? _('Registered') : payloads.ims.registered !== undefined ? _('Not registered') : '') : ''),
					this.row(_('LTE-NR dual connectivity'), payloads.endc ? (endc.available === 1 ? _('Enabled') : endc.available !== undefined ? _('Disabled') : '') : '')
				])
			])
		].concat(extra));
	},

	/* 扫频弹窗的渲染入口（导出以便测试脚本直接驱动，与 radioDiagnostics 同级） */
	renderCellScan: renderCellScan,

	lockNeighbourSection: function(title, list) {
		var cards = list.map(function(nb, i) {
			return c.cellLockCard(nb, i, nb.rat === 'NR' ? 'nr' : 'lte', nb.band, nb.rat === 'NR');
		});
		return E('section', { 'class': 'mt-card', 'style': 'margin-top:12px' }, [
			E('h3', { 'class': 'mt-card-title' }, title),
			cards.length ? E('div', { 'class': 'mt-lock-cell-grid' }, cards)
				: E('div', { 'class': 'mt-row' }, [ E('span', { 'class': 'mt-muted' }, ''), E('strong', {}, _('None reported')) ])
		]);
	},

	ssbPanel: function(info) {
		if (!info) {
			return E('section', { 'class': 'mt-card', 'style': 'margin-top:12px' }, [
				E('h3', { 'class': 'mt-card-title' }, _('SSB information')),
				E('div', { 'class': 'mt-row' }, [ E('span', { 'class': 'mt-muted' }, _('NR SSB measurement')), E('strong', {}, _('Not available')) ])
			]);
		}
		var scBand = info.band;
		var serving = E('div', { 'class': 'mt-ssb-serving' }, [
			E('div', { 'class': 'mt-ssb-serving-head' }, [
				E('span', { 'class': 'mt-ssb-serving-title' }, _('Serving cell') + (scBand ? ' · ' + scBand : '')),
				E('span', { 'class': 'mt-ssb-serving-meta' },
					(info.pci && info.pci !== '65535' ? 'PCI:' + info.pci : '') +
					(info.arfcn && info.arfcn !== '4294967295' ? ' · ARFCN:' + info.arfcn : '')
				)
			]),
			c.signalBar(parser.ssbValue(info.rsrp, [ '32767' ]), 'rsrp', 'RSRP'),
			c.signalBar(parser.ssbValue(info.sinr, [ '32767' ]), 'sinr', 'SINR')
		]);
		var beamGrid;
		if (info.beams.length) {
			beamGrid = E('div', { 'class': 'mt-beam-grid' }, info.beams.map(function(b) { return beamCard(b); }));
		} else {
			beamGrid = E('div', { 'class': 'mt-row' }, [ E('span', { 'class': 'mt-muted' }, _('Serving beams')), E('strong', {}, _('No measurement')) ]);
		}
		var nbSection;
		if (info.neighbours.length) {
			nbSection = E('div', {}, [
				E('h4', { 'style': 'margin:14px 0 8px;font-size:13px' }, _('NR neighbour cells (%d)').format(info.neighbours.length)),
				E('div', { 'class': 'mt-lock-cell-grid' }, info.neighbours.map(function(nb, i) {
					return c.cellLockCard(nb, i, 'nr', nb.band, true);
				}))
			]);
		} else {
			nbSection = E('div', { 'style': 'margin-top:10px' }, [ E('h4', { 'style': 'font-size:13px;margin:0 0 6px' }, _('NR neighbour cells')), E('div', { 'class': 'mt-row' }, [ E('span', { 'class': 'mt-muted' }, ''), E('strong', {}, _('None reported')) ]) ]);
		}
		return E('section', { 'class': 'mt-card', 'style': 'margin-top:12px' }, [
			E('h3', { 'class': 'mt-card-title' }, _('SSB information')),
			serving,
			E('h4', { 'style': 'margin:12px 0 8px;font-size:13px' }, _('Serving SSB beams (%d)').format(info.beams.length)),
			beamGrid,
			nbSection
		]);
	},

	/* ---------- A) 渲染 ---------- */

	row: function(label, value) {
		// value 可能为富 DOM 节点（mcsDetailNode / signalBar），直接渲染；标量才包 <strong>
		var valueNode = c.isNode(value) ? value : E('strong', {}, (value == null || value === '') ? '--' : String(value));
		return E('div', { 'class': 'mt-row' }, [ E('span', { 'class': 'mt-muted' }, label), valueNode ]);
	},

	// 渐进渲染：骨架屏立即显示，数据到达后整体替换（后端慢不挡前端）
	render: function() {
		var self = this;
		var holder = E('div', { 'class': 'mt-view' });
		holder.appendChild(c.skeletonPage(5));
		this.contentReady = this.pending.then(function(data) {
			holder.replaceChildren(self.renderPage(data));
			return data;
		}, function(err) {
			holder.replaceChildren(E('div', { 'class': 'mt-page' }, [
				E('div', { 'class': 'alert-message error' }, String(err && err.message || err))
			]));
		});
		return holder;
	},

	renderPage: function(results) {
		var radioSettings = results[0] || {};
		var radioRaw = radioSettings.stdout || '';
		var signal = results[1] || {}, cell = results[2] || {}, registration = results[3] || {};
		var network = results[4] || {}, rrc = results[5] || {}, temperaturePayload = results[6] || {};
		var lteLock = results[7], nrLock = results[8];
		var rrcState = this.rrcText(rrc);
		var registered = registration.state === 1 || registration.state === 5;
		var opInfo = parser.operatorInfo(network.operator);
		var operatorName = opInfo.name;
		var temperature = (temperaturePayload.peak === undefined || temperaturePayload.peak === null)
			? '' : String(temperaturePayload.peak);
		// 「Technical details」折叠块：这里以前直接倾倒整段 `mt5700m-at network`
		// 文本帧（^HCSQ/^MONSC/^RRCSTAT/+CEREG/+COPS 加温度行）。文本帧已不再
		// 经过前端，改为倾倒本页消费的路由载荷——同一批读数，领域模型形态。
		var technical = [
			[ 'signal.get', signal ], [ 'cell.get', cell ], [ 'registration.get', registration ],
			[ 'network.get', network ], [ 'network.rrc', rrc ], [ 'system.temperature', temperaturePayload ],
			[ 'network.lock_get (lte)', lteLock ], [ 'network.lock_get (nr)', nrLock ]
		].map(function(pair) {
			return '===== api.' + pair[0] + ' =====' + '\n' +
				JSON.stringify(pair[1] === undefined ? null : pair[1], null, 2);
		}).join('\n' + '\n');
		// 仪表量程按 label 取（见下）。RAT 以服务小区为准（旧版 cell.rat），
		// 读数来自 signal 载荷。
		cell.metrics = signalMetrics(signal, cell.sysmode);
		// 仪表量程表：按 label 查表，而不是按下标硬编码。上面 signalMetrics
		// 给出的读数随 RAT 变化（NR: RSRP/RSRQ/SINR，LTE: RSRP/RSRQ/RSSI，
		// WCDMA: RSCP/RXLEV/ECIO），下标固定会在 LTE/WCDMA 下量程错配、
		// 指针顶到刻度外。量程取自 MT5700M 手册的典型取值区间。
		var gaugeScale = [
			{ label: 'RSRP', unit: 'dBm', min: -120, max: -70, cls: 'accent' },
			{ label: 'RSRQ', unit: 'dB',  min: -25,  max: -3,  cls: 'accent' },
			{ label: 'SINR', unit: 'dB',  min: -10,  max: 30,  cls: 'accent' },
			{ label: 'RSSI', unit: 'dBm', min: -120, max: -60, cls: 'accent' },
			{ label: 'RSCP', unit: 'dBm', min: -120, max: -25, cls: 'accent' },
			{ label: 'RXLEV', unit: 'dBm', min: -120, max: -60, cls: 'accent' },
			{ label: 'ECIO', unit: 'dB',  min: -25,  max: 10,  cls: 'accent' }
		];
		var lteLockState = this.lockStateText(lteLock);
		var nrLockState = this.lockStateText(nrLock);
		var systemValues = parser.matchValues(parser.section(radioRaw, 'Radio mode'), '^SYSCFGEX');
		var radioCode = systemValues[0] || '';
		var wcdmaMask = systemValues[1] || '3FFFFFFF';
		var roamValue = systemValues[2] || '1';
		var serviceDomain = systemValues[3] || '2';
		var lteMask = systemValues[4] || '7FFFFFFFFFFFFFFF';
		var radioLabels = {
			'00': _('Automatic'), '01': 'GSM', '02': 'WCDMA', '03': 'LTE', '08': '5G NR',
			'0302': 'LTE / WCDMA', '030201': 'LTE / WCDMA / GSM',
			'0803': '5G NR / LTE', '080302': '5G NR / LTE / WCDMA'
		};
		var radioMode = radioLabels[radioCode] ? radioLabels[radioCode] + ' · ' + radioCode : radioCode;
		var radioModeSelect = c.select([
			['080302',_('5G NR / LTE / WCDMA (recommended)')],['0803',_('5G NR / LTE')],['08',_('5G NR only')],
			['03',_('LTE only')],['0302',_('LTE / WCDMA')],['02','WCDMA']
		], radioCode || '080302');
		var roaming = c.select([['0',_('Home network only')],['1',_('Allow roaming')]], roamValue);
		var service = c.select([['1',_('Data service only')],['2',_('Voice and data service')]], serviceDomain);
		var wcdmaBands = c.bandChecklist([['400000','B1 · 2100 MHz'],['2000000000000','B8 · 900 MHz']], wcdmaMask, '3FFFFFFF');
		var lteBands = c.bandChecklist([
			['1','B1'],['4','B3'],['10','B5'],['80','B8'],['200000000','B34'],
			['2000000000','B38'],['4000000000','B39'],['8000000000','B40'],['10000000000','B41']
		], lteMask, '7FFFFFFFFFFFFFFF');
		var accessValues = parser.matchValues(parser.section(radioRaw, '5G access mode'), '^C5GOPTION');
		var accessCode = accessValues.slice(0, 3).join(',');
		var accessPreset = c.select([
			['option23',_('SA + NSA (Option 2 + 3)')],['option2',_('SA only (Option 2)')],['option3',_('NSA only (Option 3)')]
		], accessCode === '1,0,1' ? 'option2' : accessCode === '0,1,0' ? 'option3' : 'option23');
		var ca = parser.pick(parser.section(radioRaw, 'NR carrier aggregation'), /\^NRRCCAPQRY:\s*3,(\d+)/, '');
		var vonr = parser.pick(parser.section(radioRaw, 'VoNR'), /\^NRRCCAPQRY:\s*2,(\d+)/, '');
		var dssMatch = parser.section(radioRaw, 'DSS').match(/\^NRRCCAPQRY:\s*5,(\d+),(\d+)/);
		var caEnabled = c.select([['1',_('Enabled')],['0',_('Disabled')]], ca);
		var vonrMode = c.select([['0',_('Disabled')],['1','FR1 VoNR'],['2','FR2 VoNR'],['3','FR1 + FR2 VoNR']], vonr);
		var dssRate = c.select([['0',_('Keep factory capability')],['1',_('Force capability off')]], dssMatch ? dssMatch[1] : '0');
		var dssDmrs = c.select([['0',_('Keep factory capability')],['1',_('Force capability off')]], dssMatch ? dssMatch[2] : '0');
		var diagnosticHost = E('div', { 'class': 'mt-diag-host' }, E('div', { 'class': 'alert-message notice' }, _('Loading detailed radio diagnostics…')));
		var self = this;
		window.setTimeout(function() {
			if (!document.body.contains(diagnosticHost)) return;
			self.fetchDiagnostics().then(function(results) {
				var payloads = {
					neighbors: results[0], ssb: results[1], mcs: results[2], txPower: results[3], qos: results[4],
					endc: results[5], ca: results[6], registration: results[7], ims: results[8]
				};
				if (document.body.contains(diagnosticHost))
					dom.content(diagnosticHost, self.radioDiagnostics(payloads));
			}, function(err) {
				if (document.body.contains(diagnosticHost))
					dom.content(diagnosticHost, E('div', { 'class': 'alert-message warning' }, err.message || String(err)));
			});
		}, 0);
		var radioControls = E('section', { 'class': 'mt-card', 'style': 'margin-top:20px' }, [
			E('div', { 'class': 'mt-card-head' }, [
				E('h3', { 'class': 'mt-card-title' }, _('Radio preferences')),
				E('p', { 'class': 'mt-card-desc' }, _('5G service capabilities reported by the MT5700M. Keep the carrier defaults unless compatibility troubleshooting requires a change.'))
			]),
			radioSettings.stderr ? E('div', { 'class': 'alert-message warning' }, radioSettings.stderr) : null,
			E('div', { 'class': 'mt-grid' }, [
				c.card(_('Network access policy'), _('Select radio priority, roaming and the service domain. These values are applied together as required by the MT5700M manual.'), [
					c.formRow(_('Radio access order'), radioModeSelect),
					c.formRow(_('Roaming policy'), roaming),
					c.formRow(_('Service domain'), service)
				]),
				c.bandPanel(_('WCDMA bands'), _('Select the WCDMA bands the module may use.'), wcdmaBands),
				c.bandPanel(_('LTE bands'), _('Select the LTE bands the module may use.'), lteBands),
				E('section', { 'class': 'mt-card' }, [
				/*
				 * mt-advanced-actions--stacked：说明文字在上、主按钮在下的竖排。
				 *
				 * 原先这里是内联 `style: 'flex-direction:column;align-items:stretch'`。
				 * 移动端 style.css 有一条 `.mt-advanced-actions > * { flex: 1 1 140px }`
				 * （本意是让 row 方向的按钮组「至少 140px 宽」），它同样命中了这个
				 * 竖排容器 —— 主轴变成纵向，flex-basis 就作用在**高度**上，
				 * 结果「应用网络与频段设置」这个单行按钮在手机上被撑成
				 * 264×140 的方块（实测 rect）。
				 *
				 * 现在竖排语义收进 class，CSS 里对 --stacked 单独声明
				 * `flex: 0 0 auto`，高度回到内容高度，横向仍铺满。
				 */
				E('div', { 'class': 'mt-advanced-actions mt-advanced-actions--stacked' }, [
					E('p', { 'class': 'mt-scan-note' }, _('Keep all bands selected for normal use. Restricting bands can prevent registration when travelling.')),
					E('button', { 'type': 'button', 'class': 'btn cbi-button-apply', 'click': function() {
						var selectedWcdma = c.selectedBandMask(wcdmaBands, '3FFFFFFF');
						var selectedLte = c.selectedBandMask(lteBands, '7FFFFFFFFFFFFFFF');
						if (!selectedWcdma || !selectedLte)
							return ui.addNotification(null, E('p', {}, _('Select at least one WCDMA band and one LTE band.')), 'warning');
						c.confirmRun(_('Change network policy'), _('The module may lose service if the selected radio technology or bands are unavailable.'), [ 'advanced-set', 'radio-policy', radioModeSelect.value, selectedWcdma, roaming.value, service.value, selectedLte ], true);
					} }, _('Apply network and band settings'))
				])
			]),
				c.card(_('5G access architecture'), _('Choose whether the module may use standalone 5G, non-standalone 5G, or both.'), [
					c.formRow(_('5G access mode'), accessPreset),
					E('div', { 'class': 'mt-scan-note' }, _('The MT5700M manual requires an airplane-mode cycle before this setting and a module restart afterwards. The cycle is handled automatically; restart when ready.')),
					c.actionBar(c.btn(_('Apply 5G access mode'), function() {
						c.confirmRun(_('Change 5G access mode'), _('Mobile service will disconnect briefly while the module enters airplane mode.'), [ 'advanced-set', '5g-access', accessPreset.value ], true);
					}))
				]),
				c.card(_('5G service capabilities'), _('Carrier aggregation and voice capability advertised by the module.'), [
					c.stateRow(_('Current radio mode'), radioMode),
					c.formRow(_('NR carrier aggregation capability'), caEnabled),
					c.actionBar(c.btn(_('Apply carrier aggregation'), function() {
						c.confirmRun(_('NR carrier aggregation capability'), _('Apply the selected carrier aggregation capability?'), [ 'advanced-set', 'carrier-aggregation', caEnabled.value ], true);
					})),
					c.formRow(_('VoNR mode'), vonrMode),
					c.actionBar(c.btn(_('Apply VoNR mode'), function() {
						c.confirmRun(_('VoNR mode'), _('Apply the selected VoNR capability?'), [ 'advanced-set', 'vonr', vonrMode.value ], true);
					}))
				]),
				c.card(_('DSS compatibility'), _('Restrict optional DSS capabilities only when required by the mobile network.'), [
					c.formRow(_('DSS rate matching capability'), dssRate),
					c.formRow(_('Additional DMRS capability'), dssDmrs),
					E('div', { 'class': 'mt-scan-note' }, _('Force capability off is a compatibility override. Keep the factory capability for normal operation.')),
					c.actionBar(c.btn(_('Apply DSS settings'), function() {
						c.confirmRun('DSS', _('Apply the selected DSS capability restrictions?'), [ 'advanced-set', 'dss', dssRate.value, dssDmrs.value ], true);
					}))
				])
			])
		]);

		return E('div', { 'class': 'mt-page' }, [
			c.cssLink(),
			// 页面顶部这条告警以前报的是 `network` 文本帧的 stderr。状态区块
			// 改走路由后没有 stderr（某个路由取不到就是那一行留空，与其它
			// 路由消费方一致），本页只剩 `radio` 这一次 CLI 调用，因此由它报错。
			radioSettings.stderr ? E('div', { 'class': 'alert-message warning' }, radioSettings.stderr) : null,
			c.hero(_('NETWORK AND CELL'), operatorName, _('Serving-cell and registration information reported by the modem.'), [
				E('div', { 'class': 'mt-conn-state' }, [
					c.svgStatusPulse(registered ? 'ok' : 'bad', 18),
					c.badge(registered ? _('Registered') : _('Not registered'), registered ? 'ok' : 'warn')
				])
			], null, c.svgTower({ active: registered, status: registered ? 'ok' : 'bad' })),
			// 仪表量程按 label 取，而不是按下标硬编码：cell.metrics
			// 随 RAT 变化（NR 是 RSRP/RSRQ/SINR，LTE 是 RSRP/RSRQ/RSSI，
			// WCDMA 是 RSCP/RXLEV/ECIO），下标固定会在 LTE/WCDMA 下把量程配错，
			// 指针会顶到刻度外。unit 同样由 parser 提供，不在此硬编码。
			// 原实现调用的是 c.circularGaugeCard()，该函数在组件库中根本不存在
			// （导出名是 svgCircularGauge），抛 TypeError 中断整个页面渲染。
			E('div', { 'class': 'mt-circular-gauges-grid', 'style': 'margin-bottom:18px' },
				gaugeScale
					.filter(function(g) {
						return cell.metrics.some(function(m) { return m.label === g.label; });
					})
					.map(function(g) {
						var m = cell.metrics.filter(function(x) { return x.label === g.label; })[0];
						return c.svgCircularGauge(
							parseFloat(m.value), g.min, g.max,
							' ' + (m.unit || g.unit),
							_(g.label === 'Temperature' ? 'Temperature' : g.label),
							g.cls
						);
					})
					.concat([
						c.svgCircularGauge(parseFloat(temperature), 20, 80, '°C',
							_('Temperature'), 'accent')
					])
			),
			E('div', { 'class': 'mt-grid' }, [
				E('section', { 'class': 'mt-card' }, [
					E('h3', { 'class': 'mt-card-title' }, _('Serving cell')),
					this.row(_('Radio access'), cell.sysmode || signal.sysmode),
					this.row('MCC / MNC', cell.mcc && cell.mnc ? '%s / %s'.format(cell.mcc, cell.mnc) : ''),
					this.row('ARFCN', cell.channel),
					this.row('PCI', cell.pci),
					this.row(_('Cell ID'), cell.cid),
					this.row('TAC / LAC', cell.lac),
					cell.scs !== undefined ? this.row(_('SCS type'), cell.scs + ' · ' + ([ '15', '30', '60', '120', '240' ][Number(cell.scs)] || '?') + ' kHz') : null,
					this.row(_('Registration'), registered ? (registration.state === 5 ? _('Roaming') : _('Home network')) : _('Not registered'))
				]),
				E('section', { 'class': 'mt-card' }, [
					E('h3', { 'class': 'mt-card-title' }, _('Radio status')),
					this.row(_('Operator'), operatorName),
					this.row(_('RRC state'), rrcState),
					this.row(_('LTE Lock'), lteLockState),
					this.row(_('NR Lock'), nrLockState)
				])
			]),
			diagnosticHost,
			E('div', { 'class': 'mt-advanced-actions' }, [
				E('button', { 'class': 'btn cbi-button-action', 'click': function() { window.location.reload(); } }, _('Refresh status')),
				E('button', { 'class': 'btn cbi-button', 'click': function() {
					return ui.showModal(_('Confirm Action'), [
						E('p', {}, _('Cell scan may take some time and can briefly increase modem load.')),
						E('div', { 'class': 'right' }, [ E('button', { 'class': 'btn', 'click': ui.hideModal }, _('Cancel')), ' ', E('button', { 'class': 'btn cbi-button-apply', 'click': function() {
							ui.hideModal();
							/*
							 * 扫频是后端的长任务（几分钟），这条调用只负责「取回当前结果」：
							 * 还没有结果时会顺带发起一次扫描，然后轮询 cellscan-result，
							 * 扫完再取一次，弹窗里呈现的内容与旧版同步扫频完全一致。
							 */
							var showScan = function(scan, payloads) {
								var body = scan.stdout ? renderCellScan(scan.stdout, payloads) : E('div', { 'class': 'alert-message warning' }, _('No response.'));
								ui.showModal(_('Cell Scan'), [ body, E('div', { 'class': 'right', 'style': 'margin-top:14px' }, E('button', { 'class': 'btn', 'click': ui.hideModal }, _('Close'))) ]);
							};
							// 邻区与扫频帧并发取：邻区走 cell.neighbors（与 WebUI 同一份解码），
							// 帧里只剩服务小区与 ^CELLSCAN 原文。
							Promise.all([ api.atCellscan(), api.route('cell.neighbors') ]).then(function(both) {
								var scan = both[0], neighbors = both[1];
								showScan(scan, { neighbors: neighbors });
								var waited = 0;
								var poll = window.setInterval(function() {
									waited += 3000;
									api.atCellscanResult().then(function(res) {
										var state = null;
										try { state = JSON.parse((res && res.stdout) || 'null'); } catch (e) { state = null; }
										// 没有 state 说明后端读不到结果，别再空转。
										if (!state || !state.running || waited > 600000) {
											window.clearInterval(poll);
											if (state && !state.running) api.atCellscan().then(function(next) { showScan(next, { neighbors: neighbors }); }, function() {});
										}
									}, function() { window.clearInterval(poll); });
								}, 3000);
							}, function(err) {
								ui.addNotification(null, E('p', {}, err.message || _('Cell scan failed.')), 'danger');
							});
						} }, _('Continue')) ])
					]);
				} }, _('Cell Scan'))
			]),
			c.details(_('Technical details'), null, E('pre', { 'class': 'mt-raw' }, technical || _('No response.'))),
			radioControls,
			E('section', { 'class': 'mt-card', 'style': 'margin-top:20px' }, [
				E('div', { 'class': 'mt-card-head' }, [
					E('h3', { 'class': 'mt-card-title' }, _('Frequency and cell selection')),
					E('p', { 'class': 'mt-card-desc' }, _('Advanced controls for limiting LTE or 5G NR bands, frequencies and cells. Leave these unlocked for normal automatic network selection.'))
				])
			]),
			E('div', { 'class': 'mt-grid' }, [ this.lockPanel(_('LTE network'), 'lte', lteLock), this.lockPanel(_('5G NR network'), 'nr', nrLock) ])
		]);
	},

	handleSave: null,
	handleSaveApply: null,
	handleReset: null
});
