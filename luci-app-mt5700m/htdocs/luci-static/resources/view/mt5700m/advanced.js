'use strict';
'require view';
'require mt5700m.api as api';
'require mt5700m.parser as parser';
'require mt5700m.components as c';

/*
 * MT5700M LuCI — 高级设置（advanced）
 * ---------------------------------------------
 * 数据：5 条读路由（network.usb_mode / network.interface_cfg /
 *       system.device_control / sim.slot / system.thermal）—— `advanced
 *       hardware` 的 8 段读帧不再经过前端；写：6 条写路由（usb_mode_set /
 *       power_control_set / nic_rate_set / interface_mode_set / hotplug_set /
 *       thermal_set）。
 * 结构：英雄区 → 数据接口（USB / PCIe / 接口模式）→ 模块硬件行为（SIM / 温控）
 *       → 诊断工具 → 技术细节
 */

return view.extend({
	load: function() {
		// 请求发起即返回，不阻塞首屏；render() 等 pending 填充。
		this.pending = Promise.all([
			api.route('network.usb_mode'),
			api.route('network.interface_cfg'),
			api.route('system.device_control'),
			api.route('sim.slot'),
			api.route('system.thermal')
		]);
		return Promise.resolve();
	},

	// 渐进渲染：骨架屏立即显示，数据到达后整体替换（后端慢不挡前端）
	render: function() {
		var self = this;
		var holder = E('div', { 'class': 'mt-view' });
		holder.appendChild(c.skeletonPage(4));
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

	renderPage: function(modules) {
		var usbPayload = modules[0] || {}, cfgPayload = modules[1] || {},
			devPayload = modules[2] || {}, simPayload = modules[3] || {},
			thermalPayload = modules[4] || {};
		// 载荷缺席（modem 无应答）即旧版帧解析失败的回落：USB mode 默认 '4'、
		// PCIe 默认 Enabled、温控默认开/2 s。parse_usb_mode 也认无前缀数字行，
		// 旧版的两级 fallback 语义由 Rust 解析器保真。
		var usbModeValue = (usbPayload.mode === undefined || usbPayload.mode === null) ? '4' : String(usbPayload.mode);
		var nic = (devPayload.nic_rate === undefined || devPayload.nic_rate === null) ? '' : String(devPayload.nic_rate);
		var pcieController = (devPayload.power_control === undefined || devPayload.power_control === null) ? '' : (devPayload.power_control ? '1' : '0');
		var hotplug = (simPayload.hotplug === undefined || simPayload.hotplug === null) ? '' : (simPayload.hotplug ? '1' : '0');
		var simSlot = (simPayload.slot === undefined || simPayload.slot === null) ? '' : String(simPayload.slot);
		// 旧正则 match 失败 → 温控默认 '1' / '2'；载荷层缺席走同一分支
		//（enabled: false 是明确的「关」，不回落）。
		var thermalEnabledValue = (thermalPayload.enabled === undefined) ? '1' : (thermalPayload.enabled ? '1' : '0');
		var thermalIntervalValue = (thermalPayload.interval === undefined || thermalPayload.interval === null) ? '2' : String(thermalPayload.interval);
		// TDCFG 帧的 Mode 字段。旧正则 /Mode:\s*(\d+)/ 漏了冒号前的 \s*，
		// 真实抓包形态（`Mode : 1`）永远匹配不上；载荷来自 Rust 宽松解析，
		// 空格形态也能取出（有意修复，与连接页 PostRoute 同款）。
		var interfaceMode = (cfgPayload.mode === undefined || cfgPayload.mode === null) ? '' : String(cfgPayload.mode);

		var usbMode = c.select([
			['0','Linux ECM'],['1','Windows NCM'],['2','Linux ECM · Debug'],['3','Windows NCM · Debug'],
			['4','Linux NCM'],['5','Linux NCM · Debug'],['6','Windows RNDIS'],['8','PPP']
		], usbModeValue);
		var nicProfile = c.select([['1','RTL8111 · 1 Gbps'],['2','RTL8125 · 2.5 Gbps']], nic);
		var pcieEnabled = c.select([['1',_('Enabled')],['0',_('Disabled')]], pcieController || '1');
		var ifaceMode = c.select([
			['1',_('USB Stick + Ethernet router mode')],
			['2',_('USB router + Ethernet router mode')]
		], interfaceMode);
		var simHotplug = c.select([['1',_('Enabled')],['0',_('Disabled')]], hotplug);
		var thermalEnabled = c.select([['1',_('Enabled')],['0',_('Disabled')]], thermalEnabledValue);
		var thermalInterval = c.select([['1','1 s'],['2','2 s'],['3','3 s'],['5','5 s'],['10','10 s'],['30','30 s'],['60','60 s']], thermalIntervalValue);

		// 「Technical details」折叠块：这里以前倾倒整段 `advanced hardware`
		// 文本帧。文本帧已不经过前端，改倾倒本页消费的路由载荷 —— 与系统页
		// 同一形态（api.<route> + JSON）。
		var technical = [
			[ 'network.usb_mode', usbPayload ], [ 'network.interface_cfg', cfgPayload ],
			[ 'system.device_control', devPayload ], [ 'sim.slot', simPayload ],
			[ 'system.thermal', thermalPayload ]
		].map(function(pair) {
			return '===== api.' + pair[0] + ' =====' + '\n' +
				JSON.stringify(pair[1] === undefined ? null : pair[1], null, 2);
		}).join('\n' + '\n');
		return E('div', { 'class': 'mt-page' }, [
			c.cssLink(),
			c.hero(_('ADVANCED'), _('Advanced Settings'), _('Hardware interfaces, safeguards and diagnostic tools for experienced users. Normal operation does not require changes on this page.'), [
				E('div', { 'class': 'mt-conn-state' }, [
					c.svgStatusPulse('ok', 14),
					E('span', { 'class': 'mt-badge mt-badge--primary' }, _('Hardware'))
				])
			], 'slate', c.svgChip(64, 'slate')),
			// 旧版这里还有一行 `res.stderr` 告警条。路由世界里没有 stderr：
			// 任一路由失败会让 load() 的 Promise.all 整体 reject，由 render()
			// 的失败分支画整页 alert-message error。
			E('div', { 'class': 'alert-message warning', 'style': 'margin-bottom:14px' }, _('Changing an interface profile can interrupt both mobile data and module management. Record the current value before applying a change.')),
			E('section', { 'class': 'mt-card' }, [
				E('div', { 'class': 'mt-card-head' }, [
					E('h3', { 'class': 'mt-card-title' }, _('Data interfaces')),
					E('p', { 'class': 'mt-card-desc' }, _('Profiles used by the host USB network driver and the optional PCIe Ethernet path.'))
				]),
				E('div', { 'class': 'mt-grid' }, [
					c.card(_('USB data mode'), _('Select the USB network-driver profile presented by the module.'), [
						c.formRow(_('USB mode'), usbMode),
						c.actionBar(c.btn(_('Apply USB mode'), function() {
							c.confirmRoute(_('Change USB mode'), _('USB connectivity may disappear until the module restarts.'), 'network.usb_mode_set', { mode: Number(usbMode.value) }, true);
						}))
					]),
					c.card(_('PCIe Ethernet'), _('Match the PHY profile to the Ethernet controller connected to the MT5700M.'), [
						c.formRow(_('PCIe controller'), pcieEnabled),
						c.actionBar(c.btn(_('Apply PCIe controller'), function() {
							c.confirmRoute(_('PCIe controller'), _('Disabling the PCIe controller removes the module Ethernet data path and may reduce power use.'), 'system.power_control_set', { enabled: pcieEnabled.value === '1' }, true);
						})),
						c.formRow(_('PHY profile'), nicProfile),
						E('div', { 'class': 'mt-scan-note' }, _('This selects a hardware PHY profile; it does not force Ethernet link negotiation speed.')),
						c.actionBar(c.btn(_('Apply PHY profile'), function() {
							c.confirmRoute(_('Change PCIe Ethernet PHY'), _('An incorrect PHY profile can make the module Ethernet link unavailable.'), 'system.nic_rate_set', { rate: Number(nicProfile.value) }, true);
						}))
					]),
					c.card(_('Interface operating mode'), _('Select the documented relationship between the USB and Ethernet data paths.'), [
						c.formRow(_('Interface mode'), ifaceMode),
						c.actionBar(c.btn(_('Apply interface mode'), function() {
							c.confirmRoute(_('Change interface mode'), _('This changes USB and Ethernet addressing and may disconnect the current session.'), 'network.interface_mode_set', { mode: Number(ifaceMode.value) }, true);
						}))
					])
				])
			]),
			E('section', { 'class': 'mt-card', 'style': 'margin-top:18px' }, [
				E('div', { 'class': 'mt-card-head' }, [
					E('h3', { 'class': 'mt-card-title' }, _('Module hardware behavior')),
					E('p', { 'class': 'mt-card-desc' }, _('SIM detection and the built-in temperature-protection polling policy.'))
				]),
				E('div', { 'class': 'mt-grid' }, [
					c.card(_('SIM hardware'), _('Review the active SIM path and configure physical-card change detection.'), [
						c.stateRow(_('Active SIM slot'), simSlot === '0' ? _('External SIM') : simSlot === '1' ? _('Internal SIM') : '--'),
						c.formRow(_('SIM hotplug'), simHotplug),
						c.actionBar(c.btn(_('Apply SIM hotplug'), function() {
							c.confirmRoute(_('SIM hotplug'), _('Apply the selected physical SIM detection behavior?'), 'sim.hotplug_set', { hotplug: simHotplug.value === '1' });
						}))
					]),
					c.card(_('Thermal protection'), _('Configure the MT5700M built-in temperature-protection polling.'), [
						c.formRow(_('Thermal protection'), thermalEnabled),
						c.formRow(_('Temperature interval'), thermalInterval),
						E('div', { 'class': 'mt-scan-note' }, _('Keep thermal protection enabled for normal operation.')),
						c.actionBar(c.btn(_('Apply thermal settings'), function() {
							c.confirmRoute(_('Thermal protection'), _('Apply these thermal-protection settings?'), 'system.thermal_set', { enabled: thermalEnabled.value === '1', interval: Number(thermalInterval.value) });
						}))
					])
				])
			]),
			E('section', { 'class': 'mt-card', 'style': 'margin-top:18px' }, [
				E('div', { 'class': 'mt-card-head' }, [
					E('h3', { 'class': 'mt-card-title' }, _('Diagnostics and developer tools')),
					E('p', { 'class': 'mt-card-desc' }, _('Open low-level communication settings or send AT commands directly to the module.'))
				]),
				E('div', { 'class': 'mt-advanced-actions' }, [
					E('a', { 'class': 'btn', 'href': L.url('admin/modem/mt5700m/settings') }, _('Communication diagnostics')),
					E('a', { 'class': 'btn cbi-button-action', 'href': L.url('admin/modem/mt5700m/terminal') }, _('AT Console'))
				])
			]),
			c.details(_('Technical details'), null, E('pre', { 'class': 'mt-raw' }, technical || _('No response.')))
		]);
	},

	handleSave: null,
	handleSaveApply: null,
	handleReset: null
});
