'use strict';
'require view';
'require fs';
'require ui';

return view.extend({
	load: function() {
		return fs.exec('/usr/sbin/mt5700m-at', [ 'system' ]).catch(function(err) {
			return { stdout: '', stderr: err.message || String(err) };
		});
	},

	styleNode: function() {
		return E('style', {}, [
			'.mt5700m-row{display:grid;grid-template-columns:130px 1fr;gap:10px;align-items:center;margin:10px 0;max-width:840px}',
			'.mt5700m-row input{width:100%}',
			'.mt5700m-actions{display:flex;flex-wrap:wrap;gap:8px;margin:12px 0 18px}',
			'.mt5700m-output{white-space:pre-wrap;word-break:break-word;background:#16191d;color:#d7dde5;border-radius:6px;padding:12px;min-height:360px;overflow:auto;font-family:monospace;font-size:13px}'
		].join(''));
	},

	run: function(output, args, confirmText) {
		var execute = function() {
			output.textContent = _('Running...');
			return fs.exec('/usr/sbin/mt5700m-at', args).then(function(res) {
				output.textContent = res.stdout || _('Command completed.');
				if (res.stderr)
					ui.addNotification(null, E('pre', {}, res.stderr), 'warning');
			}, function(err) {
				output.textContent = err.message || String(err);
			});
		};

		if (!confirmText)
			return execute();

		return ui.showModal(_('Confirm Action'), [
			E('p', {}, confirmText),
			E('div', { 'class': 'right' }, [
				E('button', { 'class': 'btn', 'click': ui.hideModal }, _('Cancel')),
				' ',
				E('button', {
					'class': 'btn cbi-button-negative',
					'click': function() {
						ui.hideModal();
						execute();
					}
				}, _('Continue'))
			])
		]);
	},

	render: function(res) {
		var output = E('pre', { 'class': 'mt5700m-output' }, res.stdout || _('No response.'));
		var fotaUrl = E('input', { 'class': 'cbi-input-text', 'placeholder': 'http://server/path/' });
		var self = this;

		return E('div', {}, [
			this.styleNode(),
			E('h2', {}, _('System and FOTA')),
			E('div', { 'class': 'cbi-section-descr' }, _('View module system information and perform guarded FOTA operations. Firmware upgrade actions are high risk and require confirmation.')),
			E('div', { 'class': 'mt5700m-actions' }, [
				E('button', { 'class': 'btn cbi-button', 'click': function() { self.run(output, [ 'system' ]); } }, _('Refresh')),
				E('button', { 'class': 'btn cbi-button', 'click': function() { self.run(output, [ 'fota-state' ]); } }, _('FOTA State')),
				E('button', { 'class': 'btn cbi-button', 'click': function() { self.run(output, [ 'fota-progress' ]); } }, _('FOTA Progress')),
				E('button', { 'class': 'btn cbi-button-negative', 'click': function() { self.run(output, [ 'restart' ], _('This will restart the MT5700M module and temporarily interrupt 5G connectivity.')); } }, _('Restart Module'))
			]),
			E('h3', {}, _('FOTA Upgrade')),
			E('div', { 'class': 'mt5700m-row' }, [ E('label', {}, _('FOTA Server URL')), fotaUrl ]),
			E('div', { 'class': 'mt5700m-actions' }, [
				E('button', { 'class': 'btn cbi-button', 'click': function() { self.run(output, [ 'fota-init' ], _('Initialize FOTA mode?')); } }, _('Initialize FOTA')),
				E('button', { 'class': 'btn cbi-button-action', 'click': function() { self.run(output, [ 'fota-download', fotaUrl.value ], _('Start FOTA download from the configured server URL?')); } }, _('Start Download')),
				E('button', { 'class': 'btn cbi-button', 'click': function() { self.run(output, [ 'fota-resume' ], _('Resume suspended FOTA download?')); } }, _('Resume Download')),
				E('button', { 'class': 'btn cbi-button-negative', 'click': function() { self.run(output, [ 'fota-upgrade' ], _('Start firmware upgrade now? Do not power off the device during upgrade.')); } }, _('Start Upgrade'))
			]),
			res.stderr ? E('div', { 'class': 'alert-message warning' }, res.stderr) : null,
			output
		]);
	}
});
