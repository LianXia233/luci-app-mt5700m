'use strict';
'require view';
'require fs';
'require ui';

function splitCsv(value) {
	value = (value || '').replace(/\s+/g, '').replace(/^,+|,+$/g, '').replace(/,+/g, ',');
	return value ? value.split(',') : [];
}

function buildCommand(rat, type, bands, arfcns, scs, pcis) {
	var bandList = splitCsv(bands);
	var arfcnList = splitCsv(arfcns);
	var scsList = splitCsv(scs);
	var pciList = splitCsv(pcis);
	var prefix = rat === 'nr' ? 'AT^NRFREQLOCK' : 'AT^LTEFREQLOCK';

	if (type === '0')
		return prefix + '=0';

	if (type === '3') {
		if (!bandList.length)
			return prefix + '=0';
		return '%s=3,0,%d,"%s"'.format(prefix, bandList.length, bandList.join(','));
	}

	if (rat === 'lte') {
		if (!bandList.length || bandList.length !== arfcnList.length)
			return prefix + '=0';
		if (type === '1')
			return '%s=1,0,%d,"%s","%s"'.format(prefix, bandList.length, bandList.join(','), arfcnList.join(','));
		if (type === '2' && bandList.length === pciList.length)
			return '%s=2,0,%d,"%s","%s","%s"'.format(prefix, bandList.length, bandList.join(','), arfcnList.join(','), pciList.join(','));
		return prefix + '=0';
	}

	if (!bandList.length || bandList.length !== arfcnList.length || bandList.length !== scsList.length)
		return prefix + '=0';
	if (type === '1')
		return '%s=1,0,%d,"%s","%s","%s"'.format(prefix, bandList.length, bandList.join(','), arfcnList.join(','), scsList.join(','));
	if (type === '2' && bandList.length === pciList.length)
		return '%s=2,0,%d,"%s","%s","%s","%s"'.format(prefix, bandList.length, bandList.join(','), arfcnList.join(','), scsList.join(','), pciList.join(','));
	return prefix + '=0';
}

return view.extend({
	styleNode: function() {
		return E('style', {}, [
			'.mt5700m-lock-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(320px,1fr));gap:16px;margin-top:12px}',
			'.mt5700m-panel{border:1px solid var(--border-color-medium,#d8d8d8);border-radius:8px;padding:14px;background:var(--background-color-high,#fff)}',
			'.mt5700m-row{display:grid;grid-template-columns:110px 1fr;gap:10px;align-items:center;margin:10px 0}',
			'.mt5700m-row input,.mt5700m-row select{width:100%}',
			'.mt5700m-preview{white-space:pre-wrap;word-break:break-word;background:#16191d;color:#d7dde5;border-radius:6px;padding:10px;font-family:monospace;font-size:13px}',
			'.mt5700m-actions{display:flex;flex-wrap:wrap;gap:8px;margin-top:12px}'
		].join(''));
	},

	lockPanel: function(title, rat) {
		var type = E('select', { 'class': 'cbi-input-select' }, [
			E('option', { 'value': '0' }, _('Unlock')),
			E('option', { 'value': '3', 'selected': 'selected' }, _('Band Lock')),
			E('option', { 'value': '1' }, _('ARFCN Lock')),
			E('option', { 'value': '2' }, _('Cell Lock'))
		]);
		var bands = E('input', { 'class': 'cbi-input-text', 'placeholder': rat === 'nr' ? '78,41' : '3,8' });
		var arfcns = E('input', { 'class': 'cbi-input-text', 'placeholder': rat === 'nr' ? '630000,520000' : '1850,3450' });
		var scs = E('input', { 'class': 'cbi-input-text', 'placeholder': '1,1' });
		var pcis = E('input', { 'class': 'cbi-input-text', 'placeholder': '100,200' });
		var preview = E('pre', { 'class': 'mt5700m-preview' }, '');
		var self = this;

		var update = function() {
			preview.textContent = buildCommand(rat, type.value, bands.value, arfcns.value, scs.value, pcis.value);
		};

		[ type, bands, arfcns, scs, pcis ].forEach(function(node) {
			node.addEventListener('input', update);
			node.addEventListener('change', update);
		});

		update();

		return E('div', { 'class': 'mt5700m-panel' }, [
			E('h3', {}, title),
			E('div', { 'class': 'mt5700m-row' }, [ E('label', {}, _('Lock Type')), type ]),
			E('div', { 'class': 'mt5700m-row' }, [ E('label', {}, _('Bands')), bands ]),
			E('div', { 'class': 'mt5700m-row' }, [ E('label', {}, _('ARFCNs')), arfcns ]),
			rat === 'nr' ? E('div', { 'class': 'mt5700m-row' }, [ E('label', {}, _('SCS Types')), scs ]) : null,
			E('div', { 'class': 'mt5700m-row' }, [ E('label', {}, _('PCI')), pcis ]),
			E('label', {}, _('Command Preview')),
			preview,
			E('div', { 'class': 'mt5700m-actions' }, [
				E('button', {
					'class': 'btn cbi-button-apply',
					'click': function() {
						var args = rat === 'nr'
							? [ 'lock', rat, type.value, bands.value, arfcns.value, scs.value, pcis.value ]
							: [ 'lock', rat, type.value, bands.value, arfcns.value, pcis.value ];

						return ui.showModal(_('Confirm Action'), [
							E('p', {}, _('This command changes modem frequency lock state and may interrupt current connectivity.')),
							E('pre', { 'class': 'mt5700m-preview' }, preview.textContent),
							E('div', { 'class': 'right' }, [
								E('button', { 'class': 'btn', 'click': ui.hideModal }, _('Cancel')),
								' ',
								E('button', {
									'class': 'btn cbi-button-negative',
									'click': function() {
										ui.hideModal();
										fs.exec('/usr/sbin/mt5700m-at', args).then(function(res) {
											ui.addNotification(null, E('pre', {}, res.stdout || _('Command completed.')));
										}, function(err) {
											ui.addNotification(null, E('p', {}, err.message), 'danger');
										});
									}
								}, _('Continue'))
							])
						]);
					}
				}, _('Apply'))
			])
		]);
	},

	render: function() {
		return E('div', {}, [
			this.styleNode(),
			E('h2', {}, _('Frequency Lock')),
			E('div', { 'class': 'cbi-section-descr' }, _('Configure LTE and NR band, ARFCN or cell locks using native LuCI controls. Keep values empty to unlock.')),
			E('div', { 'class': 'mt5700m-lock-grid' }, [
				this.lockPanel(_('LTE Lock'), 'lte'),
				this.lockPanel(_('NR Lock'), 'nr')
			])
		]);
	}
});
