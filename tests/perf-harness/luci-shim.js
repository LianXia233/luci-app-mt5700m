/*
 * Mini LuCI loader + framework shims for the perf harness.
 *
 * Replicates openwrt/luci luci.js's module loading schedule:
 *   fetch the view file → scan its leading quoted `require ...` directives →
 *   fetch all dependencies of a level in parallel → eval each module wrapped
 *   in (window, document, L, ...deps) → instantiate.
 * The require-scanner state machine is adapted from luci.js (Apache-2.0).
 *
 * Framework modules (view / baseclass / ui / rpc / fs) are provided as local
 * shims — they are identical for both variants, so they cancel out of the
 * before/after comparison. Only the mt5700m application sources are fetched
 * from the variant trees under measurement.
 *
 * Progress marks (performance.now(), ms since navigation start):
 *   modulesReady   view + app module graph evaluated
 *   loadReturned   view.load() resolved (progressive views: immediately)
 *   shellReady     first view DOM mounted (skeleton or full page)
 *   contentReady   data-filled DOM mounted (== shellReady for blocking views)
 *   styleInject    <link mt5700m/style.css> first appears
 *   styleLoaded    stylesheet load event
 *   frontendReady  max(shellReady, styleLoaded) — styled shell visible
 */
'use strict';

(function () {
	var base = '';
	var marks = {};

	function mark(name) {
		if (marks[name] == null)
			marks[name] = Math.round(performance.now() * 10) / 10;
	}

	/* ---------------- globals expected by LuCI modules ---------------- */

	window.L = {
		resource: function (p) { return base + '/luci-static/resources/' + p; },
		url: function () { return '#'; },
		bind: function (fn, ctx) { return fn.bind(ctx); },
		env: {}
	};

	window._ = function (s) { return s; };

	if (!String.prototype.format) {
		String.prototype.format = function () {
			var args = Array.prototype.slice.call(arguments), i = 0;
			return this.replace(/%s/g, function () {
				return i < args.length ? args[i++] : '%s';
			});
		};
	}

	function isNode(v) {
		return v && typeof v === 'object' && (v.nodeType === 1 || v.nodeType === 11);
	}

	function append(parent, child) {
		if (child == null || child === false)
			return;
		if (Array.isArray(child)) {
			child.forEach(function (c) { append(parent, c); });
			return;
		}
		if (typeof child === 'string' || typeof child === 'number') {
			parent.appendChild(document.createTextNode(String(child)));
			return;
		}
		parent.appendChild(child);
	}

	window.E = function (tag, attrs, children) {
		if (attrs != null && (typeof attrs !== 'object' || Array.isArray(attrs) || isNode(attrs))) {
			children = attrs;
			attrs = null;
		}
		var el = document.createElement(tag);
		if (attrs) {
			Object.keys(attrs).forEach(function (k) {
				var v = attrs[k];
				if (v == null)
					return;
				if (typeof v === 'function')
					el.addEventListener(k, v);
				else if (k === 'class')
					el.className = String(v);
				else
					el.setAttribute(k, String(v));
			});
		}
		append(el, children);
		return el;
	};

	/* ---------------- stylesheet timing watch ---------------- */

	var styleSeen = false;

	function considerStyle() {
		if (styleSeen)
			return;
		var link = document.querySelector('link[href*="mt5700m/style.css"]');
		if (!link)
			return;
		styleSeen = true;
		mark('styleInject');
		if (link.sheet)
			mark('styleLoaded');
		else {
			link.addEventListener('load', function () { mark('styleLoaded'); maybeDone(); });
			link.addEventListener('error', function () { mark('styleError'); maybeDone(); });
		}
	}

	if (window.MutationObserver) {
		new MutationObserver(considerStyle)
			.observe(document.documentElement, { childList: true, subtree: true });
	}

	/* ---------------- framework shims (identical for both variants) ---- */

	var SHIMS = {
		view: { extend: function (o) { return o; } },
		baseclass: { extend: function (o) { return o; } },
		dom: {},
		poll: { add: function () {}, remove: function () {} },
		ui: {
			addNotification: function () {},
			createNotification: function () { return function () {}; },
			showModal: function () {},
			hideModal: function () {},
			assert: false
		},
		uci: {
			load: function () { return Promise.resolve(); },
			get: function () { return null; },
			set: function () {},
			save: function () {},
			changes: function () { return []; }
		},
		form: { Map: function () {} },
		rpc: {
			declare: function (opts) {
				return function () {
					var params = {};
					(opts.params || []).forEach(function (name, i) {
						params[name] = arguments[i];
					});
					return fetch(base + '/mock/rpc', {
						method: 'POST',
						headers: { 'content-type': 'application/json' },
						body: JSON.stringify({ object: opts.object, method: opts.method, params: params })
					}).then(function (r) { return r.json(); });
				};
			}
		},
		fs: {
			exec: function (bin, args) {
				return fetch(base + '/mock/fs', {
					method: 'POST',
					headers: { 'content-type': 'application/json' },
					body: JSON.stringify({ bin: bin, args: args || [] })
				}).then(function (r) { return r.json(); });
			}
		}
	};

	/* ---------------- module loader (luci.js schedule) ---------------- */

	var moduleCache = {};

	function fetchText(url) {
		return fetch(url).then(function (r) {
			if (!r.ok)
				throw new Error(url + ' -> HTTP ' + r.status);
			return r.text();
		});
	}

	function parseRequires(source) {
		var requirematch = /^require[ \t]+(\S+)(?:[ \t]+as[ \t]+([a-zA-Z_]\S*))?$/;
		var strictmatch = /^use[ \t]+strict$/;
		var depends = [], args = '';
		var i, off = -1, prev = -1, quote = -1, comment = -1, esc = false, chr, s, m;

		for (i = 0; i < source.length; i++) {
			chr = source.charCodeAt(i);
			if (esc)
				esc = false;
			else if (comment != -1) {
				if ((comment == 47 && chr == 10) || (comment == 42 && prev == 42 && chr == 47))
					comment = -1;
			}
			else if ((chr == 42 || chr == 47) && prev == 47)
				comment = chr;
			else if (chr == 92)
				esc = true;
			else if (chr == quote) {
				s = source.substring(off, i);
				m = requirematch.exec(s);
				if (m) {
					depends.push(m[1]);
					args += ', ' + (m[2] || m[1].replace(/[^a-zA-Z0-9_]/g, '_'));
				}
				else if (!strictmatch.exec(s))
					break;
				off = -1;
				quote = -1;
			}
			else if (quote == -1 && (chr == 34 || chr == 39)) {
				off = i + 1;
				quote = chr;
			}
			prev = chr;
		}
		return { depends: depends, args: args };
	}

	function loadModule(name) {
		if (SHIMS[name])
			return Promise.resolve(SHIMS[name]);
		if (moduleCache[name])
			return moduleCache[name];

		var url = L.resource(name.replace(/\./g, '/') + '.js');
		var p = fetchText(url).then(function (source) {
			var req = parseRequires(source);
			return Promise.all(req.depends.map(loadModule)).then(function (instances) {
				var factory = eval(
					'(function(window, document, L' + req.args + ') {\n' + source +
					'\n})\n//# sourceURL=' + url + '\n'
				);
				return factory.apply(factory, [window, document, L].concat(instances));
			});
		});
		moduleCache[name] = p;
		return p;
	}

	/* ---------------- metrics & completion ---------------- */

	function round(n) {
		return n == null ? null : Math.round(n * 10) / 10;
	}

	function collectResources() {
		var ents = performance.getEntriesByType('resource');
		var bytes = 0;
		ents.forEach(function (e) {
			bytes += e.encodedBodySize || e.transferSize || 0;
		});
		return { requests: ents.length, bytes: bytes };
	}

	function maybeDone() {
		var perf = window.__perf;
		if (!perf || perf.done)
			return;
		if (marks.shellReady == null || marks.contentReady == null)
			return;
		if (marks.styleLoaded == null && marks.styleError == null)
			return;

		var res = collectResources();
		var fcp = performance.getEntriesByName('first-contentful-paint')[0];

		perf.modulesReady = round(marks.modulesReady);
		perf.loadReturned = round(marks.loadReturned);
		perf.shellReady = round(marks.shellReady);
		perf.contentReady = round(marks.contentReady);
		perf.styleInject = round(marks.styleInject);
		perf.styleLoaded = round(marks.styleLoaded);
		perf.frontendReady = round(Math.max(marks.shellReady, marks.styleLoaded || 0));
		perf.fullReady = round(Math.max(marks.contentReady, marks.styleLoaded || 0));
		perf.tFcp = fcp ? round(fcp.startTime) : null;
		perf.styleAfterShell = round((marks.styleLoaded || 0) - marks.shellReady);
		perf.requests = res.requests;
		perf.bytes = res.bytes;
		perf.done = true;
	}

	/* safety: never leave the runner hanging if something went wrong */
	setTimeout(function () {
		if (marks.styleLoaded == null && marks.styleError == null)
			mark('styleError');
		if (marks.contentReady == null && marks.shellReady != null)
			mark('contentReady');
		maybeDone();
	}, 8000);

	/* ---------------- entry point ---------------- */

	window.__harnessStart = function (variant, viewPath) {
		base = '/' + variant;
		window.__perf = { variant: variant, done: false };

		loadModule(viewPath).then(function (viewObj) {
			mark('modulesReady');
			return Promise.resolve(viewObj.load()).then(function (loadRes) {
				mark('loadReturned');
				var node = viewObj.render(loadRes);
				document.getElementById('content').replaceChildren(node);
				mark('shellReady');

				var fill = (viewObj.contentReady && typeof viewObj.contentReady.then === 'function')
					? viewObj.contentReady
					: Promise.resolve(loadRes);
				return fill;
			}).then(function () {
				mark('contentReady');
				maybeDone();
			});
		}).catch(function (err) {
			window.__perf.error = String((err && err.stack) || err);
			window.__perf.done = true;
			document.getElementById('content').textContent =
				'HARNESS ERROR: ' + ((err && err.message) || err);
		});

		/* stylesheet can arrive before or after content; poll both edges */
		var styleTimer = setInterval(function () {
			considerStyle();
			if (marks.styleLoaded != null || marks.styleError != null)
				clearInterval(styleTimer);
		}, 10);
	};
})();
