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

/* ---------- fs.exec 封装 ---------- */

// 原始调用：reject 时携带错误（send / terminal 等需要区分成败的场景）
function at(args) {
	return fs.exec(AT_BIN, args);
}

// 归一化调用：永不 reject，失败返回 { stdout:'', stderr: message }
function atSafe(args) {
	return at(args).catch(function(err) {
		return { stdout: '', stderr: err.message || String(err) };
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

	/* fs.exec（AT） */
	at: at,
	atSafe: atSafe,
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
