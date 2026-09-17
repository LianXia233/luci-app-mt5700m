'use strict';
'require view';
'require form';
'require mt5700m.components as c';

/*
 * MT5700M LuCI — 通信诊断（settings）
 * ---------------------------------------------
 * 纯 LuCI form 配置页（uci 'mt5700m' section 'settings'）：管理通道连接方式。
 * 表单框架保留 LuCI 原生能力；外观统一到 mt- 设计系统。
 */

return view.extend({
	render: function() {
		var m, s, o;

		m = new form.Map('mt5700m');

		s = m.section(form.NamedSection, 'settings', 'mt5700m');
		s.anonymous = true;

		o = s.option(form.Flag, 'enabled', _('Enable module management'));
		o.default = '1';
		o.rmempty = false;

		o = s.option(form.ListValue, 'mode', _('Connection method'));
		o.value('auto', _('Automatic (recommended)'));
		o.value('serial', _('Serial Port'));
		o.value('network', _('Network TCP'));
		o.default = 'auto';
		o.rmempty = false;

		o = s.option(form.Value, 'at_port', _('AT Serial Port'));
		o.placeholder = _('Automatically detect the PCUI interface');
		o.description = _('The manager validates the TD Tech PCUI descriptor instead of assuming a fixed ttyUSB number. Leave this empty unless troubleshooting.');
		o.depends('mode', 'serial');
		o.rmempty = true;

		o = s.option(form.Value, 'host', _('AT Host'));
		o.datatype = 'host';
		o.default = '192.168.8.1';
		o.rmempty = false;
		o.depends('mode', 'network');

		o = s.option(form.Value, 'port', _('AT Port'));
		o.datatype = 'port';
		o.default = '20249';
		o.rmempty = false;
		o.depends('mode', 'network');

		o = s.option(form.Value, 'timeout', _('Response timeout'));
		o.datatype = 'range(1,60)';
		o.default = '8';
		o.rmempty = false;

		return m.render().then(function(formNode) {
			return E('div', { 'class': 'mt-page', 'style': 'max-width:900px;margin:0 auto' }, [
				c.cssLink(),
				c.hero(null, _('Communication diagnostics'), _('Low-level AT channel settings for troubleshooting module communication.'), [
					c.badge(_('Automatic mode recommended'), 'primary')
				], 'slate'),
				c.card(_('AT communication'), _('These settings do not change APN or mobile data. Leave them unchanged unless automatic detection fails.'), formNode),
				E('div', { 'class': 'mt-advanced-actions', 'style': 'margin-top:14px;justify-content:flex-start' },
					E('a', { 'class': 'btn', 'href': L.url('admin/modem/mt5700m/system') }, _('Back to Device and SIM')))
			]);
		});
	}
});
