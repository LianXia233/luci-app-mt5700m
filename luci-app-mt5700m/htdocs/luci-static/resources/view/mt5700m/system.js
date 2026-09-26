'use strict';
'require view';
'require ui';
'require mt5700m.api as api';
'require mt5700m.parser as parser';
'require mt5700m.components as c';

/*
 * MT5700M LuCI — 模块与 SIM（system）
 * ---------------------------------------------
 * 数据：fs.exec system（身份 / 版本 / SIM / 订阅 / FOTA / 温控）
 * 结构：英雄区（含峰值温度）→ 信息卡 → 运行时状态 → 温控 → SIM 与无线电控制
 *       → 维护工具 → FOTA 面板 → 技术细节
 * FOTA 状态机与阈值校验逻辑保持 v2.4.8 行为等价。
 */

return view.extend({
	load: function() {
		// 请求发起即返回，不阻塞首屏；render() 等 pending 填充。
		this.pending = api.atSystem();
		return Promise.resolve();
	},

	row: function(label, value) {
		return c.row(label, value);
	},

	showPinManager: function(simState) {
		var operation = c.select([
			['verify',_('Verify current PIN')],['enable',_('Enable PIN lock')],['disable',_('Disable PIN lock')],
			['change',_('Change PIN')],['unblock',_('Unlock with PUK')]
		], simState === 'SIM PUK' ? 'unblock' : 'verify');
		var first = E('input', { 'class': 'cbi-input-text', 'type': 'password', 'inputmode': 'numeric', 'autocomplete': 'off' });
		var second = E('input', { 'class': 'cbi-input-text', 'type': 'password', 'inputmode': 'numeric', 'autocomplete': 'new-password' });
		var firstLabel = E('label', {}, _('PIN'));
		var secondRow = c.formRow(_('New PIN'), second);
		function update() {
			firstLabel.textContent = operation.value === 'unblock' ? _('PUK code') : operation.value === 'change' ? _('Current PIN') : _('PIN');
			secondRow.style.display = operation.value === 'change' || operation.value === 'unblock' ? '' : 'none';
		}
		operation.addEventListener('change', update);
		window.setTimeout(update, 0);
		return ui.showModal(_('SIM PIN management'), [
			E('div', { 'class': 'alert-message warning' }, _('Incorrect PIN or PUK attempts can permanently lock the SIM. Check the carrier documentation before continuing.')),
			c.formRow(_('Operation'), operation), E('div', { 'class': 'mt-advanced-row' }, [ firstLabel, E('div', { 'class': 'mt-advanced-value' }, first) ]), secondRow,
			E('div', { 'class': 'right' }, [ E('button', { 'class': 'btn', 'click': ui.hideModal }, _('Cancel')), ' ', E('button', { 'class': 'btn cbi-button-negative', 'click': function() {
				var firstValue = first.value.trim(), secondValue = second.value.trim();
				if ((operation.value === 'unblock' ? !/^\d{8}$/.test(firstValue) : !/^\d{4,8}$/.test(firstValue)) || ((operation.value === 'change' || operation.value === 'unblock') && !/^\d{4,8}$/.test(secondValue)))
					return ui.addNotification(null, E('p', {}, _('PIN must contain 4–8 digits; PUK must contain exactly 8 digits.')), 'warning');
				ui.hideModal();
				api.at([ 'sim-pin', operation.value, firstValue, secondValue ]).then(function() { window.location.reload(); }, function(err) { ui.addNotification(null, E('p', {}, err.message || String(err)), 'danger'); });
			} }, _('Apply')) ])
		]);
	},

	showThermalManager: function(values, logValues) {
		var labels = [
			_('Normal threshold'), _('First power reduction'), _('First recovery'),
			_('Second power reduction'), _('Second recovery'), _('Continuous power limit'),
			_('Continuous-limit recovery'), _('Emergency airplane mode'), _('Emergency recovery')
		];
		var inputs = labels.map(function(label, index) {
			return E('input', { 'class': 'cbi-input-text', 'type': 'number', 'min': '0', 'max': '150', 'value': values[index] || '' });
		});
		var serialLog = c.select([['0',_('Disabled')],['1',_('Enabled')]], logValues[0] || '0');
		var fileLog = c.select([['0',_('Disabled')],['1',_('Enabled')]], logValues[1] || '0');
		return ui.showModal(_('Thermal protection settings'), [
			E('div', { 'class': 'alert-message warning' }, _('Incorrect thresholds can cause performance loss, overheating or an emergency radio shutdown. Keep every recovery value below its matching trigger value.')),
			E('div', { 'style': 'display:grid;grid-template-columns:1fr 1fr;gap:0 14px;max-height:46vh;overflow:auto' }, labels.map(function(label, index) { return c.formRow(label + ' (°C)', inputs[index]); })),
			c.formRow(_('Serial thermal log'), serialLog), c.formRow(_('Stored thermal log'), fileLog),
			E('div', { 'class': 'right' }, [ E('button', { 'class': 'btn', 'click': ui.hideModal }, _('Cancel')), ' ', E('button', { 'class': 'btn cbi-button-negative', 'click': function() {
				var next = inputs.map(function(input) { return String(input.value || ''); });
				if (next.some(function(value) { return !/^\d+$/.test(value) || Number(value) > 150; }))
					return ui.addNotification(null, E('p', {}, _('Every threshold must be a temperature from 0 to 150°C.')), 'warning');
				if (!(Number(next[1]) > Number(next[0]) && Number(next[3]) > Number(next[1]) && Number(next[5]) > Number(next[3]) && Number(next[7]) > Number(next[5]) && Number(next[2]) < Number(next[1]) && Number(next[4]) < Number(next[3]) && Number(next[6]) < Number(next[5]) && Number(next[8]) < Number(next[7])))
					return ui.addNotification(null, E('p', {}, _('Trigger temperatures must rise by level, and each recovery temperature must be lower than its trigger.')), 'warning');
				ui.hideModal();
				Promise.all([
					api.at([ 'advanced-set', 'thermal-thresholds' ].concat(next)),
					api.at([ 'advanced-set', 'thermal-log', serialLog.value, fileLog.value ])
				]).then(function() { ui.addNotification(null, E('p', {}, _('Thermal settings saved.'))); window.setTimeout(function() { window.location.reload(); }, 1200); }, function(err) { ui.addNotification(null, E('p', {}, err.message || String(err)), 'danger'); });
			} }, _('Apply')) ])
		]);
	},

	showIdentityLab: function(currentImei) {
		var input = E('input', { 'class': 'cbi-input-text', 'inputmode': 'numeric', 'maxlength': '15', 'placeholder': currentImei || '123456789012345' });
		return ui.showModal(_('Device identity laboratory'), [
			E('div', { 'class': 'alert-message warning' }, _('IMEI writing is not documented in the supplied MT5700M AT manual and may be restricted by local law or the mobile operator. This control is provided only for restoring the original device identity.')),
			c.formRow(_('New IMEI'), input),
			E('div', { 'class': 'right' }, [ E('button', { 'class': 'btn', 'click': ui.hideModal }, _('Cancel')), ' ', E('button', { 'class': 'btn cbi-button-negative', 'click': function() {
				var value = input.value.trim();
				if (!/^\d{15}$/.test(value) || value === currentImei)
					return ui.addNotification(null, E('p', {}, _('Enter a different 15-digit IMEI.')), 'warning');
				ui.hideModal();
				c.confirmRun(_('Write device identity'), _('This unsupported operation can prevent network registration. Continue only when restoring the identity printed on the module label.'), [ 'set-imei', value ], true);
			} }, _('Review change')) ])
		]);
	},

	showFactoryReset: function() {
		var confirm = E('input', { 'class': 'cbi-input-text', 'placeholder': 'RESET', 'autocomplete': 'off' });
		return ui.showModal(_('Restore module factory settings'), [
			E('div', { 'class': 'alert-message warning' }, _('This erases module-side APN, radio, SIM, interface and thermal settings, then restarts the MT5700M. OpenWrt settings are not erased.')),
			c.formRow(_('Type RESET to confirm'), confirm),
			E('div', { 'class': 'right' }, [ E('button', { 'class': 'btn', 'click': ui.hideModal }, _('Cancel')), ' ', E('button', { 'class': 'btn cbi-button-negative', 'click': function() {
				if (confirm.value !== 'RESET') return ui.addNotification(null, E('p', {}, _('Confirmation text does not match.')), 'warning');
				ui.hideModal(); api.at([ 'factory-reset' ]).then(function() { ui.addNotification(null, E('p', {}, _('Factory reset accepted. The module will restart.'))); }, function(err) { ui.addNotification(null, E('p', {}, err.message || String(err)), 'danger'); });
			} }, _('Restore factory settings')) ])
		]);
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

	renderPage: function(res) {
		var raw = res.stdout || '';
		var identity = parser.section(raw, 'Identity');
		var version = parser.section(raw, 'Version');
		var sim = parser.lineValue(parser.section(raw, 'SIM'), '+CPIN');
		var iccid = parser.lineValue(parser.section(raw, 'ICCID'), '^ICCID');
		var imsi = (parser.section(raw, 'IMSI').match(/(?:^|\n)(\d{10,18})(?:\r?\n|$)/) || [,''])[1];
		var phone = parser.lineValue(parser.section(raw, 'Subscriber number'), '+CNUM').replace(/"/g, '').split(',').map(function(value) { return value.trim(); }).filter(function(value) { return /^\+?\d{5,20}$/.test(value); })[0] || _('Not stored on SIM');
		var subscription = parser.lineValue(parser.section(raw, 'Subscription rate'), '^DSAMBR').replace(/"/g, '').split(',').map(function(value) { return value.trim(); });
		var operatorValues = parser.lineValue(parser.section(raw, 'Operator'), '+COPS').replace(/"/g, '').split(',');
		var networkTime = parser.lineValue(parser.section(raw, 'Network time'), '^NWTIME').replace(/"/g, '');
		var functionLevel = parser.lineValue(parser.section(raw, 'Function level'), '+CFUN');
		var ledState = parser.lineValue(parser.section(raw, 'LED'), '^LEDSWITCH');
		var simActivation = parser.lineValue(parser.section(raw, 'SIM activation'), '^HVSST').split(',');
		var simSlots = parser.lineValue(parser.section(raw, 'SIM slot'), '^SCICHG').split(',');
		var temperatures = parser.lineValue(parser.section(raw, 'Temperature'), '^CHIPTEMP').split(',').map(function(value) { return value.trim(); }).filter(function(value) { return /^-?\d+$/.test(value) && Number(value) > -1000 && Number(value) < 2000; }).map(Number);
		var temperature = temperatures.length ? (Math.max.apply(null, temperatures) / 10).toFixed(1) : '';
		var fotaMode = parser.lineValue(parser.section(raw, 'FOTA mode'), '^FOTAMODE');
		var fotaModeName = fotaMode === '0,1,0,1' ? _('HTTP update mode') : fotaMode;
		var fotaState = parser.lineValue(parser.section(raw, 'FOTA state'), '^FOTASTATE');
		var fotaProgress = parser.lineValue(parser.section(raw, 'FOTA progress'), '^FOTADLQ').replace(/"/g, '').split(',');
		var total = Number(fotaProgress[fotaProgress.length - 2]) || 0;
		var received = Number(fotaProgress[fotaProgress.length - 1]) || 0;
		var percent = total > 0 ? Math.max(0, Math.min(100, Math.floor(received / total * 100))) : 0;
		var model = 'MT5700M';
		var revision = parser.lineValue(identity, 'Revision') || parser.lineValue(parser.section(raw, 'Revision'), '');
		var imei = parser.lineValue(identity, 'IMEI') || parser.section(raw, 'IMEI').split(/\n/)[0];
		var buildDate = parser.lineValue(version, '^VERSION:BDT');
		var software = parser.lineValue(version, '^VERSION:EXTS');
		var hardware = parser.lineValue(version, '^VERSION:EXTH');
		var thermalStatus = parser.lineValue(parser.section(raw, 'Thermal status'), '^THERMLDAUTOSTATUS') || parser.lineValue(parser.section(raw, 'Thermal status'), '+THERMLDAUTOSTATUS');
		var thermalValues = thermalStatus.split(',').map(function(value) { return value.trim(); });
		var thermalLevel = Number(thermalValues[5]) || 0;
		var thermalThresholds = parser.lineValue(parser.section(raw, 'Thermal thresholds'), '^THERMLDAUTOPARA').split(',').map(function(value) { return value.trim(); });
		var thermalLog = parser.lineValue(parser.section(raw, 'Thermal log'), '^THERMLDLOGSW').replace(/\s+/g, ',').split(',');
		var stateName = fotaState ? (parser.FOTA_STATE_NAMES[fotaState] || _('Unknown state')) : _('Status unavailable');
		var stateBadge = fotaState === '30' || fotaState === '40' || fotaState === '50' ? 'active' : fotaState === '13' || fotaState === '20' ? 'bad' : 'slate';
		var fotaUrl = E('input', { 'class': 'cbi-input-text', 'placeholder': 'http://server/path/' });
		var ledSelect = c.select([['1',_('Enabled')],['0',_('Disabled')]], ledState || '1');
		var simEnabled = c.select([['1',_('Active')],['0',_('Inactive')]], simActivation[1] || '1');
		var simSlot = c.select([['0',_('SIM slot 0')],['1',_('SIM slot 1')]], simSlots[0] || simActivation[2] || '0');
		var self = this;

		return E('div', { 'class': 'mt-page' }, [
			c.cssLink(),
			res.stderr ? E('div', { 'class': 'alert-message warning' }, res.stderr) : null,
			c.hero(_('MT5700M SYSTEM'), model, revision || _('Module information'), [
				E('div', { 'style': 'min-width:96px;text-align:center;padding:11px 14px;border-radius:12px;background:rgba(255,255,255,.14)' }, [
					E('strong', { 'style': 'display:block;font-size:24px' }, temperature ? temperature + '°' : '--'),
					E('span', { 'style': 'font-size:11px;opacity:.8' }, _('Peak sensor temperature'))
				])
			], 'slate'),
			E('div', { 'class': 'mt-grid' }, [
				E('section', { 'class': 'mt-card' }, [ E('h3', { 'class': 'mt-card-title' }, _('Module information')), this.row(_('Model'), model), this.row(_('Firmware version'), software || revision), this.row(_('Hardware version'), hardware), this.row('IMEI', imei), this.row(_('Build date'), buildDate) ]),
				E('section', { 'class': 'mt-card' }, [ E('h3', { 'class': 'mt-card-title' }, _('SIM and subscription')), this.row(_('Phone Number'), phone), this.row('ICCID', iccid), this.row('IMSI', imsi), this.row(_('Subscription downlink'), parser.subscriptionRate(subscription[1])), this.row(_('Subscription uplink'), parser.subscriptionRate(subscription[2])), E('div', { 'class': 'mt-scan-note' }, _('Device and SIM identifiers are displayed only in this local management page.')) ]),
				E('section', { 'class': 'mt-card' }, [ E('h3', { 'class': 'mt-card-title' }, _('Runtime status')), this.row(_('SIM Status'), sim), this.row(_('Operator'), operatorValues[2] || '--'), this.row(_('Network time'), networkTime), this.row(_('Radio function'), functionLevel === '0' ? _('Airplane mode') : functionLevel === '1' ? _('Online') : functionLevel), this.row(_('SIM power path'), simActivation[1] === '1' ? _('Active') : simActivation.length > 1 ? _('Inactive') : ''), this.row(_('FOTA mode'), fotaModeName) ]),
				E('section', { 'class': 'mt-card' }, [ E('h3', { 'class': 'mt-card-title' }, _('Thermal protection status')), this.row(_('Protection level'), thermalValues.length ? (thermalLevel === 0 ? _('Normal') : _('Level %d').format(thermalLevel)) : ''), this.row(_('First power reduction'), thermalThresholds[1] ? thermalThresholds[1] + '°C / ' + thermalThresholds[2] + '°C' : ''), this.row(_('Second power reduction'), thermalThresholds[3] ? thermalThresholds[3] + '°C / ' + thermalThresholds[4] + '°C' : ''), this.row(_('Continuous power limit'), thermalThresholds[5] ? thermalThresholds[5] + '°C / ' + thermalThresholds[6] + '°C' : ''), this.row(_('Emergency airplane mode'), thermalThresholds[7] ? thermalThresholds[7] + '°C / ' + thermalThresholds[8] + '°C' : ''), this.row(_('Thermal logging'), thermalLog[0] === '1' || thermalLog[1] === '1' ? _('Enabled') : thermalLog.length > 1 ? _('Disabled') : '') ]),
				E('section', { 'class': 'mt-card', 'style': 'grid-column:1/-1' }, [
					E('h3', { 'class': 'mt-card-title' }, _('SIM and radio')),
					E('p', { 'class': 'mt-card-desc' }, _('Daily SIM and radio controls defined by the MT5700M AT command manual.')),
					c.stateRow(_('Current radio state'), functionLevel === '0' ? _('Airplane mode') : _('Online')),
					c.actionBar(c.btn(functionLevel === '0' ? _('Resume mobile radio') : _('Enter airplane mode'), function() { c.runConfirmed(_('Change radio function'), functionLevel === '0' ? _('Resume mobile registration and data service?') : _('Airplane mode immediately disconnects mobile data and voice service.'), [ 'airplane', functionLevel === '0' ? '1' : '0' ], functionLevel !== '0'); })),
					c.formRow(_('Module status LED'), ledSelect),
					c.actionBar(c.btn(_('Apply LED setting'), function() { c.confirmRun(_('Module status LED'), _('The LED setting is stored by the module and takes effect after restart.'), [ 'advanced-set', 'led', ledSelect.value ], true); })),
					c.actionBar(c.btn(_('Manage SIM PIN'), function() { self.showPinManager(sim); })),
					c.formRow(_('SIM activation'), simEnabled),
					c.actionBar(c.btn(_('Apply SIM activation'), function() { c.confirmRun(_('SIM activation'), simEnabled.value === '1' ? _('Activate the physical SIM for network registration?') : _('Deactivating the SIM immediately removes mobile service.'), [ 'advanced-set', 'sim-activation', simEnabled.value ], simEnabled.value === '0'); })),
					c.formRow(_('Active SIM slot'), simSlot),
					c.actionBar(c.btn(_('Switch SIM slot'), function() { c.confirmRun(_('Switch SIM slot'), _('The MT5700M will detach from the network while changing the physical SIM path.'), [ 'advanced-set', 'sim-slot', simSlot.value ], true); }))
				])
			]),
			E('section', { 'class': 'mt-card', 'style': 'margin-top:14px' }, [
				E('div', { 'class': 'mt-card-head' }, [ E('h3', { 'class': 'mt-card-title' }, _('Protection and maintenance')), E('p', { 'class': 'mt-card-desc' }, _('Module protection, recovery and communication troubleshooting tools.')) ]),
				E('div', { 'class': 'mt-advanced-actions' }, [
					E('a', { 'class': 'btn', 'href': L.url('admin/modem/mt5700m/settings') }, _('Communication diagnostics')),
					E('button', { 'class': 'btn', 'click': function() { self.showThermalManager(thermalThresholds, thermalLog); } }, _('Configure thermal protection')),
					E('button', { 'class': 'btn', 'click': function() { self.showIdentityLab(imei); } }, _('Device identity laboratory')),
					E('button', { 'class': 'btn cbi-button-negative', 'click': function() { self.showFactoryReset(); } }, _('Restore factory settings'))
				])
			]),
			E('section', { 'class': 'mt-card', 'style': 'margin-top:14px;border:1px solid var(--mt-warn-border, #ead7b2)' }, [
				E('div', { 'class': 'mt-card-head', 'style': 'align-items:flex-start' }, [
					E('div', {}, [ E('h3', { 'class': 'mt-card-title' }, _('Firmware update')), E('p', { 'class': 'mt-card-desc' }, _('Use an MT5700M-compatible HTTP update server. Do not interrupt power during installation.')) ]),
					c.badge(stateName, stateBadge)
				]),
				E('div', { 'class': 'mt-gauge-track', 'style': 'height:8px;margin:14px 0 6px' }, E('i', { 'class': 'mt-gauge-fill', 'style': 'width:%d%%'.format(percent) })),
				E('div', { 'class': 'mt-traffic-stat-label', 'style': 'text-align:right' }, _('%d%% complete').format(percent)),
				E('div', { 'style': 'display:flex;gap:9px;margin-top:14px;flex-wrap:wrap' }, [
					E('div', { 'style': 'flex:1 1 240px;min-width:0' }, fotaUrl),
					E('button', { 'class': 'btn cbi-button-action', 'click': function() {
						if (!/^http:\/\//.test(fotaUrl.value || ''))
							return ui.addNotification(null, E('p', {}, _('Enter a valid HTTP update-server URL.')), 'warning');
						c.runConfirmed(_('Start firmware download'), _('The modem will contact the specified server and may temporarily use mobile data.'), [ 'fota-start', fotaUrl.value ], false);
					} }, _('Check and download'))
				]),
				E('div', { 'class': 'mt-advanced-actions' }, [
					E('button', { 'class': 'btn cbi-button', 'click': function() { window.location.reload(); } }, _('Refresh status')),
					E('button', { 'class': 'btn cbi-button', 'disabled': fotaState === '31' ? null : 'disabled', 'click': function() { c.runConfirmed(_('Resume download'), _('Resume the paused firmware download?'), [ 'fota-resume' ], false); } }, _('Resume Download')),
					E('button', { 'class': 'btn cbi-button-negative', 'disabled': fotaState === '40' ? null : 'disabled', 'click': function() { c.runConfirmed(_('Install firmware'), _('The modem will restart. Do not disconnect power until installation is complete.'), [ 'fota-upgrade' ], true); } }, _('Install update')),
					E('button', { 'class': 'btn cbi-button-negative', 'click': function() { c.runConfirmed(_('Restart Module'), _('This will restart the MT5700M module and temporarily interrupt 5G connectivity.'), [ 'restart' ], true, 24000); } }, _('Restart Module'))
				])
			]),
			c.details(_('Technical details'), null, E('pre', { 'class': 'mt-raw' }, raw || _('No response.')))
		]);
	},

	handleSave: null,
	handleSaveApply: null,
	handleReset: null
});
