'use strict';
'require baseclass';
'require rpc';

/*
 * MT5700M LuCI — api.js
 * ---------------------------------------------
 * 数据通道统一封装层（共享后端，LuCI / WebUI 同源）：
 *   - rpcd：mt5700m status/log/connect/disconnect/redial（拨号管理，LuCI 独占）、
 *     mt5700m-traffic summary、network.device status
 *   - ucode（mt5700 对象）：at / cached —— 经 nc 转发到 Rust 后端 at-webserver
 *     的 8765 端口；WebUI 走同一后端的 WebSocket。两侧共享 StateCache /
 *     EventBus / AtArbiter：相同只读 AT 命令命中缓存后不重复下发，
 *     AT 串口由 daemon 独占，LuCI 不再自己开端口，互不抢占。
 * 页面代码一律经本模块调用，不再各自 declare / 各自写 catch 归一化。
 *
 * 超时兜底（AT_TIMEOUT_MS）：
 *   三层防挂起：
 *     - ucode 插件侧：nc 转发套 timeout 外壳（读类 12s / 写类 25s），
 *       到点杀掉 nc，rpcd 进程立即释放 —— 防止「AT 挂起 -> rpcd 进程
 *       被 nc 永久占用 -> 并发请求耗尽 rpcd -> 整个 LuCI 拖死」。
 *     - 后端侧：arbiter 命令有 queued_timeout + timeout 预算兜底。
 *     - 前端侧：对每个 AT 调用套 Promise.race 硬超时（30s，> ucode 外壳），
 *       保证 Promise 一定 settle，页面必然完成渲染。
 *   at() 超时按 reject 处理（send / terminal 需要区分成败）；
 *   atSafe() 超时被 catch 归一化为 { stdout:'', stderr:message }。
 */

/* ---------- rpcd 声明 ---------- */

var callManagerStatus = rpc.declare({ object: 'mt5700m', method: 'status', expect: { } });
var callDialLog = rpc.declare({ object: 'mt5700m', method: 'log', expect: { } });
var callDial = rpc.declare({ object: 'mt5700m', method: 'connect', expect: { } });
var callHang = rpc.declare({ object: 'mt5700m', method: 'disconnect', expect: { } });
var callRedial = rpc.declare({ object: 'mt5700m', method: 'redial', expect: { } });
var callTraffic = rpc.declare({ object: 'mt5700m-traffic', method: 'summary', expect: { } });
var callDeviceStatus = rpc.declare({ object: 'network.device', method: 'status', params: [ 'name' ], expect: { } });

/* ---------- ucode（共享后端）声明 ---------- */

var callAt = rpc.declare({ object: 'mt5700', method: 'at', params: [ 'cmd', 'args', '_rid' ], expect: { } });
var callCached = rpc.declare({ object: 'mt5700', method: 'cached', expect: { } });

/*
 * 单个 AT 调用的硬超时（毫秒）。
 * 取值依据：守护进程侧预算 = queued_timeout(10s) + timeout(8s) + 2s = 20s；
 * ucode 插件侧另有超时外壳（读类 12s / 写类 25s，防止 rpcd 进程被 nc
 * 永久占用拖死整个 LuCI）。前端兜底必须 > ucode 超时外壳，否则前端先
 * 放弃而 rpcd 进程仍卡着：30s 覆盖写类 25s，读类 12s 会先返回错误，
 * 正常帧（数十条 AT 的聚合命令走共享控制通道通常 < 3s）远不会触发。
 */
var AT_TIMEOUT_MS = 30000;

/* rpc/ubus 调用方的自增请求号（ucode 侧用于临时文件唯一性） */
var rpcSeq = 0;
function nextRid() {
	return String(Date.now()) + '-' + (rpcSeq++);
}

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

/* ---------- ucode 封装 ---------- */

/*
 * 原始调用：reject 时携带错误（send / terminal 等需要区分成败的场景）。
 * 后端应答 { success:true, data } / { success:false, error }，归一化为页面
 * 既有的 { stdout, stderr } 契约；失败按 reject 抛出。
 */
function at(args) {
	return deadline(callAt(args.join(' '), args, nextRid()), AT_TIMEOUT_MS, 'mt5700.at ' + args.join(' ')).then(function(res) {
		if (res && res.success === true)
			return { stdout: res.data || '', stderr: '' };
		var msg = (res && res.error) || 'AT command failed';
		var err = new Error(msg);
		err.stdout = '';
		err.stderr = msg;
		throw err;
	});
}

// 归一化调用：永不 reject，失败返回 { stdout:'', stderr: message }
function atSafe(args) {
	return at(args).catch(function(err) {
		return { stdout: '', stderr: err.message || String(err) };
	});
}

/*
 * cachedSnapshot —— Async Architecture 缓存优先（SWR）快照。
 * 走 ucode mt5700.cached（nc → daemon StateCache）：零 AT 流量、毫秒级返回，
 * 即使模组离线 / AT 卡住也能立即拿到最近一次后台采集器写入的状态。
 * 永不 reject：后端未运行或解析失败时返回 null，调用方回退到常规查询帧。
 *
 * 返回形状（每 topic 一项）：
 *   { "signal": { "value": { "rsrp": -86, ... }, "fresh": true,
 *                 "age_ms": 42, "source": "snapshot" }, ... }
 */
function cachedSnapshot() {
	return deadline(callCached(), AT_TIMEOUT_MS, 'mt5700.cached').then(function(res) {
		if (!res || typeof res !== 'object') return null;

		// 形状 A（daemon RPC）：{ "ok": true, "snapshot": { ... } }
		if (res.ok === true && res.snapshot && typeof res.snapshot === 'object')
			return res.snapshot;

		// 形状 C（实机兜底）：裸 topic 映射 { "signal": {value,age_ms}, ... }
		var topics = Object.keys(res).filter(function(k) {
			var v = res[k];
			return v && typeof v === 'object' && ('value' in v || 'age_ms' in v);
		});
		if (topics.length) return res;
		return null;
	}).catch(function() { return null; });
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

	/* ucode（共享后端 AT 通道） */
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
