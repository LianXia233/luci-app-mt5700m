'use strict';
'require view';
'require form';
'require uci';
'require ui';
'require dom';
'require mt5700m.api as api';
'require mt5700m.parser as parser';
'require mt5700m.components as c';

/*
 * MT5700M LuCI — 移动数据（connection）
 * ---------------------------------------------
 * 数据：manager status + network.device status + 四条设置路由（autodial /
 * interface_cfg / pdp_contexts / direct_ip）+ network.session（会话面板；
 * AT 命令的解码全部在后端）。设置区七条写入也走统一路由。
 * 结构：英雄区 → 事实卡 → 会话面板 → 连接操作 → 拨号配置（LuCI form）→ 高级工具（PDP / 模块数据通道）→ 拨号日志
 * 无内联样式；表单框架（form.Map）保留 LuCI 原生能力。
 */

return view.extend({
	load: function() {
		// AT 查询与 manager/接口状态全部并行发起，且 load() 不等待它们：
		// 页面先渲染骨架屏，数据到达后由 render() 填充。
		var self = this;
		var previousManager = this.manager || {};
		this.managerError = '';
		// 模块设置区块读统一路由（Auto dial / Interface+DMZ / PDP 表 / 直通），
		// `advanced connection-settings` 的五段文本帧与前端那份第二份解析随本批
		// 消失。单条路由取不到时按旧行为降级（控件回落默认值 / 禁用）。
		var settings = Promise.all([
			api.route('network.autodial'),
			api.route('network.interface_cfg'),
			api.route('network.pdp_contexts'),
			api.route('network.direct_ip')
		]);
		// 会话面板读统一路由（与概览页的「移动 IP」卡同一条）。route() 失败返回
		// null，面板按空值渲染 —— 与旧版 CLI 帧读不到时完全一样的观感（那时
		// `advanced session` 的 stderr 也只在帧里，页面同样是空卡）。
		var session = api.route('network.session');
		this.pending = uci.load('mt5700m').then(function() {
			return api.managerStatus().catch(function(err) {
					self.managerError = err && err.message || String(err);
					return previousManager;
				}).then(function(manager) {
				self.manager = manager || {};
				return Promise.all([
					Promise.resolve(self.manager),
					api.deviceStatus(self.manager.network || '').then(function(device) {
						self.deviceStatus = device || {};
						return self.deviceStatus;
					}, function(err) {
						return Object.assign({}, self.deviceStatus || {}, { _rpcError: err && err.message || String(err) });
					}),
					settings,
					session
				]);
			});
		});
		return Promise.resolve();
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

	sessionPanel: function(session) {
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
				E('div', { 'class': 'mt-session-columns', 'style': 'display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:0 18px' }, addressRows)
			]),
			E('section', { 'class': 'mt-card' }, [
				E('div', { 'class': 'mt-gauge-head', 'style': 'margin-bottom:6px;align-items:center' }, [
					E('div', {}, [ E('h3', { 'class': 'mt-card-title', 'style': 'margin:0 0 4px' }, _('Module traffic counters')), E('p', { 'class': 'mt-card-desc', 'style': 'margin:0' }, _('Counters maintained by the modem firmware for the current and accumulated sessions.')) ]),
					c.svgTrafficArrows()
				]),
				self.sessionRow(_('Current duration'), parser.formatDuration(session.currentDuration)),
				self.sessionRow(_('Current total'), parser.formatBytes(session.currentRx + session.currentTx)),
				self.sessionRow(_('Accumulated duration'), parser.formatDuration(session.totalDuration)),
				self.sessionRow(_('Accumulated total'), parser.formatBytes(session.totalRx + session.totalTx)),
				self.sessionRow(_('Network maximum downlink'), parser.formatRate(session.maximumDown)),
				self.sessionRow(_('Network maximum uplink'), parser.formatRate(session.maximumUp)),
				E('div', { 'class': 'mt-session-actions' }, E('button', { 'class': 'btn', 'click': function() { c.confirmRoute(_('Clear module traffic counters'), _('This permanently clears current and accumulated MT5700M data-flow counters.'), 'network.flow_clear'); } }, _('Clear counters')))
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
				api.routeCall('network.pdp_set', { cid: Number(cidValue), type: type.value, apn: apn.value.trim() }).then(function() { window.location.reload(); }, function(err) { ui.addNotification(null, E('p', {}, err.message || String(err)), 'danger'); });
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

	renderPage: function(results) {
		var manager = this.manager || results[0] || {};
		var self = this;

		var dial = results[0] || {};
		var device = results[1] || {};
		var moduleSettings = results[2] || [];
		var session = parser.sessionInfo(results[3]);
		var managerError = this.managerError || '';
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

		/*
		 * 模块设置：四条路由载荷 → 页面取值。变量名与旧版文本解析的产物同名
		 * 同型，下面的控件构造一行没动。
		 *
		 * 两处照旧仿出来的怪癖（与 docs/architecture-v2/migration.md 一致）：
		 * 1. autoKnown 对齐旧正则——旧 `autoMatch` 要求 ^SETAUTODIAL 应答里
		 *    auth 字段存在，模块只答 `1,0,"IPV4V6"`（关拨号的缩短形态）时整行
		 *    不识别、控件全部回落默认值。载荷层 auth 缺席即同一个分支。
		 * 2. postRouteValue / directIp 沿用「值不是 1|2 / 0|1 就禁用控件」，
		 *    载荷缺席（undefined）与旧帧解析失败（''）同一条路。
		 */
		var autodialPayload = moduleSettings[0] || {};
		var cfgPayload = moduleSettings[1] || {};
		var pdpPayload = moduleSettings[2] || {};
		var directIpPayload = moduleSettings[3] || {};
		var autoKnown = autodialPayload.enable !== undefined && autodialPayload.enable !== null
			&& autodialPayload.authType !== undefined && autodialPayload.authType !== null;
		var enabled = c.select([['1',_('Enabled')],['0',_('Disabled')]], autoKnown ? String(autodialPayload.enable) : '1');
		var dialMode = c.select([['0',_('Module internal dialing')],['1',_('Host dialing over USB')],['2',_('Host dialing over Ethernet')]], autoKnown ? String(autodialPayload.dialMode) : '1');
		var protocol = c.select([['IPV4V6','IPv4 / IPv6'],['IP','IPv4'],['IPV6','IPv6']], autoKnown ? autodialPayload.protocol : 'IPV4V6');
		var moduleApn = E('input', { 'class': 'cbi-input-text', 'placeholder': _('Leave empty to use carrier default'), 'value': autoKnown ? autodialPayload.apn : '' });
		var moduleUsername = E('input', { 'class': 'cbi-input-text', 'autocomplete': 'off', 'value': autoKnown ? autodialPayload.username : '' });
		var modulePassword = E('input', { 'class': 'cbi-input-text', 'type': 'password', 'autocomplete': 'new-password', 'value': autoKnown ? autodialPayload.password : '' });
		var moduleAuth = c.select([['0',_('None')],['1','PAP'],['2','CHAP']], autoKnown ? String(autodialPayload.authType) : '0');
		var postRouteValue = (cfgPayload.postRoute === undefined || cfgPayload.postRoute === null) ? '' : String(cfgPayload.postRoute);
		var dmzValue = (cfgPayload.dmz && cfgPayload.dmz.enabled) ? String(cfgPayload.dmz.host || '') : '';
		var postRouteKnown = postRouteValue === '1' || postRouteValue === '2';
		var postRouteOptions = [['2',_('Disabled')],['1',_('Enabled')]];
		if (!postRouteKnown)
			postRouteOptions.unshift(['', postRouteValue ? _('Unsupported value: %s').format(postRouteValue) : _('Unavailable')]);
		var postRoute = c.select(postRouteOptions, postRouteKnown ? postRouteValue : '');
		postRoute.disabled = !postRouteKnown;
		var dmz = E('input', { 'class': 'cbi-input-text', 'placeholder': '192.168.8.100', 'value': dmzValue });
		var directIpKnown = directIpPayload.enabled === true || directIpPayload.enabled === false;
		var directIpOptions = directIpKnown ? [['0',_('Disabled')],['1',_('Enabled')]] : [['',_('Unavailable')]];
		var directIp = c.select(directIpOptions, directIpKnown ? (directIpPayload.enabled ? '1' : '0') : '');
		directIp.disabled = !directIpKnown;
		var contexts = (pdpPayload.contexts || []).map(function(entry) {
			return { cid: String(entry.cid), type: entry.type || '', apn: entry.apn || '', active: entry.active === true };
		});
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
					E('button', { 'class': 'btn', 'click': function() { c.confirmRoute(context.active ? _('Deactivate PDP context') : _('Activate PDP context'), _('Changing a module PDP context can interrupt mobile service.'), 'network.pdp_state', { cid: Number(context.cid), active: !context.active }); } }, context.active ? _('Deactivate') : _('Activate')),
					E('button', { 'class': 'btn cbi-button-negative', 'click': function() { c.confirmRoute(_('Remove PDP context'), _('Remove CID %s from the module?').format(context.cid), 'network.pdp_remove', { cid: Number(context.cid) }); } }, _('Remove'))
				])
			]);
		}) : [ E('div', { 'class': 'alert-message notice' }, _('No PDP contexts were reported.')) ]));
		var moduleControls = E('section', { 'class': 'mt-card', 'style': 'margin-top:16px' }, [
			E('div', { 'class': 'mt-card-head' }, [
				E('h3', { 'class': 'mt-card-title' }, _('MT5700M data path')),
				E('p', { 'class': 'mt-card-desc' }, _('Module-side dialing and inbound-routing controls. Most OpenWrt installations should keep host dialing selected.'))
			]),
			// 旧版这里有一行 `moduleSettings.stderr` 告警条。路由世界里没有
			// stderr：单条路由失败按各自降级（控件禁用 / 回落默认值），
			// Promise.all 整体失败才由 render() 的失败分支画整页 error。
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
						c.confirmRoute(_('Apply MT5700M dialing settings'), _('The mobile data session may disconnect and reconnect.'), 'network.autodial_set', { enabled: enabled.value === '1', dialMode: Number(dialMode.value), protocol: protocol.value, apn: moduleApn.value, username: moduleUsername.value, password: modulePassword.value, auth: Number(moduleAuth.value) });
					}))
				]),
				c.card(_('Inbound routing'), _('Optional module-side forwarding for devices connected behind the MT5700M data path.'), [
					c.formRow(_('IP passthrough'), directIp),
					E('div', { 'class': 'mt-scan-note' }, directIpKnown ? _('IP passthrough is an original-manager compatibility feature. Keep it disabled when OpenWrt owns the mobile connection.') : _('This MT5700M firmware does not expose a readable IP passthrough setting. The control is disabled to prevent false success reports.')),
					directIpKnown ? c.actionBar(c.btn(_('Apply IP passthrough'), function() {
						c.confirmRoute(_('Change IP passthrough'), _('Changing passthrough can remove the module management address and interrupt connectivity.'), 'network.direct_ip_set', { enabled: directIp.value === '1' }, true);
					})) : null,
					c.formRow(_('Post-routing'), postRoute),
					E('div', { 'class': 'mt-scan-note' }, _('Post-routing and DMZ are mutually exclusive. Leave both disabled unless the module itself is providing the downstream LAN.')),
					postRouteKnown ? c.actionBar(c.btn(_('Apply post-routing'), function() {
						c.confirmRoute(_('Change post-routing'), _('The module must restart or cycle airplane mode before the new routing path is used.'), 'network.postroute_set', { mode: Number(postRoute.value) }, true);
					})) : E('div', { 'class': 'mt-scan-note' }, _('The modem reported a post-routing value that this firmware cannot safely change.')),
					c.formRow(_('DMZ address'), dmz),
					c.actionBar(c.btn(_('Apply DMZ'), function() {
						c.confirmRoute(_('Change DMZ host'), _('The selected IPv4 host may be exposed to unsolicited traffic from the mobile network.'), 'network.dmz_set', { host: dmz.value.trim() || '0' });
					}))
				])
			])
		]);

		// LuCI 的 view 契约要求 load() 同步返回一个 DOM 节点；返回 Promise
		// 会被当成字符串塞进容器，页面上直接出现字面量 “[object Promise]”
		// （本页此前就是这样——整个正文被这一行占满）。
		// 正确做法：同步返回外层容器，form.Map 渲染完成后异步注入正文。
		// 这也是本项目其它页面统一采用的异步化架构（Async Architecture）：
		// 首屏不等任何 AT 命令。
		//
		// 注意两点：
		//  1) LuCI 的 dom 模块只有 dom.content()/dom.append()，没有 dom.prepend()
		//     ——容器一律用原生 appendChild 组装。
		//  2) c.skeletonPage() 返回的元素**自带 mt-page 类**。若把它放进另一个
		//     .mt-page 里会得到嵌套的 .mt-page，两个都带 padding 与装饰伪元素，
		//     内容会被双倍内缩。所以这里：外层容器不带 mt-page，mt-page 放在 slot 上，
		//     骨架屏与 slot 平级。
		var skeleton = c.skeletonPage(4);
		var slot = E('div', { 'class': 'mt-page' }, []);
		var host = E('div', { 'class': 'mt-async-host' }, [ c.cssLink(), skeleton, slot ]);

		m.render().then(function(formNode) {
			dom.content(slot, [
				managerError ? E('div', { 'class': 'alert-message warning' }, _('Dial manager status is temporarily unavailable. ') + managerError) : null,
				device._rpcError ? E('div', { 'class': 'alert-message warning' }, _('Network interface status is temporarily unavailable. ') + device._rpcError) : null,
				manager.usb_state && manager.usb_state !== 'normal' ? E('div', { 'class': 'alert-message warning' }, _('The MT5700M is not in normal USB mode. Connection settings remain available, but dialing cannot start.')) : null,
					c.hero(_('Mobile Data'), _('Mobile data'), _('Configure how the MT5700M connects to the mobile network.'), [
					E('div', { 'class': 'mt-conn-state' }, [
						c.svgStatusPulse(online ? 'ok' : 'bad', 18),
						E('span', { 'class': 'mt-conn-state-text' }, online ? _('Connected') : _('Disconnected'))
					])
				], null, c.svgTower({ active: online, status: online ? 'ok' : 'bad' })),
				E('div', { 'class': 'mt-facts-grid', 'style': 'margin-bottom:14px' }, [
					self.fact(_('Automatic dialing'), uci.get('mt5700m', 'connection', 'enabled') === '0' ? _('Disabled') : _('Enabled')),
					self.fact(_('Network interface'), manager.network),
					self.fact('APN', configuredApn),
					self.fact(_('IP protocol'), configuredProtocol)
				]),
				self.sessionPanel(session),
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
			// 骨架屏整体移除（不是 dom.content 清空——那会留下一个空的
			// .mt-page .mt-skeleton-page 节点，继续占着 padding 与间距）
			if (skeleton && skeleton.parentNode)
				skeleton.parentNode.removeChild(skeleton);
		}).catch(function(err) {
			if (skeleton && skeleton.parentNode)
				skeleton.parentNode.removeChild(skeleton);
			dom.content(slot, E('div', { 'class': 'alert-message danger' },
				[ _('Failed to render the connection form.'), E('br'), String(err && err.message || err) ]));
		});

		// 同步返回容器，满足 view 契约
		return host;
	}
});
