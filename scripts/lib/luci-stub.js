'use strict';

/*
 * 迁移期一致性证明的公共桩（scripts/lib/luci-stub.js）
 *
 * 两个 prove-* 脚本都要把 LuCI 的 view/component 模块装进一个「够用的 DOM」
 * 里跑起来，再比较渲染结果。这里集中一份，避免每把刀各复制一套桩：
 *
 *   - makeScope(api)      LuCI 全局（E/_/ui/dom/window/L/String.format）+ 记录调用的 api 桩
 *   - loadModule(src, s)  用 new Function 把模块源码在桩作用域里求值
 *   - loadSide(read, api) 按 parser → components → view 的顺序装载（与 LuCI 一致）
 *   - serialize(node)     结构与文案（可选：取值槽只比「有无」）
 *   - slotValues/slotsOf  取值槽文本（按 DOM 顺序 / 按 class）
 *   - textOf/lines/countTag 取值工具
 *   - collect/pressButton/findButtons  事件驱动（`click` 属性即监听器，同 LuCI）
 *   - tick()              跑完微任务（驱动 routeCall/at 的 Promise 链）
 *
 * 桩的行为刻意贴近 LuCI：`E('button', { click: fn })` 的属性会被当成事件
 * 监听器；`ui.showModal` 会把标题与子节点记下来，便于脚本点确认按钮；
 * `window.setTimeout(fn, 0)` 立即执行，其它延时记进 `window.pending`。
 */

class El {
	constructor(tag, attrs, children) {
		this.nodeType = 1;
		this.tagName = String(tag).toUpperCase();
		this.attrs = attrs || {};
		this.children = [];
		this.style = {};
		this.value = undefined;
		this._listeners = {};
		const list = children === undefined || children === null ? []
			: Array.isArray(children) ? children : [ children ];
		for (const child of list)
			if (child !== null && child !== undefined && child !== false) {
				this.children.push(child);
				if (child && typeof child === 'object') child.parentNode = this;
			}
		// LuCI 的 E() 把 attrs 里的 click 等属性接成监听器（btn() 就是这么发的）
		for (const key of Object.keys(this.attrs))
			if (typeof this.attrs[key] === 'function' && key.indexOf('on') !== 0)
				this.addEventListener(key, this.attrs[key]);
		// 浏览器会把 checked/value 这类属性反射成同名 property，桩必须一样：
		// bandChecklist 用 `checked` 属性标记选中，selectedBandMask 读的是
		// `checkbox.checked` —— 不反射的话「全选」看起来一个都没选。
		if (this.attrs.checked !== undefined && this.attrs.checked !== null)
			this.checked = !!this.attrs.checked;
		if (this.attrs.value !== undefined)
			this.value = this.attrs.value;
	}
	appendChild(child) {
		this.children.push(child);
		if (child && typeof child === 'object') child.parentNode = this;
		return child;
	}
	replaceChildren(...kids) {
		for (const old of this.children)
			if (old && typeof old === 'object' && old.parentNode === this) old.parentNode = null;
		this.children = kids.filter(k => k !== null && k !== undefined);
		for (const child of this.children)
			if (child && typeof child === 'object') child.parentNode = this;
	}
	/* status.js 的 updateRegions() 走的是真实的 DOM 增量替换：按
	 * [data-live-region="…"] 找到旧节点、用 parentNode.replaceChild 换掉。桩必须
	 * 提供同样的入口，否则「详情帧到达后只替换一块区域」这条路径根本跑不到。 */
	removeChild(child) {
		const i = this.children.indexOf(child);
		if (i === -1) return child;
		if (child && typeof child === 'object') child.parentNode = null;
		this.children.splice(i, 1);
		return child;
	}
	replaceChild(next, old) {
		const i = this.children.indexOf(old);
		if (i === -1) return old;
		if (old && typeof old === 'object') old.parentNode = null;
		if (next && typeof next === 'object') next.parentNode = this;
		this.children[i] = next;
		return old;
	}
	querySelectorAll(selector) {
		const found = [];
		collectNodes(this, node => { if (matchesSelector(node, selector)) found.push(node); });
		return found;
	}
	querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
	addEventListener(type, fn) { (this._listeners[type] = this._listeners[type] || []).push(fn); }
	dispatchEvent(event) { return this.fire(event && event.type ? event.type : 'change', event); }
	click() { return this.fire('click', {}); }
	fire(type, event) {
		(this._listeners[type] || []).forEach(fn => fn(event));
		return true;
	}
	setAttribute(k, v) {
		this.attrs[k] = v;
		if (k === 'checked') this.checked = !!v;
		if (k === 'value') this.value = v;
	}
	getAttribute(k) { return this.attrs[k]; }
	closest() { return null; }
	scrollIntoView() {}
	contains(node) {
		if (node === this) return true;
		return this.children.some(ch => ch && ch.contains && ch.contains(node));
	}
}

/* 桩选择器：够 [attr="value"] / tag / .class 三种写法即可（页面只用
 * `[data-live-region="x"]`），不做完整 CSS 解析。 */
function matchesSelector(node, selector) {
	const sel = String(selector || '').trim();
	if (!sel || !node || !node.tagName) return false;
	const attr = /^\[([\w-]+)(?:=("([^"]*)"|'([^']*)'))?\]$/.exec(sel);
	if (attr) {
		const want = attr[3] !== undefined ? attr[3] : attr[4];
		const have = node.attrs ? node.attrs[attr[1]] : undefined;
		return want === undefined ? have !== undefined && have !== null
			: have !== undefined && have !== null && String(have) === want;
	}
	if (sel[0] === '.') return String((node.attrs && node.attrs['class']) || '').split(' ').indexOf(sel.slice(1)) !== -1;
	return node.tagName === sel.toUpperCase();
}
function collectNodes(node, visit) {
	if (node === null || node === undefined || node === false) return;
	if (typeof node !== 'object' || !node.tagName) return;
	visit(node);
	node.children.forEach(ch => collectNodes(ch, visit));
}

class StubEvent {
	constructor(type) { this.type = type; }
}

/* --------------------------------------------------------------- api 桩 */

/*
 * 记录 LuCI 侧的数据调用，并按名字给出应答。
 *   api.at(args)           -> 记录 { kind:'at', args }，应答由 `answers` 决定
 *   api.route(name, p)     -> 记录 { kind:'route', name, params }，永不 reject
 *   api.routeCall(name, p) -> 记录后按 `answers` 里的 { ok:false } / { error } 决定成败
 * `answers` 键：'route:<name>'、'call:<name>'、'at:<首词>'。
 */
function makeApi(answers) {
	const calls = [];
	const answer = (key) => (answers && answers[key] !== undefined) ? answers[key] : null;
	const api = {
		calls: calls,
		route(name, params) {
			calls.push({ kind: 'route', name: name, params: params });
			const a = answer('route:' + name);
			return Promise.resolve(a === null ? null : a);
		},
		routeCall(name, params) {
			calls.push({ kind: 'routeCall', name: name, params: params });
			const a = answer('call:' + name);
			if (a && a.ok === false)
				return Promise.reject(new Error(a.error || 'call failed'));
			return Promise.resolve(a === null ? { results: [ { rat: params && params.rat, applied: true, verified: true } ] } : a);
		},
		at(args) {
			calls.push({ kind: 'at', args: args });
			const a = answer('at:' + (args && args[0]));
			if (a && a.ok === false)
				return Promise.reject(new Error(a.error || 'at failed'));
			return Promise.resolve(a === null ? { stdout: '', stderr: '' } : a);
		},
		atSafe(args) { return api.at(args).catch(err => ({ stdout: '', stderr: String(err.message || err) })); },
		cachedSnapshot() { return Promise.resolve(null); }
	};
	return api;
}

/* ------------------------------------------------------------ 作用域 */

function makeScope(api) {
	const E = (tag, attrs, children) => new El(tag, attrs, children);
	const document = {
		body: new El('body', {}, []),
		head: new El('head', {}, []),
		createElement: tag => new El(tag, {}, []),
		createElementNS: (ns, tag) => new El(tag, {}, []),
		getElementById: () => null,
		importNode: node => node,
		// 文本节点就是节点树里的字符串（textOf/walk 都接受字符串子节点）
		createTextNode: text => String(text),
	};
	// LuCI 给 String.prototype 挂了 format()（页面里到处是 `_('%s / %s').format(a, b)`
	// 和 `'…'.format(x)`），桩必须同样挂上；`_()` 就返回普通字符串，与 LuCI 一致
	// （组件会对标签调 .replace() 等字符串方法）。
	if (typeof String.prototype.format !== 'function') {
		Object.defineProperty(String.prototype, 'format', {
			value: function () {
				let i = 0;
				const args = arguments;
				return String(this).replace(/%[sd]/g, () => String(args[i++]));
			},
			writable: true, configurable: true, enumerable: false
		});
	}
	const t = (s) => String(s);
	const window = {
		pending: [],
		setTimeout: (fn, delay) => {
			if (!delay)
				fn();
			else
				window.pending.push({ fn: fn, delay: delay });
			return window.pending.length;
		},
		clearTimeout: () => {},
		setInterval: () => 0,
		clearInterval: () => {},
		location: { reload() { window.reloaded = true; } },
		reloaded: false
	};
	const modals = [];
	const notifications = [];
	const ui = {
		modals: modals,
		notifications: notifications,
		showModal: (title, children) => { modals.push({ title: String(title), children: children }); return { title: title }; },
		hideModal: () => {},
		addNotification: (node, message, level) => { notifications.push({ level: level || '', text: textOf(message) }); }
	};
	// 页面能用的最小 localStorage：短信页的本地发送历史（含旧键迁移）要用它，
	// 老/新两侧各自持有一份独立存储，比较才不会被对方写入污染。
	const store = new Map();
	window.localStorage = {
		getItem: k => (store.has(String(k)) ? store.get(String(k)) : null),
		setItem: (k, v) => { store.set(String(k), String(v)); },
		removeItem: k => { store.delete(String(k)); },
		clear: () => store.clear()
	};
	const dom = { content: (host, child) => host.replaceChildren(child), append: () => {}, parse: () => null };
	// components.js 在模块求值时注入 <style>，需要 L.resource；概览页的
	// 卡片底部有 `L.url('admin/modem/…')` 的跳转链接，L.url 也要在。
	const L = { resource: p => p, url: p => '/cgi-bin/luci/' + String(p).replace(/^\//, '') };
	return {
		E, document, window, ui, dom, L, modals, notifications,
		localStorage: window.localStorage,
		// components.js / view 里的 `api` 就是要装在作用域里的数据层
		api: api || makeApi({}),
		HTMLElement: El, Event: StubEvent, _: t,
		baseclass: { extend: o => o }, view: { extend: o => o },
		Promise, console, Math, JSON, Number, String, Object, Array, Date, RegExp, Boolean,
		parseFloat, parseInt, isNaN, isFinite, setTimeout: window.setTimeout, clearTimeout: window.clearTimeout
	};
}

function loadModule(source, scope) {
	const names = Object.keys(scope);
	return new Function(...names, source)(...names.map(k => scope[k]));
}

/*
 * 与 LuCI 一样的装载顺序：parser → components（拿到 parser）→ view
 * （拿到 api/parser/components/dom/ui）。`read(rel)` 读一份源码。
 *
 * `globals` 用来补那些「LuCI 框架提供、stub 默认没有」的全局：连接页要
 * `uci` 与 `form`（拨号表单），其它页面暂时不需要。
 */
function loadSide(read, api, viewRel, globals) {
	const RES = 'luci-app-mt5700m/htdocs/luci-static/resources';
	const scope = makeScope(api);
	Object.keys(globals || {}).forEach((key) => { scope[key] = globals[key]; });
	const parser = loadModule(read(RES + '/mt5700m/parser.js'), scope);
	scope.parser = parser;
	scope.c = loadModule(read(RES + '/mt5700m/components.js'), scope);
	const view = loadModule(read(viewRel || RES + '/view/mt5700m/network.js'), scope);
	return { parser, c: scope.c, view, scope, api: scope.api };
}

/* ------------------------------------------------------------ 序列化 */

function textOf(node) {
	if (node === null || node === undefined || node === false) return '';
	if (typeof node === 'string' || typeof node === 'number') return String(node);
	if (Array.isArray(node)) return node.map(textOf).join('');
	if (typeof node !== 'object' || !node.tagName) return String(node);
	return node.children.map(textOf).join('');
}

function attrText(node, skip) {
	const keys = Object.keys(node.attrs || {}).filter(k => typeof node.attrs[k] !== 'function' && (skip || []).indexOf(k) === -1);
	if (!keys.length) return '';
	return '[' + keys.sort().map(k => k + '=' + String(node.attrs[k]).replace(/\d+/g, '#')).join(',') + ']';
}

function walk(node, out, path, opts) {
	if (node === null || node === undefined || node === false) return out;
	if (typeof node === 'string' || typeof node === 'number') {
		out.push(path + ' text=' + String(node).replace(/\d+/g, '#'));
		return out;
	}
	if (Array.isArray(node)) { node.forEach(n => walk(n, out, path, opts)); return out; }
	if (typeof node !== 'object' || !node.tagName) {
		out.push(path + ' text=' + String(node).replace(/\d+/g, '#'));
		return out;
	}
	const cls = node.attrs && node.attrs['class'] ? String(node.attrs['class']) : '';
	const here = path + '/' + node.tagName.toLowerCase()
		+ (cls ? '.' + cls.split(' ').join('.') : '')
		+ (opts.attrs ? attrText(node, opts.skipAttrs) : '');
	if (!opts.keepValues && opts.valueClass && opts.valueClass.test(cls)) {
		out.push(here + ' value=' + (textOf(node) === '' ? 'empty' : 'set'));
		return out;
	}
	out.push(here);
	node.children.forEach(ch => walk(ch, out, here, opts));
	return out;
}

/*
 * 结构 + 文案（数字归一化）。
 *   keepValues  true = 取值槽也逐字比较
 *   valueClass  取值槽的正则（结构比较时只比「有无」）
 *   attrs       true = 把属性也写进比较（skipAttrs 里的除外，如 input 的 value）
 */
function serialize(node, opts) {
	opts = opts || {};
	return walk(node, [], '', {
		keepValues: !!opts.keepValues,
		valueClass: opts.valueClass || null,
		attrs: !!opts.attrs,
		skipAttrs: opts.skipAttrs || []
	}).join('\n');
}
function lines(node) { return walk(node, [], '', { keepValues: true }).join('\n'); }
function countTag(shape, tagClass) {
	return shape.split('\n').filter(l => l === tagClass || l.endsWith(tagClass)).length;
}

function slotEntries(node, valueClass, out) {
	out = out || [];
	if (node === null || node === undefined || node === false) return out;
	if (Array.isArray(node)) { node.forEach(n => slotEntries(n, valueClass, out)); return out; }
	if (typeof node !== 'object' || !node.tagName) return out;
	const cls = String((node.attrs && node.attrs['class']) || '');
	if (valueClass.test(cls)) out.push({ cls: cls, text: textOf(node) });
	node.children.forEach(ch => slotEntries(ch, valueClass, out));
	return out;
}
function slotValues(node, valueClass) { return slotEntries(node, valueClass).map(e => e.text); }
function slotsOf(node, valueClass, cls) {
	return slotEntries(node, valueClass).filter(e => e.cls.split(' ').indexOf(cls) !== -1).map(e => e.text);
}

/* ------------------------------------------------------------ 事件 */

function collect(node, pred, out) {
	out = out || [];
	if (node === null || node === undefined || node === false) return out;
	if (Array.isArray(node)) { node.forEach(n => collect(n, pred, out)); return out; }
	if (typeof node !== 'object' || !node.tagName) return out;
	if (pred(node)) out.push(node);
	node.children.forEach(ch => collect(ch, pred, out));
	return out;
}
function buttons(node) { return collect(node, n => n.tagName === 'BUTTON'); }
/* modal.children 是数组；collect 已支持数组入参，这里只补语义名 */
function selectsIn(nodes) { return collect(nodes, n => n.tagName === 'SELECT'); }
function inputsIn(nodes) { return collect(nodes, n => n.tagName === 'INPUT'); }
/* 最近一次 showModal：弹窗里的控件、标题、按钮都从这里取 */
function lastModal(scope) {
	const modal = scope.ui.modals[scope.ui.modals.length - 1];
	if (!modal) throw new Error('no modal open');
	return modal;
}
/* 按可见文字找按钮（LuCI 里按钮文字就是 E(...) 的子文本） */
function findButton(node, text) {
	return buttons(node).filter(b => textOf(b) === String(text))[0] || null;
}
function pressButton(node, text) {
	const button = findButton(node, text);
	if (!button)
		throw new Error('button not found: ' + text);
	button.click();
	return button;
}
/* 最近一次 showModal 里的按钮 */
function modalButton(scope, text) {
	const modal = scope.ui.modals[scope.ui.modals.length - 1];
	if (!modal) throw new Error('no modal open');
	return pressButton(modal.children, text);
}
function modalTitle(scope) {
	const modal = scope.ui.modals[scope.ui.modals.length - 1];
	return modal ? modal.title : '';
}

/* 跑完微任务，让 routeCall/at 的 Promise 链落地 */
function tick() {
	return new Promise(resolve => setImmediate(resolve));
}

module.exports = {
	El, StubEvent, makeApi, makeScope, loadModule, loadSide,
	serialize, lines, countTag, textOf,
	slotEntries, slotValues, slotsOf,
	collect, buttons, findButton, pressButton,
	selectsIn, inputsIn, lastModal,
	modalButton, modalTitle, tick
};
