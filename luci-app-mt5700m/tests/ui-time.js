/*
 * Executable UI checks for the LuCI layer: unified tabs, modal/button helpers
 * and the Time & DNS page. Loads the real view/component sources in a vm with
 * a minimal DOM, so a syntax error or a broken lifecycle path fails here.
 */
'use strict';

const fs = require('fs');
const vm = require('vm');
const path = require('path');

const ROOT = path.join(__dirname, '..', 'htdocs', 'luci-static', 'resources');
let pass = 0, fail = 0;
function check(name, cond, extra) {
	if (cond) { pass++; console.log('ok   ' + name); }
	else { fail++; console.log('FAIL ' + name + (extra ? ' :: ' + extra : '')); }
}

/* --------------------------------------------------------------- mini DOM */
class ClassList {
	constructor(node) { this.node = node; }
	_tokens() { return String(this.node.attrs.class || '').split(/\s+/).filter(Boolean); }
	_set(tokens) { this.node.attrs.class = tokens.join(' '); }
	add(name) { const t = this._tokens(); if (!t.includes(name)) { t.push(name); this._set(t); } }
	remove(name) { this._set(this._tokens().filter(x => x !== name)); }
	contains(name) { return this._tokens().includes(name); }
	toggle(name, on) {
		if (on === undefined) on = !this.contains(name);
		if (on) this.add(name); else this.remove(name);
		return on;
	}
}

class FakeNode {
	constructor(tag) {
		this.tagName = String(tag || 'div');
		this.nodeType = 1;
		this.children = [];
		this.attrs = {};
		this.listeners = {};
		this.style = {};
		this.parentNode = null;
		this.classList = new ClassList(this);
		this.hidden = false;
		this.disabled = false;
		this.checked = false;
		this.value = '';
		this.isConnected = true;
		this.offsetLeft = 0;
		this.offsetWidth = 80;
		this.clientWidth = 600;
	}
	appendChild(c) {
		if (c && c.nodeType) { c.parentNode = this; this.children.push(c); }
		else if (c != null) this.children.push({ text: String(c) });
		return c;
	}
	replaceChildren() {
		this.children = [];
		for (const c of arguments) this.appendChild(c);
	}
	replaceWith(n) {
		if (!this.parentNode) return;
		const i = this.parentNode.children.indexOf(this);
		if (i >= 0) {
			n.parentNode = this.parentNode;
			this.parentNode.children[i] = n;
		}
	}
	setAttribute(k, v) {
		this.attrs[k] = v;
		if (k === 'class') this.attrs.class = v;
		if (k === 'disabled') this.disabled = true;
	}
	getAttribute(k) { return this.attrs[k]; }
	removeAttribute(k) { delete this.attrs[k]; if (k === 'disabled') this.disabled = false; }
	addEventListener(type, fn) {
		(this.listeners[type] = this.listeners[type] || []).push(fn);
	}
	removeEventListener(type, fn) {
		this.listeners[type] = (this.listeners[type] || []).filter(x => x !== fn);
	}
	dispatchEvent(ev) {
		(this.listeners[ev.type] || []).forEach(fn => fn.call(this, ev));
		return true;
	}
	querySelector(sel) {
		const cls = String(sel).split('.').filter(Boolean);
		const walk = (n) => {
			for (const c of n.children || []) {
				if (!c.classList) continue;
				if (cls.every(x => c.classList.contains(x))) return c;
				const hit = walk(c);
				if (hit) return hit;
			}
			return null;
		};
		return walk(this);
	}
	get textContent() {
		return (this.children || []).map(c => c.text != null ? c.text : (c.textContent || '')).join('');
	}
	set textContent(v) { this.children = [{ text: String(v) }]; }
	get firstElementChild() {
		return (this.children || []).find(c => c.nodeType === 1) || null;
	}
	set innerHTML(markup) {
		const tag = /^\s*<([a-zA-Z0-9]+)/.exec(markup);
		this.children = [new FakeNode(tag ? tag[1] : 'div')];
	}
	get innerHTML() { return ''; }
}

const documentStub = {
	createElement: (t) => new FakeNode(t),
	createElementNS: (ns, t) => new FakeNode(t),
	importNode: (n) => n,
	getElementById: () => null,
	head: new FakeNode('head'),
	body: new FakeNode('body'),
	hidden: false,
	addEventListener() {}, removeEventListener() {}
};

function E(tag, attrs) {
	const node = new FakeNode(tag);
	for (const k of Object.keys(attrs || {})) {
		if (k === 'class') { node.attrs.class = attrs[k]; if (attrs[k]) attrs[k].split(/\s+/).forEach(x => x && node.classList.add(x)); }
		else if (typeof attrs[k] === 'function') node.addEventListener(k.replace(/^on/, ''), attrs[k]);
		else if (k === 'disabled') { node.disabled = !!attrs[k]; node.attrs[k] = attrs[k]; }
		else if (k === 'checked') { node.checked = !!attrs[k]; }
		else if (k === 'value') { node.value = attrs[k]; node.attrs[k] = attrs[k]; }
		else if (k === 'style' && typeof attrs[k] === 'string') {
			attrs[k].split(';').forEach(p => { const i = p.indexOf(':'); if (i > 0) node.style[p.slice(0, i).trim()] = p.slice(i + 1).trim(); });
		} else node.attrs[k] = attrs[k];
	}
	for (let i = 2; i < arguments.length; i++) addChild(node, arguments[i]);
	return node;
}
function addChild(node, c) {
	if (c == null || c === false || c === true) return;
	if (Array.isArray(c)) { c.forEach(x => addChild(node, x)); return; }
	if (typeof c === 'object' && c.nodeType === 1) node.appendChild(c);
	else node.appendChild(c);
}

const notifications = [];
const modals = [];
const ui = {
	showModal(title, children) {
		const m = new FakeNode('div');
		m.title = title;
		(children || []).forEach(c => addChild(m, c));
		modals.push(m);
		return m;
	},
	hideModal() { ui.hidden = (ui.hidden || 0) + 1; },
	addNotification(n, node, kind) { notifications.push({ node, kind }); }
};
ui.hidden = 0;

const urlState = { href: 'http://router/cgi-bin/luci/admin/modem/mt5700m/time?timeTab=dns' };
const windowStub = {
	location: urlState,
	history: {
		pushState(state, title, url) {
			urlState.href = String(url);
			const q = urlState.href.indexOf('?');
			urlState.search = q >= 0 ? urlState.href.slice(q) : '';
		}
	},
	setTimeout(fn) { try { fn(); } catch (e) {} return 0; },
	clearTimeout() {},
	setInterval() { return 1; },
	clearInterval() {},
	addEventListener() {},
	removeEventListener() {}
};
Object.defineProperty(urlState, 'search', {
	get() { const q = this.href.indexOf('?'); return q >= 0 ? this.href.slice(q) : ''; },
	configurable: true
});

const L = { url: (p) => '/cgi-bin/luci/' + p, resource: (p) => '/luci-static/resources/' + p };

function sandboxBase(extra) {
	const sb = {
		E, ui, L, console,
		document: documentStub,
		window: windowStub,
		HTMLElement: FakeNode,
		Node: FakeNode,
		DOMParser: undefined,
		URL, URLSearchParams,
		_: (s) => s,
		baseclass: { extend: (o) => o },
		view: { extend: (o) => o },
		AbortController: function () {
			this.signal = { aborted: false, addEventListener() {}, removeEventListener() {} };
			this.abort = function () { this.signal.aborted = true; };
		},
		setTimeout: windowStub.setTimeout,
		clearTimeout() {},
		encodeURIComponent,
		...extra
	};
	return sb;
}

function loadModule(file, extra) {
	let src = fs.readFileSync(file, 'utf8');
	const aliases = [];
	src = src.replace(/^\s*'require\s+([^']+?)\s+as\s+([A-Za-z_$][\w$]*)';[^\n]*$/gm,
		(m, mod, name) => { aliases.push([mod, name]); return ''; });
	src = src.replace(/^\s*'require\s+[^']+';[^\n]*$/gm, '');
	src = src.replace(/return\s+view\.extend\(/, 'view.extend(');
	src = src.replace(/return\s+baseclass\.extend\(/, 'baseclass.extend(');
	const sb = sandboxBase(extra || {});
	for (const [mod, name] of aliases) {
		if (!(name in sb)) throw new Error('no stub for alias ' + name + ' (' + mod + ')');
	}
	vm.createContext(sb);
	vm.runInContext("String.prototype.format=function(){var a=[this].concat([].slice.call(arguments));var i=0;return (''+a[0]).replace(/%[sd]/g,function(){i++;return i<a.length?a[i]:'';});};", sb);
	return vm.runInContext(src, sb, { filename: file });
}

/* ------------------------------------------------------------ load sources */
const components = loadModule(path.join(ROOT, 'mt5700m/components.js'), {
	api: { at: () => Promise.resolve({ stdout: '' }), route: () => Promise.resolve(null) },
	parser: {}
});
check('components module exports navTabs', typeof components.navTabs === 'function');

const calls = [];
const timeStatus = {
	trusted: true,
	verified: true,
	pending: false,
	applying: false,
	syncing: false,
	saved: {
		enabled: true,
		servers: [
			{ host: 'ntp.tencent.com', enabled: true },
			{ host: 'time.cloudflare.com', enabled: true },
			{ host: 'ntp.aliyun.com', enabled: false }
		]
	},
	defaults: { enabled: true, servers: [] },
	effectiveServers: ['ntp.tencent.com', 'time.cloudflare.com'],
	provider: { name: 'sysntpd', running: true, configurable: true },
	time: { unix: 1790000000, unixMs: 1790000000000, local: '2026-09-21 12:00:00', timezone: 'CST-8', plausible: true },
	state: {
		synced: true, offsetMs: 12, server: 'ntp.tencent.com',
		lastSuccessText: '2026-09-21 11:55:00', lastAttemptText: '2026-09-21 11:55:00',
		lastError: null, lastErrorKind: null, failures: 0, nextRetryAt: null,
		apply: { result: 'ok', atText: '2026-09-21 11:00:00', error: null }
	},
	warnings: []
};
const dnsStatus = {
	applying: false,
	saved: {
		ipv4: { enabled: true, primary: '119.29.29.29', secondary: '223.5.5.5' },
		ipv6: { enabled: true, primary: '2402:4e00::', secondary: '2400:3200::1' }
	},
	defaults: {
		ipv4: { enabled: true, primary: '119.29.29.29', secondary: '223.5.5.5' },
		ipv6: { enabled: true, primary: '2402:4e00::', secondary: '2400:3200::1' }
	},
	state: { result: 'ok', appliedAtText: '2026-09-21 11:00:00', error: null, applied: { ipv4: ['119.29.29.29'], ipv6: ['2402:4e00::'] } },
	runtime: {
		ipv4: { name: 'MT5700M', configured: true, up: true, dns: ['119.29.29.29', '223.5.5.5'], matchesSaved: true, l3Device: 'wwan0' },
		ipv6: { name: 'MT5700Mv6', configured: true, up: true, dns: ['2402:4e00::', '2400:3200::1'], matchesSaved: true, l3Device: 'wwan0' }
	},
	verified: { ipv4: true, ipv6: true },
	pending: { ipv4: false, ipv6: false },
	interfaces: { ipv4: 'MT5700M', ipv6: 'MT5700Mv6' },
	warnings: []
};
const api = {
	route: (name) => {
		calls.push(name);
		if (name === 'time.status') return Promise.resolve(timeStatus);
		if (name === 'dns.status') return Promise.resolve(dnsStatus);
		if (name === 'task.get') return Promise.resolve({ task: { status: 'completed', result: {} } });
		return Promise.resolve(null);
	},
	routeCall: (name) => {
		calls.push(name);
		if (name === 'dns.config_set' || name === 'time.config_set') return Promise.resolve({});
		if (name === 'time.apply' || name === 'dns.apply') return Promise.resolve({ task_id: 7 });
		return Promise.resolve({});
	},
	waitTask: () => Promise.resolve({ status: 'completed', result: { synced: true } }),
	at: () => Promise.resolve({ stdout: '' })
};

const view = loadModule(path.join(ROOT, 'view/mt5700m/time.js'), {
	api: api, c: components
});
check('time view is an object with load()', view && typeof view.load === 'function');

/* --------------------------------------------------------------- tab checks */
const nav = components.navTabs('time');
const links = [];
(function walk(n) {
	if (n.classList && n.classList.contains('mt-tab')) links.push(n);
	(n.children || []).forEach(walk);
})(nav);
check('nav has 9 tabs', links.length === 9, 'got ' + links.length);
check('time tab is active', links.some(a => a.classList.contains('is-active') && a.attrs['data-tab'] === 'time'));
check('each tab has non-empty href', links.every(a => typeof a.attrs.href === 'string' && a.attrs.href.indexOf('admin/modem/mt5700m/') >= 0));
check('tab icons are svg', links.every(a => {
	const icon = (a.children || []).find(c => c.classList && c.classList.contains('mt-tab-icon'));
	return icon && icon.firstElementChild && icon.firstElementChild.tagName === 'svg';
}));

/* --------------------------------------------------------------- page render */
const instance = Object.assign(Object.create(view), view);
instance.holders = [];
(async function main() {
const page = instance.load();
await new Promise(r => setImmediate(r));
await new Promise(r => setImmediate(r));
check('load() returns a page node', page && page.nodeType === 1);
check('load() starts from cache routes only', calls.indexOf('time.status') >= 0 && calls.indexOf('dns.status') >= 0);
check('no AT command is issued by the page', !calls.some(c => /^command\s/.test(c)));

const all = [];
(function walk(n) { if (n.classList) all.push(n); (n.children || []).forEach(walk); })(page);
check('page includes unified tab bar', all.some(n => n.classList.contains('mt-tabs')));
check('page includes inner time tab bar', all.some(n => n.classList.contains('mt-tabs--inset')));
const ntpPanel = all.find(n => n.classList.contains('mt-time-panels') === false && n.hidden === true && n.children.length > 1);
check('URL-selected DNS inner tab is visible; NTP panel hidden', !!ntpPanel);
check('DNS panel is not hidden', all.some(n => n.classList.contains('mt-dns-family') && n.hidden === false));
check('all three NTP servers are listed', all.filter(n => n.classList.contains('mt-ntp-row')).length === 3,
	'got ' + all.filter(n => n.classList.contains('mt-ntp-row')).length);
check('DNS note reflects both families', all.some(n => n.classList.contains('mt-dns-note') && /IPv4 \+ IPv6/.test(n.textContent)));

/* ------------------------------------------- validation and state retention */
instance.dnsModel.ipv4.primary = '999.1.1.1';
const err = instance.validateDns();
check('invalid IPv4 is rejected before submit', err && err.family === 'ipv4' && err.field === 'primary');
instance.dnsModel.ipv4.primary = '';
const err2 = instance.validateDns();
check('enabled family requires a primary', err2 && /primary DNS/i.test(err2.message));
instance.dnsModel.ipv4.primary = '119.29.29.29';
check('valid DNS config passes validation', instance.validateDns() === null);

const dnsInputs = all.filter(n => n.tagName === 'input' && n.attrs['aria-label'] === 'IPv4 DNS Primary');
check('IPv4 primary input rendered', dnsInputs.length === 1);
if (dnsInputs.length) {
	dnsInputs[0].value = '1.2.3.4';
	dnsInputs[0].dispatchEvent({ type: 'input' });
	check('typing updates the model without a request', instance.dnsModel.ipv4.primary === '1.2.3.4' && instance.dnsDirty === true);
	const dnsTab = all.find(n => n.attrs && n.attrs['data-tab'] === 'dns' && n.classList && n.classList.contains('mt-tab'));
	const ntpTab = all.find(n => n.attrs && n.attrs['data-tab'] === 'ntp' && n.classList && n.classList.contains('mt-tab'));
	ntpTab.dispatchEvent({ type: 'click' });
	dnsTab.dispatchEvent({ type: 'click' });
	check('switching inner tabs keeps unsaved input', dnsInputs[0].value === '1.2.3.4' && instance.dnsModel.ipv4.primary === '1.2.3.4');
	check('tab switch persists timeTab in URL', /[?&]timeTab=dns/.test(urlState.href));
}

/* --------------------------------------------------------------- lifecycle */
instance.stopPolling = function () { this.pollStopped = true; };
instance.abort = { signal: { aborted: false }, abort() { this.signal.aborted = true; } };
instance.onPageHide();
check('pagehide stops polling', instance.pollStopped === true);
check('pagehide aborts task observation', instance.abort.signal.aborted === true);

console.log('\n' + pass + ' passed, ' + fail + ' failed');
process.exit(fail ? 1 : 0);
})();
