'use strict';
'require view';
'require fs';
'require ui';

return view.extend({
	load: function() {
		return fs.exec('/usr/sbin/mt5700m-at', [ 'sms-list' ]).catch(function(err) {
			return { stdout: '', stderr: err.message || String(err) };
		});
	},

	styleNode: function() {
		return E('style', {}, [
			'.mt5700m-row{display:grid;grid-template-columns:110px 1fr;gap:10px;align-items:center;margin:10px 0;max-width:760px}',
			'.mt5700m-row input,.mt5700m-row textarea{width:100%}',
			'.mt5700m-actions{display:flex;flex-wrap:wrap;gap:8px;margin:12px 0 18px}',
			'.mt5700m-output{white-space:pre-wrap;word-break:break-word;background:#16191d;color:#d7dde5;border-radius:6px;padding:12px;min-height:300px;overflow:auto;font-family:monospace;font-size:13px}'
		].join(''));
	},

	refresh: function(output) {
		return fs.exec('/usr/sbin/mt5700m-at', [ 'sms-list' ]).then(function(res) {
			output.textContent = res.stdout || _('No response.');
		}, function(err) {
			output.textContent = err.message || String(err);
		});
	},

	render: function(res) {
		var number = E('input', { 'class': 'cbi-input-text', 'placeholder': '+8613800000000' });
		var text = E('textarea', { 'class': 'cbi-input-text', 'rows': 4, 'placeholder': _('Message text') });
		var index = E('input', { 'class': 'cbi-input-text', 'placeholder': '1' });
		var output = E('pre', { 'class': 'mt5700m-output' }, res.stdout || _('No response.'));
		var self = this;

		return E('div', {}, [
			this.styleNode(),
			E('h2', {}, _('SMS Center')),
			E('div', { 'class': 'cbi-section-descr' }, _('Read, send and delete SMS messages through the MT5700M AT interface.')),
			E('div', { 'class': 'mt5700m-row' }, [ E('label', {}, _('Phone Number')), number ]),
			E('div', { 'class': 'mt5700m-row' }, [ E('label', {}, _('Message')), text ]),
			E('div', { 'class': 'mt5700m-actions' }, [
				E('button', {
					'class': 'btn cbi-button-apply',
					'click': function() {
						return ui.showModal(_('Confirm Action'), [
							E('p', {}, _('Send this SMS message now?')),
							E('pre', {}, text.value || ''),
							E('div', { 'class': 'right' }, [
								E('button', { 'class': 'btn', 'click': ui.hideModal }, _('Cancel')),
								' ',
								E('button', {
									'class': 'btn cbi-button-apply',
									'click': function() {
										ui.hideModal();
										fs.exec('/usr/sbin/mt5700m-at', [ 'sms-send', number.value, text.value ]).then(function(sendRes) {
											output.textContent = sendRes.stdout || _('Command completed.');
										}, function(err) {
											output.textContent = err.message || String(err);
										});
									}
								}, _('Continue'))
							])
						]);
					}
				}, _('Send SMS')),
				E('button', { 'class': 'btn cbi-button', 'click': function() { self.refresh(output); } }, _('Refresh'))
			]),
			E('div', { 'class': 'mt5700m-row' }, [ E('label', {}, _('SMS Index')), index ]),
			E('div', { 'class': 'mt5700m-actions' }, [
				E('button', {
					'class': 'btn cbi-button-negative',
					'click': function() {
						return ui.showModal(_('Confirm Action'), [
							E('p', {}, _('Delete the selected SMS message?')),
							E('div', { 'class': 'right' }, [
								E('button', { 'class': 'btn', 'click': ui.hideModal }, _('Cancel')),
								' ',
								E('button', {
									'class': 'btn cbi-button-negative',
									'click': function() {
										ui.hideModal();
										fs.exec('/usr/sbin/mt5700m-at', [ 'sms-delete', index.value ]).then(function(delRes) {
											output.textContent = delRes.stdout || _('Command completed.');
										}, function(err) {
											output.textContent = err.message || String(err);
										});
									}
								}, _('Continue'))
							])
						]);
					}
				}, _('Delete SMS'))
			]),
			res.stderr ? E('div', { 'class': 'alert-message warning' }, res.stderr) : null,
			output
		]);
	}
});
