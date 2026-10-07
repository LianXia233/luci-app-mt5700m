'use strict';
/*
 * rpcd ucode 插件：mt5700。
 * OpenWrt ucode 语法：无 ===/模板字符串；无 require('json')，
 * 序列化用 sprintf('%J')，反序列化用内置迷你解析器。
 * 参数在 req.args 上（不是 req 顶层）。
 *
 * 数据面（at/cached/events）经 nc 转发到 Rust 后端 at-webserver
 * 的 8765 端口（newline-JSON RPC）。WebUI 走同一后端的 WebSocket，
 * 两侧共享同一 StateCache / EventBus / AtArbiter：
 *   - 相同只读 AT 命令命中缓存后不再重复下发；
 *   - 事件由同一 EventBus 记录，前端增量拉取；
 *   - AT 串口由 daemon 独占，LuCI 不再自己开端口，避免互相抢占。
 * netrate / usb 为本地 sysfs 直读，全程零 AT 流量。
 */

const fs = require('fs');
const uci = require('uci');

/* 调用方没传 _rid 时的兜底自增号（同一秒内多次调用也能区分） */
let rpcFallbackSeq = 0;

function readRpcConfig() {
	const cursor = uci.cursor();
	const port = int(cursor.get('at-webserver', 'config', 'websocket_port')) || 8765;
	const authKey = cursor.get('at-webserver', 'config', 'websocket_auth_key') || '';
	return { port: port, authKey: authKey };
}

function getStr(obj, key) {
	if (obj == null) {
		return null;
	}
	let v = obj[key];
	if (v == null) {
		return null;
	}
	return v;
}

/* 迷你 JSON 解析：ucode 无 s[i]、嵌套函数不提升，用 substr + 前置声明 */
function jsonParse(s) {
	let i = 0;
	let n = length(s);
	let parseVal;

	function ch() {
		if (i >= n) {
			return '';
		}
		return substr(s, i, 1);
	}

	function ws() {
		while (i < n) {
			let c = ch();
			if (c == ' ' || c == '\n' || c == '\r' || c == '\t') {
				i++;
			} else {
				break;
			}
		}
	}

	function parseStr() {
		i++;
		let out = '';
		while (i < n) {
			let c = ch();
			if (c == '\\') {
				i++;
				let e = ch();
				if (e == 'n') { out += '\n'; }
				else if (e == 't') { out += '\t'; }
				else if (e == 'r') { out += '\r'; }
				else if (e == '"') { out += '"'; }
				else if (e == '\\') { out += '\\'; }
				else if (e == '/') { out += '/'; }
				else { out += e; }
				i++;
			} else if (c == '"') {
				i++;
				return out;
			} else {
				out += c;
				i++;
			}
		}
		return out;
	}

	function parseNum() {
		let start = i;
		if (ch() == '-') { i++; }
		while (i < n) {
			let c = ch();
			if ((c >= '0' && c <= '9') || c == '.' || c == 'e' || c == 'E' || c == '+' || c == '-') {
				i++;
			} else {
				break;
			}
		}
		return int(substr(s, start, i - start));
	}

	function parseArr() {
		i++;
		let arr = [];
		ws();
		if (ch() == ']') { i++; return arr; }
		while (i < n) {
			// 本固件 ucode 的数组没有 push 方法（调用会抛
			// "left-hand side is not a function"），只能按下标追加。
			arr[length(arr)] = parseVal();
			ws();
			if (ch() == ',') { i++; ws(); continue; }
			if (ch() == ']') { i++; break; }
			break;
		}
		return arr;
	}

	function parseObj() {
		i++;
		let obj = {};
		ws();
		if (ch() == '}') { i++; return obj; }
		while (i < n) {
			ws();
			if (ch() != '"') { break; }
			let k = parseStr();
			ws();
			if (ch() == ':') { i++; }
			let v = parseVal();
			obj[k] = v;
			ws();
			if (ch() == ',') { i++; continue; }
			if (ch() == '}') { i++; break; }
			break;
		}
		return obj;
	}

	parseVal = function () {
		ws();
		let c = ch();
		if (c == '{') { return parseObj(); }
		if (c == '[') { return parseArr(); }
		if (c == '"') { return parseStr(); }
		if (c == 't') { i += 4; return true; }
		if (c == 'f') { i += 5; return false; }
		if (c == 'n') { i += 4; return null; }
		return parseNum();
	};

	return parseVal();
}

/*
 * netrateCall / trafficCall 已下移到 rpcCall 之后。
 *
 * 原因：两者现在都要经 rpcCall 读后端 StateCache，而 rpcCall 定义在文件后部。
 * ucode 虽会把 function 声明提升到块作用域，但本文件刻意保持
 * 「先定义、后引用」的单向顺序（与既有 jsonParse / strIndexOf 的
 * 引用方向一致），避免依赖提升行为。
 */

/* 查单字符分隔符下标（ucode 全局 index 在本固件未必可用，用 substr 逐字符扫描） */
function strIndexOf(s, ch) {
	let n = length(s);
	for (let i = 0; i < n; i++) {
		if (substr(s, i, 1) == ch) {
			return i;
		}
	}
	return -1;
}

/*
 * 取模组与路由器之间的 USB 链路速率（自动识别，不硬编码设备路径/产品名）。
 *
 * 原理：扫描 /sys/bus/usb/devices 下每个 USB 设备，跳过 xHCI 根集线器
 * （idVendor = 1d6b），剩下的真实 USB 设备即模组（本机实测为 2-1，
 * TDTECH MT5700M-CN，speed=5000 = USB 3.0 5 Gbps）。
 *
 * 实现细节：本固件的 ucode 数组/对象受限（fs.readdir 返回的数组无 length，
 * 直接 length(arr) 会抛 left-hand side is not a function），故复用 fs.popen
 * 走 busybox /bin/sh 一行脚本在设备侧枚举并逐行回显，本函数只做串行读取与解析，
 * 与 rpcCall() 里 fs.popen(nc ...) 的既有做法一致；speed/version 均为 sysfs
 * 直读，全程不发任何 AT 命令，不占用 AT 通道、不干扰模组。
 */
function usbCall(req) {
	const script = 'echo USB_SCAN; '
		+ 'for x in /sys/bus/usb/devices/*/; do '
		+ 'p=$(cat "$x/product" 2>/dev/null); '
		+ '[ -z "$p" ] && continue; '
		+ 'v=$(cat "$x/idVendor" 2>/dev/null); '
		+ '[ "$v" = "1d6b" ] && continue; '
		+ 'echo "USB=$(cat "$x/speed" 2>/dev/null) | $p | $(cat "$x/version" 2>/dev/null)"; '
		+ 'break; done';
	let p;
	try {
		p = fs.popen('/bin/sh -c ' + script, 'r');
	} catch (e) {
		return { success: false, error: '无法读取 USB 信息: ' + e.message };
	}
	if (!p) {
		return { success: false, error: '无法读取 USB 信息' };
	}
	let speed = '';
	let product = '';
	let version = '';
	let guard = 0;
	while (1) {
		if (guard++ >= 20) {
			break;
		}
		let line = p.read('line');
		if (line == null) {
			break;
		}
		if (substr(line, 0, 4) == 'USB=') {
			let rest = trim(substr(line, 4));
			let i = strIndexOf(rest, '|');
			if (i < 0) {
				continue;
			}
			speed = trim(substr(rest, 0, i));
			let tail = substr(rest, i + 1);
			let k = strIndexOf(tail, '|');
			if (k >= 0) {
				product = trim(substr(tail, 0, k));
				version = trim(substr(tail, k + 1));
			} else {
				product = trim(tail);
			}
			break;
		}
	}
	p.close();
	if (speed == '') {
		return { success: false, error: '未检测到 USB 模组设备' };
	}
	let mbps = int(speed) || 0;
	return { success: true, speed_mbps: mbps, product: product, version: version };
}

/*
 * 拨号日志：拨号由 LuCI（mt5700m-manager）负责，日志写在本机文件。
 * 这里只做本地读，不发 AT、不占通道。limit 截断到末尾 N 行。
 */
function logsCall(req) {
	let a = req.args;
	let limit = int(getStr(a, 'limit')) || 300;
	if (limit <= 0 || limit > 1200) {
		limit = 300;
	}
	const logFile = '/tmp/mt5700m-manager.log';
	let f;
	try {
		f = fs.open(logFile, 'r');
	} catch (e) {
		return { success: false, error: '无法读取拨号日志: ' + e.message };
	}
	if (!f) {
		return { success: false, error: '无法读取拨号日志' };
	}
	let lines = [];
	let guard = 0;
	while (1) {
		if (guard++ >= 10000) {
			break;
		}
		let line = f.read('line');
		if (line == null) {
			break;
		}
		lines[length(lines)] = line;
	}
	f.close();
	let total = length(lines);
	let start = total - limit;
	if (start < 0) {
		start = 0;
	}
	let text = '';
	for (let i = start; i < total; i++) {
		text += lines[i];
	}
	return { success: true, log: text, lines: total };
}

function rpcCall(method, params, timeoutS) {
	const rpcCfg = readRpcConfig();
	const port = rpcCfg.port;
	const authKey = rpcCfg.authKey;

	/* 超时外壳秒数：读类 12s / 写类 25s（由调用方传入），
	 * 必须小于前端兜底（30s），保证 rpcd 进程一定先于前端释放。 */
	if (timeoutS == null || timeoutS <= 0 || timeoutS > 60) {
		timeoutS = 12;
	}

	const payload = { id: 1, method: method, params: params };
	if (authKey != '') {
		payload.params.auth_key = authKey;
	}

	/* 本固件 ucode fs 无 connect，经 busybox nc 管道访问回环 RPC。
	 * 各固件的 busybox 未必编译 nc applet，因此先定位可执行文件：
	 * 找不到时必须给出可诊断的报错，而不是笼统的「后端无应答」——
	 * 后者会让人误判成服务没起来，实际是装了插件却缺 nc。 */
	let ncBin = '';
	const ncCands = ['/usr/bin/nc', '/bin/nc', '/usr/sbin/nc', '/sbin/nc'];
	for (let i = 0; i < length(ncCands); i++) {
		let probe;
		try {
			probe = fs.open(ncCands[i], 'r');
		} catch (e) {
			probe = null;
		}
		if (probe) {
			probe.close();
			ncBin = ncCands[i];
			break;
		}
	}
	if (ncBin == '') {
		return {
			success: false,
			error: '系统缺少 nc（busybox 未编译 nc applet），无法连接后端：请安装 netcat 后重试'
		};
	}

	/*
	 * 超时外壳（关键兜底，防止拖死整个 LuCI）：
	 * rpcd 的 ucode 插件是「每请求一个进程」，rpcCall 里的 p.read('line')
	 * 一旦没有超时，nc 就会一直等后端响应 —— 后端 AT 命令排队 / 串口挂起时
	 * 该 rpcd 进程被永久占用，页面并发几个请求就把 rpcd 的并发进程耗尽，
	 * 连静态页面 / 其它插件都转不动，表现为「LuCI 整个拖死」。
	 * 前端 15s 超时只救浏览器（Promise reject），救不了 rpcd 进程。
	 *
	 * 因此这里给 nc 套一层硬超时：优先用 timeout applet（OpenWrt busybox
	 * 自带，coreutils 也有）；个别裁剪固件没有 timeout 时，退化为 nc 自身
	 * 的 -w（GNU / busybox nc 都支持，作为「等待响应」超时）。
	 * 超时后 nc 退出 -> read 返回 null -> 本请求立即返回错误，rpcd 进程
	 * 马上释放，不再占坑。
	 */
	let rpcTimeoutS = timeoutS;
	let tmoBin = '';
	const tmoCands = ['/usr/bin/timeout', '/bin/timeout', '/usr/sbin/timeout', '/sbin/timeout'];
	for (let i = 0; i < length(tmoCands); i++) {
		let probe;
		try {
			probe = fs.open(tmoCands[i], 'r');
		} catch (e) {
			probe = null;
		}
		if (probe) {
			probe.close();
			tmoBin = tmoCands[i];
			break;
		}
	}

	/*
	 * busybox/coreutils 的 timeout 均支持 `timeout N cmd ...` 位置参数。
	 *
	 * ncCmd 必须在 tmp 声明**之后**构造：ucode 的 let/const 是块级作用域，
	 * 提前引用会抛 "access to undeclared variable tmp"，rpcd 把整个方法
	 * 报成 Unknown error（实测踩过：rpcd -S 才看得到这句 Reference error，
	 * 普通 logread 里什么都没有）。
	 */
	const body = sprintf('%J', payload);

	/*
	 * 临时文件名必须唯一。
	 *
	 * 实测问题：原先固定为 /tmp/mt5700-rpc.json，而页面会**并发**发起多个 RPC
	 * （日志页一次就发 3 个：后端日志 + syslog + 通知文件），多个请求写同一个
	 * 文件、再各自读同一个文件，互相覆盖，表现为「RPC 返回空对象」这种极难排查的
	 * 间歇性故障 —— 单独手动调用却完全正常。
	 *
	 * 唯一性由调用方传入的 _rid 提供；没传时退化为「时间戳 + 自增」，
	 * 同一秒内的多次调用也能区分（ucode 的 time() 只有秒级）。
	 */
	let rid = getStr(params, '_rid');
	if (rid == null || rid == '') {
		rid = sprintf('%d-%d', time(), rpcFallbackSeq++);
	}
	const tmp = '/tmp/mt5700-rpc-' + rid + '.json';

	let ncCmd;
	if (tmoBin != '') {
		ncCmd = tmoBin + ' ' + rpcTimeoutS + ' ' + ncBin + ' 127.0.0.1 ' + port + ' < ' + tmp;
	} else {
		ncCmd = ncBin + ' -w ' + rpcTimeoutS + ' 127.0.0.1 ' + port + ' < ' + tmp;
	}

	let f;
	try {
		f = fs.open(tmp, 'w');
	} catch (e) {
		return { success: false, error: '无法写临时文件' };
	}
	if (!f) {
		return { success: false, error: '无法写临时文件' };
	}
	f.write(body + '\n');
	f.close();

	let p;
	try {
		p = fs.popen(ncCmd, 'r');
	} catch (e) {
		return { success: false, error: '无法连接 Rust 后端: ' + e.message };
	}
	if (!p) {
		return { success: false, error: '无法连接 Rust 后端' };
	}

	let line = p.read('line');
	p.close();
	/* 用完即删，避免每次 RPC 都在 /tmp 留一个文件 */
	try {
		fs.unlink(tmp);
	} catch (e) {
		/* 删不掉不影响主流程 */
	}

	if (!line) {
		/* 后端无应答：可能是服务未运行，也可能是命令超时被外壳杀掉 */
		return {
			success: false,
			error: '后端响应超时（>' + rpcTimeoutS + 's，AT 命令可能仍挂起）或服务未运行：Rust 后端端口 ' + port + ' 无应答'
		};
	}
	if (substr(line, 0, 1) != '{') {
		return { success: false, error: 'Rust 后端应答异常: ' + substr(line, 0, 160) };
	}

	try {
		let resp = jsonParse(line);
		if (resp.error) {
			let msg = 'RPC 错误';
			if (resp.error.message) {
				msg = resp.error.message;
			}
			return { success: false, error: msg };
		}
		if (resp.result) {
			return resp.result;
		}
		return {};
	} catch (e) {
		return { success: false, error: '解析应答失败: ' + e.message };
	}
}

/*
 * 从后端 cached 快照里取出某个 topic 的 entry（含 value/fresh/age_ms/source）。
 * 找不到时返回 null，调用方负责降级。
 *
 * 注意：rpcCall 成功时返回的是后端的 result 对象，`cached` 方法里用的是
 * `ok:true` 而**不是** `success:true`（后者是本文件自己的返回约定）。
 * 这里只判「有没有拿到 snapshot」，不要用 success 去筛，否则一律判失败。
 */
function cachedTopic(topic, rid) {
	let res = rpcCall('cached', { _rid: rid }, 6);
	if (res == null) {
		return null;
	}
	let snap = res.snapshot;
	if (snap == null) {
		return null;
	}
	return snap[topic];
}

/*
 * 网络接口累计字节数 —— 经 Rust 后端 StateCache 读取（单后端）。
 *
 * 2026-10-04 收口：原先这里自己 open('/sys/class/net/eth2/statistics/rx_bytes')
 * 直读网卡计数器，而 WebUI 读的是后端 netrate topic。同一个物理量、两条采集
 * 路径，采样时刻不同就会差几百 KB。现在两侧统一读后端采集器（5 s 周期）
 * 写入的 netrate topic，LuCI 与 WebUI 拿到的是同一份数值。
 *
 * 后端不可用时**不静默回落到本地直读**：那正是本次要消灭的双路径。
 * 直接返回失败，由前端显示「后端未运行」，避免两侧数字悄悄分叉。
 * 保留 device 入参仅用于回显，不再影响取值。
 *
 * 实时速率仍由前端按两次采样差 / 时间差计算；这里额外返回后端 timestamp，
 * 前端优先用它做时间基准，规避 rpcd 与浏览器时钟不同源的抖动。
 */
function netrateCall(req) {
	let a = req.args;
	let rid = getStr(a, '_rid');
	let wantDev = getStr(a, 'device');

	let entry = cachedTopic('netrate', rid);
	if (entry == null) {
		return {
			success: false,
			error: '后端 netrate 采集器尚无数据（at-webserver 未运行或刚启动，请稍后重试）'
		};
	}

	let v = entry.value;
	if (v == null) {
		return { success: false, error: '后端 netrate topic 为空' };
	}

	/* available=false 是后端显式标注的「接口不存在/计数器不可读」，
	 * 必须原样透传给前端，不能当成 0 —— 否则页面会显示一个假的 0 B/s。 */
	if (v.available !== true) {
		return {
			success: false,
			available: false,
			device: v.device != null ? v.device : wantDev,
			reason: v.reason != null ? v.reason : '接口不可用',
			fresh: entry.fresh,
			age_ms: entry.age_ms
		};
	}

	return {
		success: true,
		available: true,
		device: v.device,
		rx_bytes: v.rx_bytes,
		tx_bytes: v.tx_bytes,
		timestamp: v.timestamp,
		/* 后端采集器周期 5 s，这里额外暴露新鲜度便于前端提示「数据可能已过期」 */
		fresh: entry.fresh,
		age_ms: entry.age_ms,
		source: 'at-webserver'
	};
}

/*
 * 累计/日/月流量 —— 经 Rust 后端 StateCache 读取（单后端）。
 *
 * 历史文件 /etc/mt5700m/traffic-history 的唯一写入方是 mt5700m-traffic 的
 * daemon；后端 netrate 采集器调 `mt5700m-traffic json` 读回并放进 netrate
 * topic。LuCI 与 WebUI 都只读这个 topic，因此两侧看到的是同一份快照。
 *
 * 返回形状保持与旧的 `mt5700m-traffic summary`（直接 exec 二进制）完全
 * 一致：{ interfaces:[{name,updated,traffic:{total,day,month}}] }，
 * 所以 status.js 的 trafficPanel 无需改动。
 */
function trafficCall(req) {
	let rid = getStr(req.args, '_rid');

	let entry = cachedTopic('netrate', rid);
	if (entry == null) {
		return { success: false, error: '后端 netrate 采集器尚无数据，无法读取累计流量' };
	}

	let v = entry.value;
	if (v == null) {
		return { success: false, error: '后端 netrate topic 为空' };
	}
	if (v.available !== true) {
		return {
			success: false,
			available: false,
			reason: v.reason != null ? v.reason : '接口不可用'
		};
	}
	if (v.traffic == null) {
		return {
			success: false,
			available: true,
			source: v.source != null ? v.source : 'unavailable',
			error: '后端未取到 mt5700m-traffic 输出（历史可能尚未初始化）'
		};
	}

	return { success: true, source: 'at-webserver', report: v.traffic };
}

return {
	mt5700: {
		at: {
			args: { cmd: '', args: [], _rid: '' },
			call: function (req) {
				let a = req.args;
				let cmd = getStr(a, 'cmd');
				if (cmd == null || cmd == '') {
					return { success: false, error: '缺少参数 cmd' };
				}
				// 参数数组透传：sms-send 等多词参数时前端按参数边界传递，
				// 避免「空格分词」丢失参数；后端优先使用 args，缺省才拆 cmd。
				let params = { cmd: cmd, _rid: getStr(a, '_rid') };
				let argList = a.args;
				if (argList != null && length(argList) > 0) {
					params.args = argList;
				}
				// 超时分配：原生 AT（command 前缀）/ SMS / 写类命令放宽到 25s
				// （SMS 收发、PDP 配置、恢复出厂等可能慢），读类 12s 快速失败。
				// 前端兜底 30s > 25s，rpcd 进程一定先释放，LuCI 不会被拖死。
				let tmo = 12;
				if (substr(cmd, 0, 8) == 'command ' || substr(cmd, 0, 4) == 'sms-') {
					tmo = 25;
				} else {
					const slowWrites = ['pdp-set', 'factory-reset', 'sim-pin', 'restart', 'unlock', 'advanced-set'];
					for (let i = 0; i < length(slowWrites); i++) {
						if (cmd == slowWrites[i]) {
							tmo = 25;
							break;
						}
					}
					// 已迁到统一 API 的写操作，与它们原来的 CLI 动词同样给 25s：
					//   - lock_apply 走「关机-写-开机-轮询校验」（原来是 lock 动词）；
					//   - c5goption_set 自带飞行模式循环（原来是 advanced-set 5g-access）；
					//   - syscfg_set / nr_capability_set 是单/多命令配置写
					//     （原来是 advanced-set radio-policy / carrier-aggregation /
					//     vonr / dss）。超时截断的代价是前端看不到完成，
					//     但写本身已下发 —— 与 CLI 时期的语义一致。
					// 列表按迁移进度增长（写路由 = api.<module>.<name>）。
					const slowRouteWrites = [
						'api.network.lock_apply',
						'api.network.c5goption_set',
						'api.network.syscfg_set',
						'api.modem.nr_capability_set'
					];
					for (let i = 0; i < length(slowRouteWrites); i++) {
						if (substr(cmd, 0, length(slowRouteWrites[i])) == slowRouteWrites[i]) {
							tmo = 25;
							break;
						}
					}
				}
				return rpcCall('at', params, tmo);
			}
		},
		cached: {
			args: { _rid: '' },
			call: function (req) {
				// 缓存快照：零 AT 流量，LuCI 首屏立即拿到后台采集器状态。
				let a = req.args;
				return rpcCall('cached', { _rid: getStr(a, '_rid') });
			}
		},
		events: {
			args: { since: 0, _rid: '' },
			call: function (req) {
				let a = req.args;
				let since = 0;
				let s = getStr(a, 'since');
				if (s != null && s != '') {
					since = int(s) || 0;
				}
				if (since < 0) {
					since = 0;
				}
				return rpcCall('events', { since: since, _rid: getStr(a, '_rid') });
			}
		},
		netrate: {
			args: { device: '', _rid: '' },
			call: function (req) {
				return netrateCall(req);
			}
		},
		traffic: {
			args: { _rid: '' },
			call: function (req) {
				return trafficCall(req);
			}
		},
		usb: {
			args: { _rid: '' },
			call: function (req) {
				return usbCall(req);
			}
		},
		logs: {
			args: { since: 0, limit: 300, _rid: '' },
			call: function (req) {
				let a = req.args;
				let since = int(getStr(a, 'since')) || 0;
				if (since < 0) {
					since = 0;
				}
				return logsCall(req);
			}
		}
	}
};
