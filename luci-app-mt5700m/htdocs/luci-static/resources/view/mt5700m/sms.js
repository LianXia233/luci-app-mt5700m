'use strict';
'require view';
'require ui';
'require mt5700m.api as api';
'require mt5700m.parser as parser';
'require mt5700m.components as c';

/*
 * MT5700M LuCI — 短信（sms）
 * ---------------------------------------------
 * 数据：fs.exec sms-list / sms-info（+CMGL PDU 解码 / +CPMS / ^IMSSWITCH）
 * 会话视图 + 聊天 + 本地发送历史（localStorage，旧键 sms_sent_messages_cache 兼容）
 * 无内联样式；PDU 解码在 parser.js。
 */

var STORAGE_KEY = 'mt5700m.sms.sent';
var LEGACY_KEY = 'sms_sent_messages_cache';

return view.extend({
	load: function() {
		return Promise.all([
			api.atSmsList().catch(function(err) { return { stdout: '', stderr: err.message || String(err) }; }),
			api.atSmsInfo().catch(function(err) { return { stdout: '', stderr: err.message || String(err) }; })
		]);
	},

	sentHistory: function() {
		try {
			var raw = window.localStorage.getItem(STORAGE_KEY) || window.localStorage.getItem(LEGACY_KEY) || '[]';
			var parsed = JSON.parse(raw);
			if (Array.isArray(parsed))
				return parsed.filter(function(msg) { return msg && msg.number; });
			if (parsed && Array.isArray(parsed.messages))
				return parsed.messages.filter(function(msg) { return msg && msg.number; });
		} catch (e) { /* 损坏的历史按空处理 */ }
		return [];
	},

	saveSent: function(messages) {
		try {
			window.localStorage.setItem(STORAGE_KEY, JSON.stringify(messages));
			window.localStorage.removeItem(LEGACY_KEY);
		} catch (e) { /* 存储满或禁用时静默 */ }
	},

	clearSent: function() {
		var self = this;
		return ui.showModal(_('Clear sent history'), [
			E('p', {}, _('This removes the local sent-history entry.')),
			E('div', { 'class': 'right' }, [
				E('button', { 'class': 'btn', 'click': ui.hideModal }, _('Cancel')),
				' ',
				E('button', { 'class': 'btn cbi-button-negative', 'click': function() {
					ui.hideModal();
					self.saveSent([]);
					window.location.reload();
				} }, _('Clear all'))
			])
		]);
	},

	exportSent: function() {
		var history = this.sentHistory();
		if (!history.length)
			return ui.addNotification(null, E('p', {}, _('There is no sent history to export.')), 'warning');
		var blob = new Blob([ JSON.stringify({ format: 'mt5700m-sent-history', version: 1, messages: history }, null, 2) ], { type: 'application/json' });
		var url = URL.createObjectURL(blob);
		var a = E('a', { 'href': url, 'download': 'mt5700m-sent-history.json' });
		document.body.appendChild(a);
		a.click();
		a.remove();
		window.setTimeout(function() { URL.revokeObjectURL(url); }, 4000);
	},

	importSent: function() {
		var self = this;
		var file = E('input', { 'type': 'file', 'accept': '.json,application/json' });
		file.addEventListener('change', function() {
			var selected = file.files && file.files[0];
			if (!selected) return;
			var reader = new FileReader();
			reader.onload = function() {
				try {
					var data = JSON.parse(String(reader.result || ''));
					var messages = Array.isArray(data) ? data : (data && Array.isArray(data.messages) ? data.messages : null);
					if (!messages)
						throw new Error('bad format');
					var valid = messages.every(function(m) { return m && typeof m.number === 'string' && /^\+?[0-9]{5,20}$/.test(m.number) && typeof m.text === 'string'; });
					if (!valid)
						throw new Error('bad fields');
					self.saveSent(messages);
					window.location.reload();
				} catch (e) {
					ui.addNotification(null, E('p', {}, _('The selected file is not a valid MT5700M sent-history backup.')), 'danger');
				}
			};
			reader.onerror = function() {
				ui.addNotification(null, E('p', {}, _('The selected history file could not be read.')), 'danger');
			};
			reader.readAsText(selected);
		});
		file.click();
	},

	settingsModal: function(parseInfo) {
		var smsc = E('input', { 'class': 'cbi-input-text', 'placeholder': '+8613800100500', 'value': parseInfo.smsc || '' });
		var storage = c.select([['ME','ME'],['SM','SM']], parseInfo.storage[0] || 'ME');
		var ims = c.select([['1',_('Enabled')],['0',_('Disabled')]], parseInfo.ims || '1');
		return ui.showModal(_('Message settings'), [
			E('p', {}, _('Changing SMS service cycles airplane mode and also changes the IMS PDP context. Leave it enabled unless the carrier does not support IMS messaging.')),
			c.formRow(_('Message center'), smsc),
			c.formRow(_('Storage location'), storage),
			c.formRow(_('SMS service'), ims),
			E('div', { 'class': 'right' }, [
				E('button', { 'class': 'btn', 'click': ui.hideModal }, _('Cancel')),
				' ',
				E('button', { 'class': 'btn cbi-button-apply', 'click': function() {
					if (!/^\+?[0-9]{5,20}$/.test(smsc.value || ''))
						return ui.addNotification(null, E('p', {}, _('Enter a valid message center number.')), 'warning');
					ui.hideModal();
					Promise.all([
						api.at([ 'sms-set', 'smsc', smsc.value.trim() ]),
						api.at([ 'sms-set', 'storage', storage.value ]),
						api.at([ 'sms-ims', ims.value ])
					]).then(function() {
						ui.addNotification(null, E('p', {}, _('Message settings saved.')));
						window.setTimeout(function() { window.location.reload(); }, 1500);
					}, function(err) {
						ui.addNotification(null, E('p', {}, err.message || _('Message operation failed.')), 'danger');
					});
				} }, _('Save settings'))
			])
		]);
	},

	render: function(results) {
		var listResult = results[0] || {}, infoResult = results[1] || {};
		var messages = parser.parseMessages(listResult.stdout || '');
		var info = parser.parseInfo(infoResult.stdout || '');
		var history = this.sentHistory();
		var groups = parser.groupMessages(messages);
		var self = this;

		var slots = '';
		if (info.storage.length === 3)
			slots = _('%s of %s message slots used').format(info.storage[1] || '0', info.storage[2] || '0');
		else
			slots = _('Storage status unavailable');

		/* ---------- 会话侧栏 ---------- */

		var navItems = groups.map(function(group, gi) {
			var last = group.messages[group.messages.length - 1];
			return E('button', {
				'type': 'button',
				'class': 'mt-sms-nav-item' + (gi === 0 ? ' selected' : ''),
				'click': function() {
					navItems.forEach(function(item) { item.classList.remove('selected'); });
					this.classList.add('selected');
					chatBody.innerHTML = '';
					group.messages.forEach(function(msg) { chatBody.appendChild(chatItem(msg)); });
					navEmpty.style.display = 'none';
					chatShell.style.display = '';
				}
			}, [
				E('span', { 'class': 'mt-sms-nav-name' }, group.number),
				E('span', { 'class': 'mt-sms-nav-meta' }, last.date + ' · ' + group.messages.length)
			]);
		});
		var navEmpty = E('div', { 'class': 'mt-scan-note' }, _('No messages yet.'));
		E('div', { 'class': 'mt-sms-nav' }, [ E('div', { 'class': 'mt-sms-nav-title' }, _('Conversations')), navItems.length ? navItems : navEmpty ]);

		/* ---------- 聊天区 ---------- */

		function chatItem(msg) {
			var isOutgoing = msg.direction === 'out';
			return E('div', { 'class': 'mt-sms-chat-item' + (isOutgoing ? ' outgoing' : '') }, [
				E('div', { 'class': 'mt-sms-chat-message' }, msg.text),
				E('div', { 'class': 'mt-sms-chat-meta' }, (isOutgoing ? _('Sent') : _('Received')) + ' · ' + msg.date)
			]);
		}
		var chatBody = E('div', { 'class': 'mt-sms-chat-list' }, []);
		var chatShell = E('div', { 'class': 'mt-sms-chat' }, [ chatBody ]);
		var composeInput = E('input', { 'class': 'mt-sms-compose-input', 'type': 'tel', 'placeholder': _('Phone number…') });
		var composeText = E('input', { 'class': 'mt-sms-compose-input', 'type': 'text', 'placeholder': _('Write a message…') });
		var sendBtn = E('button', { 'type': 'button', 'class': 'mt-sms-compose-action mt-sms-toolbar-action' }, _('Send'));
		var compose = E('div', { 'class': 'mt-sms-compose' }, [
			composeInput,
			E('div', { 'class': 'mt-sms-compose-actions' }, [ composeText, sendBtn ])
		]);

		function sendMessage() {
			var number = composeInput.value.trim(), text = composeText.value.trim();
			if (!/^\+?[0-9]{5,20}$/.test(number) || !text)
				return ui.addNotification(null, E('p', {}, _('Enter a valid phone number and message.')), 'warning');
			ui.showModal(_('Send message?'), [
				E('p', {}, _('Send this message to %s?').format(number)),
				E('div', { 'class': 'right' }, [
					E('button', { 'class': 'btn', 'click': ui.hideModal }, _('Cancel')),
					' ',
					E('button', { 'class': 'btn cbi-button-apply', 'click': function() {
						ui.hideModal();
						api.at([ 'sms-send', number, text ]).then(function() {
							ui.addNotification(null, E('p', {}, _('Message sent.')));
							composeText.value = '';
							history.unshift({ number: number, text: text, date: 'now', direction: 'out', order: Date.now() });
							self.saveSent(history);
							window.setTimeout(function() { window.location.reload(); }, 1200);
						}, function(err) {
							ui.addNotification(null, E('p', {}, err.message || _('Message operation failed.')), 'danger');
						});
					} }, _('Send'))
				])
			]);
		}
		sendBtn.addEventListener('click', sendMessage);
		composeText.addEventListener('keydown', function(ev) { if (ev.key === 'Enter') sendMessage(); });

		/* ---------- 工具条 ---------- */

		function deleteMessage(number, index) {
			ui.showModal(_('Delete message?'), [
				E('p', {}, _('This removes the selected message from the module.')),
				E('div', { 'class': 'right' }, [
					E('button', { 'class': 'btn', 'click': ui.hideModal }, _('Cancel')),
					' ',
					E('button', { 'class': 'btn cbi-button-negative', 'click': function() {
						ui.hideModal();
						api.at([ 'sms-delete', index ]).then(function() { window.location.reload(); }, function(err) {
							ui.addNotification(null, E('p', {}, err.message || _('Message operation failed.')), 'danger');
						});
					} }, _('Delete'))
				])
			]);
		}

		var toolbar = E('div', { 'class': 'mt-sms-toolbar' }, [
			c.btnLink(_('Settings'), '#', { 'cls': 'mt-sms-toolbar-action', 'click': function() { self.settingsModal(info); } }),
			c.btnLink(_('Export sent history'), '#', { 'cls': 'mt-sms-toolbar-action', 'click': function() { self.exportSent(); } }),
			c.btnLink(_('Import sent history'), '#', { 'cls': 'mt-sms-toolbar-action', 'click': function() { self.importSent(); } }),
			c.btnLink(_('Clear received messages'), '#', { 'cls': 'mt-sms-toolbar-action', 'click': function() {
				ui.showModal(_('Clear all messages?'), [
					E('p', {}, _('Every received message stored on the module will be permanently deleted.')),
					E('div', { 'class': 'right' }, [
						E('button', { 'class': 'btn', 'click': ui.hideModal }, _('Cancel')),
						' ',
						E('button', { 'class': 'btn cbi-button-negative', 'click': function() {
							ui.hideModal();
							api.at([ 'sms-clear' ]).then(function() { window.location.reload(); }, function(err) {
								ui.addNotification(null, E('p', {}, err.message || _('Message operation failed.')), 'danger');
							});
						} }, _('Clear all'))
					])
				]);
			} }),
			c.btnLink(_('Clear sent history'), '#', { 'cls': 'mt-sms-toolbar-action', 'click': function() { self.clearSent(); } }),
			c.btnLink(_('Refresh'), '#', { 'cls': 'mt-sms-toolbar-action', 'click': function() { window.location.reload(); } })
		]);

		return E('div', { 'class': 'mt-page' }, [
			c.cssLink(),
			listResult.stderr ? E('div', { 'class': 'alert-message warning' }, listResult.stderr) : null,
			c.hero(_('MESSAGING'), _('Messages'), _('Conversations using the SIM installed in the MT5700M.'), [
				E('span', { 'class': 'mt-badge mt-badge--primary' }, slots)
			], 'teal'),
			E('div', { 'class': 'mt-sms-shell' }, [
				E('div', { 'class': 'mt-sms-sidebar' }, [
					toolbar,
					navItems.length ? E('div', { 'class': 'mt-sms-nav' }, navItems) : null,
					!navItems.length ? navEmpty : null
				]),
				E('div', { 'class': 'mt-sms-chat-shell' }, [
					chatShell,
					compose
				])
			])
		]);
	}
});
