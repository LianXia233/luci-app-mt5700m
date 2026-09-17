'use strict';
'require view';
'require form';
'require uci';
'require ui';
'require mt5700m.api as api';
'require mt5700m.parser as parser';
'require mt5700m.components as c';

/*
 * MT5700M LuCI — 移动数据（connection）
 * ---------------------------------------------
 * 数据：manager status + network.device status + advanced connection-settings + advanced session
 * 结构：英雄区 → 事实卡 → 会话面板 → 连接操作 → 拨号配置（LuCI form）→ 高级工具（PDP / 模块数据通道）→ 拨号日志
 * 无内联样式；表单框架（form.Map）保留 LuCI 原生能力。
 */

return view.extend({
	load: function() {
		return uci.load('mt5700m').then(L.bind(function() {
			return api.managerStatus().catch(function() { return {}; }).then(L.bind(function(manager) {
				this.manager = manager || {};
				return Promise.all([
					Promise.resolve(this.manager),
					api.deviceStatus(this.manager.network || '').catch(function() { return {}; }),
					api.atConnectionSettings(),
					api.atSession()
				]);
			}, this));
		}, this));
	},

	fact: function(label, value) {
		// 模组返回的字面 '--' 表示"空/运营商默认"，不是真实值
		if (value === '--')
			value = '';
		return c.fact(label, value || '--');
	},

	sessionRow: function(label, value) {
		return E('div', { 'class': 'mt-session-row' }, [ E('span', { 'class': 'mt-muted' }, label), E('strong', {}, value || '--') ]);
	},

	sessionPanel: function(session, error) {
		var self = this;
		var active = session.ipv4Connected || session.ipv6Connected;
		var addressRows = [
			self.sessionRow(_('IPv4 address'), session.ipv4Address), self.sessionRow(_('IPv4 gateway'), session.ipv4Gateway),
			self.sessionRow(_('IPv4 DNS'), session.ipv4Dns), self.sessionRow(_('IPv6 address'), session.ipv6Address),
			self.sessionRow(_('IPv6 DNS'), session.ipv6Dns), self.sessionRow(_('IP capability'), session.capability),
			self.sessionRow('MTU', session.mtu)
		];
		if (session.detailed.length)
			session.detailed.forEach(function(item) {
				addressRows.push(self.sessionRow('CID ' + item.cid + ' · ' + ((item.apn && item.apn !== '--') ? item.apn : _('Carrier default')), [ item.ipv4 ? 'IPv4' : '', item.ipv6 ? 'IPv6' : '', item.ethernet ? _('Ethernet') : '' ].filter(Boolean).join(' · ')));
			});
		return E('div', { 'class': 'mt-grid' }, [
			E('section', { 'class': 'mt-card' }, [
				E('div', { 'class': 'mt-gauge-head', 'style': 'margin-bottom:6px' }, [
					E('div', {}, [ E('h3', { 'class': 'mt-card-title', 'style': 'margin:0 0 4px' }, _('Assigned addresses')), E('p', { 'class': 'mt-card-desc', 'style': 'margin:0' }, _('Gateway, DNS and PDP session details reported by the MT5700M.')) ]),
					c.badge(active ? _('Active') : _('Disconnected'), active ? 'active' : 'slate')
				]),
				error ? E('div', { 'class': 'alert-message warning' }, error) : null,
				E('div', { 'class': 'mt-session-columns', 'style': 'display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:0 18px' }, addressRows)
			]),
			E('section', { 'class': 'mt-card' }, [
				E('div', { 'class': 'mt-gauge-head', 'style': 'margin-bottom:6px' }, [
					E('div', {}, [ E('h3', { 'class': 'mt-card-title', 'style': 'margin:0 0 4px' }, _('Module traffic counters')), E('p', { 'class': 'mt-card-desc', 'style': 'margin:0' }, _('Counters maintained by the modem firmware for the current and accumulated sessions.')) ]),
					c.badge(_('Module'), 'primary')
				]),
				self.sessionRow(_('Current duration'), parser.formatDuration(session.currentDuration)),
				self.sessionRow(_('Current total'), parser.formatBytes(session.currentRx + session.currentTx)),
				self.sessionRow(_('Accumulated duration'), parser.formatDuration(session.totalDuration)),
				self.sessionRow(_('Accumulated total'), parser.formatBytes(session.totalRx + session.totalTx)),
				self.sessionRow(_('Network maximum downlink'), parser.formatRate(session.maximumDown)),
				self.sessionRow(_('Network maximum uplink'), parser.formatRate(session.maximumUp)),
				E('div', { 'class': 'mt-session-actions' }, E('button', { 'class': 'btn', 'click': function() { c.confirmRun(_('Clear module traffic counters'), _('This permanently clears current and accumulated MT5700M data-flow counters.'), [ 'flow-clear' ]); } }, _('Clear counters')))
			])
		]);
	},

	editPdp: function(context) {
		var cid = E('input', { 'class': 'cbi-input-text', 'type': 'number', 'min': '1', 'max': '11', 'value': context ? context.cid : '1' });
		var type = c.select([['IPV4V6','IPv4 / IPv6'],['IP','IPv4'],['IPV6','IPv6']], context ? context.type : 'IPV4V6');
		var apn = E('input', { 'class': 'cbi-input-text', 'maxlength': '99', 'placeholder': _('Carrier default'), 'value': (context && context.apn !== '--') ? context.apn : '' });
		return ui.showModal(context ? _('Edit PDP context') : _('Add PDP context'), [
			E('p', {}, _('This changes a module-native PDP profile. The OpenWrt dialing profile above remains the normal place to configure APN.')),
			c.formRow('CID', cid), c.formRow(_('IP protocol'), type), c.formRow('APN', apn),
			E('div', { 'class': 'right' }, [ E('button', { 'class': 'btn', 'click': ui.hideModal }, _('Cancel')), ' ', E('button', { 'class': 'btn cbi-button-apply', 'click': function() {
				var cidValue = String(cid.value || '');
				if (!/^(?:[1-9]|1[01])$/.test(cidValue) || /[",\r\n]/.test(apn.value || ''))
					return ui.addNotification(null, E('p', {}, _('Enter a CID from 1 to 11 and a valid APN.')), 'warning');
				ui.hideModal();
				api.at([ 'pdp-set', cidValue, type.value, apn.value.trim() ]).then(function() { window.location.reload(); }, function(err) { ui.addNotification(null, E('p', {}, err.message || String(err)), 'danger'); });
			} }, _('Save')) ])
		]);
	},

	loadLog: function(details, output) {
		if (!details.open || details.getAttribute('data-loaded') === '1')
			return;

		details.setAttribute('data-loaded', '1');
		output.textContent = _('Loading dialing log…');
		api.dialLog().then(function(result) {
			output.textContent = result.log || _('No dialing log is available.');
		}).catch(function(err) {
			details.setAttribute('data-loaded', '0');
			output.textContent = err.message || String(err);
		});
	},

	runAction: function(action, success, confirmText) {
		var run = function() {
			ui.showModal(_('Working…'), [ E('p', { 'class': 'spinning' }, _('Applying the connection action…')) ]);
			return action().then(function() {
				ui.hideModal();
				ui.addNotification(null, E('p', {}, success));
				window.setTimeout(function() { window.location.reload(); }, 1200);
			}).catch(function(err) {
				ui.hideModal();
				ui.addNotification(null, E('p', {}, err.message || String(err)), 'danger');
			});
		};

		if (!confirmText)
			return run();

		return ui.showModal(_('Confirm Action'), [
			E('p', {}, confirmText),
			E('div', { 'class': 'right' }, [
				E('button', { 'class': 'btn', 'click': ui.hideModal }, _('Cancel')),
				' ',
				E('button', { 'class': 'btn cbi-button-action', 'click': function() { ui.hideModal(); run(); } }, _('Continue'))
			])
		]);
	},

	render: function(results) {
		var manager = this.manager || results[0] || {};
		var self = this;

		var dial = results[0] || {};
		var device = results[1] || {};
		var moduleSettings = results[2] || {};
		var sessionResult = results[3] || {};
		var session = parser.parseSession(sessionResult.stdout || '');
		var moduleRaw = moduleSettings.stdout || '';
		var online = dial.connected === true && device.up === true && device.carrier !== false;
		var configuredApn = uci.get('mt5700m', 'connection', 'apn') || _('Automatic');
		var configuredProtocol = { ip:'IPv4', ipv6:'IPv6', ipv4v6:'IPv4 / IPv6' }[uci.get('mt5700m', 'connection', 'pdp_type')] || 'IPv4 / IPv6';
		var m = new form.Map('mt5700m');
		var s, o;
		var logOutput = E('pre', { 'class': 'mt-raw' }, _('Expand to load the dialing log.'));
		var logDetails = E('details', {
			'class': 'mt-details',
			'toggle': function(ev) { self.loadLog(ev.currentTarget, logOutput); }
		}, [
			E('summary', { 'class': 'mt-details-summary' }, [
				E('span', { 'class': 'mt-chevron', 'aria-hidden': 'true' }),
				E('span', { 'style': 'min-width:0' }, E('span', { 'class': 'mt-details-title' }, _('Recent dialing log')))
			]),
			logOutput
		]);

		s = m.section(form.NamedSection, 'connection', 'connection');
		s.anonymous = true;

		o = s.option(form.Flag, 'enabled', _('Enable automatic dialing'));
		o.default = '1';
		o.rmempty = false;

		o = s.option(form.Value, 'apn', _('APN'));
		o.placeholder = _('Automatic');
		o.rmempty = true;

		o = s.option(form.ListValue, 'pdp_type', _('IP protocol'));
		o.value('ip', _('IPv4'));
		o.value('ipv6', _('IPv6'));
		o.value('ipv4v6', _('IPv4 / IPv6'));
		o.default = 'ipv4v6';
		o.rmempty = false;

		o = s.option(form.ListValue, 'auth', _('Authentication'));
		o.value('none', _('None'));
		o.value('pap', 'PAP');
		o.value('chap', 'CHAP');
		o.default = 'none';

		o = s.option(form.Value, 'username', _('Username'));
		o.depends('auth', 'pap');
		o.depends('auth', 'chap');

		o = s.option(form.Value, 'password', _('Password'));
		o.password = true;
		o.depends('auth', 'pap');
		o.depends('auth', 'chap');

		o = s.option(form.Value, 'metric', _('Route metric'));
		o.datatype = 'uinteger';
		o.default = '50';
		o.description = _('A smaller value gives this mobile connection a higher route priority.');

		o = s.option(form.DynamicList, 'dns_list', _('Custom DNS'));
		o.datatype = 'ipaddr';
		o.description = _('Leave empty to use DNS supplied by the mobile network.');

		var autoRaw = parser.section(moduleRaw, 'Auto dial');
		var interfaceRaw = parser.section(moduleRaw, 'Interface mode');
		var autoMatch = autoRaw.match(/\^SETAUTODIAL:\s*(\d+),(\d+),"([^"]*)",?"?([^",]*)"?,?"?([^",]*)"?,?"?([^",]*)"?,(\d+)/);
		var enabled = c.select([['1',_('Enabled')],['0',_('Disabled')]], autoMatch ? autoMatch[1] : '1');
		var dialMode = c.select([['0',_('Module internal dialing')],['1',_('Host dialing over USB')],['2',_('Host dialing over Ethernet')]], autoMatch ? autoMatch[2] : '1');
		var protocol = c.select([['IPV4V6','IPv4 / IPv6'],['IP','IPv4'],['IPV6','IPv6']], autoMatch ? autoMatch[3] : 'IPV4V6');
		var moduleApn = E('input', { 'class': 'cbi-input-text', 'placeholder': _('Leave empty to use carrier default'), 'value': autoMatch ? autoMatch[4] : '' });
		var moduleUsername = E('input', { 'class': 'cbi-input-text', 'autocomplete': 'off', 'value': autoMatch ? autoMatch[5] : '' });
		var modulePassword = E('input', { 'class': 'cbi-input-text', 'type': 'password', 'autocomplete': 'new-password', 'value': autoMatch ? autoMatch[6] : '' });
		var moduleAuth = c.select([['0',_('None')],['1','PAP'],['2','CHAP']], autoMatch ? autoMatch[7] : '0');
		var postRouteValue = parser.pick(interfaceRaw, /PostRoute:\s*(\d+)/, '');
		var dmzValue = parser.pick(interfaceRaw, /Dmz:\s*([^\n]+)/, '').trim();
		var postRouteKnown = postRouteValue === '1' || postRouteValue === '2';
		var postRouteOptions = [['2',_('Disabled')],['1',_('Enabled')]];
		if (!postRouteKnown)
			postRouteOptions.unshift(['', postRouteValue ? _('Unsupported value: %s').format(postRouteValue) : _('Unavailable')]);
		var postRoute = c.select(postRouteOptions, postRouteKnown ? postRouteValue : '');
		postRoute.disabled = !postRouteKnown;
		var dmz = E('input', { 'class': 'cbi-input-text', 'placeholder': '192.168.8.100', 'value': dmzValue.indexOf('not cfg') < 0 ? dmzValue : '' });
		var directIpValue = parser.pick(parser.section(moduleRaw, 'Direct IP'), /\^SETDIRECTIP:\s*(\d+)/, '');
		var directIpKnown = directIpValue === '0' || directIpValue === '1';
		var directIpOptions = directIpKnown ? [['0',_('Disabled')],['1',_('Enabled')]] : [['',_('Unavailable')]];
		var directIp = c.select(directIpOptions, directIpKnown ? directIpValue : '');
		directIp.disabled = !directIpKnown;
		var contexts = parser.parseContexts(parser.section(moduleRaw, 'PDP contexts'), parser.section(moduleRaw, 'PDP activation'));
		var pdpPanel = E('section', { 'class': 'mt-card', 'style': 'margin-top:16px' }, [
			E('div', { 'class': 'mt-gauge-head', 'style': 'align-items:flex-start' }, [
				E('div', {}, [ E('h3', { 'class': 'mt-card-title', 'style': 'margin:0 0 4px' }, _('Module PDP contexts')), E('p', { 'class': 'mt-card-desc', 'style': 'margin:0' }, _('Advanced module-native profiles. IMS contexts are protected from editing; normal OpenWrt users should configure APN in the dialing profile above.')) ]),
				E('button', { 'class': 'btn', 'click': function() { self.editPdp(null); } }, _('Add profile'))
			])
		].concat(contexts.length ? contexts.map(function(context) {
			var reserved = context.cid === '0' || context.cid === '5' || context.cid === '6' || String(context.apn || '').toLowerCase() === 'ims';
			return E('div', { 'class': 'mt-pdp-row' }, [
				E('strong', {}, 'CID ' + context.cid), E('span', {}, context.type), E('span', {}, context.apn || _('Carrier default')),
				E('span', { 'class': 'mt-pdp-state' + (context.active ? ' on' : '') }, context.active ? _('Active') : _('Inactive')),
				E('div', { 'class': 'mt-pdp-actions' }, reserved ? E('span', {}, String(context.apn || '').toLowerCase() === 'ims' ? _('IMS reserved') : _('System reserved')) : [
					E('button', { 'class': 'btn', 'click': function() { self.editPdp(context); } }, _('Edit')),
					E('button', { 'class': 'btn', 'click': function() { c.confirmRun(context.active ? _('Deactivate PDP context') : _('Activate PDP context'), _('Changing a module PDP context can interrupt mobile service.'), [ 'pdp-state', context.active ? '0' : '1', context.cid ]); } }, context.active ? _('Deactivate') : _('Activate')),
					E('button', { 'class': 'btn cbi-button-negative', 'click': function() { c.confirmRun(_('Remove PDP context'), _('Remove CID %s from the module?').format(context.cid), [ 'pdp-remove', context.cid ]); } }, _('Remove'))
				])
			]);
		}) : [ E('div', { 'class': 'alert-message notice' }, _('No PDP contexts were reported.')) ]));
		var moduleControls = E('section', { 'class': 'mt-card', 'style': 'margin-top:16px' }, [
			E('div', { 'class': 'mt-card-head' }, [
				E('h3', { 'class': 'mt-card-title' }, _('MT5700M data path')),
				E('p', { 'class': 'mt-card-desc' }, _('Module-side dialing and inbound-routing controls. Most OpenWrt installations should keep host dialing selected.'))
			]),
			moduleSettings.stderr ? E('div', { 'class': 'alert-message warning' }, moduleSettings.stderr) : null,
			E('div', { 'class': 'mt-grid' }, [
				c.card(_('Module dialing policy'), _('Select how the MT5700M firmware exposes its mobile data session.'), [
					E('div', { 'class': 'mt-scan-note' }, _('Use one dialing owner for each data path. With the integrated OpenWrt service, keep host dialing over USB selected to avoid repeated reconnects.')),
					c.formRow(_('Automatic dialing'), enabled),
					c.formRow(_('Dial mode'), dialMode),
					c.formRow(_('PDP protocol'), protocol),
					c.formRow('APN', moduleApn),
					c.formRow(_('Username'), moduleUsername),
					c.formRow(_('Password'), modulePassword),
					c.formRow(_('Authentication'), moduleAuth),
					c.actionBar(c.btn(_('Apply module dialing'), function() {
						c.confirmRun(_('Apply MT5700M dialing settings'), _('The mobile data session may disconnect and reconnect.'), [ 'advanced-set', 'autodial', enabled.value, dialMode.value, protocol.value, moduleApn.value, moduleUsername.value, modulePassword.value, moduleAuth.value ]);
					}))
				]),
				c.card(_('Inbound routing'), _('Optional module-side forwarding for devices connected behind the MT5700M data path.'), [
					c.formRow(_('IP passthrough'), directIp),
					E('div', { 'class': 'mt-scan-note' }, directIpKnown ? _('IP passthrough is an original-manager compatibility feature. Keep it disabled when OpenWrt owns the mobile connection.') : _('This MT5700M firmware does not expose a readable IP passthrough setting. The control is disabled to prevent false success reports.')),
					directIpKnown ? c.actionBar(c.btn(_('Apply IP passthrough'), function() {
						c.confirmRun(_('Change IP passthrough'), _('Changing passthrough can remove the module management address and interrupt connectivity.'), [ 'advanced-set', 'direct-ip', directIp.value ], true);
					})) : null,
					c.formRow(_('Post-routing'), postRoute),
					E('div', { 'class': 'mt-scan-note' }, _('Post-routing and DMZ are mutually exclusive. Leave both disabled unless the module itself is providing the downstream LAN.')),
					postRouteKnown ? c.actionBar(c.btn(_('Apply post-routing'), function() {
						c.confirmRun(_('Change post-routing'), _('The module must restart or cycle airplane mode before the new routing path is used.'), [ 'advanced-set', 'postroute', postRoute.value ], true);
					})) : E('div', { 'class': 'mt-scan-note' }, _('The modem reported a post-routing value that this firmware cannot safely change.')),
					c.formRow(_('DMZ address'), dmz),
					c.actionBar(c.btn(_('Apply DMZ'), function() {
						c.confirmRun(_('Change DMZ host'), _('The selected IPv4 host may be exposed to unsolicited traffic from the mobile network.'), [ 'advanced-set', 'dmz', dmz.value.trim() || '0' ]);
					}))
				])
			])
		]);

		return m.render().then(function(formNode) {
			return E('div', { 'class': 'mt-page' }, [
				c.cssLink(),
				manager.usb_state && manager.usb_state !== 'normal' ? E('div', { 'class': 'alert-message warning' }, _('The MT5700M is not in normal USB mode. Connection settings remain available, but dialing cannot start.')) : null,
				c.hero(_('Mobile Data'), _('Mobile data'), _('Configure how the MT5700M connects to the mobile network.'), [
					E('div', { 'class': 'mt-conn-state' + (online ? ' online' : '') }, [
						E('span', { 'class': 'mt-conn-dot' }),
						online ? _('Connected') : _('Disconnected')
					])
				]),
				E('div', { 'class': 'mt-facts-grid', 'style': 'margin-bottom:14px' }, [
					self.fact(_('Automatic dialing'), uci.get('mt5700m', 'connection', 'enabled') === '0' ? _('Disabled') : _('Enabled')),
					self.fact(_('Network interface'), manager.network),
					self.fact('APN', configuredApn),
					self.fact(_('IP protocol'), configuredProtocol)
				]),
				self.sessionPanel(session, sessionResult.stderr),
				E('div', { 'class': 'mt-advanced-actions', 'style': 'margin:0 0 18px' }, online ? [
					E('button', { 'class': 'btn cbi-button-action', 'click': function() { return self.runAction(api.redial, _('Redial started.'), _('The 5G connection will be interrupted briefly while the modem redials.')); } }, _('Redial')),
					E('button', { 'class': 'btn cbi-button-negative', 'click': function() { return self.runAction(api.disconnect, _('Connection stopped.'), _('Disconnect the mobile data connection now?')); } }, _('Disconnect'))
				] : [
					E('button', { 'class': 'btn cbi-button-action', 'click': function() { return self.runAction(api.connect, _('Dialing started.')); } }, _('Connect'))
				]),
				E('section', { 'class': 'mt-card' }, [
					E('div', { 'class': 'mt-card-head' }, [
						E('h3', { 'class': 'mt-card-title' }, _('Connection settings')),
						E('p', { 'class': 'mt-card-desc' }, _('APN is normally detected automatically. Save changes, then redial to use the new settings.'))
					]),
					formNode
				]),
				c.details(_('Advanced connection tools'), _('PDP profiles, module dialing modes and inbound routing for troubleshooting or special deployments.'), [ pdpPanel, moduleControls ]),
				logDetails
			]);
		});
	}
});
