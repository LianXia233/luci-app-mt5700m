#!/usr/bin/env node
/*
 * Perf harness server for the luci-app-mt5700m LuCI frontend.
 *
 * Serves two variants of the frontend — "before" (pristine git HEAD) and
 * "after" (optimized + minified) — under /before/... and /after/..., applies
 * configurable router-like latencies to every response, and mocks the ubus
 * (rpc) and fs.exec (AT) data endpoints with realistic delays.
 *
 * Usage:
 *   node server.js                       # http://127.0.0.1:8901
 *   BEFORE_DIR=... AFTER_DIR=... PORT=... node server.js
 *
 * Latency model (ms, override via env):
 *   LAT_STATIC=6     per static file (uhttpd round trip + send)
 *   LAT_RPC_STATUS=30 / LAT_RPC_TRAFFIC=25 / LAT_RPC_DEVICE=20
 *   LAT_FS_STATUS=150 / LAT_FS_SESSION=180   (AT over UART dominates)
 * All responses carry Cache-Control: no-store so every measured run is a
 * cold first visit (the scenario under optimization).
 */
'use strict';

const http = require('http');
const fs = require('fs');
const path = require('path');

const PORT = Number(process.env.PORT || 8901);
const ROOT = __dirname;
const VARIANTS = {
	before: process.env.BEFORE_DIR || path.resolve(ROOT, '../../../_analysis/before'),
	after: process.env.AFTER_DIR || path.resolve(ROOT, '../../../_analysis/after')
};

const LAT = {
	static: Number(process.env.LAT_STATIC || 6),
	rpc: {
		'mt5700m.status': Number(process.env.LAT_RPC_STATUS || 30),
		'mt5700m-traffic.summary': Number(process.env.LAT_RPC_TRAFFIC || 25),
		'network.device.status': Number(process.env.LAT_RPC_DEVICE || 20)
	},
	rpcDefault: Number(process.env.LAT_RPC_DEFAULT || 25),
	fs: {
		'status': Number(process.env.LAT_FS_STATUS || 150),
		'advanced session': Number(process.env.LAT_FS_SESSION || 180)
	},
	fsDefault: Number(process.env.LAT_FS_DEFAULT || 120)
};

/* Plausible mock payloads (shape matters, values only feed the render). */
const RPC_DATA = {
	'mt5700m.status': {
		at_port: '/dev/ttyUSB2', network: 'eth2', connected: true,
		network_mode: 'NR SA', version: '2.5.0'
	},
	'mt5700m-traffic.summary': {
		interfaces: [{ name: 'eth2', rx: 987654321, tx: 123456789 }]
	},
	'network.device.status': { up: true, carrier: true, speed: '1000baseT' }
};

const AT_STDOUT = {};
AT_STDOUT['status'] = [
	'connected=1', 'sysmode=NR', 'operator=China Mobile',
	'rsrp=-85', 'rsrq=-11', 'sinr=17', 'temperature=42.5',
	'carrier_count=1', 'carrier_1=NR|n41|5200|5200|100||5200|100',
	'usb_state=normal', 'network_mode=NR SA'
].join('\n') + '\n';

const sleep = ms => new Promise(r => setTimeout(r, ms));

const MIME = {
	'.html': 'text/html; charset=utf-8',
	'.js': 'application/javascript; charset=utf-8',
	'.css': 'text/css; charset=utf-8',
	'.json': 'application/json; charset=utf-8'
};

function send(res, code, type, body, delay) {
	Promise.resolve(delay).then(ms => sleep(ms)).then(() => {
		res.writeHead(code, {
			'Content-Type': type || 'application/octet-stream',
			'Cache-Control': 'no-store',
			'Content-Length': Buffer.byteLength(body)
		});
		res.end(body);
	});
}

function readBody(req) {
	return new Promise(resolve => {
		let data = '';
		req.on('data', c => { data += c; });
		req.on('end', () => resolve(data));
	});
}

const server = http.createServer(async (req, res) => {
	const url = new URL(req.url, 'http://x');
	const parts = url.pathname.replace(/^\/+/, '').split('/');
	const variant = parts[0];

	if (req.method === 'POST' && (variant === 'before' || variant === 'after')) {
		const body = JSON.parse((await readBody(req)) || '{}');
		if (parts[1] === 'mock' && parts[2] === 'rpc') {
			const key = body.object + '.' + body.method;
			const delay = LAT.rpc[key] != null ? LAT.rpc[key] : LAT.rpcDefault;
			return send(res, 200, 'application/json',
				JSON.stringify(RPC_DATA[key] || {}), delay);
		}
		if (parts[1] === 'mock' && parts[2] === 'fs') {
			const key = (body.args || []).join(' ');
			const delay = LAT.fs[key] != null ? LAT.fs[key] : LAT.fsDefault;
			return send(res, 200, 'application/json',
				JSON.stringify({ stdout: AT_STDOUT[key] || '', stderr: '', code: 0 }),
				delay);
		}
		return send(res, 404, 'text/plain', 'not found', 0);
	}

	if (req.method !== 'GET')
		return send(res, 405, 'text/plain', 'method not allowed', 0);

	/* the loader shim itself lives next to this script, not in the trees */
	if (req.method === 'GET' && parts.length === 2 && parts[1] === 'luci-shim.js') {
		const body = fs.readFileSync(path.join(ROOT, 'luci-shim.js'), 'utf8');
		return send(res, 200, MIME['.js'], body, LAT.static);
	}

	/* /before/  /before/page.html  → harness page */
	if (parts.length <= 2 && (parts[1] === undefined || parts[1] === '' || parts[1] === 'page.html')) {
		const html = fs.readFileSync(path.join(ROOT, 'page.html'), 'utf8')
			.replace(/__VARIANT__/g, variant);
		return send(res, 200, MIME['.html'], html, LAT.static);
	}
	/* /before/luci-static/... → variant tree */
	if (!VARIANTS[variant])
		return send(res, 404, 'text/plain', 'unknown variant', 0);

	const file = path.join(VARIANTS[variant], parts.slice(1).join('/'));
	if (!file.startsWith(VARIANTS[variant]))
		return send(res, 403, 'text/plain', 'forbidden', 0);

	fs.readFile(file, (err, buf) => {
		if (err)
			return send(res, 404, 'text/plain', 'not found: ' + parts.slice(1).join('/'), 0);
		send(res, 200, MIME[path.extname(file)] || 'application/octet-stream',
			buf.toString('utf8'), LAT.static);
	});
});

server.listen(PORT, '127.0.0.1', () => {
	console.log(`perf harness: http://127.0.0.1:${PORT}/before/  and  /after/`);
	console.log('latency model:', JSON.stringify(LAT));
});
