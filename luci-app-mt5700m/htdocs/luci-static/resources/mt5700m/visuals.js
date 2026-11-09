'use strict';
'require baseclass';

/*
 * MT5700M LuCI — visuals.js
 * ---------------------------------------------
 * 自绘动态 SVG 视觉层（零依赖、零第三方图表库）：
 *   signalOrb        信号球：RSRP 圆弧 + 质量环 + 轨道粒子
 *   throughputWave   吞吐波形：近 7 日 RX/TX 面积图，路径描边动画
 *   ringGauge        环形仪表：温度 / 任意百分比
 *   bandBars         频段柱：每个载波的 RSRP/带宽，逐条生长动画
 *   beamRadar        SSB 波束雷达：极化网格 + 旋转扫描 + 波束强度
 *   linkTopology     链路拓扑：模组 → 基站 → 互联网，虚线流动
 *
 * 约定：
 *   - 只产出 SVG 节点，不放文案（文案在页面里，保持 i18n 与可选中）；
 *   - 颜色全部走 style.css 的 --mt-* 令牌，暗色模式自动适配；
 *   - 动效全部由 CSS 驱动，prefers-reduced-motion: reduce 时静止；
 *   - 每个实例的 gradient/clip id 唯一，同页多次渲染不串色。
 */

var seq = 0;

function uid(prefix) {
	seq += 1;
	return 'mtv-' + prefix + '-' + seq;
}

/* 与 components.js 相同的 SVG 构建方式：DOMParser 优先，innerHTML 兜底 */
function svg(markup) {
	try {
		if (typeof DOMParser !== 'undefined') {
			var doc = new DOMParser().parseFromString(markup, 'image/svg+xml');
			var el = doc.documentElement;
			if (el && el.tagName && el.tagName.toLowerCase() === 'svg')
				return document.importNode(el, true);
		}
	} catch (e) { /* fall through */ }
	var div = document.createElement('div');
	div.innerHTML = markup;
	return div.firstElementChild || div;
}

function clamp(value, low, high) {
	return Math.max(low, Math.min(high, value));
}

function number(value, fallback) {
	var n = parseFloat(value);
	return isFinite(n) ? n : fallback;
}

function round(value, digits) {
	var factor = Math.pow(10, digits || 0);
	return Math.round(value * factor) / factor;
}

/* 折线平滑：中点三次贝塞尔，视觉上接近单调曲线，计算量可忽略 */
function smoothPath(points) {
	if (!points.length) return '';
	var d = 'M' + round(points[0][0], 1) + ',' + round(points[0][1], 1);
	for (var i = 1; i < points.length; i++) {
		var p0 = points[i - 1], p1 = points[i];
		var mx = round((p0[0] + p1[0]) / 2, 1);
		d += ' C' + mx + ',' + round(p0[1], 1) + ' ' + mx + ',' + round(p1[1], 1)
			+ ' ' + round(p1[0], 1) + ',' + round(p1[1], 1);
	}
	return d;
}

function areaPath(points, baseY) {
	if (!points.length) return '';
	var d = smoothPath(points);
	var first = points[0][0], last = points[points.length - 1][0];
	return d + ' L' + round(last, 1) + ',' + baseY + ' L' + round(first, 1) + ',' + baseY + ' Z';
}

/* ---------------------------------------------------------------- 信号球 */

function signalOrb(opts) {
	opts = opts || {};
	var percent = clamp(number(opts.percent, 0), 0, 100);
	var connected = opts.connected !== false;
	var cls = connected ? (opts.cls || 'ok') : 'bad';
	var color = cls === 'bad' ? 'var(--mt-bad)' : cls === 'warn' ? 'var(--mt-warn)' : 'var(--mt-ok)';
	var size = 168, center = size / 2;
	var radius = 62, circumference = 2 * Math.PI * radius;
	var arc = circumference * percent / 100;
	var grad = uid('orb-grad');
	var glow = uid('orb-glow');

	var markup = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ' + size + ' ' + size + '"'
		+ ' class="mtv-svg mtv-orb" width="' + size + '" height="' + size + '" role="img" aria-hidden="true">'
		+ '<defs>'
		+ '<linearGradient id="' + grad + '" x1="0%" y1="0%" x2="100%" y2="100%">'
		+ '<stop offset="0%" stop-color="' + color + '" stop-opacity="0.95"/>'
		+ '<stop offset="100%" stop-color="var(--mt-accent-2)" stop-opacity="0.55"/>'
		+ '</linearGradient>'
		+ '<filter id="' + glow + '" x="-40%" y="-40%" width="180%" height="180%">'
		+ '<feGaussianBlur stdDeviation="4" result="b"/><feMerge><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge>'
		+ '</filter>'
		+ '</defs>'
		/* 外层刻度环（缓慢反向旋转） */
		+ '<g class="mtv-rotor mtv-rotor--slow" style="transform-origin:' + center + 'px ' + center + 'px">'
		+ '<circle cx="' + center + '" cy="' + center + '" r="78" fill="none" stroke="var(--mt-border-soft)"'
		+ ' stroke-width="1" stroke-dasharray="2 6" opacity="0.9"/>'
		+ '</g>'
		/* 轨道 */
		+ '<g class="mtv-rotor" style="transform-origin:' + center + 'px ' + center + 'px;animation-duration:26s">'
		+ '<circle cx="' + center + '" cy="' + center + '" r="72" fill="none" stroke="var(--mt-border)" stroke-width="1" opacity="0.55"/>'
		+ '<circle cx="' + center + '" cy="' + (center - 72) + '" r="3" fill="var(--mt-accent)" opacity="0.9"/>'
		+ '</g>'
		+ '<g class="mtv-rotor" style="transform-origin:' + center + 'px ' + center + 'px;animation-duration:17s;animation-direction:reverse">'
		+ '<circle cx="' + center + '" cy="' + center + '" r="52" fill="none" stroke="var(--mt-border-soft)" stroke-width="1" opacity="0.7"/>'
		+ '<circle cx="' + (center + 52) + '" cy="' + center + '" r="2.5" fill="var(--mt-accent-2)" opacity="0.9"/>'
		+ '</g>'
		/* 进度弧 + 底环 */
		+ '<circle cx="' + center + '" cy="' + center + '" r="' + radius + '" fill="none" stroke="var(--mt-border-soft)" stroke-width="8"/>'
		+ '<circle cx="' + center + '" cy="' + center + '" r="' + radius + '" fill="none" stroke="url(#' + grad + ')"'
		+ ' stroke-width="8" stroke-linecap="round" filter="url(#' + glow + ')"'
		+ ' stroke-dasharray="' + round(arc, 2) + ' ' + round(circumference, 2) + '"'
		+ ' transform="rotate(-90 ' + center + ' ' + center + ')" class="mtv-arc"/>'
		/* 核心 */
		+ '<circle cx="' + center + '" cy="' + center + '" r="30" fill="var(--mt-surface-soft)" stroke="var(--mt-border-soft)" stroke-width="1"/>'
		+ '<circle cx="' + center + '" cy="' + center + '" r="10" fill="' + color + '" class="mtv-core" opacity="0.9"/>'
		+ '<circle cx="' + center + '" cy="' + center + '" r="18" fill="none" stroke="' + color + '" stroke-width="1" opacity="0.35" class="mtv-core-ring"/>'
		+ '</svg>';

	return svg(markup);
}

/* ------------------------------------------------------------ 吞吐波形 */

function throughputWave(opts) {
	opts = opts || {};
	var days = opts.days || [];
	var width = 340, height = 116, padX = 6, padTop = 10, padBottom = 14;
	var baseY = height - padBottom;
	var plotW = width - padX * 2;
	var plotH = baseY - padTop;
	var max = 1;

	days.forEach(function(day) {
		max = Math.max(max, number(day.rx, 0), number(day.tx, 0));
	});

	var rxPoints = [], txPoints = [];
	days.forEach(function(day, index) {
		var x = days.length === 1 ? padX + plotW / 2 : padX + plotW * index / (days.length - 1);
		rxPoints.push([ x, baseY - plotH * (number(day.rx, 0) / max) ]);
		txPoints.push([ x, baseY - plotH * (number(day.tx, 0) / max) ]);
	});

	var fillId = uid('wave-fill');
	var lineId = uid('wave-line');
	var grid = '';
	for (var g = 1; g <= 3; g++) {
		var gy = round(padTop + plotH * g / 4, 1);
		grid += '<line x1="' + padX + '" y1="' + gy + '" x2="' + (width - padX) + '" y2="' + gy
			+ '" stroke="var(--mt-border-soft)" stroke-width="1" stroke-dasharray="1 5" opacity="0.8"/>';
	}

	var last = rxPoints.length ? rxPoints[rxPoints.length - 1] : [ width - padX, baseY ];

	var markup = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ' + width + ' ' + height + '"'
		+ ' class="mtv-svg mtv-wave" preserveAspectRatio="none" role="img" aria-hidden="true">'
		+ '<defs>'
		+ '<linearGradient id="' + fillId + '" x1="0" y1="0" x2="0" y2="1">'
		+ '<stop offset="0%" stop-color="var(--mt-accent)" stop-opacity="0.28"/>'
		+ '<stop offset="100%" stop-color="var(--mt-accent)" stop-opacity="0"/>'
		+ '</linearGradient>'
		+ '<linearGradient id="' + lineId + '" x1="0" y1="0" x2="1" y2="0">'
		+ '<stop offset="0%" stop-color="var(--mt-accent-2)"/>'
		+ '<stop offset="100%" stop-color="var(--mt-accent)"/>'
		+ '</linearGradient>'
		+ '</defs>'
		+ grid
		+ (rxPoints.length
			? '<path d="' + areaPath(rxPoints, baseY) + '" fill="url(#' + fillId + ')" class="mtv-wave-area"/>'
			+ '<path d="' + smoothPath(rxPoints) + '" fill="none" stroke="url(#' + lineId + ')" stroke-width="2.4"'
			+ ' stroke-linecap="round" stroke-linejoin="round" pathLength="1" class="mtv-wave-line"/>'
			+ '<path d="' + smoothPath(txPoints) + '" fill="none" stroke="var(--mt-accent-2)" stroke-width="1.4"'
			+ ' stroke-linecap="round" stroke-dasharray="4 4" opacity="0.75" pathLength="1" class="mtv-wave-line mtv-wave-line--alt"/>'
			+ '<circle cx="' + round(last[0], 1) + '" cy="' + round(last[1], 1) + '" r="4" fill="var(--mt-accent)" class="mtv-dot"/>'
			: '<line x1="' + padX + '" y1="' + baseY + '" x2="' + (width - padX) + '" y2="' + baseY
				+ '" stroke="var(--mt-border)" stroke-width="1" stroke-dasharray="6 6" opacity="0.7"/>')
		+ '</svg>';

	return svg(markup);
}

/* ------------------------------------------------------------ 环形仪表 */

function ringGauge(opts) {
	opts = opts || {};
	var size = opts.size || 96;
	var center = size / 2;
	var radius = center - 8;
	var circumference = 2 * Math.PI * radius;
	var percent = clamp(number(opts.percent, 0), 0, 100);
	var color = opts.cls === 'bad' ? 'var(--mt-bad)' : opts.cls === 'warn' ? 'var(--mt-warn)' : 'var(--mt-ok)';
	var track = uid('ring-track');

	var markup = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ' + size + ' ' + size + '"'
		+ ' class="mtv-svg mtv-ring" width="' + size + '" height="' + size + '" role="img" aria-hidden="true">'
		+ '<defs><linearGradient id="' + track + '" x1="0" y1="0" x2="1" y2="1">'
		+ '<stop offset="0%" stop-color="' + color + '"/><stop offset="100%" stop-color="var(--mt-accent-2)"/>'
		+ '</linearGradient></defs>'
		+ '<circle cx="' + center + '" cy="' + center + '" r="' + radius + '" fill="none"'
		+ ' stroke="var(--mt-border-soft)" stroke-width="7"/>'
		+ '<circle cx="' + center + '" cy="' + center + '" r="' + radius + '" fill="none"'
		+ ' stroke="url(#' + track + ')" stroke-width="7" stroke-linecap="round"'
		+ ' stroke-dasharray="' + round(circumference * percent / 100, 2) + ' ' + round(circumference, 2) + '"'
		+ ' transform="rotate(-90 ' + center + ' ' + center + ')" class="mtv-arc"/>'
		+ '</svg>';

	return svg(markup);
}

/* -------------------------------------------------------------- 频段柱 */

function bandBars(opts) {
	opts = opts || {};
	var items = opts.items || [];
	var width = 340, height = 118, padBottom = 18, padTop = 8;
	var plotH = height - padBottom - padTop;
	var slot = items.length ? (width - 16) / items.length : width;
	var max = 1;

	items.forEach(function(item) {
		max = Math.max(max, Math.abs(number(item.value, 0)));
	});

	var bars = '';
	items.forEach(function(item, index) {
		var value = Math.abs(number(item.value, 0));
		var ratio = clamp(value / max, 0.04, 1);
		var barW = Math.min(26, slot * 0.56);
		var x = 8 + slot * index + (slot - barW) / 2;
		var barH = plotH * ratio;
		var y = padTop + plotH - barH;
		var color = item.active === false ? 'var(--mt-border)' : (ratio < 0.34 ? 'var(--mt-bad)' : ratio < 0.67 ? 'var(--mt-warn)' : 'var(--mt-ok)');
		bars += '<rect x="' + round(x, 1) + '" y="' + round(y, 1) + '" width="' + round(barW, 1) + '" height="' + round(barH, 1)
			+ '" rx="' + round(barW / 3, 1) + '" fill="' + color + '" opacity="0.92"'
			+ ' class="mtv-bar" style="--mtv-delay:' + (index * 90) + 'ms"/>';
		if (item.primary)
			bars += '<circle cx="' + round(x + barW / 2, 1) + '" cy="' + round(y - 7, 1) + '" r="2.4" fill="var(--mt-accent)"/>';
	});

	return svg('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ' + width + ' ' + height + '"'
		+ ' class="mtv-svg mtv-bars" preserveAspectRatio="none" role="img" aria-hidden="true">'
		+ '<line x1="8" y1="' + (height - padBottom) + '" x2="' + (width - 8) + '" y2="' + (height - padBottom)
		+ '" stroke="var(--mt-border)" stroke-width="1"/>'
		+ bars + '</svg>');
}

/* -------------------------------------------------------------- 波束雷达 */

function beamRadar(opts) {
	opts = opts || {};
	var size = 208, center = size / 2, radius = 86;
	var beams = opts.beams || [];
	var sweepId = uid('radar-sweep');
	var spokes = '', rings = '';

	for (var i = 1; i <= 3; i++)
		rings += '<circle cx="' + center + '" cy="' + center + '" r="' + round(radius * i / 3, 1) + '" fill="none"'
			+ ' stroke="var(--mt-border-soft)" stroke-width="1" opacity="0.85"/>';
	for (var s = 0; s < 8; s++) {
		var angle = s * Math.PI / 4;
		spokes += '<line x1="' + center + '" y1="' + center + '" x2="' + round(center + Math.sin(angle) * radius, 1)
			+ '" y2="' + round(center - Math.cos(angle) * radius, 1) + '" stroke="var(--mt-border-soft)"'
			+ ' stroke-width="1" opacity="0.45"/>';
	}

	var dots = '';
	beams.forEach(function(beam, index) {
		var angle = number(beam.angle, 0) * Math.PI / 180;
		var strength = clamp(number(beam.strength, 0.5), 0.15, 1);
		var r = radius * (0.35 + 0.6 * strength);
		var x = center + Math.sin(angle) * r;
		var y = center - Math.cos(angle) * r;
		dots += '<circle cx="' + round(x, 1) + '" cy="' + round(y, 1) + '" r="' + round(2 + 3 * strength, 1) + '"'
			+ ' fill="' + (beam.active === false ? 'var(--mt-faint)' : 'var(--mt-accent)') + '" opacity="' + round(0.35 + 0.6 * strength, 2) + '"'
			+ ' class="mtv-beam" style="--mtv-delay:' + (index * 120) + 'ms"/>';
	});

	return svg('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ' + size + ' ' + size + '"'
		+ ' class="mtv-svg mtv-radar" width="' + size + '" height="' + size + '" role="img" aria-hidden="true">'
		+ '<defs><linearGradient id="' + sweepId + '" x1="0" y1="0" x2="1" y2="0">'
		+ '<stop offset="0%" stop-color="var(--mt-accent)" stop-opacity="0.42"/>'
		+ '<stop offset="100%" stop-color="var(--mt-accent)" stop-opacity="0"/>'
		+ '</linearGradient></defs>'
		+ rings + spokes
		+ '<g class="mtv-sweep" style="transform-origin:' + center + 'px ' + center + 'px">'
		+ '<path d="M' + center + ',' + center + ' L' + center + ',' + (center - radius)
		+ ' A' + radius + ',' + radius + ' 0 0 1 ' + round(center + radius * Math.sin(Math.PI / 4), 1) + ','
		+ round(center - radius * Math.cos(Math.PI / 4), 1) + ' Z" fill="url(#' + sweepId + ')"/>'
		+ '</g>'
		+ '<circle cx="' + center + '" cy="' + center + '" r="3" fill="var(--mt-accent)"/>'
		+ dots + '</svg>');
}

/* -------------------------------------------------------------- 链路拓扑 */

function linkTopology(opts) {
	opts = opts || {};
	var connected = !!opts.connected, reachable = opts.reachable !== false;
	var color = connected ? 'var(--mt-ok)' : reachable ? 'var(--mt-warn)' : 'var(--mt-bad)';
	var width = 340, height = 104, y = 46;
	var nodes = [ [ 52, 'chip' ], [ 170, 'tower' ], [ 288, 'cloud' ] ];

	function icon(kind, x) {
		if (kind === 'chip')
			return '<rect x="' + (x - 11) + '" y="' + (y - 11) + '" width="22" height="22" rx="5" fill="none"'
				+ ' stroke="var(--mt-text)" stroke-width="1.8" opacity="0.85"/>'
				+ '<rect x="' + (x - 4) + '" y="' + (y - 4) + '" width="8" height="8" rx="2" fill="' + color + '"/>';
		if (kind === 'tower')
			return '<path d="M' + (x - 8) + ',' + (y + 12) + ' L' + x + ',' + (y - 10) + ' L' + (x + 8) + ',' + (y + 12)
				+ '" fill="none" stroke="var(--mt-text)" stroke-width="1.8" stroke-linejoin="round" opacity="0.85"/>'
				+ '<circle cx="' + x + '" cy="' + (y - 13) + '" r="3.2" fill="' + color + '" class="mtv-core"/>'
				+ '<path d="M' + (x - 12) + ',' + (y - 6) + ' a12 12 0 0 1 24 0" fill="none" stroke="' + color
				+ '" stroke-width="1.2" opacity="0.45"/>';
		return '<path d="M' + (x - 13) + ',' + (y + 4) + ' a7 7 0 0 1 13,-6 a6 6 0 0 1 12,2 a6 6 0 0 1 1,10 H'
			+ (x - 13) + ' Z" fill="none" stroke="var(--mt-text)" stroke-width="1.7" stroke-linejoin="round" opacity="0.85"/>';
	}

	var links = '';
	for (var i = 0; i < nodes.length - 1; i++) {
		var x1 = nodes[i][0] + 16, x2 = nodes[i + 1][0] - 18;
		links += '<line x1="' + x1 + '" y1="' + y + '" x2="' + x2 + '" y2="' + y + '" stroke="' + color
			+ '" stroke-width="2" stroke-dasharray="6 6" class="mtv-dash" opacity="' + (connected ? 0.85 : 0.4) + '"/>';
		links += '<circle cx="' + round((x1 + x2) / 2, 1) + '" cy="' + y + '" r="2.6" fill="' + color
			+ '" class="mtv-packet" style="--mtv-delay:' + (i * 320) + 'ms"/>';
	}

	return svg('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ' + width + ' ' + height + '"'
		+ ' class="mtv-svg mtv-topo" preserveAspectRatio="xMidYMid meet" role="img" aria-hidden="true">'
		+ links
		+ icon('chip', nodes[0][0]) + icon('tower', nodes[1][0]) + icon('cloud', nodes[2][0])
		+ '</svg>');
}

return baseclass.extend({
	signalOrb: signalOrb,
	throughputWave: throughputWave,
	ringGauge: ringGauge,
	bandBars: bandBars,
	beamRadar: beamRadar,
	linkTopology: linkTopology,
	clamp: clamp,
	number: number
});
