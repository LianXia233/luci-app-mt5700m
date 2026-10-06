function extractATData(data: string, command: string): string | null {
  const escaped = command.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = data.match(new RegExp(`${escaped}:\\s*([^\\r\\n]+)`));
  return match ? match[1] : null;
}

function extractATDataMultiline(data: string, command: string): string[] {
  const escaped = command.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const regex = new RegExp(`${escaped}:\\s*(.+)`);
  return data
    .split('\n')
    .map((line) => {
      const match = line.match(regex);
      return match?.[1]?.trim();
    })
    .filter((v): v is string => Boolean(v));
}

function convertRsrp(raw: number): number {
  if (raw === 0) return -140;
  if (raw >= 97) return -44;
  return -140 + raw;
}

function convertRsrq(raw: number): number {
  if (raw === 0) return -19.5;
  if (raw >= 34) return -3;
  return -19.5 + raw * 0.5;
}

function convertSinr(raw: number): number {
  const v = raw === 0 ? -20 : raw >= 251 ? 30 : -20 + raw * 0.2;
  return Math.min(30, Math.max(-20, v));
}

// 手册 13.5.3：rssi 档位 0~96 对应 -120~-25 dBm，255 为不可测。
function convertRssi(raw: number): number {
  if (raw === 0) return -120;
  if (raw >= 96) return -25;
  return -121 + raw;
}

// 信号百分比的量程：-110 dBm 视作 0%，-70 dBm 及以上视作 100%。
// 原来是 5 档阶梯（-80 就直接算满格），调天线时数值要么不动、要么整档跳，
// 既看不出细微变化，也会把 -80 这种其实一般的信号显示成 100%。
export const SIGNAL_RSRP_RANGE: [number, number] = [-110, -70];

export function calculateSignalPercent(rsrp: number): string {
  // 拿不到 rsrp 时按 0 传进来，别把“不可测”显示成满格。
  if (!rsrp || rsrp >= 0) return '';
  const [worst, best] = SIGNAL_RSRP_RANGE;
  const ratio = (rsrp - worst) / (best - worst);
  return `${Math.round(Math.max(0, Math.min(1, ratio)) * 100)}%`;
}

function parseTemperature(rawValue: string | number): number {
  const intValue = typeof rawValue === 'number' ? rawValue : parseInt(rawValue, 10);
  if (intValue >= 65535 || Number.isNaN(intValue) || intValue > 1500) return 0;
  return parseFloat((intValue / 10).toFixed(1));
}

export function formatFlow(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(2)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
  if (bytes < 1024 * 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  return `${(bytes / (1024 * 1024 * 1024 * 1024)).toFixed(2)} TB`;
}

export function splitSpeed(bytesPerSecond: number): { value: string; unit: string } {
  const bits = bytesPerSecond * 8;
  if (bits >= 1e9) return { value: (bits / 1e9).toFixed(2), unit: 'Gbps' };
  if (bits >= 1e6) return { value: (bits / 1e6).toFixed(2), unit: 'Mbps' };
  if (bits >= 1e3) return { value: (bits / 1e3).toFixed(1), unit: 'Kbps' };
  return { value: Math.round(bits).toString(), unit: 'bps' };
}

export function formatDuration(seconds: number, showDays: boolean): string {
  if (showDays) {
    const days = Math.floor(seconds / 86400);
    const hours = Math.floor((seconds % 86400) / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    const remaining = seconds % 60;
    return `${days}天${hours}时${minutes}分${remaining}秒`;
  }
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const remaining = seconds % 60;
  return `${hours}时${minutes}分${remaining}秒`;
}

export const NR_BANDS: Record<string, string> = {
  1: '2100 MHz (FDD)',
  2: '1900 MHz (FDD)',
  3: '1800 MHz (FDD)',
  5: '850 MHz (FDD)',
  7: '2600 MHz (FDD)',
  8: '900 MHz (FDD)',
  20: '800 MHz (FDD)',
  28: '700 MHz (FDD)',
  38: '2600 MHz (TDD)',
  40: '2300 MHz (TDD)',
  41: '2500 MHz (TDD)',
  77: '3700 MHz (TDD)',
  78: '3500 MHz (TDD)',
  79: '4700 MHz (TDD)',
};

export const LTE_BANDS: Record<string, string> = {
  1: '2100 MHz (FDD)',
  2: '1900 MHz (FDD)',
  3: '1800 MHz (FDD)',
  5: '850 MHz (FDD)',
  7: '2600 MHz (FDD)',
  8: '900 MHz (FDD)',
  20: '800 MHz (FDD)',
  38: '2600 MHz (TDD)',
  40: '2300 MHz (TDD)',
  41: '2500 MHz (TDD)',
};

export function operatorFromCode(code: string): string {
  switch (code) {
    case '46000':
    case '46002':
    case '46004':
    case '46007':
    case '46008':
    case '46020':
      return '中国移动';
    case '46001':
    case '46006':
    case '46009':
      return '中国联通';
    case '46003':
    case '46005':
    case '46011':
      return '中国电信';
    case '46015':
      return '中国广电';
    default:
      return '未知运营商';
  }
}

export function qciLabel(value: string | undefined): string {
  switch (value) {
    case '1':
      return '等级1：GBR业务,延迟100ms,丢包率10^-2,高优先级语音通话';
    case '2':
      return '等级2：GBR业务,延迟150ms,丢包率10^-3,标准语音通话';
    case '3':
      return '等级3：GBR业务,延迟50ms,丢包率10^-3,实时游戏';
    case '4':
      return '等级4：GBR业务,延迟300ms,丢包率10^-6,非会话视频';
    case '5':
      return '等级5：非GBR业务,延迟100ms,丢包率10^-6,IMS信令';
    case '6':
      return '等级6：非GBR业务,延迟300ms,丢包率10^-6,视频流媒体';
    case '7':
      return '等级7：非GBR业务,延迟100ms,丢包率10^-3,语音、视频、互动游戏';
    case '8':
      return '等级8：非GBR业务,延迟300ms,丢包率10^-6,视频流媒体、TCP应用';
    case '9':
      return '等级9：非GBR业务,延迟300ms,丢包率10^-6,标准数据传输';
    default:
      return value ? `QCI ${value}：未知服务等级` : '未能获取服务等级信息';
  }
}

export function ipv6CapDescription(capValue: number): string {
  switch (capValue) {
    case 0x01:
      return '仅支持IPv4协议';
    case 0x02:
      return '仅支持IPv6协议';
    case 0x07:
      return '支持IPv4、IPv6和双栈模式（使用相同APN）';
    case 0x0b:
      return '支持IPv4、IPv6和双栈模式（使用不同APN）';
    default:
      return `未知能力值 (0x${capValue.toString(16).toUpperCase()})`;
  }
}

export function psRegText(stat: number): string {
  switch (stat) {
    case 0:
      return '未搜索网络';
    case 1:
      return '已注册，本地网络';
    case 2:
      return '正在搜索网络...';
    case 3:
      return '注册被拒绝';
    case 4:
      return '未知状态';
    case 5:
      return '已注册，漫游网络';
    default:
      return '未知状态';
  }
}

export function getMCSModulation(mcs: number): string {
  if (mcs === 255) return '未使用';
  if (mcs <= 9) return 'QPSK';
  if (mcs <= 16) return '16QAM';
  if (mcs <= 28) return '64QAM';
  return '256QAM';
}

export function getMCSPerformance(mcs: number): { level: string; color: string } {
  if (mcs === 255) return { level: '未使用', color: 'var(--semi-color-text-2)' };
  if (mcs <= 9) return { level: '差', color: 'var(--semi-color-danger)' };
  if (mcs <= 16) return { level: '一般', color: 'var(--semi-color-warning)' };
  if (mcs <= 23) return { level: '好', color: 'var(--semi-color-success)' };
  return { level: '优秀', color: 'var(--semi-color-primary)' };
}

export interface CarrierInfo {
  band: string;
  bandShortName: string;
  bandDesc: string;
  dlFcn: string;
  dlFreq: string;
  dlBandwidth: number;
  ulFcn: string;
  ulFreq: string;
  ulBandwidth: number;
  sysMode: 'NR' | 'LTE';
}

export function getDefaultScsType(band?: number): number {
  if (!band) return 1;
  if ([77, 78, 79, 41].includes(band)) return 1;
  return 0;
}

export function signalColor(percent: string): string {
  const value = parseInt(percent, 10);
  if (!Number.isFinite(value)) return 'var(--semi-color-danger)';
  if (value >= 70) return 'var(--semi-color-success)';
  if (value >= 40) return 'var(--semi-color-warning)';
  return 'var(--semi-color-danger)';
}

export function rsrpColor(v: number): string {
  if (v >= -85) return 'var(--semi-color-success)';
  if (v >= -95) return 'var(--semi-color-warning)';
  return 'var(--semi-color-danger)';
}

