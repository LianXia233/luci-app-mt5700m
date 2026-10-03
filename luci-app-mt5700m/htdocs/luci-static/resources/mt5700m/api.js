'use strict';
'require baseclass';
'require rpc';
'require fs';

/*
 * MT5700M LuCI — api.js
 * ---------------------------------------------
 * 数据通道统一封装层：
 *   - rpc.declare：mt5700m status/log/connect/disconnect/redial、
 *     mt5700m-traffic summary、network.device status
 *   - fs.exec('/usr/sbin/mt5700m-at', args)：AT 子命令封装
 * 页面代码一律经本模块调用，不再各自 declare / 各自写 catch 归一化。
 *
 * 超时兜底（AT_TIMEOUT_MS）：
 *   rpcd 的 fs.exec 没有超时概念 —— 后端 AT 命令一旦挂起（例如串口被
 *   AT 守护进程独占），Promise 永不 settle，页面会永久停在骨架屏。
 *   这里对每个 AT 调用套一层 Promise.race 硬超时：
 *     - at()     超时按 reject 处理（send / terminal 需要区分成败）；
 *     - atSafe() 超时被 catch 归一化为 { stdout:'', stderr:message }；
 *   两者都保证 Promise 一定 settle，页面必然完成渲染。
 */

/* ---------- rpcd 声明 ---------- */

var callManagerStatus = rpc.declare({ object: 'mt5700m', method: 'status', expect: { } });
var callDialLog = rpc.declare({ object: 'mt5700m', method: 'log', expect: { } });
var callDial = rpc.declare({ object: 'mt5700m', method: 'connect', expect: { } });
var callHang = rpc.declare({ object: 'mt5700m', method: 'disconnect', expect: { } });
var callRedial = rpc.declare({ object: 'mt5700m', method: 'redial', expect: { } });
var callTraffic = rpc.declare({ object: 'mt5700m-traffic', method: 'summary', expect: { } });
var callDeviceStatus = rpc.declare({ object: 'network.device', method: 'status', params: [ 'name' ], expect: { } });

var AT_BIN = '/usr/sbin/mt5700m-at';

/*
 * 单个 AT 调用的硬超时（毫秒）。
 * 取值依据：守护进程侧预算 = queued_timeout(10s) + timeout(8s) + 2s = 20s，
 * 前端不应比后端更久地空等；15s 足以覆盖正常一帧（数十条 AT 的聚合命令
 * 走共享控制通道通常 < 3s），又能在挂起时及时让路给渲染。
 */
var AT_TIMEOUT_MS = 15000;

/*
 * 给任意 Promise 套一层硬超时。超时的 Promise 永不 settle 时，
 * 由计时器接管并 reject，调用方即可继续渲染。
 */
function deadline(promise, ms, label) {
	var timer = null;
	var guarded = new Promise(function(resolve, reject) {
		timer = setTimeout(function() {
			reject(new Error(_('AT command timed out after %s ms').format(ms) + ' (' + label + ')'));
		}, ms);
		promise.then(function(v) {
			clearTimeout(timer);
			resolve(v);
		}, function(e) {
			clearTimeout(timer);
			reject(e);
		});
	});
	return guarded;
}

/* ---------- fs.exec 封装 ---------- */

// 原始调用：reject 时携带错误（send / terminal 等需要区分成败的场景）
function at(args) {
	return deadline(fs.exec(AT_BIN, args), AT_TIMEOUT_MS, 'mt5700m-at ' + args.join(' '));
}

// 归一化调用：永不 reject，失败返回 { stdout:'', stderr: message }
function atSafe(args) {
	return at(args).catch(function(err) {
		return { stdout: '', stderr: err.message || String(err) };
	});
}

/*
 * cachedSnapshot —— Async Architecture 缓存优先（SWR）快照。
 * 走 mt5700m-at cached 子命令（本地控制 socket → daemon StateCache）：
 * 零 AT 流量、毫秒级返回，即使模组离线 / AT 卡住也能立即拿到最近一次
 * 后台采集器写入的状态。永不 reject：daemon 未运行或解析失败时返回 null，
 * 调用方回退到常规查询帧。
 *
 * 返回形状（每 topic 一项）：
 *   { "signal": { "value": { "rsrp": -86, ... }, "fresh": true,
 *                 "age_ms": 42, "source": "snapshot" }, ... }
 */
function cachedSnapshot() {
	return atSafe([ 'cached' ]).then(function(result) {
		try {
			var parsed = JSON.parse(result.stdout || '');
			if (!parsed || typeof parsed !== 'object') return null;

			// 形状 A（守护进程控制通道）：{ "ok": true, "snapshot": { ... } }
			if (parsed.ok === true && parsed.snapshot && typeof parsed.snapshot === 'object')
				return parsed.snapshot;

			// 形状 B：裸 snapshot 对象（带或不带 snapshot 键）
			if (parsed.snapshot && typeof parsed.snapshot === 'object')
				return parsed.snapshot;

			// 形状 C（实机实测）：裸 topic 映射 { "signal": {value,age_ms}, ... }
			// mt5700m-at cached 直接输出 StateCache.snapshot() 的结果，没有
			// ok/snapshot 包装。旧实现只认形状 A，导致快照恒为 null、
			// 首屏永远渲染不出来 —— 这里必须兼容。
			var topics = Object.keys(parsed).filter(function(k) {
				var v = parsed[k];
				return v && typeof v === 'object' && ('value' in v || 'age_ms' in v);
			});
			if (topics.length) return parsed;
		} catch (e) { /* 快照不可解析：忽略，走常规查询帧 */ }
		return null;
	});
}

/* ---------- AT 子命令速记 ---------- */

function atStatus()            { return atSafe([ 'status' ]); }
function atSession()           { return atSafe([ 'advanced', 'session' ]); }
function atNetwork()           { return atSafe([ 'network' ]); }
function atRadio()             { return atSafe([ 'advanced', 'radio' ]); }
function atRadioDiagnostics()  { return at([ 'advanced', 'radio-diagnostics' ]); }
function atHardware()          { return atSafe([ 'advanced', 'hardware' ]); }
function atSystem()            { return atSafe([ 'system' ]); }
function atConnectionSettings(){ return atSafe([ 'advanced', 'connection-settings' ]); }
function atCommand(cmd)        { return at([ 'command', cmd ]); }
function atSmsList()           { return at([ 'sms-list' ]); }
function atSmsInfo()           { return at([ 'sms-info' ]); }
function atCellscan()          { return at([ 'cellscan' ]); }

return baseclass.extend({
	/* rpcd */
	managerStatus: callManagerStatus,
	dialLog: callDialLog,
	connect: callDial,
	disconnect: callHang,
	redial: callRedial,
	trafficSummary: callTraffic,
	deviceStatus: callDeviceStatus,

	/* 超时配置（供页面展示/调试） */
	atTimeoutMs: AT_TIMEOUT_MS,

	/* fs.exec（AT） */
	at: at,
	atSafe: atSafe,
	cachedSnapshot: cachedSnapshot,
	atStatus: atStatus,
	atSession: atSession,
	atNetwork: atNetwork,
	atRadio: atRadio,
	atRadioDiagnostics: atRadioDiagnostics,
	atHardware: atHardware,
	atSystem: atSystem,
	atConnectionSettings: atConnectionSettings,
	atCommand: atCommand,
	atSmsList: atSmsList,
	atSmsInfo: atSmsInfo,
	atCellscan: atCellscan
});
