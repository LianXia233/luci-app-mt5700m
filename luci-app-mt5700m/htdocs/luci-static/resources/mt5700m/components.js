'use strict';
'require baseclass';
'require ui';
'require mt5700m.api as api';
'require mt5700m.parser as parser';

/*
 * MT5700M LuCI — components.js
 * ---------------------------------------------
 * 组件库：页面只负责组装，不写内联 <style>、不重复声明数据通道。
 * 全部类名使用 style.css 的 mt- 设计系统（令牌：var(--mt-*) + luci-base 变量）。
 * i18n 键与原版逐字一致。
 */

/* ---------- 工具 ---------- */

// 健壮 DOM 节点检测：兼容旧 L.dom（nodeType===1 但非 instanceof HTMLElement）
function isNode(v) {
	return v && typeof v === 'object' && (v instanceof HTMLElement || v.nodeType === 1);
}

// 设计系统样式表：模块求值时立即注入 <head>，让 style.css 与 load() 的
// ubus/AT 数据请求并行下载。原实现把 <link> 放在 render() 返回的 DOM 里，
// CSS 要等数据全部返回后才开始下载，首屏必然先无样式再整体重排。
// ?v= 须与 Makefile 的 PKG_VERSION 保持一致，升级后立即失效旧样式缓存。
var STYLE_VERSION = '2.5.0';

function injectStyle() {
	if (!document.getElementById('mt5700m-style'))
		document.head.appendChild(E('link', {
			'rel': 'stylesheet',
			'id': 'mt5700m-style',
			'href': L.resource('mt5700m/style.css') + '?v=' + STYLE_VERSION
		}));
}

injectStyle();

// 兼容保留：样式已提前注入，render() 中无需再插入 <link>（E() 跳过 null）
function cssLink() {
	return null;
}

/* ---------- 首屏骨架屏 ---------- */

// 页面先画骨架、不等 ubus/AT 数据；数据到达后由视图整体替换。
// 骨架结构只用 mt- 设计系统类，暗色模式由令牌自动适配。
function skeletonPage(cardCount) {
	var cards = [], i;
	for (i = 0; i < (cardCount || 4); i++)
		cards.push(E('div', { 'class': 'mt-card mt-skeleton-card' }, [
			E('div', { 'class': 'mt-skeleton-bar w30' }),
			E('div', { 'class': 'mt-skeleton-bar w70' }),
			E('div', { 'class': 'mt-skeleton-bar w50' })
		]));
	return E('div', { 'class': 'mt-page mt-skeleton-page' }, [
		E('div', { 'class': 'mt-hero mt-skeleton-hero' }, [
			E('div', { 'class': 'mt-skeleton-bar w25' }),
			E('div', { 'class': 'mt-skeleton-bar mt-skeleton-title w60' })
		]),
		E('div', { 'class': 'mt-grid' }, cards)
	]);
}

/* ---------- 基础标签 ---------- */

function badge(text, cls) {
	return E('span', { 'class': 'mt-badge' + (cls ? ' mt-badge--' + cls : '') }, text);
}

// 信号质量徽标：excellent/good→ok，fair→warn，weak→bad，unknown→slate
function signalBadge(quality) {
	var map = { excellent:'ok', good:'ok', fair:'warn', weak:'bad', unknown:'slate' };
	return badge(quality.label, map[quality.cls] || 'slate');
}

function hero(kicker, title, desc, side, variant) {
	return E('section', { 'class': 'mt-hero' + (variant ? ' mt-hero--' + variant : '') }, [
		E('div', { 'class': 'mt-hero-main' }, [
			kicker ? E('div', { 'class': 'mt-hero-kicker' }, kicker) : null,
			E('h2', { 'class': 'mt-hero-title' }, title),
			desc ? E('p', { 'class': 'mt-hero-desc' }, desc) : null
		]),
		side ? E('div', { 'class': 'mt-hero-side' }, side) : null
	]);
}

function card(title, desc, body) {
	var children = [
		E('div', { 'class': 'mt-card-head' }, [
			E('h3', { 'class': 'mt-card-title' }, title),
			desc ? E('p', { 'class': 'mt-card-desc' }, desc) : null
		])
	];

	// LuCI E() only flattens the top-level children array; a nested array would
	// be stringified ("[object HTMLDivElement],[object HTMLDivElement]...").
	if (Array.isArray(body))
		for (var i = 0; i < body.length; i++)
			children.push(body[i]);
	else if (body != null)
		children.push(body);

	return E('section', { 'class': 'mt-card' }, children);
}

// 数据行（label + 值/节点；值可为 DOM 节点或节点数组）
function row(label, value) {
	var valueNode;
	if (isNode(value)) {
		valueNode = value;
	} else if (Array.isArray(value)) {
		valueNode = E('span', { 'class': 'mt-row-value' }, value.filter(function(v) { return v != null; }));
	} else {
		valueNode = E('strong', {}, (value == null || value === '') ? '--' : String(value));
	}
	return E('div', { 'class': 'mt-row' }, [ E('span', { 'class': 'mt-muted' }, label), valueNode ]);
}

// 表单行（label + 控件）
function formRow(label, input) {
	return E('div', { 'class': 'mt-advanced-row' }, [
		E('span', { 'class': 'mt-advanced-label' }, label),
		E('div', { 'class': 'mt-advanced-value' }, input)
	]);
}

// 状态行（label + 只读值）
function stateRow(label, value) {
	return E('div', { 'class': 'mt-advanced-row' }, [
		E('span', { 'class': 'mt-advanced-label' }, label),
		E('strong', { 'style': 'text-align:right' }, value || '--')
	]);
}

// 事实网格单元格
function fact(label, value, note) {
	return E('div', { 'class': 'mt-facts-cell' }, [
		E('div', { 'class': 'mt-facts-label' }, label),
		E('div', { 'class': 'mt-facts-value' }, value || '--'),
		note ? E('div', { 'class': 'mt-facts-note' }, note) : null
	]);
}

function facts(cells) {
	return E('div', { 'class': 'mt-facts-grid' }, cells);
}

/* ---------- 按钮 ---------- */

function btn(label, handler, opts) {
	opts = opts || {};
	var cls = opts.cls || 'mt-session-action';
	if (opts.primary) cls += ' mt-diag-action--primary';
	if (opts.danger) cls += ' mt-session-action--danger';
	return E('button', {
		'type': 'button',
		'class': cls,
		'click': handler,
		'disabled': opts.disabled ? 'disabled' : null
	}, label);
}

function btnLink(label, href, opts) {
	opts = opts || {};
	var cls = opts.cls || 'mt-session-action';
	if (opts.primary) cls += ' mt-diag-action--primary';
	if (opts.danger) cls += ' mt-session-action--danger';
	var attrs = { 'class': cls, 'href': href };
	if (opts.click) attrs.click = opts.click;
	return E('a', attrs, label);
}

function actionBar(children) {
	return E('div', { 'class': 'mt-advanced-actions' }, children);
}

/* ---------- 表单控件 ---------- */

function select(options, value) {
	var node = E('select', { 'class': 'cbi-input-select' }, options.map(function(item) {
		return E('option', { 'value': item[0] }, item[1]);
	}));
	if (value != null)
		node.value = String(value);
	return node;
}

/* ---------- 模态确认（写入类命令） ---------- */

function confirmRun(title, message, args, restartRequired) {
	return ui.showModal(title, [
		E('p', {}, message),
		restartRequired ? E('div', { 'class': 'alert-message warning' }, _('A module restart or airplane-mode cycle is required before this change takes effect.')) : null,
		E('div', { 'class': 'right' }, [
			E('button', { 'type': 'button', 'class': 'btn', 'click': ui.hideModal }, _('Cancel')), ' ',
			E('button', {
				'type': 'button',
				'class': 'btn cbi-button-negative',
				'click': function() {
					ui.hideModal();
					api.at(args).then(function() {
						ui.addNotification(null, E('p', {}, _('Settings applied.')));
						window.setTimeout(function() { window.location.reload(); }, 900);
					}, function(err) {
						ui.addNotification(null, E('p', {}, err.message || String(err)), 'danger');
					});
				}
			}, _('Apply'))
		])
	]);
}

// 通用确认执行（system 页），支持 danger 样式与自定义恢复延迟
function runConfirmed(title, message, args, danger, recoveryDelay) {
	return ui.showModal(title, [
		E('p', {}, message),
		E('div', { 'class': 'right' }, [
			E('button', { 'class': 'btn', 'click': ui.hideModal }, _('Cancel')), ' ',
			E('button', { 'class': 'btn ' + (danger ? 'cbi-button-negative' : 'cbi-button-apply'), 'click': function() {
				ui.hideModal();
				api.at(args).then(function() {
					ui.addNotification(null, E('p', {}, recoveryDelay ? _('Restart accepted. The USB interface normally returns in about 22 seconds.') : _('Command accepted by the modem.')));
					window.setTimeout(function() { window.location.reload(); }, recoveryDelay || 1500);
				}, function(err) { ui.addNotification(null, E('p', {}, err.message || String(err)), 'danger'); });
			} }, _('Continue'))
		])
	]);
}

/* ---------- 折叠区 / 原始输出 ---------- */

function details(title, desc, body) {
	return E('details', { 'class': 'mt-details' }, [
		E('summary', { 'class': 'mt-details-summary' }, [
			E('span', { 'class': 'mt-chevron', 'aria-hidden': 'true' }),
			E('span', { 'style': 'min-width:0' }, [
				E('span', { 'class': 'mt-details-title' }, title),
				desc ? E('div', { 'class': 'mt-details-desc' }, desc) : null
			])
		]),
		E('div', { 'class': 'mt-details-body' }, body)
	]);
}

function raw(text) {
	return E('pre', { 'class': 'mt-raw' }, text || _('No response.'));
}

/* ---------- 信号 / 仪表 ---------- */

// 数值 → 质量类（RSRP/RSRQ/SINR 阈值）
function signalColorClass(value, kind) {
	var v = parseFloat(value);
	if (isNaN(v)) return 'unknown';
	if (kind === 'rsrp') { if (v >= -80) return 'excellent'; if (v >= -90) return 'good'; if (v >= -100) return 'fair'; return 'weak'; }
	if (kind === 'rsrq') { if (v >= -10) return 'excellent'; if (v >= -15) return 'good'; if (v >= -20) return 'fair'; return 'weak'; }
	if (v >= 20) return 'excellent'; if (v >= 13) return 'good'; if (v >= 0) return 'fair'; return 'weak';
}

function signalPercent(value, kind) {
	var v = parseFloat(value);
	if (isNaN(v)) return 0;
	if (kind === 'rsrp') return Math.max(0, Math.min(100, (v + 140) * 1.67));
	if (kind === 'rsrq') return Math.max(0, Math.min(100, (v + 35) * 2.86));
	return Math.max(0, Math.min(100, (v + 10) * 3.33));
}

// 横向信号条 [label] [track>fill] [value]
function signalBar(value, kind, label) {
	var v = String(value || '--');
	var cls = signalColorClass(value, kind);
	var pct = signalPercent(value, kind);
	return E('div', { 'class': 'mt-signal-bar' }, [
		E('span', { 'class': 'mt-signal-label' }, label),
		E('div', { 'class': 'mt-signal-track', 'role': 'progressbar', 'aria-valuenow': String(Math.round(pct)), 'aria-valuemin': '0', 'aria-valuemax': '100' },
			E('i', { 'class': 'mt-signal-fill ' + cls, 'style': 'width:' + pct.toFixed(1) + '%' })),
		E('span', { 'class': 'mt-signal-value ' + cls }, v + (kind === 'rsrp' ? 'dBm' : kind === 'rsrq' ? 'dB' : 'dB'))
	]);
}

// 14 格信号柱（按百分比点亮；off 格保持默认底色）
function signalBars(percentage, cls) {
	var active = isNaN(percentage) ? 0 : Math.max(1, Math.round(percentage / 100 * 14));
	var color = cls === 'fair' ? 'var(--mt-warn)' : cls === 'weak' ? 'var(--mt-bad)' : cls === 'unknown' ? 'var(--mt-neutral)' : 'var(--mt-ok)';
	var bars = [];
	for (var i = 0; i < 14; i++)
		bars.push(E('span', { 'style': 'height:%dpx;background:%s'.format(8 + i * 3, i < active ? color : '') }));
	return E('div', { 'class': 'mt-signal-bars', 'aria-hidden': 'true' }, bars);
}

// 统一彩色仪表（rsrp/rsrq/sinr/temp），带质量标签与刻度
function gauge(label, kind, rawValue, unit, scaleLow, scaleHigh) {
	var num = parseFloat(rawValue), has = !isNaN(num), pct = 0, cls = 'unknown', ql = '';
	var tags = { excellent:_('Excellent'), good:_('Good'), fair:_('Fair'), weak:_('Weak') };
	if (has) {
		if (kind === 'rsrp') { pct = (num + 120) * 2.5; cls = num >= -80 ? 'excellent' : num >= -90 ? 'good' : num >= -100 ? 'fair' : 'weak'; }
		else if (kind === 'rsrq') { pct = (num + 25) * 4; cls = num >= -10 ? 'excellent' : num >= -15 ? 'good' : num >= -20 ? 'fair' : 'weak'; }
		else if (kind === 'sinr') { pct = (num + 10) * 2.5; cls = num >= 20 ? 'excellent' : num >= 13 ? 'good' : num >= 0 ? 'fair' : 'weak'; }
		else { pct = (num - 20) / 60 * 100; cls = num < 45 ? 'excellent' : num < 55 ? 'good' : num < 65 ? 'fair' : 'weak'; }
		pct = Math.max(4, Math.min(100, pct));
		ql = tags[cls] || '';
	}
	return E('div', { 'class': 'mt-gauge' }, [
		E('div', { 'class': 'mt-gauge-head' }, [
			E('span', { 'class': 'mt-gauge-label' }, label),
			E('span', {}, [
				has ? E('span', { 'class': 'mt-gauge-value' }, String(rawValue)) : badge(_('No data'), 'slate'),
				has ? E('span', { 'class': 'mt-gauge-unit' }, unit) : null
			])
		]),
		E('div', { 'class': 'mt-gauge-track' }, E('i', { 'class': 'mt-gauge-fill ' + cls, 'style': 'width:' + (has ? pct : 0) + '%' })),
		E('div', { 'class': 'mt-gauge-scale', 'style': 'height:auto;display:flex;justify-content:space-between;background:none;color:var(--mt-faint);font-size:9px;margin-top:4px' }, [
			E('span', {}, scaleLow || ''),
			E('span', {}, scaleHigh || '')
		])
	]);
}

// 大数字指标（英雄区 4 列用）
function metric(label, value, unit) {
	return E('div', { 'class': 'mt-facts-cell' }, [
		E('div', { 'class': 'mt-facts-label' }, label),
		E('div', { 'class': 'mt-facts-value' }, value || '--'),
		unit ? E('div', { 'class': 'mt-facts-note' }, unit) : null
	]);
}

/* ---------- MCS 调制摘要 ---------- */

function mcsDetailNode(text) {
	var groups = parser.parseMcsSection(text);
	if (!groups.length)
		return E('span', {}, _('Not available'));
	if (groups.length === 1 && groups[0].carriers.length === 1) {
		var g = groups[0], c = g.carriers[0];
		var c0 = parser.mcsModulation(c.code0, c.table, g.rat);
		var c1 = parser.mcsModulation(c.code1, c.table, g.rat);
		var ratName = g.rat === '1' ? 'NR' : g.rat === '0' ? 'LTE' : '';
		var parts = [];
		if (c0) parts.push('MCS ' + c.code0 + ' · ' + c0);
		if (c1) parts.push('MCS ' + c.code1 + ' · ' + c1);
		if (!parts.length)
			return E('span', {}, _('Not available'));
		return E('strong', {}, (ratName ? ratName + ' · ' : '') + parts.join(' / '));
	}
	var rows = [];
	groups.forEach(function(group) {
		var ratName = group.rat === '1' ? 'NR' : group.rat === '0' ? 'LTE' : '';
		group.carriers.forEach(function(c, idx) {
			var c0 = parser.mcsModulation(c.code0, c.table, group.rat);
			var c1 = parser.mcsModulation(c.code1, c.table, group.rat);
			var parts = [];
			if (c0) parts.push('MCS ' + c.code0 + ' · ' + c0);
			if (c1) parts.push('MCS ' + c.code1 + ' · ' + c1);
			if (!parts.length)
				return;
			var label = _('Carrier %d').format(idx + 1);
			if (ratName && groups.length > 1)
				label = ratName + ' ' + label;
			rows.push(E('div', { 'class': 'mt-mcs-row' }, [
				E('span', { 'class': 'mt-muted' }, label),
				E('span', {}, parts.join('   ·   '))
			]));
		});
	});
	if (!rows.length)
		return E('span', {}, _('Not available'));
	return E('div', { 'class': 'mt-mcs-list' }, rows);
}

/* ---------- 频段勾选 / 锁频面板 ---------- */

function bandChecklist(options, mask, anyMask) {
	var numeric = parseInt(mask || '0', 16);
	var all = String(mask || '').toUpperCase() === anyMask;
	var checks = options.map(function(item) {
		var value = parseInt(item[0], 16);
		return E('input', {
			'type': 'checkbox',
			'value': item[0],
			'checked': all || (numeric && Math.floor(numeric / value) % 2 === 1) ? 'checked' : null
		});
	});
	var node = E('div', { 'class': 'mt-band-options' }, options.map(function(item, index) {
		return E('label', { 'class': 'mt-band-option' }, [ checks[index], E('span', {}, item[1]) ]);
	}));
	node._bandCheckboxes = checks;
	return node;
}

function selectedBandMask(node, anyMask) {
	var selected = node._bandCheckboxes.filter(function(checkbox) { return checkbox.checked; });
	if (!selected.length)
		return '';
	if (selected.length === node._bandCheckboxes.length)
		return anyMask;
	return selected.reduce(function(total, checkbox) { return total + parseInt(checkbox.value, 16); }, 0).toString(16).toUpperCase();
}

function bandPanel(title, description, checklist) {
	return E('div', { 'class': 'mt-card' }, [
		E('div', { 'class': 'mt-card-head' }, [
			E('h3', { 'class': 'mt-card-title' }, title),
			E('p', { 'class': 'mt-card-desc' }, description)
		]),
		E('div', { 'class': 'mt-advanced-actions', 'style': 'justify-content:flex-start;margin-bottom:10px' },
			btn(_('Select all'), function() { checklist._bandCheckboxes.forEach(function(checkbox) { checkbox.checked = true; }); }, { 'cls': 'mt-session-action' })),
		checklist
	]);
}

// 锁频面板字段注册表 —— cellLockCard 的 "Fill" 按钮据此回填表单
var lockPanelFields = { lte: null, nr: null };

// 邻区卡片 + 一键锁定 / 回填锁频面板
function cellLockCard(nb, index, rat, bandName, hideRsqr) {
	var ratLabel = nb.rat === '101' || nb.rat === 'NR' ? 'NR'
		: nb.rat === '1' || nb.rat === 'LTE' ? 'LTE'
		: rat === 'nr' ? 'NR' : rat === 'lte' ? 'LTE' : nb.rat || '?';
	var bandDisplay = bandName || ratLabel;
	var bandNum = parser.bandNameToNumber(bandName);
	var panelRat = (ratLabel === 'NR') ? 'nr' : 'lte';

	function applyFill() {
		var fields = lockPanelFields[panelRat];
		if (!fields || !fields.type || !fields.bands || !fields.arfcns)
			return false;
		fields.type.value = (nb.pci && nb.pci !== '?') ? '2' : '1';
		fields.type.dispatchEvent(new Event('change'));
		fields.bands.value = bandNum || '';
		fields.arfcns.value = nb.arfcn || '';
		if (fields.scs) fields.scs.value = '0';
		if (fields.pcis) fields.pcis.value = nb.pci || '';
		return true;
	}

	var lockBtn = E('button', { 'type': 'button', 'class': 'mt-lock-btn mt-lock-btn--danger' }, _('Lock'));
	lockBtn.addEventListener('click', function() {
		var isNr = (ratLabel === 'NR');
		var lockType = nb.pci && nb.pci !== '?' ? '2' : '1';
		var args = isNr
			? ['lock', 'nr', lockType, bandNum || '41', nb.arfcn || '',
			   isNr && lockType === '2' ? '0' : '', nb.pci || '']
			: ['lock', 'lte', lockType, bandNum || '3', nb.arfcn || '', '', nb.pci || ''];
		ui.showModal(_('Confirm lock'), [
			E('p', {}, _('Lock to %s cell? %s · ARFCN %s · PCI %s. Mobile service will reconnect.')
				.format(ratLabel, bandDisplay, nb.arfcn || '?', nb.pci || '?')),
			E('div', { 'class': 'right' }, [
				E('button', { 'type': 'button', 'class': 'btn', 'click': ui.hideModal }, _('Cancel')),
				E('button', { 'type': 'button', 'class': 'btn cbi-button-negative', 'click': function() {
					ui.hideModal();
					applyFill();
					api.at(args).then(function() {
						ui.addNotification(null, E('p', {}, _('Frequency lock applied.')));
						window.setTimeout(function() { window.location.reload(); }, 2500);
					}, function(err) {
						ui.addNotification(null, E('p', {}, err.message || _('Lock failed.')), 'danger');
					});
				} }, _('Lock'))
			])
		]);
	});

	var fillBtn = E('button', { 'type': 'button', 'class': 'mt-lock-btn' }, _('Fill'));
	fillBtn.addEventListener('click', function() {
		if (!applyFill()) {
			ui.addNotification(null, E('p', {}, _('Lock panel not found. Scroll down to "Frequency and cell selection".')), 'warning');
			return;
		}
		ui.addNotification(null, E('p', {}, _('%s cell data filled into %s lock panel. Review and click "Review and apply".')
			.format(bandDisplay, panelRat === 'nr' ? '5G NR' : 'LTE')));
		var fields = lockPanelFields[panelRat];
		var card = fields && fields.type && fields.type.closest ? fields.type.closest('.mt-card') : null;
		if (card) card.scrollIntoView({ behavior: 'smooth', block: 'center' });
	});

	var thirdBar = (ratLabel === 'LTE' && nb.sinr === '' && nb.rxlev !== undefined)
		? signalBar(nb.rxlev, 'rsrp', 'RXLEV')
		: signalBar(nb.sinr, 'sinr', 'SINR');
	var children = [
		E('div', { 'class': 'mt-lock-cell-head' }, [
			E('div', { 'style': 'min-width:0' }, [
				E('div', { 'class': 'mt-lock-cell-name' }, bandDisplay + (nb.arfcn ? ' · ' + nb.arfcn : '')),
				nb.pci ? E('div', { 'class': 'mt-lock-cell-desc' }, 'PCI ' + nb.pci) : null
			]),
			E('div', { 'class': 'mt-lock-cell-actions' }, [ fillBtn, lockBtn ])
		]),
		signalBar(nb.rsrp, 'rsrp', 'RSRP')
	];
	if (!hideRsqr)
		children.push(signalBar(nb.rsrq, 'rsrq', 'RSRQ'));
	children.push(thirdBar);
	return E('div', { 'class': 'mt-lock-cell-card' }, children);
}

// 锁频面板（LTE / NR 通用）：类型 + 频段 / ARFCN / SCS / PCI + 校验 + 确认应用
function lockPanel(title, rat, lockData) {
	var parsed = parser.parseLockData(Array.isArray(lockData) ? lockData : [], rat);
	var type = E('select', { 'class': 'cbi-input-select' }, [
		E('option', { 'value': '3' }, _('Band Lock')),
		E('option', { 'value': '1' }, _('ARFCN Lock')),
		E('option', { 'value': '2' }, _('Cell Lock')),
		E('option', { 'value': '0' }, _('Remove Lock'))
	]);
	type.value = /^(0|1|2|3)$/.test(parsed.type || '') ? parsed.type : '0';
	var bands = E('input', { 'class': 'cbi-input-text', 'placeholder': rat === 'nr' ? '78,41' : '3,8', 'inputmode': 'numeric', 'value': parsed.bands });
	var arfcns = E('input', { 'class': 'cbi-input-text', 'placeholder': rat === 'nr' ? '630000,520000' : '1850,3450', 'inputmode': 'numeric', 'value': parsed.arfcns });
	var scs = E('input', { 'class': 'cbi-input-text', 'placeholder': '1,1', 'inputmode': 'numeric', 'value': parsed.scs });
	var pcis = E('input', { 'class': 'cbi-input-text', 'placeholder': '100,200', 'inputmode': 'numeric', 'value': parsed.pcis });
	lockPanelFields[rat] = { type: type, bands: bands, arfcns: arfcns, scs: scs, pcis: pcis };

	var wraps = {};
	function field(key, label, input, help) {
		wraps[key] = E('div', { 'class': 'mt-band-field' }, [
			E('span', { 'class': 'mt-band-field-name' }, label),
			input,
			E('div', { 'class': 'mt-scan-note' }, help)
		]);
		return wraps[key];
	}
	function update() {
		var t = type.value;
		wraps.bands.style.display = t === '0' ? 'none' : '';
		wraps.arfcns.style.display = t === '1' || t === '2' ? '' : 'none';
		if (wraps.scs) wraps.scs.style.display = t === '1' || t === '2' ? '' : 'none';
		wraps.pcis.style.display = t === '2' ? '' : 'none';
	}
	function apply() {
		var t = type.value, values = [ parser.cleanCsv(bands.value), parser.cleanCsv(arfcns.value), parser.cleanCsv(scs.value), parser.cleanCsv(pcis.value) ];
		var required = t === '0' ? [] : t === '3' ? [ 0 ] : t === '1' ? (rat === 'nr' ? [ 0, 1, 2 ] : [ 0, 1 ]) : (rat === 'nr' ? [ 0, 1, 2, 3 ] : [ 0, 1, 3 ]);
		if (required.some(function(i) { return !parser.validCsv(values[i]); }))
			return ui.addNotification(null, E('p', {}, _('Complete all required fields using comma-separated numbers.')), 'warning');
		var lengths = required.map(function(i) { return values[i].split(',').length; });
		if (lengths.some(function(n) { return n !== lengths[0]; }))
			return ui.addNotification(null, E('p', {}, _('Each field must contain the same number of values.')), 'warning');
		if (lengths[0] > 20)
			return ui.addNotification(null, E('p', {}, _('The MT5700M manual allows at most 20 lock entries.')), 'warning');
		if (rat === 'nr' && (t === '1' || t === '2') && !parser.csvInRange(values[2], 0, 4))
			return ui.addNotification(null, E('p', {}, _('NR SCS type must be between 0 and 4.')), 'warning');
		if (t === '2' && !parser.csvInRange(values[3], 0, rat === 'nr' ? 1007 : 503))
			return ui.addNotification(null, E('p', {}, _('PCI is outside the valid range for the selected radio technology.')), 'warning');
		var args = rat === 'nr' ? [ 'lock', rat, t, values[0], values[1], values[2], values[3] ] : [ 'lock', rat, t, values[0], values[1], values[3] ];
		ui.showModal(_('Confirm frequency change'), [
			E('p', {}, [
				t === '0' ? _('Remove the current %s frequency lock?').format(rat.toUpperCase()) : _('Apply this %s frequency lock? Mobile connectivity may reconnect.').format(rat.toUpperCase()),
				' ', _('Mobile service will disconnect briefly while the module enters airplane mode.')
			]),
			E('div', { 'class': 'right' }, [
				E('button', { 'type': 'button', 'class': 'btn', 'click': ui.hideModal }, _('Cancel')),
				' ',
				E('button', { 'type': 'button', 'class': 'btn cbi-button-negative', 'click': function() {
					ui.hideModal();
					api.at(args).then(function() {
						ui.addNotification(null, E('p', {}, _('Frequency lock updated.')));
						window.setTimeout(function() { window.location.reload(); }, 2500);
					}, function(err) {
						ui.addNotification(null, E('p', {}, err.message || _('The modem rejected this setting.')), 'danger');
					});
				} }, t === '0' ? _('Remove Lock') : _('Apply Lock'))
			])
		]);
	}

	type.addEventListener('change', update);
	var body = [
		field('type', _('Lock Type'), type, _('Choose the least restrictive mode that meets your need.')),
		field('bands', _('Bands'), bands, _('Use numbers separated by commas.')),
		field('arfcns', _('ARFCNs'), arfcns, _('One ARFCN for each band.'))
	];
	if (rat === 'nr')
		body.push(field('scs', _('SCS Types'), scs, _('One SCS type for each NR band.')));
	body.push(field('pcis', 'PCI', pcis, _('One PCI for each band and ARFCN.')));
	body.push(E('div', { 'class': 'mt-advanced-actions' },
		btn(_('Review and apply'), apply, { 'primary': true, 'cls': 'mt-lock-btn' })));

	var card = E('div', { 'class': 'mt-card' }, [
		E('h3', { 'class': 'mt-card-title', 'style': 'margin:0 0 12px' }, title),
		E('div', { 'class': 'mt-band-fields' }, body)
	]);
	window.setTimeout(update, 0);
	return card;
}

return baseclass.extend({
	isNode: isNode,
	cssLink: cssLink,
	skeletonPage: skeletonPage,
	badge: badge,
	signalBadge: signalBadge,
	hero: hero,
	card: card,
	row: row,
	formRow: formRow,
	stateRow: stateRow,
	fact: fact,
	facts: facts,
	btn: btn,
	btnLink: btnLink,
	actionBar: actionBar,
	select: select,
	confirmRun: confirmRun,
	runConfirmed: runConfirmed,
	details: details,
	raw: raw,
	signalColorClass: signalColorClass,
	signalPercent: signalPercent,
	signalBar: signalBar,
	signalBars: signalBars,
	gauge: gauge,
	metric: metric,
	mcsDetailNode: mcsDetailNode,
	bandChecklist: bandChecklist,
	selectedBandMask: selectedBandMask,
	bandPanel: bandPanel,
	cellLockCard: cellLockCard,
	lockPanel: lockPanel
});
