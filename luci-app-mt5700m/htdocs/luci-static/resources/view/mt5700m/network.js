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
 *       + advanced radio-diagnostics（懒加载）+ cellscan（模态）
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

// 小区扫描结果（模态）——服务小区 / 邻区 / 频段扫描
function renderCellScan(raw) {
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
	var monnc = parser.parseMonnc(parser.section(raw, 'Neighbour cells: AT^MONNC') || raw);
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
				var band = parser.arfcnToBand(nb.arfcn, nb.rat);
				return c.cellLockCard(nb, i, ratType, band);
			});
			sections.push(E('section', { 'class': 'mt-card' }, [
				E('h4', { 'class': 'mt-card-title', 'style': 'margin:0 0 12px' }, activeLabel.format(activeNbs.length)),
				E('div', { 'class': 'mt-lock-cell-grid' }, nbCards)
			]));
		}
		if (otherNbs.length && otherLabel) {
			var otherCards = otherNbs.map(function(nb, i) {
				var ratType = nb.rat === 'NR' ? 'nr' : nb.rat === 'LTE' ? 'lte' : '';
				var band = parser.arfcnToBand(nb.arfcn, nb.rat);
				return c.cellLockCard(nb, i, ratType, band);
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
		return Promise.all([
			api.atNetwork(),
			api.atRadio()
		]);
	},

	/* ---------- C) 频段勾选 / 锁频面板（组件已封装，页面仅做布局） ---------- */

	lockPanel: c.lockPanel,

	/* ---------- B) 无线诊断 ---------- */

	radioDiagnostics: function(raw) {
		var uplinkMcsText = parser.section(raw, 'Uplink MCS');
		var downlinkMcsText = parser.section(raw, 'Downlink MCS');
		var txPower = parser.matchValues(parser.section(raw, 'NR transmit power'), '^NTXPOWER');
		var ssbRaw = parser.section(raw, 'NR SSB beam');
		var ssb = parser.matchValues(ssbRaw, '^NRSSBID');
		var qos = parser.matchValues(parser.section(raw, 'QoS'), '+CGEQOSRDP');
		var dataRegistration = parser.matchValues(parser.section(raw, 'Data registration'), '+C5GREG');
		var ims = parser.matchValues(parser.section(raw, 'IMS registration'), '+CIREG');
		var endc = parser.matchValues(parser.section(raw, 'Dual connectivity'), '^LENDC');
		var lteSecondary = parser.countLines(parser.section(raw, 'LTE secondary cells'), '^CASCELLINFO');
		var nsaSecondary = parser.countLines(parser.section(raw, 'NSA secondary cells'), '^MONSSC: NR');
		var ssbInfo = parser.parseNrsSbid(ssbRaw);
		var monnc = parser.parseMonnc(parser.section(raw, 'Neighbour cells') || '');
		var nrMonnc = monnc.filter(function(nb) { return nb.rat === 'NR'; });
		var lteMonnc = monnc.filter(function(nb) { return nb.rat === 'LTE'; });
		var diagNb = nrMonnc.length ? nrMonnc : lteMonnc;
		var extra = [ this.ssbPanel(ssbInfo) ];
		if (diagNb.length)
			extra.push(this.lockNeighbourSection(
				(nrMonnc.length ? _('NR neighbour cells (%d)') : _('LTE neighbour cells (%d)')).format(diagNb.length),
				diagNb, nrMonnc.length ? 'nr' : 'lte'));
		return E('div', {}, [
			E('div', { 'class': 'mt-grid', 'style': 'margin-top:12px' }, [
				E('section', { 'class': 'mt-card' }, [
					E('h3', { 'class': 'mt-card-title' }, _('Radio link details')),
					this.row(_('Uplink modulation'), c.mcsDetailNode(uplinkMcsText)), this.row(_('Downlink modulation'), c.mcsDetailNode(downlinkMcsText)),
					this.row(_('QoS class'), qos[1] ? 'QCI ' + qos[1] : ''), this.row(_('NR PUSCH power'), txPower[0] && txPower[0] !== '999' ? txPower[0] + ' dBm' : ''),
					this.row(_('NR PUCCH power'), txPower[1] && txPower[1] !== '999' ? txPower[1] + ' dBm' : ''), this.row(_('NR transmit frequency'), txPower[4] && txPower[4] !== '0' ? (Number(txPower[4]) / 1000).toFixed(1) + ' MHz' : '')
				]),
				E('section', { 'class': 'mt-card' }, [
					E('h3', { 'class': 'mt-card-title' }, _('5G beam and service')), this.row(_('LTE secondary carriers'), String(lteSecondary)), this.row(_('NSA secondary connections'), String(nsaSecondary)),
					this.row(_('NR neighbour cells'), ssbInfo ? String(ssbInfo.neighbours.length) : (ssb.length > 6 ? '0' : '')),
					this.row(_('Data registration'), dataRegistration[1] === '1' || dataRegistration[1] === '5' ? _('Registered') : dataRegistration.length ? _('Not registered') : ''),
					this.row(_('IMS registration'), ims[1] === '1' ? _('Registered') : ims.length ? _('Not registered') : ''), this.row(_('LTE-NR dual connectivity'), endc[0] === '1' ? _('Enabled') : endc.length ? _('Disabled') : '')
				])
			])
		].concat(extra));
	},

	lockNeighbourSection: function(title, list, ratType) {
		var cards = list.map(function(nb, i) {
			var band = parser.arfcnToBand(nb.arfcn, ratType === 'nr' ? 'NR' : 'LTE');
			return c.cellLockCard(nb, i, ratType, band, ratType === 'nr');
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
		var scBand = parser.arfcnToBand(info.arfcn, 'NR');
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
					var nbBand = parser.arfcnToBand(nb.arfcn, 'NR');
					return c.cellLockCard(nb, i, 'nr', nbBand, true);
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

	render: function(results) {
		var res = results[0] || {}, radioSettings = results[1] || {};
		var raw = res.stdout || '', radioRaw = radioSettings.stdout || '';
		var signal = parser.matchValues(parser.section(raw, 'Signal'), '^HCSQ');
		var cell = parser.parseServingCell(parser.matchValues(parser.section(raw, 'Serving cell'), '^MONSC'));
		var registration = parser.matchValues(parser.section(raw, 'Network registration'), '+CEREG');
		var operator = parser.matchValues(parser.section(raw, 'Operator'), '+COPS');
		var lteLock = parser.collectFreqLock(parser.section(raw, 'LTE lock'), '^LTEFREQLOCK');
		var nrLock = parser.collectFreqLock(parser.section(raw, 'NR lock'), '^NRFREQLOCK');
		var rrc = parser.matchValues(parser.section(raw, 'RRC state'), '^RRCSTAT');
		var rrcLabels = [ _('Idle'), _('Connected'), _('Inactive'), _('Invalid') ];
		var rrcState = rrc.length > 1 ? (rrcLabels[Number(rrc[1])] || rrc[1]) : '';
		if (rrc.length > 2)
			rrcState += rrc[2] === '98' ? ' · ' + _('Camped') : rrc[2] === '99' ? ' · ' + _('Not camped') : '';
		var registered = registration[1] === '1' || registration[1] === '5';
		var opInfo = parser.operatorInfo(operator[2]);
		var operatorName = opInfo.name;
		var tempMatch = raw.match(/^temperature=([\d.]+)/m);
		var temperature = tempMatch ? tempMatch[1] : '';
		var lteLockState = !lteLock[0] ? '--' : lteLock[0] === '0' ? _('Not locked') : _('Locked');
		var nrLockState = !nrLock[0] ? '--' : nrLock[0] === '0' ? _('Not locked') : _('Locked');
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
			api.atRadioDiagnostics().then(function(result) {
				dom.content(diagnosticHost, self.radioDiagnostics(result.stdout || ''));
			}, function(err) {
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
					E('div', { 'class': 'mt-advanced-actions', 'style': 'flex-direction:column;align-items:stretch;gap:12px' }, [
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
			res.stderr ? E('div', { 'class': 'alert-message warning' }, res.stderr) : null,
			c.hero(_('NETWORK AND CELL'), operatorName, _('Serving-cell and registration information reported by the modem.'), [
				c.badge(registered ? _('Registered') : _('Not registered'), registered ? 'ok' : 'warn')
			]),
			E('div', { 'class': 'mt-facts-grid', 'style': 'margin-bottom:14px' }, [
				c.gauge(cell.metrics[0].label, 'rsrp', cell.metrics[0].value, ' dBm', '-120', '-70'),
				c.gauge(cell.metrics[1].label, 'rsrq', cell.metrics[1].value, ' dB', '-25', '-3'),
				c.gauge(cell.metrics[2].label, 'sinr', cell.metrics[2].value, ' dB', '-10', '30'),
				c.gauge(_('Temperature'), 'temp', temperature, '°C', '20', '80')
			]),
			E('div', { 'class': 'mt-grid' }, [
				E('section', { 'class': 'mt-card' }, [
					E('h3', { 'class': 'mt-card-title' }, _('Serving cell')),
					this.row(_('Radio access'), cell.rat || signal[0]),
					this.row('MCC / MNC', cell.mcc && cell.mnc ? '%s / %s'.format(cell.mcc, cell.mnc) : ''),
					this.row('ARFCN', cell.arfcn),
					this.row('PCI', cell.pci),
					this.row(_('Cell ID'), cell.cellId),
					this.row('TAC / LAC', cell.tac),
					cell.scs ? this.row(_('SCS type'), cell.scs + ' · ' + ([ '15', '30', '60', '120', '240' ][Number(cell.scs)] || '?') + ' kHz') : null,
					this.row(_('Registration'), registered ? (registration[1] === '5' ? _('Roaming') : _('Home network')) : _('Not registered'))
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
							api.atCellscan().then(function(scan) {
								var body = scan.stdout ? renderCellScan(scan.stdout) : E('div', { 'class': 'alert-message warning' }, _('No response.'));
								ui.showModal(_('Cell Scan'), [ body, E('div', { 'class': 'right', 'style': 'margin-top:14px' }, E('button', { 'class': 'btn', 'click': ui.hideModal }, _('Close'))) ]);
							});
						} }, _('Continue')) ])
					]);
				} }, _('Cell Scan'))
			]),
			c.details(_('Technical details'), null, E('pre', { 'class': 'mt-raw' }, raw || _('No response.'))),
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
