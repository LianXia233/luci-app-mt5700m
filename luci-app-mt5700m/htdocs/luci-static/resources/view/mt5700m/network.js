'use strict';
'require view';
'require fs';
'require ui';

return view.extend({
	load: function() {
		return fs.exec('/usr/sbin/mt5700m-at', [ 'network' ]).catch(function(err) {
			return { stdout: '', stderr: err.message || String(err) };
		});
	},

	styleNode: function() {
		return E('style', {}, [
			'.mt5700m-actions{display:flex;flex-wrap:wrap;gap:8px;margin:12px 0 18px}',
			'.mt5700m-output{white-space:pre-wrap;word-break:break-word;background:#16191d;color:#d7dde5;border-radius:6px;padding:12px;min-height:420px;overflow:auto;font-family:monospace;font-size:13px}'
		].join(''));
	},

	runScan: function(output) {
		output.textContent = _('Scanning, please wait...');

		return fs.exec('/usr/sbin/mt5700m-at', [ 'cellscan' ]).then(function(res) {
			output.textContent = res.stdout || _('No response.');
			if (res.stderr)
				ui.addNotification(null, E('pre', {}, res.stderr), 'warning');
		}, function(err) {
			output.textContent = err.message || String(err);
		});
	},

	render: function(res) {
		var output = E('pre', { 'class': 'mt5700m-output' }, res.stdout || _('No response.'));
		var self = this;

		return E('div', {}, [
			this.styleNode(),
			E('h2', {}, _('Network and Cell Info')),
			E('div', { 'class': 'cbi-section-descr' }, _('View serving cell, signal, frequency and registration information from the MT5700M module.')),
			E('div', { 'class': 'mt5700m-actions' }, [
				E('button', {
					'class': 'btn cbi-button',
					'click': function() {
						return fs.exec('/usr/sbin/mt5700m-at', [ 'network' ]).then(function(scan) {
							output.textContent = scan.stdout || _('No response.');
						});
					}
				}, _('Refresh')),
				E('button', {
					'class': 'btn cbi-button-action',
					'click': function() {
						return ui.showModal(_('Confirm Action'), [
							E('p', {}, _('Cell scan may take some time and can briefly increase modem load.')),
							E('div', { 'class': 'right' }, [
								E('button', { 'class': 'btn', 'click': ui.hideModal }, _('Cancel')),
								' ',
								E('button', {
									'class': 'btn cbi-button-apply',
									'click': function() {
										ui.hideModal();
										self.runScan(output);
									}
								}, _('Continue'))
							])
						]);
					}
				}, _('Cell Scan'))
			]),
			res.stderr ? E('div', { 'class': 'alert-message warning' }, res.stderr) : null,
			output
		]);
	}
});
