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
var STYLE_VERSION = '2.8.1';

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

/* ---------- 动态 SVG 与图标库 ---------- */

function createSvg(markup) {
	try {
		if (typeof DOMParser !== 'undefined') {
			var doc = new DOMParser().parseFromString(markup, 'image/svg+xml');
			var el = doc.documentElement;
			if (el && el.tagName && el.tagName.toLowerCase() === 'svg')
				return document.importNode(el, true);
		}
	} catch (e) { /* fallback */ }
	var div = document.createElement('div');
	div.innerHTML = markup;
	return div.firstElementChild || div;
}

// 动态 5G 信号塔（发射电磁波脉冲、信标闪烁）
function svgTower(opts) {
	opts = opts || {};
	var active = opts.active !== false;
	var isWarn = opts.status === 'warn';
	var isBad = opts.status === 'bad';
	var beaconClass = isBad ? 'bad' : isWarn ? 'warn' : 'ok';
	var waveColor = isBad ? 'rgba(239, 68, 68, 0.7)' : isWarn ? 'rgba(245, 158, 11, 0.7)' : 'rgba(56, 189, 248, 0.85)';
	var towerColor = 'rgba(255, 255, 255, 0.9)';
	var beaconFill = isBad ? '#ef4444' : isWarn ? '#f59e0b' : '#34d399';

	var svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 110 110" width="84" height="84" style="overflow:visible">'
		+ '<defs>'
		+ '<linearGradient id="mt-tow-grad" x1="0%" y1="0%" x2="100%" y2="100%">'
		+ '<stop offset="0%" stop-color="#ffffff" stop-opacity="0.95"/>'
		+ '<stop offset="100%" stop-color="#93c5fd" stop-opacity="0.75"/>'
		+ '</linearGradient>'
		+ '</defs>'
		+ (active ? (
			'<circle cx="55" cy="24" r="14" fill="none" stroke="' + waveColor + '" class="mt-svg-pulse-ring-1"/>'
			+ '<circle cx="55" cy="24" r="14" fill="none" stroke="' + waveColor + '" class="mt-svg-pulse-ring-2"/>'
			+ '<circle cx="55" cy="24" r="14" fill="none" stroke="' + waveColor + '" class="mt-svg-pulse-ring-3"/>'
		) : '')
		// 塔体桁架
		+ '<path d="M42 96 L52 28 M68 96 L58 28" stroke="' + towerColor + '" stroke-width="2.5" stroke-linecap="round"/>'
		+ '<line x1="44" y1="82" x2="66" y2="82" stroke="' + towerColor + '" stroke-width="2"/>'
		+ '<line x1="47" y1="64" x2="63" y2="64" stroke="' + towerColor + '" stroke-width="2"/>'
		+ '<line x1="50" y1="46" x2="60" y2="46" stroke="' + towerColor + '" stroke-width="2"/>'
		+ '<line x1="44" y1="82" x2="63" y2="64" stroke="' + towerColor + '" stroke-width="1.2" opacity="0.7"/>'
		+ '<line x1="66" y1="82" x2="47" y2="64" stroke="' + towerColor + '" stroke-width="1.2" opacity="0.7"/>'
		+ '<line x1="47" y1="64" x2="60" y2="46" stroke="' + towerColor + '" stroke-width="1.2" opacity="0.7"/>'
		+ '<line x1="63" y1="64" x2="50" y2="46" stroke="' + towerColor + '" stroke-width="1.2" opacity="0.7"/>'
		// 塔顶天线阵面
		+ '<line x1="55" y1="28" x2="55" y2="15" stroke="' + towerColor + '" stroke-width="2.8" stroke-linecap="round"/>'
		+ '<line x1="46" y1="21" x2="64" y2="21" stroke="' + towerColor + '" stroke-width="2" stroke-linecap="round"/>'
		+ '<circle cx="55" cy="15" r="4" fill="' + beaconFill + '" class="mt-svg-beacon-dot ' + beaconClass + '"/>'
		+ '</svg>';
	return createSvg(svg);
}

// 动态状态呼吸光环
function svgStatusPulse(status, size) {
	size = size || 16;
	var isOk = status === 'ok' || status === 'connected' || status === true;
	var isWarn = status === 'warn';
	var col = isOk ? 'var(--mt-ok)' : isWarn ? 'var(--mt-warn)' : 'var(--mt-bad)';
	var cls = isOk ? '' : isWarn ? 'warn' : 'bad';
	var svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="' + size + '" height="' + size + '" style="vertical-align:middle;overflow:visible">'
		+ '<circle cx="12" cy="12" r="7" fill="none" stroke="' + col + '" class="mt-svg-pulse-ring-1"/>'
		+ '<circle cx="12" cy="12" r="4.5" fill="' + col + '" class="mt-svg-beacon-dot ' + cls + '"/>'
		+ '</svg>';
	return createSvg(svg);
}

// 圆形 SVG 渐变进度仪表（用于 RSRP / RSRQ / SINR / 温度）
function svgCircularGauge(val, min, max, unit, label, cls) {
	val = typeof val === 'number' ? val : parseFloat(val);
	var valid = !isNaN(val);
	var pct = 0;
	if (valid && max > min) {
		pct = Math.max(0, Math.min(100, ((val - min) / (max - min)) * 100));
	}
	var circum = 207.34;
	var offset = circum - (circum * pct / 100);
	var colorVar = cls === 'excellent' ? 'var(--mt-ok)' :
		cls === 'good' ? 'var(--mt-accent-2)' :
		cls === 'fair' ? 'var(--mt-warn)' :
		cls === 'weak' ? 'var(--mt-bad)' : 'var(--mt-accent)';

	var safeId = 'mt-cg-' + label.replace(/[^a-zA-Z0-9]/g, '');
	var svg = '<svg class="mt-circular-gauge-svg" viewBox="0 0 84 84">'
		+ '<defs>'
		+ '<linearGradient id="' + safeId + '" x1="0%" y1="0%" x2="100%" y2="100%">'
		+ '<stop offset="0%" stop-color="' + colorVar + '" stop-opacity="0.8"/>'
		+ '<stop offset="100%" stop-color="' + colorVar + '" stop-opacity="1"/>'
		+ '</linearGradient>'
		+ '</defs>'
		+ '<circle cx="42" cy="42" r="33" fill="none" stroke="var(--mt-border-soft)" stroke-width="6.5" opacity="0.6"/>'
		+ (valid ? (
			'<circle cx="42" cy="42" r="33" fill="none" stroke="url(#' + safeId + ')" stroke-width="6.5" '
			+ 'stroke-dasharray="' + circum + '" stroke-dashoffset="' + offset.toFixed(1) + '" '
			+ 'stroke-linecap="round" transform="rotate(-90 42 42)" style="transition:stroke-dashoffset 0.6s ease"/>'
		) : '')
		+ '</svg>';

	return E('div', { 'class': 'mt-circular-gauge-card' }, [
		createSvg(svg),
		E('div', { 'class': 'mt-circular-gauge-val' }, [
			valid ? String(val) : '--',
			unit ? E('span', { 'class': 'mt-circular-gauge-unit' }, unit) : null
		]),
		E('div', { 'class': 'mt-circular-gauge-lbl' }, label)
	]);
}

// 动态芯片 SVG（电路引脚 + 激光扫描线）
function svgChip(opts) {
	opts = opts || {};
	var size = opts.size || 52;
	var color = 'rgba(255, 255, 255, 0.9)';
	var svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64" width="' + size + '" height="' + size + '">'
		+ '<rect x="14" y="14" width="36" height="36" rx="6" fill="rgba(18, 100, 216, 0.25)" stroke="' + color + '" stroke-width="2"/>'
		+ '<rect x="22" y="22" width="20" height="20" rx="3" fill="rgba(7, 152, 142, 0.4)" stroke="rgba(255,255,255,0.7)" stroke-width="1.2"/>'
		// 引脚
		+ '<line x1="22" y1="8" x2="22" y2="14" stroke="' + color + '" stroke-width="2" stroke-linecap="round"/>'
		+ '<line x1="32" y1="8" x2="32" y2="14" stroke="' + color + '" stroke-width="2" stroke-linecap="round"/>'
		+ '<line x1="42" y1="8" x2="42" y2="14" stroke="' + color + '" stroke-width="2" stroke-linecap="round"/>'
		+ '<line x1="22" y1="50" x2="22" y2="56" stroke="' + color + '" stroke-width="2" stroke-linecap="round"/>'
		+ '<line x1="32" y1="50" x2="32" y2="56" stroke="' + color + '" stroke-width="2" stroke-linecap="round"/>'
		+ '<line x1="42" y1="50" x2="42" y2="56" stroke="' + color + '" stroke-width="2" stroke-linecap="round"/>'
		+ '<line x1="8" y1="22" x2="14" y2="22" stroke="' + color + '" stroke-width="2" stroke-linecap="round"/>'
		+ '<line x1="8" y1="32" x2="14" y2="32" stroke="' + color + '" stroke-width="2" stroke-linecap="round"/>'
		+ '<line x1="8" y1="42" x2="14" y2="42" stroke="' + color + '" stroke-width="2" stroke-linecap="round"/>'
		+ '<line x1="50" y1="22" x2="56" y2="22" stroke="' + color + '" stroke-width="2" stroke-linecap="round"/>'
		+ '<line x1="50" y1="32" x2="56" y2="32" stroke="' + color + '" stroke-width="2" stroke-linecap="round"/>'
		+ '<line x1="50" y1="42" x2="56" y2="42" stroke="' + color + '" stroke-width="2" stroke-linecap="round"/>'
		// 动态扫描激光线
		+ '<line x1="16" y1="32" x2="48" y2="32" stroke="#38bdf8" stroke-width="2" class="mt-svg-chip-laser"/>'
		+ '</svg>';
	return createSvg(svg);
}

// 动态载波聚合轨道 SVG
function svgCarrier(primaryBand, caCount) {
	caCount = caCount || 1;
	var svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" width="72" height="72">'
		+ '<circle cx="50" cy="50" r="38" fill="none" stroke="var(--mt-border-soft)" stroke-width="1.5" stroke-dasharray="3 3"/>'
		+ '<circle cx="50" cy="50" r="24" fill="none" stroke="var(--mt-border)" stroke-width="1.5"/>'
		+ '<g class="mt-svg-orbit">'
		+ '<circle cx="50" cy="12" r="5" fill="var(--mt-accent-2)"/>'
		+ (caCount > 1 ? '<circle cx="88" cy="50" r="4" fill="var(--mt-ok)"/>' : '')
		+ (caCount > 2 ? '<circle cx="50" cy="88" r="4" fill="var(--mt-warn)"/>' : '')
		+ '</g>'
		+ '<circle cx="50" cy="50" r="14" fill="var(--mt-accent)"/>'
		+ '<text x="50" y="54" fill="#fff" font-size="9" font-weight="800" text-anchor="middle">' + (primaryBand || '5G') + '</text>'
		+ '</svg>';
	return createSvg(svg);
}

// 动态流量吞吐双向流动箭头 SVG
function svgTrafficArrows() {
	var svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 60 28" width="54" height="25" style="vertical-align:middle">'
		+ '<path d="M4 8 L54 8" stroke="var(--mt-accent)" stroke-width="2.5" stroke-linecap="round" class="mt-svg-flow-line"/>'
		+ '<polyline points="48,4 55,8 48,12" fill="none" stroke="var(--mt-accent)" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/>'
		+ '<path d="M54 20 L4 20" stroke="var(--mt-accent-2)" stroke-width="2.5" stroke-linecap="round" class="mt-svg-flow-line-rev"/>'
		+ '<polyline points="10,16 3,20 10,24" fill="none" stroke="var(--mt-accent-2)" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"/>'
		+ '</svg>';
	return createSvg(svg);
}

// 常用动作 SVG 图标
function svgRefreshIcon() {
	var svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">'
		+ '<polyline points="23 4 23 10 17 10"></polyline>'
		+ '<polyline points="1 20 1 14 7 14"></polyline>'
		+ '<path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"></path>'
		+ '</svg>';
	return createSvg(svg);
}

function svgWebUiIcon() {
	var svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">'
		+ '<circle cx="12" cy="12" r="10"></circle>'
		+ '<line x1="2" y1="12" x2="22" y2="12"></line>'
		+ '<path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"></path>'
		+ '</svg>';
	return createSvg(svg);
}

function svgSendIcon() {
	var svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">'
		+ '<line x1="22" y1="2" x2="11" y2="13"></line>'
		+ '<polygon points="22 2 15 22 11 13 2 9 22 2"></polygon>'
		+ '</svg>';
	return createSvg(svg);
}

function svgTerminalPrompt() {
	var svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">'
		+ '<polyline points="4 17 10 11 4 5"></polyline>'
		+ '<line x1="12" y1="19" x2="20" y2="19"></line>'
		+ '</svg>';
	return createSvg(svg);
}

function hero(kicker, title, desc, side, variant, heroSvg) {
	return E('section', { 'class': 'mt-hero' + (variant ? ' mt-hero--' + variant : '') }, [
		heroSvg ? E('div', { 'class': 'mt-hero-illustration' }, heroSvg) : null,
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

/*
 * `modem.mcs` 的方向载荷 → 旧版 `parseMcsSection` 的分组形状。
 *
 * 后端按应答原样给出每个载波的分组号与制式（`group`/`rat`），这里只把
 * 制式映射回旧代码的 '0'/'1' 记号；分组顺序、行标签与迁移前逐字节相同。
 */
function mcsGroupsFromPayload(payload) {
	if (!payload || !Array.isArray(payload.carriers) || !payload.carriers.length)
		return [];
	var order = [], byGroup = {};
	payload.carriers.forEach(function(carrier) {
		var key = String(carrier.group);
		if (!byGroup[key]) {
			byGroup[key] = { rat: carrier.rat === 'NR' ? '1' : carrier.rat === 'LTE' ? '0' : '', carriers: [] };
			order.push(key);
		}
		byGroup[key].carriers.push({ table: carrier.mcs_table_index, code0: carrier.code0, code1: carrier.code1 });
	});
	return order.map(function(key) { return byGroup[key]; });
}

function mcsDetailNode(payload) {
	var groups = mcsGroupsFromPayload(payload);
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
	createSvg: createSvg,
	svgTower: svgTower,
	svgStatusPulse: svgStatusPulse,
	svgCircularGauge: svgCircularGauge,
	svgChip: svgChip,
	svgCarrier: svgCarrier,
	svgTrafficArrows: svgTrafficArrows,
	svgRefreshIcon: svgRefreshIcon,
	svgWebUiIcon: svgWebUiIcon,
	svgSendIcon: svgSendIcon,
	svgTerminalPrompt: svgTerminalPrompt,
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
