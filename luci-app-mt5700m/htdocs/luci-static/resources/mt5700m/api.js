'use strict';
'require baseclass';
'require rpc';

/*
 * MT5700M LuCI — api.js
 * ---------------------------------------------
 * 数据通道统一封装层（单后端，LuCI / WebUI 同源）：
 *   - 唯一后端：Rust `at-webserver`（8765）。它同时是 WebSocket 服务端
 *     （WebUI 连它）与 LuCI 的数据源（ucode 经 nc 转发到它的 TCP RPC）。
 *     两端共享同一份 StateCache / EventBus / AtArbiter：相同只读 AT 命令
 *     命中缓存后不重复下发，AT 串口由 daemon 独占，LuCI 不再自己开端口，
 *     互不抢占。
 *   - rpcd：mt5700m status/log/connect/disconnect/redial（拨号管理，LuCI 独占）。
 *   - ucode（mt5700 对象）：at / cached / events / netrate / traffic。
 *   - 降级旁路：mt5700m-traffic summary（后端未运行时兜底读历史文件）。
 *
 * 前端一律「只读缓存」：不频繁的（IMEI/模块信息）与频繁的（信号/载波/流量）
 * 都读后端 StateCache，缓存新鲜度由后端采集器周期保证（netrate 5 s、
 * signal 3 s 等），页面侧不再自行下发 AT 探活命令。
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
var callNetrate = rpc.declare({ object: 'mt5700', method: 'netrate', params: [ 'device', '_rid' ], expect: { } });
var callBackendTraffic = rpc.declare({ object: 'mt5700', method: 'traffic', params: [ '_rid' ], expect: { } });

/*
 * 单个 AT 调用的硬超时（毫秒）。
 * 取值依据：守护进程侧预算 = queued_timeout(10s) + timeout(8s) + 2s = 20s；
 * ucode 插件侧另有超时外壳（读类 12s / 写类 25s，防止 rpcd 进程被 nc
 * 永久占用拖死整个 LuCI）。前端兜底必须 > ucode 超时外壳，否则前端先
 * 放弃而 rpcd 进程仍卡着：30s 覆盖写类 25s，读类 12s 会先返回错误，
 * 正常帧（数十条 AT 的聚合命令走共享控制通道通常 < 3s）远不会触发。
 */
var AT_TIMEOUT_MS = 30000;
var RPC_TIMEOUT_MS = 15000;
var cachedSnapshotInFlight = null;
var trafficReportInFlight = null;

/* rpc/ubus 调用方的自增请求号（ucode 侧用于临时文件唯一性） */
var rpcSeq = 0;
function nextRid() {
	return String(Date.now()) + '-' + (rpcSeq++);
}

/*
 * 给任意 Promise 套一层硬超时。超时的 Promise 永不 settle 时，
 * 由计时器接管并 reject，调用方即可继续渲染。
 */
function deadlineWithMessage(promise, ms, message) {
	var timer = null;
	return new Promise(function(resolve, reject) {
		timer = setTimeout(function() { reject(new Error(message)); }, ms);
		promise.then(function(v) {
			clearTimeout(timer);
			resolve(v);
		}, function(e) {
			clearTimeout(timer);
			reject(e);
		});
	});
}

function deadline(promise, ms, label) {
	return deadlineWithMessage(promise, ms,
		_('AT command timed out after %s ms').format(ms) + ' (' + label + ')');
}

function rpcDeadline(promise, label) {
	return deadlineWithMessage(promise, RPC_TIMEOUT_MS,
		_('RPC request timed out after %s ms').format(RPC_TIMEOUT_MS) + ' (' + label + ')');
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
 * route —— 统一 API 路由（LuCI 侧的唯一结构化入口）。
 *
 * 单后端（2026-10-05）：命令形如 `api.beam.ssb`（可选尾随 JSON 参数），
 * daemon 把它派发到模块注册表 —— 与 WebUI 的 at().apiCommand 是同一个
 * 注册表、同一份解码代码。前端拿到的是领域数据，不再拿到 AT 原文，
 * 也就不再需要「按标签切段 + 正则取值」的文本解析。
 *
 * 应答形状 { success:true, data }；这里只把 data 交回页面。
 * 永不 reject：后端未运行、路由报错或载荷不是对象都返回 null，
 * 页面按「该卡片暂无数据」处理，与旧版拿不到文本帧时一致。
 */
function route(name, params) {
	var cmd = params ? 'api.' + name + ' ' + JSON.stringify(params) : 'api.' + name;
	return atSafe([ cmd ]).then(function(res) {
		var data = res.stdout;
		if (typeof data === 'string') {
			try { data = JSON.parse(data); } catch (e) { data = null; }
		}
		return (data && typeof data === 'object') ? data : null;
	});
}

/*
 * routeCall —— route() 的**写操作**版本：同一条 `api.<route>` 命令，但失败
 * 必须 reject，调用方才能显示错误并停在那里。
 *
 * route() 的「永不 reject」是为只读行设计的（拿不到就留空，不打断渲染）；
 * 写操作不能这样：后端没起来、路由报错、应答不成形，都绝不能当成「成功」。
 * 后端返回 success:false 时 at() 已经 reject（携带后端 message），这里只补
 * 上「应答不是对象」这一种。
 */
function routeCall(name, params) {
	var cmd = params ? 'api.' + name + ' ' + JSON.stringify(params) : 'api.' + name;
	return at([ cmd ]).then(function(res) {
		var data = res.stdout;
		if (typeof data === 'string') {
			try { data = JSON.parse(data); } catch (e) { data = null; }
		}
		if (!data || typeof data !== 'object')
			throw new Error(_('Invalid response from the backend.'));
		return data;
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
	if (cachedSnapshotInFlight) return cachedSnapshotInFlight;
	var request = deadline(callCached(), AT_TIMEOUT_MS, 'mt5700.cached').then(function(res) {
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
	var shared = request.then(function(value) {
		if (cachedSnapshotInFlight === shared) cachedSnapshotInFlight = null;
		return value;
	}, function(err) {
		if (cachedSnapshotInFlight === shared) cachedSnapshotInFlight = null;
		throw err;
	});
	cachedSnapshotInFlight = shared;
	return shared;
}

/* ---------- AT 子命令速记 ---------- */

function atStatus()            { return atSafe([ 'status' ]); }
function atSession()           { return atSafe([ 'advanced', 'session' ]); }
function atRadio()             { return atSafe([ 'advanced', 'radio' ]); }
function atHardware()          { return atSafe([ 'advanced', 'hardware' ]); }
function atSystem()            { return atSafe([ 'system' ]); }
function atConnectionSettings(){ return atSafe([ 'advanced', 'connection-settings' ]); }
function atCommand(cmd)        { return at([ 'command', cmd ]); }
function atSmsList()           { return at([ 'sms-list' ]); }
function atSmsInfo()           { return at([ 'sms-info' ]); }

/* ---------- 累计流量 / 实时速率（单后端） ---------- */

/*
 * trafficReport —— 「IP 流量统计」数据源，**唯一入口**。
 *
 * 单后端（2026-10-04）：主路径是 mt5700.traffic —— ucode 经 nc 转发到
 * Rust 后端，读 StateCache 的 netrate topic（后端采集器 5 s 一轮调
 * `mt5700m-traffic json` 写入）。WebUI 的「累计统计」读的是同一个 topic，
 * 两侧是同一份快照，数字必然一致。
 *
 * 降级路径：mt5700m-traffic summary —— 直接 exec /usr/sbin/mt5700m-traffic。
 * 保留它是为了后端未运行时概览页仍能显示历史累计；后端起来后自动切回主路径。
 * 两者物理量相同（同一份 /etc/mt5700m/traffic-history），所以降级期间也不会
 * 出现「两套数字」，最多差一个采集周期。
 *
 * 永不 reject，调用方拿到的永远是 { interfaces: [...] }。
 */
function fallbackTraffic() {
	return rpcDeadline(callTraffic(), 'mt5700m-traffic.summary')
		.catch(function() { return null; })
		.then(function(fallback) {
			return (fallback && Array.isArray(fallback.interfaces)) ? fallback : { interfaces: [] };
		});
}

function trafficReport() {
	if (trafficReportInFlight) return trafficReportInFlight;
	var request = deadline(callBackendTraffic({ _rid: nextRid() }), AT_TIMEOUT_MS, 'mt5700.traffic')
		.then(function(res) {
			if (res && res.success === true && res.report && Array.isArray(res.report.interfaces))
				return res.report;
			return fallbackTraffic();
		}, function() {
			return fallbackTraffic();
		});
	var shared = request.then(function(report) {
		if (trafficReportInFlight === shared) trafficReportInFlight = null;
		return report;
	}, function(err) {
		if (trafficReportInFlight === shared) trafficReportInFlight = null;
		throw err;
	});
	trafficReportInFlight = shared;
	return shared;
}

/*
 * netrate —— 网络接口累计字节数（与 WebUI 实时速率同源）。
 * 同样优先读后端 netrate topic；后端不可用时返回 null，**不**回落本地直读
 * sysfs —— 那正是本次要消灭的第二条采集路径。
 */
function netrate(device) {
	return deadline(callNetrate({ device: device || '', _rid: nextRid() }), AT_TIMEOUT_MS, 'mt5700.netrate')
		.then(function(res) {
			return (res && res.success === true) ? res : null;
		}, function() { return null; });
}

return baseclass.extend({
	/* rpcd */
	managerStatus: function() { return rpcDeadline(callManagerStatus(), 'mt5700m.status'); },
	dialLog: function() { return rpcDeadline(callDialLog(), 'mt5700m.log'); },
	connect: callDial,
	disconnect: callHang,
	redial: callRedial,
	trafficSummary: trafficReport,
	deviceStatus: function(name) { return rpcDeadline(callDeviceStatus(name), 'network.device.status'); },

	/* 后端 netrate（单后端数据源） */
	netrate: netrate,

	/* 超时配置（供页面展示/调试） */
	atTimeoutMs: AT_TIMEOUT_MS,

	/* ucode（共享后端 AT 通道） */
	at: at,
	atSafe: atSafe,
	cachedSnapshot: cachedSnapshot,
	atStatus: atStatus,
	atSession: atSession,
	route: route,
	routeCall: routeCall,
	atRadio: atRadio,
	atHardware: atHardware,
	atSystem: atSystem,
	atConnectionSettings: atConnectionSettings,
	atCommand: atCommand,
	atSmsList: atSmsList,
	atSmsInfo: atSmsInfo
});
