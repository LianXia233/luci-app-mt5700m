'use strict';
'require view';
'require ui';
'require mt5700m.api as api';
'require mt5700m.parser as parser';
'require mt5700m.components as c';

/*
 * MT5700M LuCI — 短信（sms）
 * ---------------------------------------------
 * 数据：统一路由 sms.list / sms.status（PDU 解码、长短信合并、+CPMS/+CSCA/
 * ^IMSSWITCH 的字段都在 modules/sms 里做过一次，与 WebUI 的短信页同一组路由）。
 * 会话视图 + 聊天 + 本地发送历史（localStorage，旧键 sms_sent_messages_cache 兼容）
 * 无内联样式；页面这一侧不再有第二份 PDU/GSM7 解码。
 */

var STORAGE_KEY = 'mt5700m.sms.sent';
var LEGACY_KEY = 'sms_sent_messages_cache';

/*
 * `sms.list` 的一条消息 → 页面既有的视图形状。
 *
 * 后端给的是领域字段（index/content/number/time/type，长短信已合并），这里换的
 * 只是名字：content→text、type→direction；`time`（PDU 时间戳 `YY/MM/DD,HH:MM:SS`）
 * 排版成列表一直用的 `20YY-MM-DD HH:MM`（旧版从解码出的六个字段拼出同一个串）。
 */
function smsMessages(payload) {
	return ((payload && payload.messages) || []).map(function(msg) {
		return {
			index: String(msg.index),
			number: msg.number,
			date: smsDate(msg.time),
			text: msg.content,
			direction: msg.type === 'sent' ? 'out' : 'in',
			order: Number(msg.index) || 0
		};
	});
}

function smsDate(time) {
	var m = /^(\d{2})\/(\d{2})\/(\d{2}),(\d{2}):(\d{2})/.exec(String(time || ''));
	if (!m)
		return String(time || '');
	return '20' + m[1] + '-' + m[2] + '-' + m[3] + ' ' + m[4] + ':' + m[5];
}

return view.extend({
	load: function() {
		// 请求发起即返回，不阻塞首屏；render() 等 pending 填充。
		//
		// 两条读都走统一路由（sms.list / sms.status —— 与 WebUI 的短信页同一组
		// 路由），旧版切的是 `sms-list`/`sms-info` 两个 CLI 文本帧。
		// 这里用 routeCall 而不是 route：短信列表就是整页内容，读失败必须报出来
		// （旧版报的是 CLI 帧的 stderr），否则用户看到的是「暂无消息」——把读取
		// 失败说成空收件箱是错的；成功时的空列表才是「暂无消息」。
		this.pending = Promise.all([
			api.routeCall('sms.list').then(null, function(err) { return { error: err.message || String(err) }; }),
			api.routeCall('sms.status').then(null, function(err) { return { error: err.message || String(err) }; })
		]);
		return Promise.resolve();
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

	settingsModal: function(status) {
		// 预填来自 sms.status：center = +CSCA、storage.read.name = +CPMS 第一面、
		// imsOn = ^IMSSWITCH 第一字段（读不到时与旧版一样落到 Enabled）。
		var statusStorage = status.storage || {}, currentStorage = statusStorage.read || {};
		var smsc = E('input', { 'class': 'cbi-input-text', 'placeholder': '+8613800100500', 'value': status.center || '' });
		var storage = c.select([['ME','ME'],['SM','SM']], currentStorage.name || 'ME');
		var ims = c.select([['1',_('Enabled')],['0',_('Disabled')]], status.imsOn === false ? '0' : '1');
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
					// 旧版是 `sms-set smsc/storage` + `sms-ims` 三个动词；`sms-set
					// storage` 把三个面都设成同一个名字（cmd_sms_set 的行为），
					// 所以新路由也显式给三面。
					Promise.all([
						api.routeCall('sms.center_set', { number: smsc.value.trim() }),
						api.routeCall('sms.storage_set', { read: storage.value, write: storage.value, receive: storage.value }),
						api.routeCall('sms.ims_set', { enabled: ims.value === '1' })
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

	// 渐进渲染：骨架屏立即显示，数据到达后整体替换（后端慢不挡前端）
	render: function() {
		var self = this;
		var holder = E('div', { 'class': 'mt-view' });
		holder.appendChild(c.skeletonPage(3));
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
		var listResult = results[0] || {}, status = results[1] || {};
		var messages = listResult.error ? [] : smsMessages(listResult);
		var history = this.sentHistory();
		var groups = parser.groupMessages(messages.concat(history));
		var self = this;

		// `sms.status` 的 storage.read 就是旧版 `parseInfo.storage` 正则抓到的
		// 那三个数（+CPMS 的第一行 = 读取面）；读不到整份状态时与旧版一样显示
		// 占位文案。
		var storage = status.storage || {};
		var slots = storage.read
			? _('%s of %s message slots used').format(String(storage.read.used), String(storage.read.total))
			: _('Storage status unavailable');

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
		/* 初始选中第一个会话：立即填充聊天面板，避免空面板直到手动点击（评审 P2） */
		if (groups.length) {
			groups[0].messages.forEach(function(msg) { chatBody.appendChild(chatItem(msg)); });
			navEmpty.style.display = 'none';
			chatShell.style.display = '';
		}
		var composeInput = E('input', { 'class': 'mt-sms-compose-input', 'type': 'tel', 'placeholder': _('Phone number…') });
		var composeText = E('input', { 'class': 'mt-sms-compose-input', 'type': 'text', 'placeholder': _('Write a message…') });
		var sendBtn = E('button', { 'type': 'button', 'class': 'mt-sms-compose-action mt-sms-toolbar-action', 'style': 'display:inline-flex;align-items:center;gap:6px' }, [ c.svgSendIcon(), document.createTextNode(_('Send')) ]);
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
						api.routeCall('sms.send', { number: number, text: text }).then(function() {
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
						api.routeCall('sms.delete', { index: Number(index) }).then(function() { window.location.reload(); }, function(err) {
							ui.addNotification(null, E('p', {}, err.message || _('Message operation failed.')), 'danger');
						});
					} }, _('Delete'))
				])
			]);
		}

		var toolbar = E('div', { 'class': 'mt-sms-toolbar' }, [
			c.btnLink(_('Settings'), '#', { 'cls': 'mt-sms-toolbar-action', 'click': function() { self.settingsModal(status); } }),
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
							api.routeCall('sms.clear_all').then(function() { window.location.reload(); }, function(err) {
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
			listResult.error ? E('div', { 'class': 'alert-message warning' }, listResult.error) : null,
			status.error ? E('div', { 'class': 'alert-message warning' }, _('SMS storage information is temporarily unavailable. ') + status.error) : null,
			c.hero(_('MESSAGING'), _('Messages'), _('Conversations using the SIM installed in the MT5700M.'), [
				E('div', { 'style': 'display:flex;align-items:center;gap:8px' }, [
					c.svgStatusPulse('ok', 14),
					E('span', { 'class': 'mt-badge mt-badge--primary' }, slots)
				])
			], 'teal', c.svgChip(64, 'teal')),
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
