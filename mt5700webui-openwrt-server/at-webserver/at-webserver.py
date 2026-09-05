#!/usr/bin/env python3
# at-webserver.py — MT5700M WebUI 后端（Go 版 at-webserver 的 Python 移植）
#
# 与 Go 版协议严格一致：
#   - WebSocket :8765，文本帧即 AT 命令，应答 {"success","data"/"error"}
#   - "ping" -> "pong"（纯文本），服务端每 30s 推一条文本 "ping"
#   - 认证握手（配置了 websocket_auth_key 时）：首条消息 {"auth_key":...}
#   - 伪命令：AT+CONNECT? / AT+SCHED? / AT+SCHED= / AT^CELLSCAN(=ABORT|=STATE)
#   - 推送：{"type":"raw_data|incoming_call|new_sms|pdcp_data|cellscan","data":...}
#
# 依赖：python3 + pyserial + websockets（ImmortalWrt: python3-pyserial / python3-websockets）
# 用法：python3 at-webserver.py [-v]

import asyncio
import glob
import json
import logging
import re
import signal
import socket
import subprocess
import sys
import threading
import time
from datetime import datetime, timedelta
from queue import Empty, Full, Queue

try:
    import serial
except ImportError:
    serial = None

try:
    import websockets
except ImportError:
    websockets = None

VERSION = "3.0.2-py1"

log = logging.getLogger("at-webserver")

# ---------------------------------------------------------------- 常量（与 Go 版一致）
COMMAND_GAP = 0.1          # 两条 AT 命令之间的最小间隔（秒）
COMMAND_TIMEOUT = 2.0      # 默认命令超时
READ_BUF_SIZE = 4096
MAX_RESPONSE_LINES = 2048
MAX_RESIDUAL_BYTES = 64 * 1024

WS_HEARTBEAT = 30.0
WS_AUTH_TIMEOUT = 10.0
WS_WRITE_TIMEOUT = 10.0
WS_OUT_BUFFER = 128

CELLSCAN_ABORT_TOKEN = "abcd"          # 手册规定的打断字符串，必须小写
DEFAULT_SCAN_TIMEOUT = 180.0
SCAN_RECOVERY_GRACE = 60.0

CALL_DEDUP_WINDOW = 30.0
SIGNAL_CHANGE_THRESHOLD = 1.0
PARTIAL_SMS_TTL = 3600.0
MAX_PARTIAL_SMS = 100

NOTIFY_INTERVAL = 60.0
NOTIFY_MAX_PENDING = 1000
NOTIFY_MAX_RETRIES = 3

AUTO_SERIAL_PORT = "auto"
PREFERRED_AT_PORT = "/dev/ttyUSB1"

SENDER_CALL = "来电提醒"
SENDER_SIGNAL = "信号监控"

# ---------------------------------------------------------------- 日志
def setup_logging(verbose):
    level = logging.DEBUG if verbose else logging.INFO
    h = logging.StreamHandler(sys.stderr)
    h.setFormatter(logging.Formatter("%(asctime)s [%(levelname)s] %(message)s", "%Y-%m-%d %H:%M:%S"))
    log.setLevel(level)
    log.addHandler(h)
    log.propagate = False


# ---------------------------------------------------------------- UCI 配置
DEFAULTS = {
    "enabled": True,
    "connection_type": "NETWORK",
    "network_host": "192.168.8.1",
    "network_port": 20249,
    "network_timeout": 10.0,
    "serial_port": PREFERRED_AT_PORT,
    "serial_baudrate": 115200,
    "serial_timeout": 10.0,
    "websocket_port": 8765,
    "websocket_auth_key": "",
    "cellscan_timeout": 180.0,
    "wechat_webhook": "",
    "log_file": "",
    "notify_sms": True,
    "notify_call": True,
    "notify_memory_full": True,
    "notify_signal": True,
    "schedule_enabled": False,
    "schedule_check_interval": 60,
    "schedule_timeout": 180,
    "schedule_unlock_lte": True,
    "schedule_unlock_nr": True,
    "schedule_toggle_airplane": True,
    "schedule_night_enabled": True,
    "schedule_night_start": "22:00",
    "schedule_night_end": "06:00",
    "schedule_night_lte_type": 3,
    "schedule_night_lte_bands": "",
    "schedule_night_lte_arfcns": "",
    "schedule_night_lte_scs_types": "",
    "schedule_night_lte_pcis": "",
    "schedule_night_nr_type": 3,
    "schedule_night_nr_bands": "",
    "schedule_night_nr_arfcns": "",
    "schedule_night_nr_scs_types": "",
    "schedule_night_nr_pcis": "",
    "schedule_day_enabled": True,
    "schedule_day_lte_type": 3,
    "schedule_day_lte_bands": "",
    "schedule_day_lte_arfcns": "",
    "schedule_day_lte_scs_types": "",
    "schedule_day_lte_pcis": "",
    "schedule_day_nr_type": 3,
    "schedule_day_nr_bands": "",
    "schedule_day_nr_arfcns": "",
    "schedule_day_nr_scs_types": "",
    "schedule_day_nr_pcis": "",
}


def _as_bool(v):
    return str(v).strip().lower() in ("1", "true", "yes", "on")


def load_uci_config():
    """一次 uci show 取回整个配置段，键名与 LuCI/Go 版一致。"""
    cfg = dict(DEFAULTS)
    try:
        out = subprocess.run(["uci", "show", "at-webserver"], capture_output=True,
                             text=True, timeout=5).stdout
    except Exception as e:
        log.warning("读取 UCI 配置失败，使用默认配置: %s", e)
        return cfg

    prefix = "at-webserver.config."
    for line in out.splitlines():
        line = line.strip()
        if not line.startswith(prefix) or "=" not in line:
            continue
        key, _, val = line.partition("=")
        key = key[len(prefix):]
        val = val.strip().strip("'\"")
        cfg[key] = val

    # 类型归一
    cfg["enabled"] = _as_bool(cfg["enabled"])
    cfg["network_port"] = int(cfg["network_port"])
    cfg["network_timeout"] = float(cfg["network_timeout"])
    cfg["serial_baudrate"] = int(cfg["serial_baudrate"])
    cfg["serial_timeout"] = float(cfg["serial_timeout"])
    cfg["websocket_port"] = int(cfg["websocket_port"])
    cfg["cellscan_timeout"] = float(cfg["cellscan_timeout"])
    for k in ("notify_sms", "notify_call", "notify_memory_full", "notify_signal",
              "schedule_unlock_lte", "schedule_unlock_nr", "schedule_toggle_airplane"):
        cfg[k] = _as_bool(cfg[k])
    cfg["schedule_enabled"] = _as_bool(cfg["schedule_enabled"])
    cfg["schedule_night_enabled"] = _as_bool(cfg["schedule_night_enabled"])
    cfg["schedule_day_enabled"] = _as_bool(cfg["schedule_day_enabled"])
    for k in ("schedule_check_interval", "schedule_timeout"):
        cfg[k] = int(cfg[k])
    for p in ("night", "day"):
        for kind in ("lte", "nr"):
            cfg["schedule_%s_%s_type" % (p, kind)] = int(cfg["schedule_%s_%s_type" % (p, kind)])
    return cfg


def uci_set_many(entries):
    """落盘配置：直接传参给 uci，不经 shell。"""
    for key, val in entries:
        subprocess.run(["uci", "set", "at-webserver.config.%s=%s" % (key, val)],
                       check=True, timeout=5)
    subprocess.run(["uci", "commit", "at-webserver"], check=True, timeout=5)


# ---------------------------------------------------------------- PDU 解码（移植 pdu.go）
GSM7_ALPHABET = (
    "@£$¥èéùìòÇ\nØø\rÅåΔ_ΦΓΛΩΠΨΣΘΞ\x1bÆæßÉ !\"#¤%&'()*+,-./0123456789:;<=>?"
    "¡ABCDEFGHIJKLMNOPQRSTUVWXYZÄÖÑÜ§¿abcdefghijklmnopqrstuvwxyzäöñüà"
)
GSM7_EXTENSION = {0x0A: "\f", 0x14: "^", 0x28: "{", 0x29: "}", 0x2F: "\\",
                  0x3C: "[", 0x3D: "~", 0x3E: "]", 0x40: "|", 0x65: "€"}


def unpack_septets(data, count):
    out = []
    acc = 0
    bits = 0
    for b in data:
        acc |= b << bits
        bits += 8
        while bits >= 7:
            if len(out) == count:
                return out
            out.append(acc & 0x7F)
            acc >>= 7
            bits -= 7
    if bits > 0 and len(out) < count:
        out.append(acc & 0x7F)
    return out


def septets_to_string(septets):
    sb = []
    i = 0
    while i < len(septets):
        c = septets[i]
        if c == 0x1B and i + 1 < len(septets):
            ext = GSM7_EXTENSION.get(septets[i + 1])
            if ext is not None:
                sb.append(ext)
                i += 2
                continue
        if c < len(GSM7_ALPHABET):
            sb.append(GSM7_ALPHABET[c])
        else:
            sb.append("?")
        i += 1
    return "".join(sb)


def decode_ucs2(data):
    return data.decode("utf-16-be", errors="replace")


def decode_timestamp(ts):
    if len(ts) < 7:
        return datetime.now()
    bcd = lambda b: (b & 0x0F) * 10 + (b >> 4)
    year = 2000 + bcd(ts[0])
    month = bcd(ts[1])
    day = bcd(ts[2])
    hour = bcd(ts[3])
    minute = bcd(ts[4])
    second = bcd(ts[5])
    try:
        return datetime(year, month, day, hour, minute, second)
    except ValueError:
        return datetime.now()


def decode_number(data, digits):
    out = []
    for b in data:
        lo, hi = b & 0x0F, b >> 4
        if lo <= 9:
            out.append(str(lo))
        if len(out) < digits and hi <= 9:
            out.append(str(hi))
    return "".join(out)


def decode_address(data, digits, toa):
    # TON=101 表示 alphanumeric 发送方（如 "CMCC"），内容是打包 7 位字符。
    if ((toa >> 4) & 0x07) == 0x05:
        septets = (digits * 4) // 7
        return septets_to_string(unpack_septets(data, septets))
    return decode_number(data, digits)


def parse_udh_concat(udh):
    i = 0
    while i + 1 < len(udh):
        iei = udh[i]
        ie_len = udh[i + 1]
        body = i + 2
        if body + ie_len > len(udh):
            return None
        if iei == 0x00 and ie_len >= 3:
            return {"reference": udh[body], "parts_count": udh[body + 1], "part_number": udh[body + 2]}
        if iei == 0x08 and ie_len >= 4:
            return {"reference": (udh[body] << 8) | udh[body + 1],
                    "parts_count": udh[body + 2], "part_number": udh[body + 3]}
        i = body + ie_len
    return None


def decode_incoming_pdu(pdu_hex):
    """解析 SMS-DELIVER PDU，返回 dict(sender, content, date, partial)。"""
    raw = bytes.fromhex(pdu_hex.strip())
    pos = 0

    def cut(n):
        nonlocal pos
        if n < 0 or pos + n > len(raw):
            raise ValueError("PDU 数据不完整")
        b = raw[pos:pos + n]
        pos += n
        return b

    def nxt():
        return cut(1)[0]

    smsc_len = nxt()
    cut(smsc_len)
    pdu_type = nxt()
    sender_digits = nxt()
    sender_toa = nxt()
    sender_bytes = cut((sender_digits + 1) // 2)
    sender = decode_address(sender_bytes, sender_digits, sender_toa)
    nxt()  # PID
    dcs = nxt()
    ts = cut(7)
    udl = nxt()
    ud = raw[pos:]

    encoding = (dcs >> 2) & 0x03

    udh_len = 0
    partial = None
    if (pdu_type & 0x40) and len(ud) > 0:
        udh_len = ud[0] + 1
        if udh_len > len(ud):
            raise ValueError("UDH 长度超出用户数据")
        partial = parse_udh_concat(ud[1:udh_len])

    if encoding == 0x02:
        content = decode_ucs2(ud[udh_len:])
    elif encoding == 0x01:
        content = "".join(chr(b) for b in ud[udh_len:])
    else:
        udh_septets = (udh_len * 8 + 6) // 7
        total = max(udl, udh_septets)
        septets = unpack_septets(ud, total)
        septets = septets[udh_septets:] if udh_septets <= len(septets) else []
        content = septets_to_string(septets)

    return {"sender": sender, "content": content, "date": decode_timestamp(ts), "partial": partial}


# ---------------------------------------------------------------- AT 应答与命令
class ATResponse:
    def __init__(self, lines):
        self.lines = lines

    def text(self):
        return "\r\n".join(self.lines)

    def ok(self):
        return "OK" in self.lines

    def has_error(self):
        return "ERROR" in self.text().upper()

    def contains(self, sub):
        return sub in self.text()


def is_terminator(line):
    if line in ("OK", "ERROR", "ABORTED"):
        return True
    return line.startswith("+CMS ERROR:") or line.startswith("+CME ERROR:")


def is_passthrough_urc(line):
    # ^REJINFO（13.14 网络拒绝原因）与带逗号的 +CUSD（网络异步回的 USSD 结果）
    if line.startswith("^REJINFO"):
        return True
    return line.startswith("+CUSD:") and "," in line


def is_exclusive_urc(line):
    if line in ("RING", "IRING", "^IRING", "NO CARRIER"):
        return True
    if (line.startswith("+CMTI:") or line.startswith("^CEND:") or
            line.startswith("^SMMEMFULL") or "MEMORY FULL" in line or
            "CMS ERROR: 322" in line):
        return True
    # 带引号的 +CLIP: 是来电上报；AT+CLIP? 的应答形如 "+CLIP: 1,1"，不会命中
    if line.startswith("+CLIP:") and '"' in line:
        return True
    return is_passthrough_urc(line)


class Transport:
    def write(self, data):
        raise NotImplementedError

    def read(self, n):
        raise NotImplementedError

    def close(self):
        raise NotImplementedError

    def describe(self):
        raise NotImplementedError


class SerialTransport(Transport):
    def __init__(self, port, baudrate):
        self.ser = serial.Serial(port, baudrate, timeout=0.1)
        self.port = port

    def write(self, data):
        self.ser.write(data)
        self.ser.flush()

    def read(self, n):
        return self.ser.read(n)

    def close(self):
        try:
            self.ser.close()
        except Exception:
            pass

    def describe(self):
        return "串口 " + self.port


class TcpTransport(Transport):
    def __init__(self, host, port):
        self.sock = socket.create_connection((host, port), timeout=10)
        self.sock.settimeout(0.1)
        self.addr = "%s:%d" % (host, port)

    def write(self, data):
        self.sock.sendall(data)

    def read(self, n):
        try:
            return self.sock.recv(n)
        except socket.timeout:
            return b""

    def close(self):
        try:
            self.sock.close()
        except Exception:
            pass

    def describe(self):
        return "网络 " + self.addr


def list_serial_candidates():
    ports = glob.glob("/dev/ttyUSB*")
    # PCUI（ttyUSB1）排最前，其余按编号升序
    rest = [p for p in ports if p != PREFERRED_AT_PORT]
    rest.sort(key=lambda p: (len(p), p))
    ordered = [p for p in ports if p == PREFERRED_AT_PORT]
    return ordered + rest


def open_transport(cfg):
    """按配置建立连接。返回 Transport。"""
    if cfg["connection_type"] == "SERIAL":
        port = cfg["serial_port"]
        baud = cfg["serial_baudrate"]
        if port == AUTO_SERIAL_PORT:
            return None  # 由调用方走探测流程
        return SerialTransport(port, baud)
    return TcpTransport(cfg["network_host"], cfg["network_port"])


def probe_at(tp, timeout=0.8):
    """发 AT 等 OK/ERROR，能应答即为可用 AT 口。"""
    try:
        tp.write(b"AT\r")
    except Exception:
        return False
    deadline = time.monotonic() + timeout
    seen = b""
    while time.monotonic() < deadline:
        try:
            chunk = tp.read(256)
        except Exception:
            return False
        if chunk:
            seen += chunk
            text = seen.decode("utf-8", "replace")
            if "OK" in text or "ERROR" in text:
                return True
    return False


def detect_serial_port(cfg):
    """逐个探测候选串口，返回第一个能正常应答 AT 的已打开 Transport。"""
    candidates = list_serial_candidates()
    if not candidates:
        raise ConnectionError("没有找到任何 /dev/ttyUSB* 设备")
    log.info("自动探测 AT 口，候选: %s", " ".join(candidates))
    for port in candidates:
        try:
            tp = SerialTransport(port, cfg["serial_baudrate"])
        except Exception as e:
            log.info("  %s 打开失败: %s", port, e)
            continue
        if probe_at(tp):
            log.info("  %s 应答正常，选用该端口", port)
            return tp
        log.info("  %s 无有效应答，跳过", port)
        tp.close()
    raise ConnectionError("候选串口都没有正常应答 AT")


class PendingCmd:
    def __init__(self, echo, stream=None):
        self.echo = echo
        self.lines = []
        self.event = threading.Event()
        self.stream = stream


class ATClient:
    """维护到模组的唯一连接；只有一个读线程，命令应答与 URC 不会交错。"""

    def __init__(self, cfg):
        self.cfg = cfg
        self.tp = None
        self.conn_lock = threading.Lock()
        self.cmd_lock = threading.Lock()
        self.pend_lock = threading.Lock()
        self.pending = None
        self.last_cmd_at = 0.0
        self.urc = Queue(maxsize=1024)
        self.connected = threading.Event()
        self.long_cmd_active = threading.Event()
        self.long_cmd_end = 0.0
        self.on_scan_line = None  # 由 WSServer 注册（扫频结果行实时回调）

    # ---- 连接管理 -------------------------------------------------
    def connect(self):
        if self.cfg["connection_type"] == "SERIAL" and self.cfg["serial_port"] == AUTO_SERIAL_PORT:
            tp = detect_serial_port(self.cfg)
        else:
            tp = open_transport(self.cfg)
        self.conn_lock.acquire()
        self.tp = tp
        self.conn_lock.release()
        self.connected.set()
        log.info("已连接到 %s", tp.describe())
        t = threading.Thread(target=self.read_loop, args=(tp,), daemon=True)
        t.start()
        self.init_modem()

    def run(self, stop_event):
        backoff = 5.0
        while not stop_event.is_set():
            try:
                self.connect()
                backoff = 5.0
            except Exception as e:
                log.warning("连接模组失败，%ds 后重试: %s", int(backoff), e)
                if stop_event.wait(backoff):
                    return
                backoff = min(backoff + 5.0, 60.0)
                continue
            # 读线程退出（连接中断）后重连
            while not stop_event.is_set() and self.connected.is_set():
                stop_event.wait(1.0)
            if stop_event.is_set():
                return
            log.warning("连接已断开，准备重连")
            time.sleep(2.0)

    def teardown(self, tp):
        self.conn_lock.acquire()
        if self.tp is tp:
            self.tp = None
        self.conn_lock.release()
        self.connected.clear()
        tp.close()

    def is_connected(self):
        return self.connected.is_set() and self.tp is not None

    # ---- 读循环 ---------------------------------------------------
    def read_loop(self, tp):
        residual = b""
        try:
            while True:
                try:
                    chunk = tp.read(READ_BUF_SIZE)
                except Exception as e:
                    log.warning("模组连接中断: %s", e)
                    break
                if chunk:
                    residual = self.consume(residual + chunk)
                if not self.connected.is_set():
                    break
        finally:
            self.teardown(tp)

    def consume(self, data):
        while True:
            i = data.find(b"\n")
            if i < 0:
                break
            self.handle_line(data[:i].decode("utf-8", "replace").strip())
            data = data[i + 1:]

        # AT+CMGS 的输入提示符 "> " 后面没有换行，需要单独识别成一次应答结束
        rest = data.decode("utf-8", "replace").strip()
        if rest == ">":
            self.pend_lock.acquire()
            p = self.pending
            if p is not None:
                p.lines.append(">")
                p.event.set()
            self.pend_lock.release()
            if p is not None:
                return b""
        if len(data) > MAX_RESIDUAL_BYTES:
            log.warning("丢弃 %d 字节无法成行的数据", len(data))
            return b""
        return data

    def handle_line(self, line):
        if not line:
            return

        self.pend_lock.acquire()
        p = self.pending
        kept = False
        if p is not None:
            if line != p.echo and len(p.lines) < MAX_RESPONSE_LINES:
                p.lines.append(line)
                kept = True
        self.pend_lock.release()

        if kept and p.stream is not None:
            p.stream(line)

        if p is None:
            # 空闲期收到的任何数据都视为主动上报：交给处理器，并按原样推给前端
            self.emit(line, broadcast=True)
            return

        # 有命令等待时，只把「绝不可能是查询结果」的行也交给处理器
        if is_exclusive_urc(line):
            self.emit(line, broadcast=is_passthrough_urc(line))

        if is_terminator(line):
            p.event.set()

    def emit(self, line, broadcast):
        try:
            self.urc.put_nowait((line, broadcast))
        except Full:
            log.warning("主动上报队列已满，丢弃: %s", line)

    # ---- 命令收发 -------------------------------------------------
    def send_command(self, command, timeout=COMMAND_TIMEOUT, stream=None):
        self.cmd_lock.acquire()
        try:
            gap = COMMAND_GAP - (time.monotonic() - self.last_cmd_at)
            if gap > 0:
                time.sleep(gap)

            self.conn_lock.acquire()
            tp = self.tp
            self.conn_lock.release()
            if tp is None:
                raise ConnectionError("AT 通道未连接")

            if not command.endswith("\r"):
                command += "\r"

            p = PendingCmd(command.strip(), stream)
            self.pend_lock.acquire()
            self.pending = p
            self.pend_lock.release()
            try:
                tp.write(command.encode("utf-8"))
            except Exception as e:
                log.warning("写入 AT 命令失败: %s", e)
                tp.close()
                raise
            self.last_cmd_at = time.monotonic()

            p.event.wait(timeout)

            self.pend_lock.acquire()
            lines = list(p.lines)
            if self.pending is p:
                self.pending = None
            self.pend_lock.release()

            if not lines:
                raise TimeoutError("模组无应答")
            return ATResponse(lines)
        finally:
            self.cmd_lock.release()

    def send_long_command(self, command, timeout, stream=None):
        self.long_cmd_active.set()
        try:
            return self.send_command(command, timeout=timeout, stream=stream)
        finally:
            self.long_cmd_end = time.monotonic()
            self.long_cmd_active.clear()

    def interrupt(self, payload):
        """绕过命令锁直接写入，专供打断长命令（abcd）。"""
        self.conn_lock.acquire()
        tp = self.tp
        self.conn_lock.release()
        if tp is None:
            raise ConnectionError("AT 通道未连接")
        self.pend_lock.acquire()
        has_pending = self.pending is not None
        self.pend_lock.release()
        if not has_pending:
            raise RuntimeError("当前没有等待应答的命令")
        if not payload.endswith("\r"):
            payload += "\r"
        tp.write(payload.encode("utf-8"))

    def connection_type(self):
        return self.cfg["connection_type"]

    def modem_busy_recently(self):
        if self.long_cmd_active.is_set():
            return True
        return self.long_cmd_end != 0.0 and (time.monotonic() - self.long_cmd_end) < SCAN_RECOVERY_GRACE

    # ---- 初始化 ---------------------------------------------------
    def init_modem(self):
        try:
            self.send_command("AT+CMEE=2")
        except Exception as e:
            log.warning("开启详细错误码失败: %s", e)
        try:
            resp = self.send_command("AT+CNMI?")
            if not resp.contains("+CNMI: 2,1,0,2,0"):
                self.send_command("AT+CNMI=2,1,0,2,0")
        except Exception as e:
            log.warning("设置短信上报模式失败: %s", e)
        try:
            resp = self.send_command("AT+CMGF?")
            if not resp.contains("+CMGF: 0"):
                self.send_command("AT+CMGF=0")
        except Exception as e:
            log.warning("设置短信 PDU 模式失败: %s", e)
        try:
            self.send_command("AT+CLIP=1")
        except Exception as e:
            log.warning("开启来电号码显示失败: %s", e)


# ---------------------------------------------------------------- 通知（移植 notify.go）
class Notifier:
    def __init__(self, cfg):
        self.cfg = cfg
        self.queue = Queue(maxsize=256)
        self.log_file = cfg["log_file"]

    def _enabled(self, kind):
        return {
            "SMS": self.cfg["notify_sms"],
            "CALL": self.cfg["notify_call"],
            "MEMORY_FULL": self.cfg["notify_memory_full"],
            "SIGNAL": self.cfg["notify_signal"],
        }.get(kind, True)

    def notify(self, sender, content, kind, memory_full=False):
        if not self._enabled(kind):
            return
        if self.log_file:
            try:
                with open(self.log_file, "a", encoding="utf-8") as f:
                    if memory_full:
                        f.write("[%s] 存储空间已满警告\n" % datetime.now().strftime("%Y-%m-%d %H:%M:%S"))
                    else:
                        f.write("[%s] 发送者: %s\n内容: %s\n" % (
                            datetime.now().strftime("%Y-%m-%d %H:%M:%S"), sender, content))
                    f.write("-" * 50 + "\n")
            except Exception as e:
                log.error("写入通知日志失败: %s", e)

        if not self.cfg["wechat_webhook"]:
            return
        try:
            self.queue.put_nowait((sender, content))
        except Full:
            log.warning("通知队列已满，丢弃一条: %s", sender)

    @staticmethod
    def _combine(pending):
        # 同一发送方的多条合并展示
        parts = []
        for sender, content in pending:
            parts.append("【%s】\n%s" % (sender, content))
        return "\n\n".join(parts)

    def _send(self, content):
        payload = json.dumps({"msgtype": "text", "text": {"content": content}}).encode("utf-8")
        for attempt in range(1, NOTIFY_MAX_RETRIES + 1):
            try:
                req_data = payload
                from urllib.request import Request, urlopen
                req = Request(self.cfg["wechat_webhook"], data=req_data,
                              headers={"Content-Type": "application/json"})
                with urlopen(req, timeout=10) as resp:
                    result = json.loads(resp.read().decode("utf-8", "replace"))
                    if result.get("errcode") == 0:
                        log.info("企业微信通知发送成功")
                        return
                    log.warning("企业微信返回错误: %s", result)
                    return
            except Exception as e:
                log.warning("企业微信发送失败 (%d/%d): %s", attempt, NOTIFY_MAX_RETRIES, e)
                if attempt < NOTIFY_MAX_RETRIES:
                    time.sleep(attempt)
        log.error("企业微信通知已达最大重试次数，放弃发送")

    def run(self, stop_event):
        pending = []
        deadline = None
        last_send = 0.0
        while not stop_event.is_set():
            now = time.monotonic()
            # 到点即合并发送：60 秒内的事件合并成一条（沿用旧实现的节流策略）
            if pending and deadline is not None and now >= deadline:
                body = self._combine(pending)
                pending = []
                last_send = now
                deadline = None
                threading.Thread(target=self._send, args=(body,), daemon=True).start()
                continue
            if pending and deadline is not None:
                timeout = max(0.05, min(0.5, deadline - now))
            else:
                timeout = 0.5
            try:
                item = self.queue.get(timeout=timeout)
                pending.append(item)
                if len(pending) >= NOTIFY_MAX_PENDING:
                    log.warning("待发通知超过 %d 条，丢弃最旧的一条", NOTIFY_MAX_PENDING)
                    pending.pop(0)
                if deadline is None:
                    deadline = now + max(0.0, NOTIFY_INTERVAL - (now - last_send))
            except Empty:
                pass
        # 退出前把攒下的消息发出去
        if pending:
            self._send(self._combine(pending))


# ---------------------------------------------------------------- URC 分发（移植 urc.go）
CLIP_RE = re.compile(r'\+CLIP: *"([^"]+)"')
CMTI_RE = re.compile(r'^\+CMTI: "(ME|SM)",(\d+)')
REG_RE = re.compile(r'\+C[A-Z0-9]*REG:\s*\d+\s*,\s*(\d+)')

PDCP_FIELDS = [
    ("id", False), ("pduSessionId", False), ("discardTimerLen", False),
    ("avgDelay", True), ("minDelay", True), ("maxDelay", True),
    ("highPriQueMaxBuffTime", True), ("lowPriQueMaxBuffTime", True),
    ("highPriQueBuffPktNums", False), ("lowPriQueBuffPktNums", False),
    ("ulPdcpRate", False), ("dlPdcpRate", False),
    ("ulDiscardCnt", False), ("dlDiscardCnt", False),
]


def split_fields(line, prefix):
    i = line.find(prefix)
    body = line[i + len(prefix):] if i >= 0 else line
    return [p.strip() for p in body.strip().split(",")]


def signal_level(rsrp):
    if rsrp >= -85:
        return "优秀"
    if rsrp >= -95:
        return "良好"
    if rsrp >= -105:
        return "一般"
    return "较差"


def hex_to_dec(s):
    try:
        return str(int(s.strip(), 16))
    except ValueError:
        return s


class Dispatcher:
    """处理模组的主动上报：来电、新短信、存储满、信号变化、PDCP 统计。"""

    def __init__(self, client, notifier, ws):
        self.client = client
        self.notifier = notifier
        self.ws = ws
        self.last_call_number = ""
        self.last_call_at = 0.0
        self.call_state = "idle"
        self.last_rsrp = None
        self.last_sysmode = ""
        self.memory_full_notified = False
        self.partials = {}

    def run(self, stop_event):
        while not stop_event.is_set():
            try:
                line, broadcast = self.urc_get()
            except Empty:
                continue
            try:
                if broadcast:
                    self.ws.broadcast("raw_data", line)
                self.handle(line)
            except Exception as e:
                log.warning("主动上报处理异常（仅丢这一条）: %s", e)

    def urc_get(self):
        try:
            return self.client.urc.get(timeout=0.5)
        except Empty:
            raise

    # ---- 来电 ----
    def handle_call(self, line):
        now = time.monotonic()
        ts = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
        if line in ("RING", "IRING", "^IRING"):
            self.call_state = "ringing"
            return

        if line.startswith("+CLIP:"):
            m = CLIP_RE.search(line)
            if not m:
                return
            number = m.group(1)
            same_call = (number == self.last_call_number and
                         now - self.last_call_at <= CALL_DEDUP_WINDOW and
                         self.call_state != "idle")
            if same_call:
                return
            self.last_call_number = number
            self.last_call_at = now
            self.call_state = "ringing"
            self.notifier.notify(SENDER_CALL,
                                 "时间：%s\n号码：%s\n状态：来电振铃" % (ts, number), "CALL")
            self.ws.broadcast("incoming_call", {"time": ts, "number": number, "state": "ringing"})
            return

        if "^CEND:" in line or line == "NO CARRIER":
            if self.last_call_number:
                self.notifier.notify(SENDER_CALL,
                                     "时间：%s\n号码：%s\n状态：通话结束" % (ts, self.last_call_number), "CALL")
                self.ws.broadcast("incoming_call", {
                    "time": ts, "number": self.last_call_number, "state": "ended"})
            self.last_call_number = ""
            self.last_call_at = 0.0
            self.call_state = "idle"

    # ---- 存储满 ----
    def handle_memory_full(self):
        if self.memory_full_notified:
            return
        self.memory_full_notified = True
        self.notifier.notify("", "", "MEMORY_FULL", memory_full=True)

    # ---- 新短信 ----
    def handle_new_sms(self, line):
        m = CMTI_RE.match(line)
        if not m:
            return
        storage, index = m.group(1), m.group(2)
        log.info("收到新短信，存储区: %s，索引: %s", storage, index)
        try:
            resp = self.client.send_command("AT+CMGR=" + index)
        except Exception as e:
            log.warning("读取短信 %s 失败: %s", index, e)
            return
        for sms in self.parse_sms_response(resp):
            if sms["partial"]:
                self.assemble_partial(sms)
                continue
            self.notifier.notify(sms["sender"], sms["content"], "SMS")
            self.ws.broadcast("new_sms", {
                "sender": sms["sender"], "content": sms["content"],
                "time": sms["date"].strftime("%Y-%m-%d %H:%M:%S")})

    @staticmethod
    def parse_sms_response(resp):
        out = []
        lines = resp.lines
        for i in range(len(lines)):
            if not lines[i].startswith("+CMG") or i + 1 >= len(lines):
                continue
            pdu = lines[i + 1].strip()
            if not pdu or not re.fullmatch(r"[0-9a-fA-F]+", pdu):
                continue
            try:
                out.append(decode_incoming_pdu(pdu))
            except Exception as e:
                log.warning("PDU 解析失败: %s", e)
        return out

    def assemble_partial(self, sms):
        now = time.monotonic()
        for k in list(self.partials):
            if now - self.partials[k]["received"] > PARTIAL_SMS_TTL:
                log.warning("清理过期的分段短信: %s", k)
                del self.partials[k]
        if len(self.partials) >= MAX_PARTIAL_SMS:
            oldest = min(self.partials, key=lambda k: self.partials[k]["received"])
            log.warning("分段短信缓存超限，删除最旧的: %s", oldest)
            del self.partials[oldest]

        p = sms["partial"]
        key = "%s_%d" % (sms["sender"], p["reference"])
        entry = self.partials.setdefault(key, {
            "sender": sms["sender"], "total": p["parts_count"],
            "parts": {}, "received": now})
        entry["parts"][p["part_number"]] = sms["content"]

        if entry["total"] <= 0 or len(entry["parts"]) < entry["total"]:
            return
        full = "".join(entry["parts"].get(i, "") for i in range(1, entry["total"] + 1))
        del self.partials[key]
        self.notifier.notify(sms["sender"], full, "SMS")
        self.ws.broadcast("new_sms", {
            "sender": sms["sender"], "content": full,
            "time": sms["date"].strftime("%Y-%m-%d %H:%M:%S"), "isComplete": True})

    # ---- 信号 ----
    def handle_signal(self, line):
        line = line.split("\n")[0]
        rsrp = None
        sysmode = ""

        if "^CERSSI:" in line:
            parts = split_fields(line, "^CERSSI:")
            # 第 19 个字段是 RSRP，字段不足就跳过
            if len(parts) >= 20:
                try:
                    rsrp = float(parts[18])
                    sysmode = "4G/5G"
                except ValueError:
                    return
        elif "^HCSQ:" in line:
            parts = split_fields(line, "^HCSQ:")
            if len(parts) >= 4:
                try:
                    raw = float(parts[1])
                    rsrp = -140 + raw
                    sysmode = parts[0].strip('"')
                except ValueError:
                    return

        if rsrp is None:
            return

        changed = (self.last_rsrp is None or
                   abs(rsrp - self.last_rsrp) >= SIGNAL_CHANGE_THRESHOLD or
                   sysmode != self.last_sysmode)
        if not changed:
            return

        mode_switched = sysmode != self.last_sysmode
        self.last_rsrp = rsrp
        self.last_sysmode = sysmode
        self.notify_signal(rsrp, mode_switched)

    def query_monsc(self):
        info = {"RAT": "未知", "ARFCN": "", "CellID": "", "PCI": "", "TAC": "",
                "RSRP": "", "RSRQ": "", "SINR": "", "RSSI": ""}
        try:
            resp = self.client.send_command("AT^MONSC")
        except Exception:
            return info
        for line in resp.lines:
            if not line.startswith("^MONSC:"):
                continue
            parts = split_fields(line, "^MONSC:")
            if len(parts) < 2:
                return info
            info["RAT"] = parts[0].strip('"')
            if info["RAT"] == "NR" and len(parts) >= 11:
                info["ARFCN"], info["CellID"] = parts[3], parts[5]
                info["PCI"], info["TAC"] = hex_to_dec(parts[6]), parts[7]
                info["RSRP"], info["RSRQ"], info["SINR"] = parts[8], parts[9], parts[10]
            elif info["RAT"] == "LTE" and len(parts) >= 10:
                info["ARFCN"], info["CellID"] = parts[3], parts[4]
                info["PCI"], info["TAC"] = hex_to_dec(parts[5]), parts[6]
                info["RSRP"], info["RSRQ"], info["RSSI"] = parts[7], parts[8], parts[9]
            return info
        return info

    def notify_signal(self, rsrp, mode_switched):
        info = self.query_monsc()
        b = []
        if mode_switched:
            b.append("⚡ 网络切换提醒")
        b.append("📶 信号变动通知")
        b.append("时间: %s" % datetime.now().strftime("%Y-%m-%d %H:%M:%S"))
        b.append("制式: %s" % info["RAT"])
        b.append("信号: %s" % signal_level(rsrp))
        if info["RAT"] == "NR":
            b.append("RSRP: %s dBm" % (info["RSRP"] or "未知"))
            b.append("RSRQ: %s dB" % (info["RSRQ"] or "未知"))
            b.append("SINR: %s dB" % (info["SINR"] or "未知"))
        elif info["RAT"] == "LTE":
            b.append("RSRP: %s dBm" % (info["RSRP"] or "未知"))
            b.append("RSRQ: %s dB" % (info["RSRQ"] or "未知"))
            b.append("RSSI: %s dBm" % (info["RSSI"] or "未知"))
        else:
            self.notifier.notify(SENDER_SIGNAL, "\n".join(b), "SIGNAL")
            return
        b.append("")
        b.append("📡 小区信息:")
        b.append("频点: %s" % (info["ARFCN"] or "未知"))
        b.append("PCI: %s" % (info["PCI"] or "未知"))
        b.append("TAC: %s" % (info["TAC"] or "未知"))
        b.append("小区ID: %s" % (info["CellID"] or "未知"))
        self.notifier.notify(SENDER_SIGNAL, "\n".join(b), "SIGNAL")

    # ---- PDCP 统计 ----
    def handle_pdcp(self, line):
        parts = split_fields(line, "^PDCPDATAINFO:")
        if len(parts) < len(PDCP_FIELDS):
            return
        data = {}
        for i, (name, tenth) in enumerate(PDCP_FIELDS):
            try:
                v = float(parts[i])
            except ValueError:
                return
            data[name] = v / 10 if tenth else int(v)
        self.ws.broadcast("pdcp_data", data)

    # ---- 总入口 ----
    def handle(self, line):
        def is_call(l):
            if l in ("RING", "IRING", "^IRING", "NO CARRIER"):
                return True
            return l.startswith("+CLIP:") or "^CEND:" in l

        def is_mem_full(l):
            return "CMS ERROR: 322" in l or "MEMORY FULL" in l or "^SMMEMFULL" in l

        if is_call(line):
            self.handle_call(line)
        elif is_mem_full(line):
            self.handle_memory_full()
        elif CMTI_RE.match(line):
            self.handle_new_sms(line)
        elif "^CERSSI:" in line or "^HCSQ:" in line:
            self.handle_signal(line)
        elif line.startswith("^PDCPDATAINFO:"):
            self.handle_pdcp(line)


# ---------------------------------------------------------------- 定时锁频（移植 schedule.go）
LTE_BAND_ARFCN = {
    1: (0, 599), 2: (600, 1199), 3: (1200, 1949), 4: (1950, 2399), 5: (2400, 2649),
    7: (2750, 3449), 8: (3450, 3799), 12: (5010, 5179), 13: (5180, 5279),
    17: (5730, 5849), 18: (5850, 5999), 19: (6000, 6149), 20: (6150, 6449),
    25: (8040, 8689), 26: (8690, 9039), 28: (9210, 9659), 38: (37750, 38249),
    39: (38250, 38649), 40: (38650, 39649), 41: (39650, 41589), 42: (41590, 43589),
    43: (43590, 45589), 66: (66436, 67335),
}
NR_BAND_ARFCN = {
    1: (422000, 434000), 2: (386000, 398000), 3: (361000, 376000),
    5: (173800, 178800), 7: (524000, 538000), 8: (185000, 192000),
    12: (145800, 149200), 20: (158200, 164200), 25: (386000, 399000),
    28: (151600, 160600), 34: (402000, 405000), 38: (514000, 524000),
    39: (376000, 384000), 40: (460000, 480000), 41: (499200, 537999),
    48: (636667, 646666), 66: (422000, 440000), 71: (123400, 130400),
    77: (620000, 680000), 78: (620000, 653333), 79: (693334, 733333),
    257: (2054166, 2104165), 258: (2016667, 2070832),
    260: (2229166, 2279165), 261: (2070833, 2084999),
}
NR_30KHZ_BANDS = {41, 48, 77, 78, 79}
NR_MMWAVE_BANDS = {257, 258, 260, 261}


def split_list(v):
    return [p.strip() for p in str(v).split(",") if p.strip()]


def parse_hhmm(v):
    try:
        h, m = str(v).strip().split(":", 1)
        hour, minute = int(h.strip()), int(m.strip())
        if 0 <= hour <= 23 and 0 <= minute <= 59:
            return hour * 60 + minute
    except Exception:
        pass
    return None


def validate_pairs(bands, arfcns, table, kind):
    for i in range(len(bands)):
        try:
            band = int(bands[i])
            arfcn = int(arfcns[i])
        except ValueError:
            log.warning("%s 锁频参数不是数字: 频段 %r 频点 %r", kind, bands[i], arfcns[i])
            return False
        rng = table.get(band)
        if rng and not (rng[0] <= arfcn <= rng[1]):
            log.warning("%s 频段 %d 与频点 %d 不匹配(应在 %d-%d)", kind, band, arfcn, rng[0], rng[1])
            return False
    return True


def auto_detect_scs(bands):
    out = []
    for b in bands:
        try:
            band = int(b)
        except ValueError:
            out.append("1")
            continue
        if band in NR_MMWAVE_BANDS:
            out.append("3")
        elif band in NR_30KHZ_BANDS:
            out.append("1")
        else:
            out.append("0")
    return out


class Scheduler:
    def __init__(self, cfg, client, notifier):
        self.client = client
        self.notifier = notifier
        self.mu = threading.Lock()
        self.cfg = cfg
        self.last_service_at = time.monotonic()
        self.current_mode = ""
        self.switch_count = 0
        self.last_applied = None
        self.applied = False
        self.announced = False

    def get_cfg(self):
        with self.mu:
            return dict(self.cfg)

    def set_cfg(self, cfg):
        with self.mu:
            self.cfg = dict(cfg)
            self.announced = False

    def run(self, stop_event):
        while not stop_event.is_set():
            cfg = self.get_cfg()
            if stop_event.wait(max(10, cfg["schedule_check_interval"])):
                return
            cfg = self.get_cfg()
            self.announce(cfg)
            if not cfg["schedule_enabled"] or not self.client.is_connected():
                continue
            # 扫频独占模组期间跳过，避免一次扫描就让「无服务超时」到点
            if self.client.long_cmd_active.is_set():
                with self.mu:
                    self.last_service_at = time.monotonic()
                continue
            try:
                self.tick(cfg)
            except Exception as e:
                log.warning("定时锁频检测异常（仅丢这一轮）: %s", e)

    def announce(self, cfg):
        with self.mu:
            if self.announced:
                return
            self.announced = True
        if not cfg["schedule_enabled"]:
            log.info("定时锁频未启用")
            return
        log.info("定时锁频已启用：检测间隔 %ss，无服务超时 %ss",
                 cfg["schedule_check_interval"], cfg["schedule_timeout"])
        log.info("  夜间模式 %s (%s-%s)，日间模式 %s",
                 "启用" if cfg["schedule_night_enabled"] else "禁用",
                 cfg["schedule_night_start"], cfg["schedule_night_end"],
                 "启用" if cfg["schedule_day_enabled"] else "禁用")

    def _lock(self, cfg, period, kind):
        return {
            "type": cfg["schedule_%s_%s_type" % (period, kind)],
            "bands": cfg["schedule_%s_%s_bands" % (period, kind)],
            "arfcns": cfg["schedule_%s_%s_arfcns" % (period, kind)],
            "scs_types": cfg["schedule_%s_%s_scs_types" % (period, kind)],
            "pcis": cfg["schedule_%s_%s_pcis" % (period, kind)],
        }

    def target_mode(self, cfg, now):
        start, end = parse_hhmm(cfg["schedule_night_start"]), parse_hhmm(cfg["schedule_night_end"])
        night = False
        if start is not None and end is not None:
            cur = now.hour * 60 + now.minute
            night = (cur >= start or cur < end) if start > end else (start <= cur < end)
        if night and cfg["schedule_night_enabled"]:
            return "夜间"
        if not night and cfg["schedule_day_enabled"]:
            return "日间"
        return ""

    def lock_for(self, cfg, mode):
        if mode == "夜间":
            return {"LTE": self._lock(cfg, "night", "lte"), "NR": self._lock(cfg, "night", "nr")}
        if mode == "日间":
            return {"LTE": self._lock(cfg, "day", "lte"), "NR": self._lock(cfg, "day", "nr")}
        return {"LTE": {"type": 0}, "NR": {"type": 0}}

    def has_service(self):
        # C5GREG 覆盖 SA 组网，CEREG 覆盖 LTE 与 NSA，CREG 兜底
        for cmd in ("AT+C5GREG?", "AT+CEREG?", "AT+CREG?"):
            try:
                resp = self.client.send_command(cmd)
            except Exception:
                continue
            if registered(resp.text()):
                return True
        return False

    def tick(self, cfg):
        target = self.target_mode(cfg, datetime.now())
        want = self.lock_for(cfg, target)

        with self.mu:
            mode, applied, last = self.current_mode, self.applied, self.last_applied

        if target != mode or not applied or want != last:
            if target:
                log.info("时段切换: %s -> %s", mode or "无", target)
                self.apply_lock(cfg, want, target)
            elif applied:
                log.info("当前时段无需锁频，解锁所有频段")
                self.apply_lock(cfg, {"LTE": {"type": 0}, "NR": {"type": 0}}, "解锁")
            with self.mu:
                self.current_mode = target
                self.last_applied = want
                self.applied = True

        if self.has_service():
            with self.mu:
                self.last_service_at = time.monotonic()
            return

        # 刚扫过频（模组重新驻留中）不做无服务判定
        if self.client.modem_busy_recently():
            log.debug("刚扫过频，跳过无服务判定")
            with self.mu:
                self.last_service_at = time.monotonic()
            return

        with self.mu:
            down = time.monotonic() - self.last_service_at
        limit = cfg["schedule_timeout"]
        if down < limit:
            log.debug("无服务已持续 %ds", int(down))
            return

        # 锁频锁到了没有覆盖的小区会一直无服务，这时解锁比守着配置更重要
        log.warning("网络无服务已持续 %ds，解锁频段恢复", int(down))
        self.apply_lock(cfg, {"LTE": {"type": 0}, "NR": {"type": 0}}, "恢复")
        with self.mu:
            self.last_service_at = time.monotonic()

    def apply_lock(self, cfg, locks, mode):
        with self.mu:
            self.switch_count += 1
            count = self.switch_count
        log.info("开始切换到%s锁频设置 (第 %d 次)", mode, count)

        done = []
        if cfg["schedule_toggle_airplane"]:
            try:
                resp = self.client.send_command("AT+CFUN=0")
                if resp.ok():
                    log.info("已进入飞行模式")
                    time.sleep(2)
                else:
                    log.warning("进入飞行模式失败")
            except Exception:
                log.warning("进入飞行模式失败")

        lte = self.lte_command(cfg, locks["LTE"])
        if lte:
            if self.run_lock_command(lte[0], lte[1]):
                done.append(lte[1])
            time.sleep(1)

        nr = self.nr_command(cfg, locks["NR"])
        if nr:
            if self.run_lock_command(nr[0], nr[1]):
                done.append(nr[1])
            time.sleep(1)

        if cfg["schedule_toggle_airplane"]:
            try:
                resp = self.client.send_command("AT+CFUN=1")
                if resp.ok():
                    log.info("已退出飞行模式")
                    done.append("切飞行模式")
                else:
                    log.warning("退出飞行模式失败")
            except Exception:
                log.warning("退出飞行模式失败")
            time.sleep(3)

        actions = "、".join(done) if done else "未执行任何操作"
        self.notifier.notify(SENDER_SIGNAL,
                             "🔄 定时锁频切换\n时间: %s\n模式: %s\nLTE: %s\nNR: %s\n执行操作: %s\n切换次数: 第 %d 次" % (
                                 datetime.now().strftime("%Y-%m-%d %H:%M:%S"), mode,
                                 lock_summary("LTE", locks["LTE"]), lock_summary("NR", locks["NR"]),
                                 actions, count), "SIGNAL")
        log.info("定时锁频切换完成: %s", actions)

    def run_lock_command(self, cmd, action):
        log.info("下发 %s: %s", action, cmd)
        try:
            resp = self.client.send_command(cmd)
        except Exception as e:
            log.warning("%s 失败: %s", action, e)
            return False
        if not resp.ok():
            log.warning("%s 失败: %s", action, resp.text())
            return False
        log.info("%s 成功", action)
        return True

    def lte_command(self, cfg, lock):
        ltype = lock.get("type", 0)
        if ltype <= 0:
            if not cfg["schedule_unlock_lte"]:
                return None
            return ("AT^LTEFREQLOCK=0", "LTE解锁")
        bands = split_list(lock.get("bands", ""))
        if not bands:
            return None
        if ltype == 3:
            return ('AT^LTEFREQLOCK=3,0,%d,"%s"' % (len(bands), ",".join(bands)),
                    "LTE锁频(类型%d)" % ltype)
        if ltype in (1, 2):
            arfcns = split_list(lock.get("arfcns", ""))
            if len(arfcns) != len(bands):
                log.warning("LTE 锁频：频段与频点数量不一致(%d/%d)，改为解锁", len(bands), len(arfcns))
                return ("AT^LTEFREQLOCK=0", "LTE解锁")
            if not validate_pairs(bands, arfcns, LTE_BAND_ARFCN, "LTE"):
                return ("AT^LTEFREQLOCK=0", "LTE解锁")
            if ltype == 1:
                return ('AT^LTEFREQLOCK=1,0,%d,"%s","%s"' % (len(bands), ",".join(bands), ",".join(arfcns)),
                        "LTE锁频(类型1)")
            pcis = split_list(lock.get("pcis", ""))
            if len(pcis) != len(bands):
                log.warning("LTE 小区锁定：PCI 数量与频段不一致(%d/%d)，改为解锁", len(bands), len(pcis))
                return ("AT^LTEFREQLOCK=0", "LTE解锁")
            return ('AT^LTEFREQLOCK=2,0,%d,"%s","%s","%s"' % (
                len(bands), ",".join(bands), ",".join(arfcns), ",".join(pcis)), "LTE锁频(类型2)")
        return ("AT^LTEFREQLOCK=0", "LTE解锁")

    def nr_command(self, cfg, lock):
        ltype = lock.get("type", 0)
        if ltype <= 0:
            if not cfg["schedule_unlock_nr"]:
                return None
            return ("AT^NRFREQLOCK=0", "NR解锁")
        bands = split_list(lock.get("bands", ""))
        if not bands:
            return None
        if ltype == 3:
            return ('AT^NRFREQLOCK=3,0,%d,"%s"' % (len(bands), ",".join(bands)),
                    "NR锁频(类型%d)" % ltype)
        if ltype in (1, 2):
            arfcns = split_list(lock.get("arfcns", ""))
            if len(arfcns) != len(bands):
                log.warning("NR 锁频：频段与频点数量不一致(%d/%d)，改为解锁", len(bands), len(arfcns))
                return ("AT^NRFREQLOCK=0", "NR解锁")
            scs = split_list(lock.get("scs_types", "")) or auto_detect_scs(bands)
            if len(scs) != len(bands):
                log.warning("NR 锁频：SCS 数量与频段不一致(%d/%d)，改为解锁", len(bands), len(scs))
                return ("AT^NRFREQLOCK=0", "NR解锁")
            if not validate_pairs(bands, arfcns, NR_BAND_ARFCN, "NR"):
                return ("AT^NRFREQLOCK=0", "NR解锁")
            if ltype == 1:
                return ('AT^NRFREQLOCK=1,0,%d,"%s","%s","%s"' % (
                    len(bands), ",".join(bands), ",".join(arfcns), ",".join(scs)), "NR锁频(类型1)")
            pcis = split_list(lock.get("pcis", ""))
            if len(pcis) != len(bands):
                log.warning("NR 小区锁定：PCI 数量与频段不一致(%d/%d)，改为解锁", len(bands), len(pcis))
                return ("AT^NRFREQLOCK=0", "NR解锁")
            return ('AT^NRFREQLOCK=2,0,%d,"%s","%s","%s","%s"' % (
                len(bands), ",".join(bands), ",".join(arfcns), ",".join(scs), ",".join(pcis)),
                "NR锁频(类型2)")
        return ("AT^NRFREQLOCK=0", "NR解锁")

    def status(self):
        cfg = self.get_cfg()
        with self.mu:
            dto = {
                "current_mode": self.current_mode or "无",
                "next_switch": next_switch_at(cfg, datetime.now()),
                "switch_count": self.switch_count,
                "applied": self.applied,
            }
        return dto

    def to_dto(self):
        cfg = self.get_cfg()
        return {
            "enabled": cfg["schedule_enabled"],
            "check_interval": cfg["schedule_check_interval"],
            "timeout": cfg["schedule_timeout"],
            "unlock_lte": cfg["schedule_unlock_lte"],
            "unlock_nr": cfg["schedule_unlock_nr"],
            "toggle_airplane": cfg["schedule_toggle_airplane"],
            "night": {
                "enabled": cfg["schedule_night_enabled"],
                "start": cfg["schedule_night_start"],
                "end": cfg["schedule_night_end"],
                "lte": {k: cfg["schedule_night_lte_%s" % k] for k in ("type", "bands", "arfcns", "scs_types", "pcis")},
                "nr": {k: cfg["schedule_night_nr_%s" % k] for k in ("type", "bands", "arfcns", "scs_types", "pcis")},
            },
            "day": {
                "enabled": cfg["schedule_day_enabled"],
                "start": "",
                "lte": {k: cfg["schedule_day_lte_%s" % k] for k in ("type", "bands", "arfcns", "scs_types", "pcis")},
                "nr": {k: cfg["schedule_day_nr_%s" % k] for k in ("type", "bands", "arfcns", "scs_types", "pcis")},
            },
        }


def lock_summary(kind, lock):
    if lock.get("type", 0) > 0 and str(lock.get("bands", "")).strip():
        return "%s类型%d" % (kind, lock["type"])
    return kind + "解锁"


def registered(text):
    for m in REG_RE.finditer(text):
        if m.group(1) in ("1", "5"):
            return True
    return False


def next_switch_at(cfg, now):
    """下一个时段边界（近似：取今天/最近的 night_start 与 night_end）。"""
    cands = []
    for key in ("schedule_night_start", "schedule_night_end"):
        hm = parse_hhmm(cfg.get(key, ""))
        if hm is not None:
            t = now.replace(hour=hm // 60, minute=hm % 60, second=0, microsecond=0)
            if t <= now:
                t += timedelta(days=1)
            cands.append(t)
    if not cands:
        return "无"
    return min(cands).strftime("%Y-%m-%d %H:%M")


def validate_sched_dto(d):
    if d.get("check_interval", 0) < 10:
        raise ValueError("检测间隔不能小于 10 秒")
    if d.get("timeout", 0) < 30:
        raise ValueError("无服务超时不能小于 30 秒")
    night = d.get("night", {})
    if parse_hhmm(night.get("start", "")) is None:
        raise ValueError("夜间开始时间格式应为 HH:MM，当前为 %r" % night.get("start"))
    if parse_hhmm(night.get("end", "")) is None:
        raise ValueError("夜间结束时间格式应为 HH:MM，当前为 %r" % night.get("end"))
    for p in ("night", "day"):
        for kind in ("lte", "nr"):
            for field in ("bands", "arfcns", "pcis", "scs_types"):
                val = d.get(p, {}).get(kind, {}).get(field, "")
                if val and not isinstance(val, str):
                    raise ValueError("%s %s 的 %s 格式错误" % (p, kind, field))


def sched_dto_to_cfg(d):
    cfg = dict(DEFAULTS)
    cfg.update({
        "schedule_enabled": bool(d.get("enabled")),
        "schedule_check_interval": int(d.get("check_interval", 60)),
        "schedule_timeout": int(d.get("timeout", 180)),
        "schedule_unlock_lte": bool(d.get("unlock_lte")),
        "schedule_unlock_nr": bool(d.get("unlock_nr")),
        "schedule_toggle_airplane": bool(d.get("toggle_airplane")),
        "schedule_night_enabled": bool(d.get("night", {}).get("enabled")),
        "schedule_night_start": d.get("night", {}).get("start", ""),
        "schedule_night_end": d.get("night", {}).get("end", ""),
        "schedule_day_enabled": bool(d.get("day", {}).get("enabled")),
    })
    for p, period in (("night", "night"), ("day", "day")):
        for kind in ("lte", "nr"):
            item = d.get(p, {}).get(kind, {})
            for field in ("type", "bands", "arfcns", "scs_types", "pcis"):
                cfg["schedule_%s_%s_%s" % (period, kind, field)] = item.get(field, 0 if field == "type" else "")
    return cfg


def write_schedule_uci(cfg):
    entries = []
    flag = lambda b: "1" if b else "0"
    entries += [
        ("schedule_enabled", flag(cfg["schedule_enabled"])),
        ("schedule_check_interval", str(cfg["schedule_check_interval"])),
        ("schedule_timeout", str(cfg["schedule_timeout"])),
        ("schedule_unlock_lte", flag(cfg["schedule_unlock_lte"])),
        ("schedule_unlock_nr", flag(cfg["schedule_unlock_nr"])),
        ("schedule_toggle_airplane", flag(cfg["schedule_toggle_airplane"])),
        ("schedule_night_enabled", flag(cfg["schedule_night_enabled"])),
        ("schedule_night_start", cfg["schedule_night_start"]),
        ("schedule_night_end", cfg["schedule_night_end"]),
        ("schedule_day_enabled", flag(cfg["schedule_day_enabled"])),
    ]
    for period in ("night", "day"):
        for kind in ("lte", "nr"):
            base = "schedule_%s_%s" % (period, kind)
            for field in ("type", "bands", "arfcns", "scs_types", "pcis"):
                entries.append((base + "_" + field, str(cfg[base + "_" + field])))
    uci_set_many(entries)


# ---------------------------------------------------------------- WebSocket 服务（移植 wsserver.go）
def normalize_syscfgex(command):
    if not command.startswith("AT^SYSCFGEX"):
        return command
    cleaned = command.replace("\r", "").replace("\n", "").replace("OK", "")
    if ',"",""' in cleaned:
        parts = cleaned.split(",")
        if len(parts) >= 5:
            bands = parts[4].strip('"')
            cleaned = ",".join(parts[:4]) + ',"%s","",""' % bands
    return cleaned


class CellScanManager:
    """扫频在后台线程跑：WebSocket 读循环立刻空出来接收打断命令，
    扫描结果通过 cellscan 推送实时送给前端。"""

    def __init__(self, client, ws, scan_timeout):
        self.client = client
        self.ws = ws
        self.scan_timeout = scan_timeout or DEFAULT_SCAN_TIMEOUT
        self.lock = threading.Lock()
        self.running = False
        self.aborted = False
        self.lines = []

    def is_scan(self, command):
        return command.strip().upper().startswith("AT^CELLSCAN")

    def handle(self, command):
        c = command.strip()
        if c.upper() == "AT^CELLSCAN=STATE":
            with self.lock:
                running, count = self.running, len(self.lines)
            if not running:
                return ATResponse(["^CELLSCAN: IDLE", "OK"])
            return ATResponse(["^CELLSCAN: RUNNING,%d" % count, "OK"])

        if c.upper() == "AT^CELLSCAN=ABORT":
            # 全程持锁：避免在「判断还在跑」和「写打断串」之间扫频正好结束
            with self.lock:
                if not self.running:
                    raise RuntimeError("当前没有正在进行的扫频")
                self.client.interrupt(CELLSCAN_ABORT_TOKEN)
                self.aborted = True
            log.info("已下发扫频打断字符串")
            return ATResponse(["OK"])

        if self.is_scan(c):
            with self.lock:
                if self.running:
                    raise RuntimeError("扫频正在进行中，请先取消")
                self.running = True
                self.aborted = False
                self.lines = []
            threading.Thread(target=self._run, args=(c,), daemon=True).start()
            # 立刻应答，让前端的命令队列不被这条几分钟的命令堵住
            return ATResponse(["^CELLSCAN: STARTED", "OK"])
        return None

    def _on_line(self, line):
        line = line.strip()
        if not line.startswith("^CELLSCAN:"):
            return
        with self.lock:
            self.lines.append(line)
            count = len(self.lines)
        self.ws.broadcast("cellscan", {"state": "running", "cell": line, "count": count})

    def _run(self, command):
        try:
            timeout = self.scan_timeout
            log.info("开始扫频: %s (超时 %ss)", command, int(timeout))
            try:
                resp = self.client.send_long_command(command, timeout + 10.0, stream=self._on_line)
            except Exception as e:
                log.warning("扫频失败: %s", e)
                with self.lock:
                    lines = list(self.lines)
                self.ws.broadcast("cellscan", {"state": "error", "error": str(e),
                                               "lines": lines, "count": len(lines)})
                return

            with self.lock:
                lines = list(self.lines)
                aborted = self.aborted
                self.running = False
                self.aborted = False
                self.lines = []

            if resp.has_error():
                log.warning("扫频被模组拒绝: %s", resp.text())
                self.ws.broadcast("cellscan", {"state": "error", "error": resp.text(),
                                               "lines": lines, "count": len(lines)})
            else:
                state = "aborted" if aborted else "done"
                log.info("扫频结束(%s): 共 %d 个小区", state, len(lines))
                self.ws.broadcast("cellscan", {"state": state, "lines": lines, "count": len(lines)})
        finally:
            with self.lock:
                stuck = self.running
                self.running = False
                self.aborted = False
                self.lines = []
            if stuck:
                self.ws.broadcast("cellscan", {"state": "error", "error": "扫频异常结束"})


class WSServer:
    def __init__(self, client, scheduler, cfg):
        self.client = client
        self.scheduler = scheduler
        self.cfg = cfg
        self.auth_key = cfg["websocket_auth_key"]
        self.scan = CellScanManager(client, self, cfg["cellscan_timeout"])
        self.loop = None
        self.clients = set()
        self.clients_lock = threading.Lock()

    # ---- 广播（线程安全：URC/扫频线程都会调用）----
    def broadcast(self, msg_type, data):
        if self.loop is None:
            return
        msg = json.dumps({"type": msg_type, "data": data}, ensure_ascii=False)
        try:
            asyncio.run_coroutine_threadsafe(self._broadcast(msg), self.loop)
        except RuntimeError:
            pass

    async def _broadcast(self, msg):
        with self.clients_lock:
            targets = list(self.clients)
        for c in targets:
            c.try_send(msg)

    # ---- 命令处理 ----
    def run_command(self, command):
        log.debug("收到 AT 命令: %s", command.strip())

        # AT+CONNECT? 不是真的 AT 命令，用来让前端知道当前走网络还是串口
        if command.strip() == "AT+CONNECT?":
            kind = "1" if self.client.connection_type() == "SERIAL" else "0"
            return {"success": True, "data": "+CONNECT: %s\r\nOK" % kind}

        # AT+SCHED? / AT+SCHED= 用来读写定时锁频配置
        resp = self.handle_schedule_command(command)
        if resp is not None:
            return resp

        # 扫频要跑几分钟，单独走异步通路
        resp = self.scan.handle(command)
        if resp is not None:
            return self._fmt(resp)

        if self.scan.running:
            return {"success": False, "error": "正在扫频，模组暂时无法响应其它命令，请先取消扫频"}

        command = normalize_syscfgex(command)

        try:
            r = self.client.send_command(command)
        except Exception as e:
            log.debug("AT 命令失败: %s -> %s", command.strip(), e)
            return {"success": False, "error": str(e)}

        if r.has_error():
            return {"success": False, "error": r.text()}
        return {"success": True, "data": r.text()}

    @staticmethod
    def _fmt(r):
        if r.ok() and not r.has_error():
            return {"success": True, "data": r.text()}
        return {"success": False, "error": r.text()}

    def handle_schedule_command(self, command):
        trimmed = command.strip()
        if trimmed == "AT+SCHED?":
            dto = self.scheduler.to_dto()
            dto["status"] = self.scheduler.status()
            payload = json.dumps(dto, ensure_ascii=False)
            # 前端会校验应答里带有命令前缀，这里按普通 AT 应答的样子加 "+SCHED: "
            return {"success": True, "data": "+SCHED: %s\r\nOK" % payload}

        if trimmed.startswith("AT+SCHED="):
            try:
                d = json.loads(trimmed[len("AT+SCHED="):])
            except json.JSONDecodeError as e:
                return {"success": False, "error": "定时锁频配置不是有效的 JSON: %s" % e}
            try:
                validate_sched_dto(d)
            except ValueError as e:
                return {"success": False, "error": str(e)}
            cfg = sched_dto_to_cfg(d)
            try:
                write_schedule_uci(cfg)
            except Exception as e:
                return {"success": False, "error": "写入 UCI 配置失败: %s" % e}
            self.scheduler.set_cfg(cfg)
            log.info("定时锁频配置已由 WebUI 更新: 启用=%s 夜间=%s 日间=%s",
                     cfg["schedule_enabled"], cfg["schedule_night_enabled"], cfg["schedule_day_enabled"])
            return {"success": True, "data": "+SCHED: OK\r\nOK"}

        return None

    # ---- 连接处理 ----
    async def handler(self, ws):
        # 认证握手是严格的一问一答
        if self.auth_key:
            try:
                payload = await asyncio.wait_for(ws.recv(), timeout=WS_AUTH_TIMEOUT)
                body = json.loads(payload)
                if body.get("auth_key") != self.auth_key:
                    log.warning("WebSocket 连接被拒绝: 密钥错误")
                    await ws.send(json.dumps({"error": "Authentication failed", "message": "密钥验证失败"},
                                             ensure_ascii=False))
                    return
                await ws.send(json.dumps({"success": True, "message": "认证成功"}, ensure_ascii=False))
            except Exception:
                log.warning("WebSocket 连接被拒绝: 认证超时或读取失败")
                try:
                    await ws.send(json.dumps({"error": "Authentication timeout", "message": "认证超时"},
                                             ensure_ascii=False))
                except Exception:
                    pass
                return

        client = WSClient(ws)
        with self.clients_lock:
            self.clients.add(client)
        writer = asyncio.ensure_future(client.write_loop())
        try:
            log.debug("WebSocket 客户端已连接")
            async for message in ws:
                if not isinstance(message, str):
                    continue
                if message == "ping":
                    client.try_send("pong")
                    continue
                loop = asyncio.get_running_loop()
                reply = await loop.run_in_executor(None, self.run_command, message)
                await client.send_blocking(json.dumps(reply, ensure_ascii=False))
        except Exception:
            pass
        finally:
            client.close()
            writer.cancel()
            with self.clients_lock:
                self.clients.discard(client)
            log.debug("WebSocket 客户端已断开")


class WSClient:
    def __init__(self, ws):
        self.ws = ws
        self.out = asyncio.Queue(maxsize=WS_OUT_BUFFER)
        self.closed = False

    def try_send(self, msg):
        # 推送类消息：客户端来不及收就丢弃，不能拖慢整个上报链路
        if self.closed:
            return False
        try:
            self.out.put_nowait(msg)
            return True
        except asyncio.QueueFull:
            return False

    async def send_blocking(self, msg):
        # 命令应答：丢了会让前端的 FIFO 匹配错位，所以宁可等
        if self.closed:
            return
        try:
            await asyncio.wait_for(self.out.put(msg), timeout=WS_WRITE_TIMEOUT)
        except (asyncio.TimeoutError, asyncio.CancelledError):
            pass

    async def write_loop(self):
        try:
            while True:
                try:
                    msg = await asyncio.wait_for(self.out.get(), timeout=WS_HEARTBEAT)
                except asyncio.TimeoutError:
                    msg = "ping"  # 心跳：30 秒一次的文本 ping（与 Go 版一致）
                await self.ws.send(msg)
        except Exception:
            pass

    def close(self):
        self.closed = True


# ---------------------------------------------------------------- 主流程
def main():
    verbose = "-v" in sys.argv or "--verbose" in sys.argv
    if "--version" in sys.argv:
        print("at-webserver %s (python)" % VERSION)
        return
    setup_logging(verbose)

    if serial is None or websockets is None:
        log.error("缺少依赖：python3-pyserial / python3-websockets")
        sys.exit(1)

    cfg = load_uci_config()
    if not cfg["enabled"]:
        log.warning("服务在配置中被禁用，退出")
        return

    if cfg["AT_TYPE"] if False else cfg["connection_type"] == "SERIAL":
        port_desc = "串口自动探测" if cfg["serial_port"] == AUTO_SERIAL_PORT else "串口 %s" % cfg["serial_port"]
        log.info("AT 通道: %s @ %d", port_desc, cfg["serial_baudrate"])
    else:
        log.info("AT 通道: 网络 %s:%d", cfg["network_host"], cfg["network_port"])
    log.info("WebSocket 端口: %d，连接密钥: %s",
             cfg["websocket_port"], "已设置" if cfg["websocket_auth_key"] else "未设置")

    client = ATClient(cfg)
    notifier = Notifier(cfg)
    scheduler = Scheduler(cfg, client, notifier)
    ws = WSServer(client, scheduler, cfg)

    stop_event = threading.Event()

    def on_signal(*_):
        stop_event.set()
        sys.exit(0)

    signal.signal(signal.SIGINT, on_signal)
    signal.signal(signal.SIGTERM, on_signal)

    threading.Thread(target=client.run, args=(stop_event,), daemon=True).start()
    threading.Thread(target=notifier.run, args=(stop_event,), daemon=True).start()
    threading.Thread(target=Dispatcher(client, notifier, ws).run, args=(stop_event,), daemon=True).start()
    threading.Thread(target=scheduler.run, args=(stop_event,), daemon=True).start()

    port = cfg["websocket_port"]
    log.info("启动完成，WebSocket ws://<路由器地址>:%d", port)

    async def serve():
        ws.loop = asyncio.get_running_loop()
        async with websockets.serve(
                ws.handler, None, port,
                max_size=64 * 1024,
                ping_interval=None,   # 心跳用应用层文本 ping，与 Go 版一致
                close_timeout=2):
            await asyncio.Future()  # run forever

    try:
        asyncio.run(serve())
    except Exception as e:
        log.error("服务退出: %s", e)
        sys.exit(1)


if __name__ == "__main__":
    main()
