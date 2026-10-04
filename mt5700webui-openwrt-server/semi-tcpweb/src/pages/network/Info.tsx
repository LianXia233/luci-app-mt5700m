import React, { useEffect, useMemo, useRef, useState } from 'react';
import { Button, InputNumber, Modal, Space, Switch, Tag, Toast, Typography } from '@douyinfe/semi-ui';
import { IconArrowDown, IconArrowUp, IconSetting } from '@douyinfe/semi-icons';
import { ATResponse, ATService, PDCPData, URCData, type StateSnapshot } from '@/services/at';
import { refreshSharedStateFeed, useSharedStateTopic } from '@/services/stateCache';
import { useATReady } from '@/hooks/useATReady';
import { useCommandQueue } from '@/hooks/useCommandQueue';
import { SvgSignalTower, SvgDataStream } from '@/ui/svgVisuals';
import {
  calculateSignalPercent,
  extractATData,
  extractATDataMultiline,
  formatDuration,
  formatFlow,
  splitSpeed,
  hexToIP,
  ipv6CapDescription,
  parseMCS,
  parseHexValue,
  psRegText,
  operatorFromCode,
  qciLabel,
  rsrpColor,
  signalColor,
  SIGNAL_RSRP_RANGE,
  type CarrierInfo,
  type MCSInfo,
} from '@/modem/parse';
import { AutoRefresh, Kv, Metric, PageCard, Panel, SectionHeader, TwoCol } from '@/ui/widgets';
import { QualityBar, RingGauge, Sparkline } from '@/ui/charts';
import { Diagnostics } from './Diagnostics';
import {
  carrierSignalFor,
  unmatchedSecondaries,
  type SecondaryLTE,
  type SecondaryNR,
} from '@/modem/carrier';

const at = () => ATService.getInstance();

// 曲线最多保留这么多采样点。速率是 PDCP 上报驱动的（默认约 0.75 秒一次），
// 60 个点差不多是最近一分钟。
const HISTORY_POINTS = 60;

const trimHistory = <T,>(list: T[]): T[] =>
  list.length > HISTORY_POINTS ? list.slice(list.length - HISTORY_POINTS) : list;

// 页面里的信号值有的是字符串有的是数字，取不到就当没有，别画成 0。
const numOrNull = (v: unknown): number | null => {
  const n = Number(v);
  return Number.isFinite(n) && n !== 0 ? n : null;
};

const displayOperatorName = (raw: unknown): string => {
  if (typeof raw !== 'string') return '';
  const value = raw.trim();
  if (!value) return '';
  if (/^\d{5,6}$/.test(value)) return operatorFromCode(value);
  const upper = value.toUpperCase();
  if (upper.includes('UNICOM')) return '中国联通';
  if (upper.includes('TELECOM') || /CHN-?CT/.test(upper)) return '中国电信';
  if (upper.includes('MOBILE') || upper.includes('CMCC')) return '中国移动';
  if (upper.includes('BROADNET') || upper.includes('CBN')) return '中国广电';
  return value;
};

const EMPTY_CELL = {
  rscp: 0,
  signalPercent: '',
  ecio: 0,
  sinr: 0,
  rssi: 0,
  mcc: '',
  mnc: '',
  lac: '',
  cid: '',
  channel: '',
  pci: 0,
  carrierInfo: [] as CarrierInfo[],
  carrierCount: 0,
  networkMode: '',
  sysMode: '未知',
};

const NetworkInfo: React.FC = () => {
  const { enqueue } = useCommandQueue();
  const signalEntry = useSharedStateTopic('signal');
  const networkEntry = useSharedStateTopic('network');
  const registrationEntry = useSharedStateTopic('registration');
  const temperatureEntry = useSharedStateTopic('temperature');
  const cellEntry = useSharedStateTopic('cell');
  const trafficEntry = useSharedStateTopic('traffic');
  const netrateEntry = useSharedStateTopic('netrate');
  const sharedSnapshot: Partial<StateSnapshot> = useMemo(
    () => ({
      signal: signalEntry,
      network: networkEntry,
      registration: registrationEntry,
      temperature: temperatureEntry,
      cell: cellEntry,
      traffic: trafficEntry,
      netrate: netrateEntry,
    }),
    [signalEntry, networkEntry, registrationEntry, temperatureEntry, cellEntry, trafficEntry, netrateEntry],
  );
  const [networkStatus, setNetworkStatus] = useState('等待状态中');
  const [operator, setOperator] = useState('未知运营商');
  const [cell, setCell] = useState(EMPTY_CELL);
  const [apn, setApn] = useState('未知');
  const [qci, setQci] = useState('未知');
  const [downSpeed, setDownSpeed] = useState(0);
  const [upSpeed, setUpSpeed] = useState(0);
  const [pdcp, setPdcp] = useState<PDCPData | null>(null);
  const [lastPdcp, setLastPdcp] = useState<PDCPData | null>(null);
  const [pdcpOn, setPdcpOn] = useState(false);
  // 每个载波各自的信号质量，合并进上面的载波聚合卡片显示。
  const [secondaryNR, setSecondaryNR] = useState<SecondaryNR[]>([]);
  const [secondaryLTE, setSecondaryLTE] = useState<SecondaryLTE[]>([]);
  // 曲线用的历史序列，只留最近一段，避免长时间开着页面把内存撑大。
  const [speedHistory, setSpeedHistory] = useState<Array<{ up: number; down: number }>>([]);
  const [signalHistory, setSignalHistory] = useState<Array<{ rsrp: number; sinr: number }>>([]);

  const dash = (v: number | null | undefined, unit = ''): string =>
    v == null ? '—' : `${v}${unit}`;
  const [pdcpInterval, setPdcpInterval] = useState(500);
  const [intervalModal, setIntervalModal] = useState(false);
  const [tempInterval, setTempInterval] = useState(500);
  const [uplinkMCS, setUplinkMCS] = useState<MCSInfo | null>(null);
  const [downlinkMCS, setDownlinkMCS] = useState<MCSInfo | null>(null);
  const [showDays, setShowDays] = useState(false);
  const [dhcpv4, setDhcpv4] = useState({
    ipv4Address: '',
    subnetMask: '',
    gateway: '',
    dhcpServer: '',
    primaryDNS: '',
    secondaryDNS: '',
  });
  const [dhcpv6, setDhcpv6] = useState({
    ipv6Address: '',
    netmask: '',
    gateway: '',
    dhcpServer: '',
    primaryDNS: '',
    secondaryDNS: '',
  });
  const [ipv6Cap, setIpv6Cap] = useState({ capValue: 0, description: '' });
  const [temps, setTemps] = useState({
    sub3GPA: 0,
    sub6GPA: 0,
    mimoPa: 0,
    tcxo: 0,
    ap1: 0,
    ap2: 0,
    modem1: 0,
  });
  const [flow, setFlow] = useState({
    lastDsTime: 0,
    lastTxFlow: 0,
    lastRxFlow: 0,
    totalDsTime: 0,
    totalTxFlow: 0,
    totalRxFlow: 0,
    // netrateSource: 'modem' = AT^DSFLOWQRY（模组 PDP 计数）
    //                'netdev'  = mt5700m-traffic（网卡字节计数，与 LuCI 同源）
    // 两套计数物理量不同，同时显示只会互相误导，因此默认走 netdev 并在
    // UI 上标注来源。切回 modem 仅用于与模组自带诊断对齐。
    netrateSource: 'netdev' as 'modem' | 'netdev',
  });
  // 自动刷新默认全部开启（2026-10-04）。此前三个开关默认 false，等于
  // 首屏之后数据就冻结了，用户必须逐个手动打开——而实时速率、流量、
  // 温度本来就是持续变化的量，冻结即失真。
  // 间隔 5s 用于刷新共享 StateCache 快照（零 AT）；daemon 自己按 topic 调度
  // 后台采集，流量计数走 sysfs / mt5700m-traffic，温度走其共享缓存。
  const [auto, setAuto] = useState({
    networkInfo: { enabled: true, interval: 5 },
    flowStats: { enabled: true, interval: 5 },
    tempMonitor: { enabled: true, interval: 5 },
  });
  const timers = useRef<Record<string, number>>({});
  const activeCidRef = useRef<number | null>(null);
  const appliedTopicValues = useRef<Record<string, string>>({});
  const pdcpOnRef = useRef(false);
  pdcpOnRef.current = pdcpOn;

  // ---- Async Architecture：事件 / 快照 -> 页面状态的统一映射 ----
  // 后端采集器写入 StateCache 并推送 *.updated 事件，字段已是物理值
  // （RSRP 单位 dBm、SINR/RSRQ 单位 dB），与页面模型一致，这里只做存在性
  // 判断，不做二次换算。各 topic 按 daemon 的采集周期异步更新；页面只订阅
  // 自己用到的 topic，不再为这些共享状态建立第二套 AT 轮询。
  const applySignalEvent = (value: Record<string, unknown>) => {
    const sig = (v: unknown): number | null =>
      typeof v === 'number' && Number.isFinite(v) && v !== 0 ? v : null;
    const rsrp = sig(value.rsrp);
    setCell((prev) => {
      const next = {
        ...prev,
        rscp: rsrp ?? prev.rscp,
        ecio: sig(value.rsrq) ?? prev.ecio,
        sinr: sig(value.sinr) ?? prev.sinr,
        rssi: sig(value.rssi) ?? prev.rssi,
        signalPercent: rsrp ? calculateSignalPercent(rsrp) : prev.signalPercent,
        sysMode:
          typeof value.sysmode === 'string' && value.sysmode
            ? (value.sysmode as string)
            : prev.sysMode,
      };
      // 事件值同样入曲线，保持调天线时能看趋势。
      if (rsrp && rsrp < 0) {
        setSignalHistory((hist) => trimHistory([...hist, { rsrp, sinr: next.sinr ?? 0 }]));
      }
      return next;
    });
  };

  const applyTempsEvent = (value: Record<string, unknown>) => {
    const pick = (k: string): number | null => numOrNull(value[k]);
    setTemps((prev) => ({
      ...prev,
      sub3GPA: pick('sub3GPA') ?? prev.sub3GPA,
      sub6GPA: pick('sub6GPA') ?? prev.sub6GPA,
      mimoPa: pick('mimoPa') ?? prev.mimoPa,
      tcxo: pick('tcxo') ?? prev.tcxo,
      ap1: pick('ap1') ?? prev.ap1,
      ap2: pick('ap2') ?? prev.ap2,
      modem1: pick('modem1') ?? prev.modem1,
    }));
  };

  // 小区参数（PCI/频点/MCC-MNC/TAC·LAC/小区ID）：后端 collect_cell 每 10s
  // 解析 ^MONSC 写入 StateCache 并推送 cell.updated，字段映射与 parseMONSC
  // 一致（cid/lac 为十进制字符串，pci 为数字），页面只做存在性判断。
  const applyCellEvent = (value: Record<string, unknown>) => {
    setCell((prev) => {
      const str = (v: unknown): string | undefined =>
        typeof v === 'string' && v ? (v as string) : undefined;
      return {
        ...prev,
        pci: typeof value.pci === 'number' ? (value.pci as number) : prev.pci,
        channel: str(value.channel) ?? prev.channel,
        mcc: str(value.mcc) ?? prev.mcc,
        mnc: str(value.mnc) ?? prev.mnc,
        lac: str(value.lac) ?? prev.lac,
        cid: str(value.cid) ?? prev.cid,
        sysMode: str(value.sysmode) ?? prev.sysMode,
      };
    });
  };

  const applyTrafficEvent = (data: Partial<PDCPData>) => {
    const up = Number(data.ulPdcpRate || 0);
    const down = Number(data.dlPdcpRate || 0);
    if (up <= 0 && down <= 0) return;
    setPdcp(data as PDCPData);
    setLastPdcp(data as PDCPData);
    setSpeedHistory((prev) =>
      trimHistory([
        ...prev,
        {
          up: Number(((up * 8) / 1_000_000).toFixed(2)),
          down: Number(((down * 8) / 1_000_000).toFixed(2)),
        },
      ]),
    );
  };

  // SWR 首屏：请求 StateCache 快照（零 AT 流量）先渲染最近一次后台状态，
  // 详细数据由 loadAll 里的命令在后台补齐；快照拿不到时静默回退。
  const applySnapshot = (snap: typeof sharedSnapshot) => {
    const applyTopic = (topic: string, apply: (value: Record<string, unknown>) => void) => {
      const value = snap[topic]?.value;
      if (!value || typeof value !== 'object') return;
      // Snapshots are refreshed periodically and may contain unchanged values.
      // Applying those again would add duplicate points to the live charts.
      const signature = JSON.stringify(value);
      if (appliedTopicValues.current[topic] === signature) return;
      appliedTopicValues.current[topic] = signature;
      apply(value as Record<string, unknown>);
    };

    applyTopic('signal', applySignalEvent);
    applyTopic('network', (net) => {
      const name = displayOperatorName(net.operator);
      if (name) setOperator(name);
    });
    applyTopic('registration', (reg) => {
      if (typeof reg.state === 'number') setNetworkStatus(psRegText(reg.state));
    });
    applyTopic('temperature', applyTempsEvent);
    applyTopic('cell', applyCellEvent);
    applyTopic('traffic', (traffic) => applyTrafficEvent(traffic as Partial<PDCPData>));
    // 累计流量与网卡计数：与 LuCI 同源的那一份，首屏就从快照渲染。
    applyTopic('netrate', applyNetrate);
  };

  // Preserve the existing per-PDP-context AMBR/QCI lookup; these values are
  // not part of the shared StateCache topics and remain normal async reads.
  const resolveActiveCid = async (force = false): Promise<number | null> => {
    if (!force && activeCidRef.current !== null) return activeCidRef.current;
    const res = await at().readCommand('AT+CGACT?');
    if (!res.success || !res.data) return activeCidRef.current;
    const active: number[] = [];
    for (const row of extractATDataMultiline(res.data as string, '+CGACT')) {
      const [cid, state] = row.split(',');
      if (state?.trim() === '1' && Number(cid) > 0) active.push(Number(cid));
    }
    activeCidRef.current = active.length ? Math.min(...active) : null;
    return activeCidRef.current;
  };

  const getAMBR = async () => {
    const cid = await resolveActiveCid();
    const candidates = Array.from(new Set([cid, 1].filter((v): v is number => !!v && v > 0)));
    for (const candidate of candidates) {
      const res = await at().readCommand(`AT^DSAMBR=${candidate}`);
      const str = res.success && res.data ? extractATData(res.data as string, '^DSAMBR') : null;
      if (!str) continue;
      const parts = str.split(',');
      if (parts.length >= 3) {
        setDownSpeed((parseInt(parts[1], 10) || 0) / 1000);
        setUpSpeed((parseInt(parts[2], 10) || 0) / 1000);
      }
      if (parts.length >= 4) {
        setApn(parts[3].trim().replace(/^["']|["']$/g, '') || '未知');
      }
      return;
    }
    activeCidRef.current = null;
  };

  const getQCI = async () => {
    const cid = await resolveActiveCid();
    let res = await at().readCommand('AT+CGEQOSRDP');
    if ((!res.success || !res.data) && cid) {
      res = await at().readCommand(`AT+CGEQOSRDP=${cid}`);
    }
    if (!res.success || !res.data) return;
    const rows = extractATDataMultiline(res.data as string, '+CGEQOSRDP');
    const row = rows.find((r) => cid !== null && Number(r.split(',')[0]) === cid) ?? rows[0];
    if (row) setQci(qciLabel(row.split(',')[1]?.trim()));
  };

  const getDHCP = async () => {
    const v6 = await at().readCommand('AT^DHCPV6?');
    if (v6.success && v6.data) {
      const str = extractATData(v6.data as string, '^DHCPV6');
      if (str) {
        const d = str.split(',');
        if (d.length >= 6) {
          setDhcpv6({
            ipv6Address: d[0].trim(),
            netmask: d[1].trim(),
            gateway: d[2].trim(),
            dhcpServer: d[3].trim(),
            primaryDNS: d[4].trim(),
            secondaryDNS: d[5].trim(),
          });
        }
      }
    }
    const v4 = await at().readCommand('AT^DHCP?');
    if (v4.success && v4.data) {
      const str = extractATData(v4.data as string, '^DHCP');
      if (str) {
        const d = str.split(',');
        if (d.length >= 6) {
          setDhcpv4({
            ipv4Address: hexToIP(d[0].trim()),
            subnetMask: hexToIP(d[1].trim()),
            gateway: hexToIP(d[2].trim()),
            dhcpServer: hexToIP(d[3].trim()),
            primaryDNS: hexToIP(d[4].trim()),
            secondaryDNS: hexToIP(d[5].trim()),
          });
        }
      }
    }
    const cap = await at().readCommand('AT^IPV6CAP?');
    if (cap.success && cap.data) {
      const str = extractATData(cap.data as string, '^IPV6CAP');
      if (str) {
        const value = parseInt(str.trim(), 10);
        if (!Number.isNaN(value)) setIpv6Cap({ capValue: value, description: ipv6CapDescription(value) });
      }
    }
  };

  /**
   * 累计流量 —— 统一从后端 StateCache 读，与 LuCI 概览页同源。
   *
   * 单一后端（2026-10-04）：数据源是 Rust daemon 的 `netrate` 采集器
   * （at-webserver，5 s 一轮）。它读两处：
   *   · /sys/class/net/<dev>/statistics/{rx,tx}_bytes —— 实时累计计数
   *   · /usr/sbin/mt5700m-traffic json —— 累计/日/月历史，
   *     与 LuCI 概览页「IP 流量统计」**同一个二进制、同一份 history**
   * 两个前端都只读这份缓存，谁都不写，不存在双写冲突。
   *
   * 为什么不走 CGI：多一个入口就多一份可能不同步的副本。WebUI 只通过
   * WebSocket 连后端，与 LuCI 经 ubus 转发到的是**同一个 daemon 进程**。
   *
   * 与旧实现的差别：原先读 AT^DSFLOWQRY（模组 PDCP 计数），与网卡计数
   * 物理量不同（前者不含协议栈开销、后者含），两个数字永远对不上。
   */
  const applyNetrate = (raw: unknown) => {
    const v = (raw ?? {}) as Record<string, any>;
    if (v.available === false) {
      // 接口未 up：清零而不是继续显示上一轮的旧值。
      setFlow((prev) => ({
        ...prev,
        totalRxFlow: 0,
        totalTxFlow: 0,
        netrateSource: 'netdev',
      }));
      return;
    }
    // 后端把 mt5700m-traffic 的原始结构放在 traffic 字段下：
    //   { interfaces: [ { name, traffic: { total:{rx,tx}, day:[…], month:[…] } } ] }
    const list: Array<Record<string, any>> = Array.isArray(v.traffic?.interfaces)
      ? v.traffic.interfaces
      : [];
    const iface =
      list.find((i) => i?.name === v.device) ||
      list.find((i) => i?.name === 'eth2') ||
      list.find((i) => typeof i?.name === 'string' && i.name !== 'lo') ||
      list[0];
    const total = iface?.traffic?.total;
    if (total && (total.rx != null || total.tx != null)) {
      setFlow((prev) => ({
        ...prev,
        totalRxFlow: Number(total.rx) || 0,
        totalTxFlow: Number(total.tx) || 0,
        netrateSource: 'netdev',
      }));
    } else {
      // 计数器可用但历史尚未初始化：实时速率仍可用，累计部分保持 0。
      setFlow((prev) => ({ ...prev, netrateSource: 'netdev' }));
    }
  };

  const refreshSharedMeasurements = () => refreshSharedStateFeed();

  const getMCS = async () => {
    const dl = await at().sendCommand('AT^MCS=1');
    if (dl.success && dl.data) setDownlinkMCS(parseMCS(dl.data as string));
    const ul = await at().sendCommand('AT^MCS=0');
    if (ul.success && ul.data) setUplinkMCS(parseMCS(ul.data as string));
  };

  const loadAll = () => {
    // 首屏状态由共享 StateCache/EventBus 驱动。这里只补 LuCI 概览同样读取的
    // APN/QCI/AMBR 等未纳入主题缓存的只读字段，以及页面特有的 MCS 诊断值；
    // 页面本身不再用 AT 轮询覆盖信号、注册、温度或累计流量。
    enqueue(async () => {
      // 保留原有的注册状态详细上报设置副作用；注册值本身只由共享缓存/事件更新。
      await at().sendCommand('AT+CGREG=2');
      await getAMBR();
      await getQCI();
      await getDHCP();
      await getMCS();
      // 保留页面特有的 PDCP 实时速率开关；累计流量仍使用与 LuCI 相同的 netrate。
      void at().setPDCPDataReport(true, pdcpInterval).catch(() => {});
    });
  };

  useATReady(loadAll);

  useEffect(() => {
    if (Object.keys(sharedSnapshot).length) applySnapshot(sharedSnapshot);
  }, [sharedSnapshot]);

  useEffect(() => {
    const handle = (response: ATResponse) => {
      if (!('type' in response) || !('data' in response)) return;
      if (response.type === 'urc_data') {
        const urc = response.data as URCData;
        if (urc.type === 'DSAMBR' && urc.parsed) {
          if (urc.parsed.apn) setApn(String(urc.parsed.apn).replace(/^["']|["']$/g, ''));
          if (urc.parsed.maxDownlinkRate) setDownSpeed(urc.parsed.maxDownlinkRate / 1000);
          if (urc.parsed.maxUplinkRate) setUpSpeed(urc.parsed.maxUplinkRate / 1000);
        }
        return;
      }
      if (response.type !== 'pdcp_data') return;
      const data = response.data as PDCPData;
      setPdcpOn(true);
      pdcpOnRef.current = true;
      if (data.ulPdcpRate > 0 || data.dlPdcpRate > 0) setLastPdcp(data);
      setPdcp(data);
      // 页面特有的 PDCP 速率曲线；公共信号/温度/网络状态统一来自 StateCache。
      const upMbps = Number(((data.ulPdcpRate * 8) / 1_000_000).toFixed(2));
      const downMbps = Number(((data.dlPdcpRate * 8) / 1_000_000).toFixed(2));
      setSpeedHistory((prev) => trimHistory([...prev, { up: upMbps, down: downMbps }]));
    };
    at().subscribe(handle);
    return () => {
      at().unsubscribe(handle);
      if (pdcpOnRef.current) at().setPDCPDataReport(false);
      Object.values(timers.current).forEach((id) => window.clearInterval(id));
    };
  }, []);

  const setAutoRefresh = (key: keyof typeof auto, enabled: boolean, interval: number) => {
    if (timers.current[key]) {
      window.clearInterval(timers.current[key]);
      delete timers.current[key];
    }
    setAuto((prev) => ({ ...prev, [key]: { enabled, interval } }));
    if (!enabled) return;
    const tick = () => {
      enqueue(async () => {
        try {
          if (key === 'networkInfo') {
            await getMCS();
          } else {
            // StateCache snapshot 请求是纯读控制帧，不会新增 AT 采集；实际更新由 EventBus 推送。
            await refreshSharedMeasurements();
          }
        } catch {
          Toast.error('自动刷新失败，已停止');
          setAutoRefresh(key, false, interval);
        }
      });
    };
    tick();
    timers.current[key] = window.setInterval(tick, interval * 1000);
  };

  const confirmPdcp = async () => {
    const res = await at().setPDCPDataReport(true, tempInterval);
    if (res.success) {
      setPdcpOn(true);
      setPdcpInterval(tempInterval);
      setIntervalModal(false);
      Toast.success('实时网速已开启');
    } else {
      Toast.error('开启失败');
    }
  };

  const togglePdcp = async (on: boolean) => {
    if (on) {
      const res = await at().setPDCPDataReport(true, pdcpInterval);
      if (res.success) {
        setPdcpOn(true);
        pdcpOnRef.current = true;
        Toast.success('实时网速已开启');
      } else {
        Toast.error('开启实时网速失败');
      }
      return;
    }
    const res = await at().setPDCPDataReport(false);
    if (res.success) {
      setPdcpOn(false);
      pdcpOnRef.current = false;
      setPdcp(null);
      Toast.success('实时网速已暂停');
    } else {
      Toast.error('关闭失败');
    }
  };

  const clearFlow = async () => {
    const res = await at().sendCommand('AT^DSFLOWCLR');
    if (res.success) {
      Toast.success('流量已清零');
      await refreshSharedMeasurements();
    } else Toast.error('清零失败');
  };

  const displayPdcp = pdcp && (pdcp.ulPdcpRate > 0 || pdcp.dlPdcpRate > 0) ? pdcp : lastPdcp;
  const nr = cell.networkMode.includes('NR') || cell.sysMode === 'NR';
  const lte = cell.sysMode === 'LTE' || cell.networkMode.includes('LTE');
  // NR 的 ^HCSQ 不上报 RSSI（手册 13.5），只有纯 LTE 且确实拿到值时才展示它
  const showRssi = lte && !nr && numOrNull(cell.rssi) !== null;

  const ulRate = displayPdcp?.ulPdcpRate || 0;
  const dlRate = displayPdcp?.dlPdcpRate || 0;
  const upSplit = splitSpeed(ulRate);
  const downSplit = splitSpeed(dlRate);
  const maxDownSpeed = speedHistory.length ? Math.max(0, ...speedHistory.map((p) => p.down)) : 0;
  const maxUpSpeed = speedHistory.length ? Math.max(0, ...speedHistory.map((p) => p.up)) : 0;

  // 合并展示不能把数据吞掉：没能对上任何载波的辅小区单独列出来。
  const orphan = unmatchedSecondaries(cell.carrierInfo, secondaryNR, secondaryLTE);

  return (
    <>
      <div className="page-stack">
        <SectionHeader title="信号与驻留" desc="当前驻留小区、信号质量与网络参数" />

        <PageCard
          variant="hero"
          title="网络信息"
          hint="当前驻留小区与载波"
          extra={<AutoRefresh enabled={auto.networkInfo.enabled} interval={auto.networkInfo.interval} onChange={(e, i) => setAutoRefresh('networkInfo', e, i)} />}
        >
          <div className="net-hero">
            <div className="net-hero-primary">
              <div className="net-hero-tags">
                {cell.networkMode ? <Tag color="red">{cell.networkMode}</Tag> : null}
                <Tag color={networkStatus.includes('本地') ? 'green' : networkStatus.includes('漫游') ? 'orange' : 'red'}>
                  {networkStatus}
                </Tag>
              </div>
              <div>
                <div className="signal-overview">
                  <div className="signal-tower-wrapper">
                    <SvgSignalTower
                      percent={cell.signalPercent ? parseInt(cell.signalPercent, 10) : 0}
                      is5G={nr}
                    />
                  </div>
                  <RingGauge
                    percent={cell.signalPercent ? parseInt(cell.signalPercent, 10) : null}
                    label="信号质量"
                    color={signalColor(cell.signalPercent)}
                  />
                  <div className="signal-overview-side">
                    <span className="signal-overview-caption">
                      {cell.signalPercent
                        ? parseInt(cell.signalPercent, 10) >= 70
                          ? '信号良好'
                          : parseInt(cell.signalPercent, 10) >= 40
                            ? '信号一般'
                            : '信号较差'
                        : '暂无测量'}
                    </span>
                    <span className="signal-overview-note">按 RSRP {SIGNAL_RSRP_RANGE[0]}~{SIGNAL_RSRP_RANGE[1]} dBm 线性映射</span>
                  </div>
                </div>
                <div className="metric-row">
                  {/* 不带小字提示：三列窄格子里"参考信号接收功率"会折行，把整块挤乱，
                      标签本身已经写明指标名，冗余提示不值一次换行 */}
                  <Metric
                    size="sm"
                    label={nr || lte ? 'RSRP (dBm)' : cell.sysMode === 'WCDMA' ? 'RSCP (dBm)' : 'RSSI (dBm)'}
                    value={cell.rscp || '—'}
                    color={rsrpColor(cell.rscp)}
                  />
                  <Metric size="sm" label="SINR (dB)" value={cell.sinr || '—'} />
                  {/* 手册 13.5：^HCSQ 在 NR 下没有 RSSI 字段，5G/EN-DC 显示 RSSI 只会是"—"。
                      有 NR 就显示 RSRQ；纯 LTE 且真拿到了 RSSI 才显示 RSSI。 */}
                  <Metric
                    size="sm"
                    label={showRssi ? 'RSSI (dBm)' : 'RSRQ (dB)'}
                    value={showRssi ? cell.rssi : cell.ecio || '—'}
                  />
                </div>
                <div className="quality-grid">
                  <QualityBar label="RSRP" value={numOrNull(cell.rscp)} domain={SIGNAL_RSRP_RANGE} unit=" dBm" />
                  <QualityBar label="SINR" value={numOrNull(cell.sinr)} domain={[0, 25]} unit=" dB" />
                </div>
              </div>
            </div>

            <div className="net-hero-details">
              <Panel title="运营商与网络参数" variant="flat" className="net-operator-panel">
                <Kv
                  columns={3}
                  dense
                  items={[
                    { label: '运营商', value: operator },
                    { label: 'APN', value: apn },
                    { label: 'QCI', value: qci.split('：')[0] },
                    { label: 'AMBR 上行', value: `${upSpeed.toFixed(1)} Mbps` },
                    { label: 'AMBR 下行', value: `${downSpeed.toFixed(1)} Mbps` },
                  ]}
                />
                {/* 只有拿到 QCI 释义时才显示这行，否则会孤零零冒出一个"未知" */}
                {qci.includes('：') ? (
                  <Typography.Text type="tertiary" size="small">
                    {qci.split('：')[1]}
                  </Typography.Text>
                ) : null}
                <div className="net-cell-params">
                  <div className="net-cell-params-title">小区参数</div>
                  <Kv
                    columns={3}
                    dense
                    items={[
                      { label: 'PCI', value: cell.pci || '—' },
                      { label: '频点', value: cell.channel || '—' },
                      { label: 'MCC-MNC', value: cell.mcc && cell.mnc ? `${cell.mcc}-${cell.mnc}` : '—' },
                      { label: 'TAC / LAC', value: cell.lac || '—' },
                      { label: '小区 ID', value: cell.cid || '—' },
                    ]}
                  />
                </div>
              </Panel>
            </div>
          </div>

          {/* 趋势图横贯整卡：左右两列高度对齐，右下不再空一块，曲线也更宽更好读。
              调天线朝向时盯着曲线比盯单个数字直观：能看出是真的变好还是在抖 */}
          <div className="net-hero-trend">
            <Sparkline
              height={86}
              minRange={6}
              empty="等待信号上报…"
              format={(v) => `${Math.round(v)} dBm`}
              series={[
                {
                  label: 'RSRP 趋势',
                  color: 'var(--app-info)',
                  values: signalHistory.map((p) => p.rsrp),
                },
              ]}
            />
          </div>
        </PageCard>

        <SectionHeader
          title="载波聚合"
          desc={`${cell.carrierCount} 载波 · DL ${cell.carrierInfo.reduce((s, c) => s + c.dlBandwidth, 0).toFixed(1)} MHz / UL ${cell.carrierInfo.reduce((s, c) => s + c.ulBandwidth, 0).toFixed(1)} MHz`}
        />
        <div className="carrier-grid">
          {cell.carrierInfo.length === 0 ? (
            <Panel>暂无载波信息</Panel>
          ) : (
            cell.carrierInfo.map((c, i) => {
              const dl = downlinkMCS?.carriers[i];
              const ul = uplinkMCS?.carriers[i];
              // ^HFREQINFO 只给频点与带宽，每个载波各自的信号质量要从
              // ^MONSSC(NSA 辅站) 与 ^CASCELLINFO(LTE CA) 里按下行频点对上来。
              const sig = carrierSignalFor(c, secondaryNR, secondaryLTE);
              return (
                <Panel
                  key={`${c.bandShortName}-${i}`}
                  title={i === 0 ? '主载波' : `辅载波 ${i}`}
                  extra={<Tag size="small">{c.sysMode}</Tag>}
                >
                  <Kv
                    items={[
                      { label: '频段', value: `${c.bandShortName} ${c.bandDesc}` },
                      { label: '下行频点 / 频率', value: `${c.dlFcn} / ${c.dlFreq} MHz` },
                      { label: '上行频点 / 频率', value: `${c.ulFcn} / ${c.ulFreq} MHz` },
                      { label: '带宽 DL / UL', value: `${c.dlBandwidth} / ${c.ulBandwidth} MHz` },
                      ...(sig
                        ? [
                            { label: 'PCI', value: String(sig.pci) },
                            {
                              label: 'RSRP / RSRQ',
                              value: (
                                <span>
                                  {/* RSRP 按好坏着色，一眼看出哪个载波信号差 */}
                                  <span style={{ color: sig.rsrp !== null ? rsrpColor(sig.rsrp) : undefined }}>
                                    {dash(sig.rsrp, ' dBm')}
                                  </span>
                                  {` / ${dash(sig.rsrq, ' dB')}`}
                                </span>
                              ),
                            },
                            {
                              label: sig.sinr !== null ? 'SINR' : 'RSSI',
                              value:
                                sig.sinr !== null
                                  ? `${dash(sig.sinr, ' dB')}${sig.measType && sig.measType !== '—' ? ` · ${sig.measType}` : ''}`
                                  : dash(sig.rssi ?? null, ' dBm'),
                            },
                          ]
                        : []),
                      {
                        label: '下行 MCS',
                        value: dl ? (
                          <span style={{ color: dl.color }}>
                            {dl.code0 === 255 ? '—' : `${dl.code0} ${dl.modulation}`}
                          </span>
                        ) : (
                          '—'
                        ),
                      },
                      {
                        label: '上行 MCS',
                        value: ul ? (
                          <span style={{ color: ul.color }}>
                            {ul.code0 === 255 ? '—' : `${ul.code0} ${ul.modulation}`}
                          </span>
                        ) : (
                          '—'
                        ),
                      },
                    ]}
                  />
                </Panel>
              );
            })
          )}
        </div>

        {orphan.nr.length > 0 || orphan.lte.length > 0 ? (
          <Panel title="未归入上方载波的辅小区" accent>
            <Typography.Text type="tertiary" size="small">
              这些小区来自 ^MONSSC / ^CASCELLINFO 上报，但频点没能和 ^HFREQINFO
              报出的载波对上（两条命令的上报时机可能不同步）。列在这里是为了不丢数据，
              正常聚合的载波都在上方卡片里。
            </Typography.Text>
            <Kv
              items={[
                ...orphan.nr.map((c) => ({
                  label: `NR 频点 ${c.arfcn} · PCI ${c.pci}`,
                  value: `${dash(c.rsrp, ' dBm')} / ${dash(c.rsrq, ' dB')} / ${dash(c.sinr, ' dB')}`,
                })),
                ...orphan.lte.map((c) => ({
                  label: `LTE B${c.band} · PCI ${c.pci}`,
                  value: `${dash(c.rsrp, ' dBm')} / ${dash(c.rsrq, ' dB')} / ${dash(c.rssi, ' dBm')}`,
                })),
              ]}
            />
          </Panel>
        ) : null}

        <Diagnostics />

        <SectionHeader title="速率与流量" desc="实时网速与累计流量统计" />
        <TwoCol>
          <PageCard
            title="网络速率"
            bodyClassName="network-speed-body"
            extra={
              <Space>
                <Button
                  size="small"
                  theme="borderless"
                  icon={<IconSetting />}
                  aria-label="设置上报间隔"
                  title="设置上报间隔"
                  onClick={() => {
                    setTempInterval(pdcpInterval);
                    setIntervalModal(true);
                  }}
                />
                <Typography.Text size="small" type="tertiary">
                  实时网速
                </Typography.Text>
                <Switch checked={pdcpOn} onChange={togglePdcp} aria-label="实时网速开关" />
              </Space>
            }
          >
            <div className="speed-dashboard">
              <div className="speed-datastream-banner">
                <SvgDataStream
                  active={Boolean(pdcpOn && (ulRate > 0 || dlRate > 0))}
                  downMbps={Number(downSplit.value) * (downSplit.unit === 'Gbps' ? 1000 : downSplit.unit === 'Kbps' ? 0.001 : 1)}
                  upMbps={Number(upSplit.value) * (upSplit.unit === 'Gbps' ? 1000 : upSplit.unit === 'Kbps' ? 0.001 : 1)}
                />
              </div>
              <div className="speed-stat-grid">
                <div className="speed-stat-card speed-stat-card--down">
                  <div className="speed-stat-head">
                    <span className="speed-stat-icon" aria-hidden="true">
                      <IconArrowDown size="small" />
                    </span>
                    下行速率
                  </div>
                  <div className="speed-stat-main">
                    <span className="speed-stat-val">{downSplit.value}</span>
                    <span className="speed-stat-unit">{downSplit.unit}</span>
                  </div>
                  <div className="speed-stat-sub">
                    {maxDownSpeed > 0 ? `峰值 ${maxDownSpeed.toFixed(2)} Mbps` : '等待采样'}
                  </div>
                </div>

                <div className="speed-stat-card speed-stat-card--up">
                  <div className="speed-stat-head">
                    <span className="speed-stat-icon" aria-hidden="true">
                      <IconArrowUp size="small" />
                    </span>
                    上行速率
                  </div>
                  <div className="speed-stat-main">
                    <span className="speed-stat-val">{upSplit.value}</span>
                    <span className="speed-stat-unit">{upSplit.unit}</span>
                  </div>
                  <div className="speed-stat-sub">
                    {maxUpSpeed > 0 ? `峰值 ${maxUpSpeed.toFixed(2)} Mbps` : '等待采样'}
                  </div>
                </div>
              </div>

              {/* 上方数字已经是图例（图标颜色与线色一致），图下不再重复一遍 */}
              <Sparkline
                height={116}
                minRange={0.5}
                showLegend={false}
                empty={pdcpOn ? '等待速率上报…' : '实时网速已关闭，打开右上角开关开始采样'}
                format={(v) => `${v.toFixed(2)} Mbps`}
                series={[
                  {
                    label: '下行',
                    color: 'var(--app-info)',
                    values: speedHistory.map((p) => p.down),
                  },
                  {
                    label: '上行',
                    color: 'var(--app-success)',
                    dashed: true,
                    values: speedHistory.map((p) => p.up),
                  },
                ]}
              />
            </div>
          </PageCard>

          <PageCard
            title="流量统计"
            extra={
              <Space>
                <Button size="small" onClick={() => setShowDays((v) => !v)}>
                  {showDays ? '显示时分秒' : '显示天数'}
                </Button>
                <Button size="small" type="danger" onClick={clearFlow}>
                  清零
                </Button>
                <AutoRefresh
                  enabled={auto.flowStats.enabled}
                  interval={auto.flowStats.interval}
                  onChange={(e, i) => setAutoRefresh('flowStats', e, i)}
                />
              </Space>
            }
          >
            <Panel title="最后一次连接" variant="flat">
              <div className="metric-row">
                <Metric label="连接时长" value={formatDuration(flow.lastDsTime, showDays)} />
                <Metric label="上传流量" value={formatFlow(flow.lastTxFlow)} />
                <Metric label="下载流量" value={formatFlow(flow.lastRxFlow)} />
              </div>
            </Panel>
            <Panel title="累计统计" variant="flat">
              <div className="metric-row">
                <Metric label="总连接时长" value={formatDuration(flow.totalDsTime, showDays)} />
                <Metric label="总上传流量" value={formatFlow(flow.totalTxFlow)} />
                <Metric label="总下载流量" value={formatFlow(flow.totalRxFlow)} />
              </div>
              {/* 数据来源必须标出来：本页读的是后端 netrate 采集器，与 LuCI
                  概览页「IP 流量统计」同源（同一份网卡计数与历史文件）。
                  旧实现读模组 PDCP 计数，与网卡口径数量级不同。 */}
              <Typography.Text type="tertiary" className="netrate-source">
                数据来源：后端 netrate 采集器 · 网卡计数器
                {flow.netrateSource === 'netdev'
                  ? '（与 LuCI「IP 流量统计」同源）'
                  : ''}
              </Typography.Text>
            </Panel>
          </PageCard>
        </TwoCol>

        <SectionHeader title="DHCP 配置" desc="IPv4 / IPv6 地址、网关与 DNS" />
        <Panel title="IPv6 能力" accent>
          <Typography.Text>
            {ipv6Cap.capValue ? `0x${ipv6Cap.capValue.toString(16).toUpperCase().padStart(2, '0')} · ${ipv6Cap.description}` : '未获取'}
          </Typography.Text>
        </Panel>
        <TwoCol>
          <Panel title="IPv4 网络配置">
            <Kv
              items={[
                { label: '地址', value: dhcpv4.ipv4Address || '未获取' },
                { label: '掩码', value: dhcpv4.subnetMask || '未获取' },
                { label: '网关', value: dhcpv4.gateway || '未获取' },
                { label: 'DHCP', value: dhcpv4.dhcpServer || '未获取' },
                { label: '主 DNS', value: dhcpv4.primaryDNS || '未获取' },
                { label: '备 DNS', value: dhcpv4.secondaryDNS || '未获取' },
              ]}
            />
          </Panel>
          <Panel title="IPv6 网络配置">
            <Kv
              items={[
                { label: '地址', value: dhcpv6.ipv6Address || '未获取' },
                { label: '前缀', value: dhcpv6.netmask || '未获取' },
                { label: '网关', value: dhcpv6.gateway || '未获取' },
                { label: 'DHCP', value: dhcpv6.dhcpServer || '未获取' },
                { label: '主 DNS', value: dhcpv6.primaryDNS || '未获取' },
                { label: '备 DNS', value: dhcpv6.secondaryDNS || '未获取' },
              ]}
            />
          </Panel>
        </TwoCol>

        <SectionHeader
          title="模组温度"
          desc="关键芯片实时温度"
          extra={<AutoRefresh enabled={auto.tempMonitor.enabled} interval={auto.tempMonitor.interval} onChange={(e, i) => setAutoRefresh('tempMonitor', e, i)} />}
        />
        <div className="temp-grid">
          {[
            ['3G PA', temps.sub3GPA],
            ['6G PA', temps.sub6GPA],
            ['MIMO PA', temps.mimoPa],
            ['TCXO', temps.tcxo],
            ['AP1', temps.ap1],
            ['AP2', temps.ap2],
            ['Modem1', temps.modem1],
          ].map(([label, value]) => (
            <div className="temp-tile" key={String(label)}>
              <QualityBar
                label={`${label}温度`}
                value={numOrNull(value)}
                unit="°C"
                domain={[20, 85]}
                higherIsWorse
              />
            </div>
          ))}
        </div>
      </div>

      <Modal
        title="主动刷新时间"
        visible={intervalModal}
        onCancel={() => setIntervalModal(false)}
        onOk={confirmPdcp}
        okText="开启"
      >
        <Typography.Paragraph type="tertiary">PDCP 上报间隔（毫秒），范围 200–65535</Typography.Paragraph>
        <InputNumber min={200} max={65535} step={100} value={tempInterval} onChange={(v) => setTempInterval(Number(v) || 500)} />
      </Modal>
    </>
  );
};

export default NetworkInfo;
