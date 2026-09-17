'use strict';
'require view';
'require mt5700m.api as api';
'require mt5700m.parser as parser';
'require mt5700m.components as c';

/*
 * MT5700M LuCI — 高级设置（advanced）
 * ---------------------------------------------
 * 数据：fs.exec advanced hardware（USB/接口/NIC/PCIe/SIM 热插拔/SIM 槽/温控）
 * 结构：英雄区 → 数据接口（USB / PCIe / 接口模式）→ 模块硬件行为（SIM / 温控）
 *       → 诊断工具 → 技术细节
 */

return view.extend({
	load: function() {
		return api.atHardware();
	},

	render: function(res) {
		var raw = res.stdout || '';
		var usbRaw = parser.section(raw, 'USB mode');
		var interfaceRaw = parser.section(raw, 'Interface mode');
		var nic = parser.pick(parser.section(raw, 'NIC speed'), /\^TDPCIELANCFG:\s*(\d+)/, '');
		var pcieController = parser.pick(parser.section(raw, 'PCIe controller'), /\^TDPMCFG:\s*(\d+)/, '');
		var hotplug = parser.pick(parser.section(raw, 'SIM hotplug'), /\^TDSIMHP:\s*(\d+)/, '');
		var simSlot = parser.pick(parser.section(raw, 'SIM slot'), /\^SCICHG:\s*(\d+)/, '');
		var thermalMatch = parser.section(raw, 'Thermal control').match(/\^THERMAUTOFUN:\s*(\d+)[,\s]+(\d+)[,\s]+(\d+)/);
		var interfaceMode = parser.pick(interfaceRaw, /Mode:\s*(\d+)/, '');
		var usbModeValue = parser.pick(usbRaw, /\^SETMODE:\s*(\d+)/, parser.pick(usbRaw, /(?:^|\n)(\d+)(?:\n|$)/, '4'));

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
		var thermalEnabled = c.select([['1',_('Enabled')],['0',_('Disabled')]], thermalMatch ? thermalMatch[1] : '1');
		var thermalInterval = c.select([['1','1 s'],['2','2 s'],['3','3 s'],['5','5 s'],['10','10 s'],['30','30 s'],['60','60 s']], thermalMatch ? thermalMatch[3] : '2');

		return E('div', { 'class': 'mt-page' }, [
			c.cssLink(),
			c.hero(_('ADVANCED'), _('Advanced Settings'), _('Hardware interfaces, safeguards and diagnostic tools for experienced users. Normal operation does not require changes on this page.'), null, 'slate'),
			res.stderr ? E('div', { 'class': 'alert-message warning', 'style': 'margin-bottom:14px' }, res.stderr) : null,
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
							c.confirmRun(_('Change USB mode'), _('USB connectivity may disappear until the module restarts.'), [ 'advanced-set', 'usb-mode', usbMode.value ], true);
						}))
					]),
					c.card(_('PCIe Ethernet'), _('Match the PHY profile to the Ethernet controller connected to the MT5700M.'), [
						c.formRow(_('PCIe controller'), pcieEnabled),
						c.actionBar(c.btn(_('Apply PCIe controller'), function() {
							c.confirmRun(_('PCIe controller'), _('Disabling the PCIe controller removes the module Ethernet data path and may reduce power use.'), [ 'advanced-set', 'pcie-controller', pcieEnabled.value ], true);
						})),
						c.formRow(_('PHY profile'), nicProfile),
						E('div', { 'class': 'mt-scan-note' }, _('This selects a hardware PHY profile; it does not force Ethernet link negotiation speed.')),
						c.actionBar(c.btn(_('Apply PHY profile'), function() {
							c.confirmRun(_('Change PCIe Ethernet PHY'), _('An incorrect PHY profile can make the module Ethernet link unavailable.'), [ 'advanced-set', 'nic-speed', nicProfile.value ], true);
						}))
					]),
					c.card(_('Interface operating mode'), _('Select the documented relationship between the USB and Ethernet data paths.'), [
						c.formRow(_('Interface mode'), ifaceMode),
						c.actionBar(c.btn(_('Apply interface mode'), function() {
							c.confirmRun(_('Change interface mode'), _('This changes USB and Ethernet addressing and may disconnect the current session.'), [ 'advanced-set', 'interface-mode', ifaceMode.value ], true);
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
							c.confirmRun(_('SIM hotplug'), _('Apply the selected physical SIM detection behavior?'), [ 'advanced-set', 'sim-hotplug', simHotplug.value ]);
						}))
					]),
					c.card(_('Thermal protection'), _('Configure the MT5700M built-in temperature-protection polling.'), [
						c.formRow(_('Thermal protection'), thermalEnabled),
						c.formRow(_('Temperature interval'), thermalInterval),
						E('div', { 'class': 'mt-scan-note' }, _('Keep thermal protection enabled for normal operation.')),
						c.actionBar(c.btn(_('Apply thermal settings'), function() {
							c.confirmRun(_('Thermal protection'), _('Apply these thermal-protection settings?'), [ 'advanced-set', 'thermal', thermalEnabled.value, thermalInterval.value ]);
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
			c.details(_('Technical details'), null, E('pre', { 'class': 'mt-raw' }, raw || _('No response.')))
		]);
	},

	handleSave: null,
	handleSaveApply: null,
	handleReset: null
});
