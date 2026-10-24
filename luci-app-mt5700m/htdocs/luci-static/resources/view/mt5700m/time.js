'use strict';
'require view';
'require ui';
'require mt5700m.api as api';
'require mt5700m.components as c';

/*
 * MT5700M LuCI — 时间同步与 DNS（time）
 * ---------------------------------------------
 * 数据通道：只有 time.* / dns.* / task.get 路由；不发送任何模组 AT 命令。
 *   - 状态卡：统一时钟（设备时区）、NTP 服务商、同步可信度、上次成功/失败、
 *     自动重试。数据来自后端运行时状态（time.status），页面不自己探测。
 *   - NTP：主开关 / 服务器顺序与启用停用 / 恢复默认 / 保存 / 应用 / 立即同步。
 *     应用和同步都是后端异步任务，页面用 task_id 观察，绝不阻塞界面。
 *   - DNS：IPv4/IPv6 分族开关和主/备地址；保存（仅持久化）与保存并应用分开，
 *     页面明确区分「已保存 / 接口已应用 / 运行时校验」三种状态。
 *
 * 生命周期：
 *   - 两个面板只构建一次，切换 Tab 仅改 hidden，未保存的输入不会丢失；
 *   - Tab 选择写在 URL（timeTab），刷新/后退/前进恢复，轮询不重建表单；
 *   - 轮询只刷新状态卡与 DNS 运行态，每 20 s 一次，页面隐藏时跳过；
 *   - 页面离开（pagehide）停止计时器并中止任务观察（后端任务继续执行）。
 */

var MAX_SERVERS = 16;

function text(value, fallback) {
	return (value == null || value === '') ? (fallback || '--') : String(value);
}

function strings(value) {
	return Array.isArray(value) ? value.filter(function(v) { return typeof v === 'string' && v !== ''; }) : [];
}

function providerLabel(name) {
	switch (name) {
	case 'sysntpd': return _('sysntpd');
	case 'chrony': return _('chrony');
	case 'ntpd': return _('External ntpd');
	case 'none': return _('Not detected');
	default: return text(name, _('Unknown'));
	}
}

function errorKindLabel(kind) {
	switch (kind) {
	case 'dns': return _('DNS resolution failed');
	case 'network': return _('Network unreachable');
	case 'timeout': return _('NTP server did not respond');
	case 'server': return _('NTP server rejected the request');
	case 'clock': return _('Clock calibration failed');
	case 'config': return _('Configuration error');
	case 'internal': return _('Internal error');
	default: return '';
	}
}

function syncState(status) {
	var state = (status && status.state) || {};
	if (!status)
		return { kind: 'unknown', label: _('Unavailable') };
	if (status.trusted === true)
		return { kind: 'ok', label: _('Synchronized and trusted') };
	if (state.synced === true)
		return { kind: 'warn', label: _('Synchronized, evidence is stale') };
	if (state.synced === false)
		return { kind: 'bad', label: _('Not synchronized') };
	return { kind: 'unknown', label: _('Sync status unknown') };
}

function validV4(value) {
	var parts = String(value).split('.');
	if (parts.length !== 4) return false;
	for (var i = 0; i < 4; i++) {
		if (!/^(0|[1-9][0-9]{0,2})$/.test(parts[i])) return false;
		var n = Number(parts[i]);
		if (n > 255) return false;
	}
	if (parts[0] === '0' && parts[1] === '0' && parts[2] === '0' && parts[3] === '0') return false;
	if (Number(parts[0]) >= 224) return false;
	if (parts[0] === '255' && parts[1] === '255' && parts[2] === '255' && parts[3] === '255') return false;
	return true;
}

function validV6(value) {
	var v = String(value).trim();
	if (!v || v.indexOf(':') < 0) return false;
	if (v === '::') return false;
	if (/^ff/i.test(v)) return false;
	try {
		// The URL parser performs complete IPv6 syntax validation; the backend
		// validates again and remains authoritative.
		void new URL('http://[' + v + ']');
		return true;
	} catch (e) {
		return false;
	}
}

function validNtpHost(host) {
	var h = String(host || '').trim();
	if (!h) return _('NTP server cannot be empty.');
	if (h.length > 253) return _('NTP server name is too long.');
	if (h.charAt(0) === '-' || /\s/.test(h)) return _('NTP server name is invalid.');
	if (validV4(h)) return null;
	if (h.indexOf(':') >= 0) return validV6(h) ? null : _('NTP server name is invalid.');
	var labels = h.split('.');
	for (var i = 0; i < labels.length; i++) {
		var label = labels[i];
		if (!label || label.length > 63) return _('NTP server name is invalid.');
		if (label.charAt(0) === '-' || label.charAt(label.length - 1) === '-')
			return _('NTP server name is invalid.');
		if (!/^[A-Za-z0-9_-]+$/.test(label)) return _('NTP server name is invalid.');
	}
	return null;
}

function timeText(seconds) {
	if (!seconds) return '--';
	var n = Number(seconds);
	if (!isFinite(n) || n <= 0) return '--';
	// Only used when the backend did not provide the device-local text form.
	// The page must not re-interpret the timestamp with browser-local time.
	return new Date(n * 1000).toISOString().replace('T', ' ').replace(/\.000Z$/, ' UTC');
}

return view.extend({
	pollIntervalMs: 20000,
	pollTimer: null,
	refreshing: null,
	holder: null,
	ntpStatus: null,
	dnsStatus: null,
	ntpModel: null,
	dnsModel: null,
	panelsBuilt: false,
	ntpPanel: null,
	dnsPanel: null,
	ntpReady: false,
	dnsReady: false,
	ntpBusy: false,
	dnsBusy: false,
	abort: null,
	onPageHide: null,
	onPageShow: null,

	/* ------------------------------------------------------------ lifecycle */

	makeAbort: function() {
		this.abort = (typeof AbortController === 'function') ? new AbortController() : null;
	},

	waitOptions: function(extra) {
		var opts = extra || {};
		if (this.abort)
			opts.signal = this.abort.signal;
		return opts;
	},

	load: function() {
		var self = this;
		this.makeAbort();

		this.heroTagHost = E('div', { 'class': 'mt-conn-state' }, [
			c.statusTag('unknown', _('Loading…'))
		]);
		this.statusHost = E('div', { 'class': 'mt-time-status-host' }, [ this.skeletonCard(3) ]);
		this.panelsHost = E('div', { 'class': 'mt-time-panels' }, [ this.skeletonCard(4) ]);

		this.holder = E('div', { 'class': 'mt-page mt-time-page' }, [
			c.cssLink(),
			c.navTabs('time'),
			c.hero(_('TIME & DNS'), _('Time & DNS'),
				_('NTP synchronization and DNS servers for the plugin-managed cellular interfaces. Timezone always follows the device configuration.'),
				[ this.heroTagHost ], 'slate', c.svgIcon('clock', 64)),
			this.statusHost,
			this.panelsHost
		]);

		this.onPageHide = function() {
			self.stopPolling();
			if (self.abort) self.abort.abort();
		};
		this.onPageShow = function() {
			if (!self.holder || !self.holder.isConnected)
				return;
			self.makeAbort();
			self.startPolling();
			self.refresh(false);
		};
		window.addEventListener('pagehide', this.onPageHide);
		window.addEventListener('pageshow', this.onPageShow);

		this.refresh(true);
		return this.holder;
	},

	startPolling: function() {
		if (this.pollTimer)
			return;
		var self = this;
		this.pollTimer = window.setInterval(function() {
			if (!self.holder || !self.holder.isConnected) {
				self.release();
				return;
			}
			if (document.hidden)
				return;
			self.refresh(false);
		}, this.pollIntervalMs);
	},

	stopPolling: function() {
		if (this.pollTimer) {
			window.clearInterval(this.pollTimer);
			this.pollTimer = null;
		}
	},

	release: function() {
		this.stopPolling();
		if (this.onPageHide)
			window.removeEventListener('pagehide', this.onPageHide);
		if (this.onPageShow)
			window.removeEventListener('pageshow', this.onPageShow);
		this.onPageHide = null;
		this.onPageShow = null;
		if (this.abort) this.abort.abort();
	},

	/* --------------------------------------------------------------- helpers */

	skeletonCard: function(lines) {
		var bars = [];
		for (var i = 0; i < (lines || 3); i++)
			bars.push(E('div', { 'class': 'mt-skeleton-bar ' + (i === 0 ? 'w30' : i === 1 ? 'w70' : 'w50') }));
		return E('section', { 'class': 'mt-card mt-skeleton-card' }, bars);
	},

	note: function(kind, message) {
		return E('div', { 'class': 'mt-state-note' + (kind ? ' mt-state-note--' + kind : '') }, message);
	},

	field: function(label, input, help) {
		return E('div', { 'class': 'mt-form-field' }, [
			E('label', { 'class': 'mt-form-label' }, label),
			input,
			help ? E('div', { 'class': 'mt-form-help' }, help) : null
		]);
	},

	iconButton: function(icon, label, handler, disabled) {
		return E('button', {
			'type': 'button',
			'class': 'mt-btn mt-btn--sm mt-session-action',
			'title': label,
			'aria-label': label,
			'click': handler,
			'disabled': disabled ? 'disabled' : null
		}, [ E('span', { 'class': 'mt-btn-icon' }, c.svgIcon(icon, 14)) ]);
	},

	notify: function(message, kind) {
		ui.addNotification(null, E('p', {}, message), kind || 'info');
	},

	refresh: function(initial) {
		var self = this;
		if (!this.holder || !this.holder.isConnected)
			return Promise.resolve();
		if (this.refreshing)
			return this.refreshing;

		this.refreshing = Promise.all([
			api.route('time.status'),
			api.route('dns.status')
		]).then(function(results) {
			self.refreshing = null;
			if (!self.holder || !self.holder.isConnected)
				return;
			var t = results[0] || null;
			var d = results[1] || null;
			if (t) self.ntpStatus = t;
			if (d) self.dnsStatus = d;
			self.renderStatus(t, d);
			if (!self.panelsBuilt)
				self.buildPanels(t, d);
			else
				self.recoverPanels(t, d);
			self.startPolling();
		}, function(err) {
			self.refreshing = null;
			if (!self.holder || !self.holder.isConnected)
				return;
			if (initial || !self.panelsBuilt) {
				self.statusHost.replaceChildren(self.note('bad',
					_('Unable to read status: ') + text(err && err.message, String(err))));
				self.panelsHost.replaceChildren(self.note('bad',
					_('The background service is unavailable. This page only reads cached configuration and will recover automatically.')));
			}
			self.startPolling();
		});
		return this.refreshing;
	},

	/* -------------------------------------------------------- status display */

	renderStatus: function(t, d) {
		var self = this;
		var status = t || this.ntpStatus;
		var dns = d || this.dnsStatus;

		if (!status) {
			this.heroTagHost.replaceChildren(c.statusTag('bad', _('Backend unavailable')));
			this.statusHost.replaceChildren(c.card(_('Clock and synchronization'),
				_('No status was returned by the backend.'),
				[ this.note('warn', _('Check that the at-webserver service is running. Polling will retry automatically.')) ]));
			return;
		}

		var st = status.state || {};
		var apply = st.apply || {};
		var provider = status.provider || {};
		var clock = status.time || {};
		var tag = syncState(status);
		this.heroTagHost.replaceChildren(c.statusTag(tag.kind, tag.label));

		var facts = E('div', { 'class': 'mt-time-status' }, [
			c.fact(_('Device local time'), text(clock.local)),
			c.fact(_('Timezone'), text(clock.timezone)),
			c.fact(_('Unix timestamp'), text(clock.unix), _('UTC, independent of timezone')),
			c.fact(_('Clock plausible'), clock.plausible === true ? _('Yes') : _('No'),
				clock.plausible === true ? null : _('The system clock is before 2020 and cannot be trusted'))
		]);

		var rows = [
			c.stateRow(_('NTP provider'),
				providerLabel(provider.name) + ' · ' + (provider.running ? _('Running') : _('Not running')) +
				(provider.configurable ? '' : ' · ' + _('Read-only'))),
			c.stateRow(_('NTP management'), status.saved && status.saved.enabled ? _('Enabled') : _('Disabled')),
			c.stateRow(_('Effective servers'), strings(status.effectiveServers).join(', ')),
			c.stateRow(_('Current / last server'), text(st.server)),
			c.stateRow(_('Measured offset'), st.offsetMs == null ? '--' : String(st.offsetMs) + ' ms'),
			c.stateRow(_('Last successful sync'), text(st.lastSuccessAtText)),
			c.stateRow(_('Last attempt'), text(st.lastAttemptAtText)),
			c.stateRow(_('Last apply'), apply.result === 'ok'
				? _('Applied') + ' · ' + text(apply.atText)
				: apply.result === 'error' || apply.result === 'partial'
					? _('Failed') + ' · ' + text(apply.error)
					: _('Never applied')),
			c.stateRow(_('Verified'), status.verified ? _('Yes') : _('No')),
			c.stateRow(_('Saved vs applied'), status.pending ? _('Pending apply') : _('In sync'))
		];

		if (st.failures > 0 || st.nextRetryAt) {
			rows.push(c.stateRow(_('Automatic retry'),
				st.failures >= 6 ? _('Stopped after repeated failures') :
					(st.nextRetryAt ? _('Failed %s time(s), next retry %s')
						.format(String(st.failures || 0), timeText(st.nextRetryAt)) : _('Scheduled'))));
		}
		if (st.lastError) {
			rows.push(c.stateRow(_('Last error'),
				(errorKindLabel(st.lastErrorKind) ? errorKindLabel(st.lastErrorKind) + ': ' : '') + st.lastError));
		}
		if (status.applying || status.syncing) {
			rows.push(c.stateRow(_('Operation'), status.applying ? _('Applying NTP configuration…') : _('Synchronizing now…')));
		}

		var warnings = strings(status.warnings);
		var warningNodes = warnings.map(function(w) {
			return self.note(status.trusted ? 'warn' : 'warn', w);
		});

		var refreshButton = c.btn(_('Refresh status'), function() {
			self.refresh(false);
		}, { small: true, icon: 'refresh' });

		var body = [
			E('div', { 'class': 'mt-inline-actions', 'style': 'margin-bottom:10px' }, [
				c.statusTag(tag.kind, tag.label),
				clock.plausible === true ? null : c.statusTag('warn', _('Untrusted clock'))
			]),
			facts,
			E('div', { 'class': 'mt-advanced-form' }, rows),
			warningNodes.length ? E('div', { 'class': 'mt-advanced-form', 'style': 'gap:6px;margin-top:10px' }, warningNodes) : null,
			E('div', { 'class': 'mt-form-actions' }, [ refreshButton ])
		];

		if (dns && dns.verified) {
			var v4 = dns.verified.ipv4, v6 = dns.verified.ipv6;
			body.splice(4, 0, E('div', { 'class': 'mt-form-help' },
				_('DNS verification: IPv4 %s, IPv6 %s').format(
					v4 === true ? _('verified') : v4 === false ? _('mismatch') : _('not verifiable'),
					v6 === true ? _('verified') : v6 === false ? _('mismatch') : _('not verifiable'))));
		}

		this.statusHost.replaceChildren(c.card(_('Clock and synchronization'),
			_('The clock is read through the backend time service. This card never sends NTP packets; use manual sync in the NTP panel.'),
			body));

		this.updateActionState();
	},

	/* ---------------------------------------------------------- panel layout */

	buildPanels: function(t, d) {
		var self = this;
		var active = c.tabParam('timeTab', 'ntp', [ 'ntp', 'dns' ]);

		this.ntpPanel = this.buildNtpPanel(t);
		this.dnsPanel = this.buildDnsPanel(d);
		this.ntpReady = !!t;
		this.dnsReady = !!d;

		var nav = c.panelTabs([
			{ key: 'ntp', label: _('NTP time sync'), icon: 'clock' },
			{ key: 'dns', label: _('DNS servers'), icon: 'sliders' }
		], active, function(key) {
			self.showPanel(key);
		}, { param: 'timeTab' });

		this.panelNav = nav;
		this.panelsHost.replaceChildren(nav, this.ntpPanel, this.dnsPanel);
		this.showPanel(active);
		this.panelsBuilt = true;
		this.updateActionState();
	},

	showPanel: function(key) {
		if (!this.ntpPanel || !this.dnsPanel)
			return;
		this.ntpPanel.hidden = key !== 'ntp';
		this.dnsPanel.hidden = key !== 'dns';
		if (this.ntpPanel.hidden) this.ntpPanel.classList.remove('mt-panel');
		else this.ntpPanel.classList.add('mt-panel');
		if (this.dnsPanel.hidden) this.dnsPanel.classList.remove('mt-panel');
		else this.dnsPanel.classList.add('mt-panel');
	},

	recoverPanels: function(t, d) {
		if (t && !this.ntpReady) {
			var ntp = this.buildNtpPanel(t);
			this.ntpPanel.replaceWith(ntp);
			this.ntpPanel = ntp;
			this.ntpReady = true;
			this.showPanel(c.tabParam('timeTab', 'ntp', [ 'ntp', 'dns' ]));
		}
		if (d && !this.dnsReady) {
			var dns = this.buildDnsPanel(d);
			this.dnsPanel.replaceWith(dns);
			this.dnsPanel = dns;
			this.dnsReady = true;
			this.showPanel(c.tabParam('timeTab', 'ntp', [ 'ntp', 'dns' ]));
		}
		if (d && this.dnsStateHost)
			this.renderDnsState(this.dnsStatus);
	},

	unavailablePanel: function(message) {
		var self = this;
		return E('section', { 'class': 'mt-card' }, [
			this.note('bad', message),
			E('div', { 'class': 'mt-form-actions' }, [
				c.btn(_('Retry now'), function() { self.refresh(false); }, { small: true, icon: 'refresh' })
			])
		]);
	},

	/* ------------------------------------------------------------- NTP panel */

	buildNtpPanel: function(status) {
		var self = this;
		if (!status)
			return this.unavailablePanel(_('NTP status is unavailable. The background service may be starting.'));

		var cfg = status.saved || status.defaults;
		if (!cfg)
			return this.unavailablePanel(_('The saved NTP configuration is unavailable.'));

		this.ntpModel = {
			enabled: cfg.enabled !== false,
			servers: (Array.isArray(cfg.servers) ? cfg.servers : []).map(function(s) {
				return { host: String(s.host || ''), enabled: s.enabled !== false };
			})
		};

		this.ntpEnabledInput = E('input', { 'type': 'checkbox' });
		this.ntpEnabledInput.checked = this.ntpModel.enabled;
		this.ntpEnabledInput.addEventListener('change', function() {
			self.ntpModel.enabled = this.checked;
			self.ntpDirty = true;
			self.updateNtpNote();
		});

		this.ntpListHost = E('div', { 'class': 'mt-ntp-list' });
		this.ntpAddInput = E('input', {
			'class': 'cbi-input-text',
			'type': 'text',
			'placeholder': _('e.g. ntp.example.com'),
			'aria-label': _('New NTP server')
		});
		this.ntpAddInput.addEventListener('keydown', function(ev) {
			if (ev.key === 'Enter') {
				ev.preventDefault();
				self.addNtpServer();
			}
		});

		this.ntpError = E('div', { 'class': 'mt-form-error', 'role': 'alert' });
		this.ntpNote = E('div', { 'class': 'mt-form-help' });

		var addButton = c.btn(_('Add server'), function() { self.addNtpServer(); }, { small: true, icon: 'plus' });

		this.ntpSaveBtn = c.btn(_('Save'), function() { self.saveNtp(this); }, { small: true });
		this.ntpApplyBtn = c.btn(_('Apply to system'), function() { self.applyNtp(this); },
			{ small: true, primary: true, icon: 'check' });
		this.ntpSyncBtn = c.btn(_('Sync now'), function() { self.syncNtp(this); },
			{ small: true, icon: 'play' });
		this.ntpResetBtn = c.btn(_('Restore defaults'), function() { self.resetNtp(this); }, { small: true });

		this.renderNtpRows();
		this.updateNtpNote();

		var savedAt = (status.state && status.state.apply && status.state.apply.atText) || null;
		var panel = c.card(_('NTP servers'),
			_('Servers are tried in order. Disabled entries stay in the list but are not applied to the system NTP service.'),
			[
				E('label', { 'class': 'mt-check-row', 'style': 'margin-bottom:10px' }, [
					this.ntpEnabledInput,
					E('span', {}, [ E('strong', {}, _('NTP management')), E('br'),
						E('span', { 'class': 'mt-form-help' },
							_('When disabled, the plugin restores the system NTP configuration it backed up before first use.')) ])
				]),
				this.ntpListHost,
				E('div', { 'class': 'mt-ntp-add' }, [ this.ntpAddInput, addButton ]),
				this.ntpNote,
				this.ntpError,
				E('div', { 'class': 'mt-form-actions' },
					[ this.ntpResetBtn, this.ntpSaveBtn, this.ntpApplyBtn, this.ntpSyncBtn ]),
				E('div', { 'class': 'mt-time-apply-row' }, [
					E('span', { 'class': 'mt-time-apply-meta' },
						savedAt ? _('Last applied: %s').format(savedAt) : _('Configuration has never been applied.')),
					E('span', { 'class': 'mt-time-apply-meta' },
						_('Saving only persists the configuration. “Apply to system” restarts the detected NTP service.'))
				])
			]);
		return panel;
	},

	updateNtpNote: function() {
		if (!this.ntpNote) return;
		var enabled = this.ntpModel && this.ntpModel.servers.filter(function(s) {
			return s.enabled && String(s.host || '').trim();
		}).length;
		this.ntpNote.textContent = this.ntpModel && this.ntpModel.enabled
			? _('%s enabled server(s); the first one is preferred.').format(String(enabled))
			: _('NTP management is disabled. Existing system servers are restored when applied.');
	},

	renderNtpRows: function() {
		var self = this;
		var rows = [];
		this.ntpModel.servers.forEach(function(server, index) {
			var hostInput = E('input', {
				'class': 'cbi-input-text',
				'type': 'text',
				'value': server.host,
				'aria-label': _('NTP server %s').format(String(index + 1))
			});
			hostInput.addEventListener('input', function() {
				server.host = this.value;
				self.ntpDirty = true;
				self.updateNtpNote();
			});

			var toggle = E('input', { 'type': 'checkbox' });
			toggle.checked = !!server.enabled;
			toggle.addEventListener('change', function() {
				server.enabled = this.checked;
				self.ntpDirty = true;
				row.classList.toggle('is-disabled', !this.checked);
				self.updateNtpNote();
			});

			var up = self.iconButton('up', _('Move up'), function() { self.moveNtpServer(index, -1); }, index === 0);
			var down = self.iconButton('down', _('Move down'), function() { self.moveNtpServer(index, 1); },
				index === self.ntpModel.servers.length - 1);
			var remove = self.iconButton('trash', _('Delete'), function() { self.removeNtpServer(index); }, false);

			var row = E('div', { 'class': 'mt-ntp-row' + (server.enabled ? '' : ' is-disabled') }, [
				E('span', { 'class': 'mt-ntp-order', 'title': _('Priority %s').format(String(index + 1)) }, String(index + 1)),
				hostInput,
				E('label', { 'class': 'mt-check-row mt-ntp-flag' }, [
					toggle, E('span', {}, _('Enabled'))
				]),
				E('div', { 'class': 'mt-inline-actions' }, [ up, down, remove ])
			]);
			rows.push(row);
		});
		if (!rows.length)
			rows.push(this.note('warn', _('No NTP servers configured. Add at least one server before saving.')));
		this.ntpListHost.replaceChildren.apply(this.ntpListHost, rows);
	},

	addNtpServer: function() {
		var value = String(this.ntpAddInput.value || '').trim();
		var err = validNtpHost(value);
		if (err) {
			this.showNtpError(err);
			this.ntpAddInput.focus();
			return;
		}
		var duplicate = this.ntpModel.servers.some(function(s) {
			return String(s.host || '').trim().toLowerCase() === value.toLowerCase();
		});
		if (duplicate) {
			this.showNtpError(_('This server is already in the list.'));
			return;
		}
		if (this.ntpModel.servers.length >= MAX_SERVERS) {
			this.showNtpError(_('A maximum of %s NTP servers is allowed.').format(String(MAX_SERVERS)));
			return;
		}
		this.ntpModel.servers.push({ host: value, enabled: true });
		this.ntpAddInput.value = '';
		this.ntpDirty = true;
		this.clearNtpError();
		this.renderNtpRows();
		this.updateNtpNote();
	},

	removeNtpServer: function(index) {
		this.ntpModel.servers.splice(index, 1);
		this.ntpDirty = true;
		this.clearNtpError();
		this.renderNtpRows();
		this.updateNtpNote();
	},

	moveNtpServer: function(index, delta) {
		var target = index + delta;
		if (target < 0 || target >= this.ntpModel.servers.length)
			return;
		var item = this.ntpModel.servers.splice(index, 1)[0];
		this.ntpModel.servers.splice(target, 0, item);
		this.ntpDirty = true;
		this.clearNtpError();
		this.renderNtpRows();
		this.updateNtpNote();
	},

	showNtpError: function(message) {
		if (!this.ntpError) return;
		this.ntpError.textContent = message;
	},

	clearNtpError: function() {
		if (this.ntpError) this.ntpError.textContent = '';
	},

	validateNtp: function() {
		var model = this.ntpModel;
		if (!model)
			return _('NTP configuration is not loaded.');
		if (!model.servers.length)
			return _('Add at least one NTP server.');
		if (model.servers.length > MAX_SERVERS)
			return _('A maximum of %s NTP servers is allowed.').format(String(MAX_SERVERS));
		var seen = {};
		var enabled = 0;
		for (var i = 0; i < model.servers.length; i++) {
			var host = String(model.servers[i].host || '').trim();
			var err = validNtpHost(host);
			if (err) return _('Server %s: %s').format(String(i + 1), err);
			var key = host.toLowerCase();
			if (seen[key]) return _('Duplicate NTP server: %s').format(host);
			seen[key] = true;
			if (model.servers[i].enabled) enabled++;
		}
		if (model.enabled && !enabled)
			return _('Enable at least one NTP server, or turn NTP management off.');
		return null;
	},

	collectNtp: function() {
		return {
			enabled: !!this.ntpModel.enabled,
			servers: this.ntpModel.servers.map(function(s) {
				return { host: String(s.host || '').trim(), enabled: !!s.enabled };
			})
		};
	},

	saveNtp: function(button) {
		var self = this;
		var err = this.validateNtp();
		if (err) {
			this.showNtpError(err);
			return Promise.resolve();
		}
		this.clearNtpError();
		this.ntpBusy = true;
		if (button && button.setBusy) button.setBusy(true);
		this.updateActionState();
		return api.routeCall('time.config_set', this.collectNtp()).then(function(res) {
			self.ntpDirty = false;
			self.notify(_('NTP configuration saved. It has not been applied to the system service yet.'), 'info');
			return self.refresh(false).then(function() { return res; });
		}).then(function(res) {
			self.ntpBusy = false;
			if (button && button.setBusy) button.setBusy(false);
			self.updateActionState();
			return res;
		}, function(err2) {
			self.ntpBusy = false;
			if (button && button.setBusy) button.setBusy(false);
			self.showNtpError(text(err2 && err2.message, String(err2)));
			self.updateActionState();
			throw err2;
		});
	},

	applyNtp: function(button) {
		var self = this;
		var progress = E('div', { 'class': 'mt-state-note' }, _('Waiting for the backend…'));
		c.confirmDialog({
			title: _('Apply NTP configuration'),
			message: _('The saved NTP configuration will be written to the detected system service and the service may be restarted.'),
			content: progress,
			confirmLabel: _('Apply'),
			run: function() {
				if (self.ntpBusy) throw new Error(_('Another NTP operation is already running.'));
				self.ntpBusy = true;
				if (button && button.setBusy) button.setBusy(true);
				self.updateActionState();
				return api.routeCall('time.apply').then(function(res) {
					if (!res.task_id) throw new Error(_('The backend did not return a task id.'));
					progress.textContent = _('Applying configuration…');
					return api.waitTask(res.task_id, self.waitOptions({
						timeout: 90000,
						onProgress: function(rec) {
							progress.textContent = rec.detail || rec.status || _('Applying configuration…');
						}
					}));
				}).then(function(rec) {
					return self.refresh(false).then(function() { return rec; });
				}).then(function(rec) {
					self.ntpBusy = false;
					if (button && button.setBusy) button.setBusy(false);
					self.updateActionState();
					return rec;
				}, function(err) {
					self.ntpBusy = false;
					if (button && button.setBusy) button.setBusy(false);
					self.updateActionState();
					throw err;
				});
			},
			onSuccess: function() {
				self.notify(_('NTP configuration applied.'), 'info');
			},
			onError: function(err) {
				if (err && err.cancelled) return;
				self.notify(_('Failed to apply NTP configuration: ') + text(err && err.message, String(err)), 'danger');
			}
		});
	},

	syncNtp: function(button) {
		var self = this;
		var progress = E('div', { 'class': 'mt-state-note' }, _('Preparing manual synchronization…'));
		c.confirmDialog({
			title: _('Synchronize now'),
			message: _('The backend will trigger the detected system NTP service to synchronize immediately and report the verified result. If no system service exists, it runs one one-shot query.'),
			content: progress,
			confirmLabel: _('Sync now'),
			run: function() {
				if (self.ntpBusy) throw new Error(_('Another NTP operation is already running.'));
				self.ntpBusy = true;
				if (button && button.setBusy) button.setBusy(true);
				self.updateActionState();
				return api.routeCall('time.sync').then(function(res) {
					if (!res.task_id) throw new Error(_('The backend did not return a task id.'));
					progress.textContent = _('Contacting NTP servers…');
					return api.waitTask(res.task_id, self.waitOptions({
						timeout: 150000,
						onProgress: function(rec) {
							progress.textContent = rec.detail || rec.status || _('Contacting NTP servers…');
						}
					}));
				}).then(function(rec) {
					return self.refresh(false).then(function() { return rec; });
				}).then(function(rec) {
					self.ntpBusy = false;
					if (button && button.setBusy) button.setBusy(false);
					self.updateActionState();
					return rec;
				}, function(err) {
					self.ntpBusy = false;
					if (button && button.setBusy) button.setBusy(false);
					self.updateActionState();
					throw err;
				});
			},
			onSuccess: function(rec) {
				var result = (rec && rec.result) || {};
				if (result.synced === true) {
					self.notify(_('Clock synchronized successfully.') +
						(result.server ? ' ' + _('Server: %s').format(result.server) : '') +
						(result.offsetMs == null ? '' : ' ' + _('Offset: %s ms').format(String(result.offsetMs))), 'info');
				} else {
					self.notify(_('The sync command completed, but the result could not be verified. The clock remains untrusted.'), 'warning');
				}
			},
			onError: function(err) {
				if (err && err.cancelled) return;
				self.notify(_('Synchronization failed: ') + text(err && err.message, String(err)), 'danger');
			}
		});
	},

	resetNtp: function(button) {
		var self = this;
		c.confirmDialog({
			title: _('Restore default NTP servers'),
			message: _('The three documented servers will replace the saved list. Nothing is applied until you press “Apply to system”.'),
			confirmLabel: _('Restore defaults'),
			run: function() {
				self.ntpBusy = true;
				if (button && button.setBusy) button.setBusy(true);
				self.updateActionState();
				return api.routeCall('time.reset').then(function(res) {
					var cfg = res.config || res;
					self.ntpModel = {
						enabled: cfg.enabled !== false,
						servers: (Array.isArray(cfg.servers) ? cfg.servers : []).map(function(s) {
							return { host: String(s.host || ''), enabled: s.enabled !== false };
						})
					};
					if (self.ntpEnabledInput) self.ntpEnabledInput.checked = self.ntpModel.enabled;
					self.ntpDirty = false;
					self.renderNtpRows();
					self.updateNtpNote();
					self.ntpBusy = false;
					if (button && button.setBusy) button.setBusy(false);
					self.updateActionState();
					return self.refresh(false).then(function() { return res; });
				}, function(err) {
					self.ntpBusy = false;
					if (button && button.setBusy) button.setBusy(false);
					self.updateActionState();
					throw err;
				});
			},
			onSuccess: function() {
				self.notify(_('Default NTP servers restored (not applied).'), 'info');
			},
			onError: function(err) {
				if (err && err.cancelled) return;
				self.notify(_('Failed to restore defaults: ') + text(err && err.message, String(err)), 'danger');
			}
		});
	},

	/* ------------------------------------------------------------- DNS panel */

	buildDnsPanel: function(status) {
		var self = this;
		if (!status)
			return this.unavailablePanel(_('DNS status is unavailable. The background service may be starting.'));

		var cfg = status.saved || status.defaults;
		if (!cfg)
			return this.unavailablePanel(_('The saved DNS configuration is unavailable.'));

		this.dnsModel = {
			ipv4: {
				enabled: cfg.ipv4 ? cfg.ipv4.enabled !== false : true,
				primary: String((cfg.ipv4 && cfg.ipv4.primary) || ''),
				secondary: String((cfg.ipv4 && cfg.ipv4.secondary) || '')
			},
			ipv6: {
				enabled: cfg.ipv6 ? cfg.ipv6.enabled !== false : true,
				primary: String((cfg.ipv6 && cfg.ipv6.primary) || ''),
				secondary: String((cfg.ipv6 && cfg.ipv6.secondary) || '')
			}
		};
		this.dnsErrors = { ipv4: {}, ipv6: {} };

		this.dnsStateHost = E('div', { 'class': 'mt-dns-state' });
		this.dnsError = E('div', { 'class': 'mt-form-error', 'role': 'alert' });
		this.dnsNote = E('div', { 'class': 'mt-form-help mt-dns-note' });

		var v4 = this.dnsFamilyBlock('ipv4', _('IPv4 DNS'), _('Used by the MT5700M IPv4 interface.'));
		var v6 = this.dnsFamilyBlock('ipv6', _('IPv6 DNS'), _('Used by the MT5700M IPv6 interface when it is available. An unavailable IPv6 interface does not affect IPv4.'));

		this.dnsSaveBtn = c.btn(_('Save'), function() { self.saveDns(this, false); }, { small: true });
		this.dnsApplyBtn = c.btn(_('Save & apply'), function() { self.saveDns(this, true); },
			{ small: true, primary: true, icon: 'check' });
		this.dnsResetBtn = c.btn(_('Restore defaults'), function() { self.resetDns(this); }, { small: true });

		var formCard = c.card(_('DNS servers'),
			_('Only the plugin-managed interfaces (MT5700M / MT5700Mv6) are changed. Global DNS, LAN, WAN and DHCP settings are not touched.'),
			[
				v4,
				v6,
				this.dnsNote,
				this.dnsError,
				E('div', { 'class': 'mt-form-actions' }, [ this.dnsResetBtn, this.dnsSaveBtn, this.dnsApplyBtn ]),
				E('div', { 'class': 'mt-time-apply-meta' },
					_('“Save” only persists the configuration. “Save & apply” also updates the interfaces and verifies the runtime resolvers.'))
			]);

		var stateCard = c.card(_('Effective DNS on the plugin interfaces'),
			_('Saved, applied and runtime-verified are separate states. Runtime values are read from the live interfaces.'),
			[ this.dnsStateHost ]);

		var panel = E('div', {}, [ formCard, stateCard ]);
		this.updateDnsNote();
		this.renderDnsState(status);
		return panel;
	},

	dnsFamilyBlock: function(family, title, description) {
		var self = this;
		var model = this.dnsModel[family];
		var enabled = E('input', { 'type': 'checkbox' });
		enabled.checked = !!model.enabled;
		enabled.addEventListener('change', function() {
			model.enabled = this.checked;
			self.dnsDirty = true;
			block.classList.toggle('is-disabled', !this.checked);
			self.updateDnsNote();
		});

		var primary = E('input', {
			'class': 'cbi-input-text',
			'type': 'text',
			'value': model.primary,
			'placeholder': family === 'ipv4' ? '119.29.29.29' : '2402:4e00::',
			'aria-label': title + ' ' + _('Primary')
		});
		primary.addEventListener('input', function() {
			model.primary = this.value;
			self.dnsDirty = true;
			self.clearDnsFieldError(family, 'primary');
		});

		var secondary = E('input', {
			'class': 'cbi-input-text',
			'type': 'text',
			'value': model.secondary,
			'placeholder': family === 'ipv4' ? '223.5.5.5' : '2400:3200::1',
			'aria-label': title + ' ' + _('Secondary')
		});
		secondary.addEventListener('input', function() {
			model.secondary = this.value;
			self.dnsDirty = true;
			self.clearDnsFieldError(family, 'secondary');
		});

		var primaryError = E('div', { 'class': 'mt-form-error', 'role': 'alert' });
		var secondaryError = E('div', { 'class': 'mt-form-error', 'role': 'alert' });
		var primaryField = this.field(_('Primary DNS'), primary, _('Required when this family is enabled.'));
		var secondaryField = this.field(_('Secondary DNS'), secondary, _('Optional fallback.'));
		this.dnsErrors[family].primary = { node: primaryError, field: primaryField };
		this.dnsErrors[family].secondary = { node: secondaryError, field: secondaryField };

		var block = E('div', { 'class': 'mt-dns-family' + (model.enabled ? '' : ' is-disabled') }, [
			E('div', { 'class': 'mt-dns-family-head' }, [
				E('div', { 'class': 'mt-dns-family-title' }, [
					E('label', { 'class': 'mt-check-row' }, [ enabled, E('span', {}, title) ])
				]),
				E('span', { 'class': 'mt-form-help' }, description)
			]),
			E('div', { 'class': 'mt-dns-family-body' }, [
				primaryField,
				primaryError,
				secondaryField,
				secondaryError
			])
		]);
		return block;
	},

	updateDnsNote: function() {
		if (!this.dnsNote) return;
		var active = [];
		if (this.dnsModel.ipv4.enabled) active.push('IPv4');
		if (this.dnsModel.ipv6.enabled) active.push('IPv6');
		this.dnsNote.textContent = active.length
			? _('Will apply: %s').format(active.join(' + '))
			: _('Both families are disabled; applying removes the plugin DNS entries.');
	},

	clearDnsFieldError: function(family, field) {
		var entry = this.dnsErrors && this.dnsErrors[family] && this.dnsErrors[family][field];
		if (entry && entry.node) {
			entry.node.textContent = '';
			if (entry.field) entry.field.classList.remove('has-error');
		}
		this.clearDnsError();
	},

	clearDnsError: function() {
		if (this.dnsError) this.dnsError.textContent = '';
	},

	showDnsFieldError: function(family, field, message) {
		var entry = this.dnsErrors && this.dnsErrors[family] && this.dnsErrors[family][field];
		if (entry && entry.node) {
			entry.node.textContent = message;
			if (entry.field) entry.field.classList.add('has-error');
		} else if (this.dnsError) {
			this.dnsError.textContent = message;
		}
	},

	validateDns: function() {
		var model = this.dnsModel;
		if (!model) return { message: _('DNS configuration is not loaded.') };
		var families = [ 'ipv4', 'ipv6' ];
		for (var i = 0; i < families.length; i++) {
			var fam = families[i];
			var cfg = model[fam];
			var valid = fam === 'ipv4' ? validV4 : validV6;
			var primary = String(cfg.primary || '').trim();
			var secondary = String(cfg.secondary || '').trim();
			if (cfg.enabled && !primary)
				return { family: fam, field: 'primary', message: _('A primary DNS server is required when this family is enabled.') };
			if (primary && !valid(primary))
				return { family: fam, field: 'primary', message: _('Invalid %s address: %s').format(fam === 'ipv4' ? 'IPv4' : 'IPv6', primary) };
			if (secondary && !valid(secondary))
				return { family: fam, field: 'secondary', message: _('Invalid %s address: %s').format(fam === 'ipv4' ? 'IPv4' : 'IPv6', secondary) };
		}
		return null;
	},

	collectDns: function() {
		var self = this;
		return {
			ipv4: {
				enabled: !!this.dnsModel.ipv4.enabled,
				primary: String(this.dnsModel.ipv4.primary || '').trim(),
				secondary: String(this.dnsModel.ipv4.secondary || '').trim()
			},
			ipv6: {
				enabled: !!this.dnsModel.ipv6.enabled,
				primary: String(this.dnsModel.ipv6.primary || '').trim(),
				secondary: String(this.dnsModel.ipv6.secondary || '').trim()
			}
		};
	},

	applyDnsConfigToForm: function(cfg) {
		if (!cfg || !cfg.ipv4 || !cfg.ipv6)
			return;
		this.dnsModel = {
			ipv4: {
				enabled: cfg.ipv4.enabled !== false,
				primary: String(cfg.ipv4.primary || ''),
				secondary: String(cfg.ipv4.secondary || '')
			},
			ipv6: {
				enabled: cfg.ipv6.enabled !== false,
				primary: String(cfg.ipv6.primary || ''),
				secondary: String(cfg.ipv6.secondary || '')
			}
		};
	},

	saveDns: function(button, apply) {
		var self = this;
		this.clearDnsError();
		var err = this.validateDns();
		if (err) {
			if (err.family) this.showDnsFieldError(err.family, err.field, err.message);
			else this.dnsError.textContent = err.message;
			return Promise.resolve();
		}
		if (this.dnsBusy)
			return Promise.resolve();

		var payload = this.collectDns();
		if (!apply) {
			this.dnsBusy = true;
			if (button && button.setBusy) button.setBusy(true);
			this.updateActionState();
			return api.routeCall('dns.config_set', payload).then(function(res) {
				self.dnsDirty = false;
				self.notify(_('DNS configuration saved. Apply it to update the interfaces.'), 'info');
				return self.refresh(false).then(function() { return res; });
			}).then(function(res) {
				self.dnsBusy = false;
				if (button && button.setBusy) button.setBusy(false);
				self.updateActionState();
				return res;
			}, function(err2) {
				self.dnsBusy = false;
				if (button && button.setBusy) button.setBusy(false);
				self.dnsError.textContent = text(err2 && err2.message, String(err2));
				self.updateActionState();
				throw err2;
			});
		}

		var progress = E('div', { 'class': 'mt-state-note' }, _('Saving and applying the DNS configuration…'));
		c.confirmDialog({
			title: _('Save and apply DNS configuration'),
			message: _('The plugin interfaces will be updated and their runtime resolvers verified. IPv4 is never blocked by an unavailable IPv6 interface.'),
			content: progress,
			confirmLabel: _('Save & apply'),
			run: function() {
				if (self.dnsBusy) throw new Error(_('Another DNS operation is already running.'));
				self.dnsBusy = true;
				if (button && button.setBusy) button.setBusy(true);
				self.updateActionState();
				return api.routeCall('dns.config_set', payload).then(function(res) {
					self.dnsDirty = false;
					progress.textContent = _('Configuration saved. Applying to interfaces…');
					return api.routeCall('dns.apply').then(function(start) {
						if (!start.task_id) throw new Error(_('The backend did not return a task id.'));
						return api.waitTask(start.task_id, self.waitOptions({
							timeout: 90000,
							onProgress: function(rec) {
								progress.textContent = rec.detail || rec.status || _('Applying DNS configuration…');
							}
						}));
					});
				}).then(function(rec) {
					return self.refresh(false).then(function() { return rec; });
				}).then(function(rec) {
					self.dnsBusy = false;
					if (button && button.setBusy) button.setBusy(false);
					self.updateActionState();
					return rec;
				}, function(err2) {
					self.dnsBusy = false;
					if (button && button.setBusy) button.setBusy(false);
					self.updateActionState();
					throw err2;
				});
			},
			onSuccess: function(rec) {
				var result = (rec && rec.result) || {};
				var warnings = strings(result.warnings);
				if (warnings.length)
					self.notify(_('DNS configuration applied with warnings: ') + warnings.join(' '), 'warning');
				else
					self.notify(_('DNS configuration applied and verified.'), 'info');
			},
			onError: function(err2) {
				if (err2 && err2.cancelled) return;
				self.notify(_('Failed to apply DNS configuration: ') + text(err2 && err2.message, String(err2)), 'danger');
			}
		});
		return Promise.resolve();
	},

	resetDns: function(button) {
		var self = this;
		c.confirmDialog({
			title: _('Restore default DNS servers'),
			message: _('The documented IPv4 and IPv6 defaults will replace the saved values. Nothing is applied until you press “Save & apply”.'),
			confirmLabel: _('Restore defaults'),
			run: function() {
				self.dnsBusy = true;
				if (button && button.setBusy) button.setBusy(true);
				self.updateActionState();
				return api.routeCall('dns.reset').then(function(res) {
					if (res.config) self.applyDnsConfigToForm(res.config);
					self.dnsDirty = false;
					self.dnsBusy = false;
					if (button && button.setBusy) button.setBusy(false);
					self.updateActionState();
					return self.refresh(false).then(function() { return res; });
				}, function(err) {
					self.dnsBusy = false;
					if (button && button.setBusy) button.setBusy(false);
					self.updateActionState();
					throw err;
				});
			},
			onSuccess: function() {
				// The form is rebuilt from the backend response, so unsaved input
				// is intentionally replaced only after an explicit reset.
				if (self.dnsReady) {
					var replacement = self.buildDnsPanel(self.dnsStatus);
					self.dnsPanel.replaceWith(replacement);
					self.dnsPanel = replacement;
					self.showPanel('dns');
				}
				self.notify(_('Default DNS servers restored (not applied).'), 'info');
			},
			onError: function(err) {
				if (err && err.cancelled) return;
				self.notify(_('Failed to restore defaults: ') + text(err && err.message, String(err)), 'danger');
			}
		});
	},

	/* ---------------------------------------------------- effective DNS view */

	renderDnsState: function(status) {
		var self = this;
		if (!this.dnsStateHost)
			return;
		if (!status) {
			this.dnsStateHost.replaceChildren(this.note('warn',
				_('Runtime DNS state is unavailable.')));
			return;
		}
		var state = status.state || {};
		var runtime = status.runtime || {};
		var verified = status.verified || {};
		var pending = status.pending || {};
		var interfaces = status.interfaces || {};
		var saved = status.saved || {};
		var applied = state.applied || {};

		var cards = [ 'ipv4', 'ipv6' ].map(function(fam) {
			var familyLabel = fam === 'ipv4' ? 'IPv4' : 'IPv6';
			var cfg = saved[fam] || {};
			var rt = runtime[fam] || {};
			var ok = verified[fam];
			var reason = verified[fam + 'Reason'];
			var kind = !cfg.enabled ? 'unknown' : pending[fam] ? 'busy' :
				ok === true ? 'ok' : ok === false ? 'bad' : 'warn';
			var label = !cfg.enabled ? _('Disabled') :
				pending[fam] ? _('Pending apply') :
					ok === true ? _('Verified') :
						ok === false ? _('Runtime mismatch') : _('Not verifiable');
			return E('div', { 'class': 'mt-dns-family' }, [
				E('div', { 'class': 'mt-dns-family-head' }, [
					E('div', { 'class': 'mt-dns-family-title' }, [ E('strong', {}, familyLabel) ]),
					c.statusTag(kind, label)
				]),
				E('div', { 'class': 'mt-advanced-form' }, [
					c.stateRow(_('Interface'), text(interfaces[fam], '--')),
					c.stateRow(_('Configured / up'),
						(rt.configured ? _('configured') : _('not configured')) + ' / ' +
						(rt.up ? _('up') : _('down'))),
					c.stateRow(_('Runtime DNS'), strings(rt.dns).join(', ') || _('none')),
					c.stateRow(_('Matches saved'), rt.matchesSaved ? _('Yes') : _('No')),
					c.stateRow(_('L3 device'), text(rt.l3Device)),
					c.stateRow(_('Applied at'), text(state.appliedAtText)),
					c.stateRow(_('Applied servers'), strings((applied[fam] || [])).join(', ') || _('none'))
				]),
				E('div', { 'class': 'mt-form-help' },
					ok === true ? _('The live interface uses exactly the saved %s resolvers.').format(familyLabel) :
						ok === false ? _('The live interface differs from the saved configuration. Apply again and check the interface.')
							: (reason ? reason : _('Runtime verification is not possible while this family is disabled or its interface is absent.')))
			]);
		});

		var warnings = strings(status.warnings).map(function(w) {
			return self.note('warn', w);
		});
		var appliedResult = state.result === 'ok' ? _('Applied') :
			state.result === 'partial' ? _('Partially applied') :
				state.result === 'error' ? _('Failed') : _('Never applied');

		this.dnsStateHost.replaceChildren(
			E('div', { 'class': 'mt-inline-actions', 'style': 'margin-bottom:10px' }, [
				c.statusTag(state.result === 'error' ? 'bad' : state.result === 'ok' ? 'ok' : state.result === 'partial' ? 'warn' : 'unknown',
					appliedResult + (state.appliedAtText ? ' · ' + state.appliedAtText : '')),
				state.v6Skipped ? c.statusTag('warn', _('IPv6 was unavailable at last apply')) : null
			]),
			E('div', { 'class': 'mt-dns-family-body' }, cards),
			state.error ? this.note('bad', _('Last apply error: ') + state.error) : null,
			warnings.length ? E('div', { 'class': 'mt-advanced-form', 'style': 'gap:6px;margin-top:10px' }, warnings) : null
		);
	},

	updateActionState: function() {
		var status = this.ntpStatus || {};
		var dns = this.dnsStatus || {};
		var ntpBusy = this.ntpBusy || status.applying || status.syncing;
		var dnsBusy = this.dnsBusy || dns.applying;
		[ this.ntpSaveBtn, this.ntpResetBtn, this.ntpApplyBtn, this.ntpSyncBtn ].forEach(function(btn) {
			if (!btn) return;
			btn.disabled = ntpBusy;
		});
		[ this.dnsSaveBtn, this.dnsResetBtn, this.dnsApplyBtn ].forEach(function(btn) {
			if (!btn) return;
			btn.disabled = dnsBusy;
		});
	},

	handleSaveApply: null,
	handleSave: null,
	handleReset: null
});
