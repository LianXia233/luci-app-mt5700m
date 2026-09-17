'use strict';
'require view';
'require ui';
'require mt5700m.api as api';
'require mt5700m.components as c';

/*
 * MT5700M LuCI — AT 终端（terminal）
 * ---------------------------------------------
 * 数据：fs.exec command（直通 AT）；本地保存命令（localStorage 'mt5700m.at.saved' 保持兼容）
 * 无内联样式；输出面板 / 快捷按钮 / 已存命令全部用设计系统类。
 */

return view.extend({
	appendOutput: function(node, text, prefix) {
		var timestamp = new Date().toLocaleTimeString();
		node.textContent += '[%s] %s %s\n'.format(timestamp, prefix, text || '');
		node.scrollTop = node.scrollHeight;
	},

	sendCommand: function(output, input) {
		var cmd = (input.value || '').trim();
		var self = this;

		if (!cmd) {
			ui.addNotification(null, E('p', {}, _('Command is empty.')), 'warning');
			return;
		}

		this.appendOutput(output, cmd, '>>>');
		input.value = '';

		return api.atCommand(cmd).then(function(res) {
			self.appendOutput(output, res.stdout || _('No response.'), '<<<');
			if (res.stderr)
				self.appendOutput(output, res.stderr, 'ERR');
		}, function(err) {
			self.appendOutput(output, err.message || String(err), 'ERR');
		});
	},

	quickButton: function(label, cmd, input, output) {
		var self = this;
		return E('button', {
			'class': 'btn cbi-button',
			'click': function() {
				input.value = cmd;
				self.sendCommand(output, input);
			}
		}, label);
	},

	loadSaved: function() {
		try { return JSON.parse(window.localStorage.getItem('mt5700m.at.saved') || '[]'); }
		catch (e) { return []; }
	},

	saveCommand: function(input, container, output) {
		var command = (input.value || '').trim(), self = this;
		if (!command)
			return ui.addNotification(null, E('p', {}, _('Command is empty.')), 'warning');
		var label = window.prompt(_('Name this command'), command);
		if (!label)
			return;
		var saved = this.loadSaved().filter(function(item) { return item.command !== command; });
		saved.push({ label: label.substring(0, 40), command: command });
		window.localStorage.setItem('mt5700m.at.saved', JSON.stringify(saved.slice(-20)));
		this.renderSaved(container, input, output);
	},

	renderSaved: function(container, input, output) {
		var self = this, saved = this.loadSaved();
		container.innerHTML = '';
		saved.forEach(function(item) {
			container.appendChild(E('span', { 'class': 'mt-terminal-saved-item' }, [
				self.quickButton(item.label, item.command, input, output),
				E('button', { 'type': 'button', 'class': 'btn', 'title': _('Remove saved command'), 'click': function() {
					window.localStorage.setItem('mt5700m.at.saved', JSON.stringify(self.loadSaved().filter(function(entry) { return entry.command !== item.command; })));
					self.renderSaved(container, input, output);
				} }, '×')
			]));
		});
		container.style.display = saved.length ? '' : 'none';
	},

	render: function() {
		var input = E('input', {
			'type': 'text',
			'class': 'cbi-input-text mt-terminal-input-field',
			'placeholder': _('Enter AT command, for example AT^HCSQ?')
		});
		var output = E('pre', { 'class': 'mt-terminal-output' }, _('Ready.'));
		var saved = E('div', { 'class': 'mt-terminal-quick' });
		var self = this;

		input.addEventListener('keydown', function(ev) {
			if (ev.key === 'Enter')
				self.sendCommand(output, input);
		});
		window.setTimeout(function() { self.renderSaved(saved, input, output); }, 0);

		return E('div', { 'class': 'mt-page' }, [
			c.cssLink(),
			c.hero(null, _('MT5700M AT command console'), _('Diagnostic console for advanced users. Commands are sent directly to the MT5700M and are not automatically validated.'), null, 'slate'),
			E('section', { 'class': 'mt-card' }, [
				E('div', { 'class': 'alert-message warning' }, _('Use query commands whenever possible. Configuration and reset commands may interrupt mobile connectivity.')),
				E('div', { 'class': 'mt-terminal-input' }, [
					E('div', { 'class': 'mt-terminal-input-field', 'style': 'flex:1' }, input),
					E('div', { 'class': 'mt-terminal-input-actions' }, [
						E('button', { 'class': 'btn cbi-button-apply', 'click': function() { self.sendCommand(output, input); } }, _('Send')),
						E('button', { 'class': 'btn', 'click': function() { output.textContent = ''; } }, _('Clear')),
						E('button', { 'class': 'btn', 'click': function() { self.saveCommand(input, saved, output); } }, _('Save command'))
					])
				]),
				E('div', { 'class': 'mt-terminal-quick' }, [
					this.quickButton('AT', 'AT', input, output),
					this.quickButton('ATI', 'ATI', input, output),
					this.quickButton('SIM', 'AT+CPIN?', input, output),
					this.quickButton(_('Signal'), 'AT^HCSQ?', input, output),
					this.quickButton(_('Temperature'), 'AT^CHIPTEMP?', input, output),
					this.quickButton(_('Operator'), 'AT+COPS?', input, output),
					this.quickButton(_('Cell Info'), 'AT^MONSC', input, output),
					this.quickButton(_('NR Lock'), 'AT^NRFREQLOCK?', input, output),
					this.quickButton(_('LTE Lock'), 'AT^LTEFREQLOCK?', input, output)
				]),
				saved,
				output
			])
		]);
	}
});
