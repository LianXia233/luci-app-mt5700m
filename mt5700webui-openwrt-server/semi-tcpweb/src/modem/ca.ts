// 载波聚合卡片的展示层：把 `ca.get` 的领域模型映射成页面结构，并把
// `^CASCELLINFO`/`^MONSSC` 报出来的辅载波信号按下行频点并到 `^HFREQINFO`
// 的载波卡片上。
//
// 这里只有映射与合并，没有解析：
//   * 频段文案来自 modem/parse.ts 的 NR_BANDS / LTE_BANDS（纯显示表）；
//   * 每条上报行的字段解码都在后端 modules/ca/parser.rs（含手册的无效值、
//     十六进制 PCI、8 倍量级还原与 <MEASTYPE>），前端不再有第二份。
//
// 早先这两个辅载波结构（SecondaryNR/SecondaryLTE）在 modem/carrier.ts 里由
// 页面自己解析 ^MONSSC / ^CASCELLINFO；解析搬走后这里只保留类型与合并规则。

import type { CaPayload, CaSecondary } from '@/services/ca';
import { LTE_BANDS, NR_BANDS, type CarrierInfo } from '@/modem/parse';

/** 手册 13.27 `^MONSSC` 的 NR 辅站小区（后端解码后的形状）。 */
export interface SecondaryNR {
  arfcn: number;
  pci: number;
  rsrp: number | null;
  rsrq: number | null;
  sinr: number | null;
  /** 手册 13.27.3 `<MEASTYPE>`：SSB / CSI-RS / —。 */
  measType: string;
}

/** 手册 13.18 `^CASCELLINFO` 的 LTE 辅小区（后端解码后的形状）。 */
export interface SecondaryLTE {
  index: number;
  pci: number;
  rssi: number | null;
  rsrp: number | null;
  rsrq: number | null;
  band: number;
  ulArfcn: number | null;
  dlArfcn: number | null;
  ulFreq: number | null;
  dlFreq: number | null;
  ulBandwidth: number | null;
  dlBandwidth: number | null;
}

const orNull = (v: number | undefined): number | null => (typeof v === 'number' ? v : null);

/** `ca.get` 的载波数组 → 载波卡片的数据（频段文案在这里补齐）。 */
export const carriersFromCa = (payload: CaPayload): CarrierInfo[] =>
  payload.carriers.map((c) => {
    // 后端给的是展示形式（n41 / B3），数字部分用来查中文说明。
    const digits = c.band.replace(/^[nB]/, '');
    const table = c.radio === 'NR' ? NR_BANDS : LTE_BANDS;
    return {
      band: digits,
      bandShortName: c.band,
      bandDesc: table[digits] || '',
      dlFcn: c.dl_arfcn,
      dlFreq: c.dl_frequency_mhz ? c.dl_frequency_mhz.toFixed(1) : '—',
      dlBandwidth: c.dl_bandwidth_mhz,
      ulFcn: c.ul_arfcn,
      ulFreq: c.ul_frequency_mhz ? c.ul_frequency_mhz.toFixed(1) : '—',
      ulBandwidth: c.ul_bandwidth_mhz,
      sysMode: c.radio,
    };
  });

/** `ca.get` 的辅小区数组 → 两张按制式分开的信号表。 */
export const secondariesFromCa = (
  secondary: CaSecondary[] | undefined,
): { nr: SecondaryNR[]; lte: SecondaryLTE[] } => {
  const nr: SecondaryNR[] = [];
  const lte: SecondaryLTE[] = [];
  (secondary || []).forEach((c) => {
    if (c.radio === 'NR' && typeof c.arfcn === 'number') {
      nr.push({
        arfcn: c.arfcn,
        pci: c.pci,
        rsrp: orNull(c.rsrp),
        rsrq: orNull(c.rsrq),
        sinr: orNull(c.sinr),
        measType: c.measType || '—',
      });
      return;
    }
    if (c.radio === 'LTE' && typeof c.index === 'number') {
      lte.push({
        index: c.index,
        pci: c.pci,
        rssi: orNull(c.rssi),
        rsrp: orNull(c.rsrp),
        rsrq: orNull(c.rsrq),
        band: typeof c.band === 'number' ? c.band : 0,
        ulArfcn: orNull(c.ulArfcn),
        dlArfcn: orNull(c.dlArfcn),
        ulFreq: orNull(c.ulFreq),
        dlFreq: orNull(c.dlFreq),
        ulBandwidth: orNull(c.ulBandwidth),
        dlBandwidth: orNull(c.dlBandwidth),
      });
    }
  });
  return { nr, lte };
};

interface CarrierSignal {
  pci: number;
  rsrp: number | null;
  rsrq: number | null;
  sinr: number | null;
  rssi?: number | null;
  measType?: string;
}

/**
 * 把 ^MONSSC / ^CASCELLINFO 的信号质量对应到 ^HFREQINFO 报出来的某个载波上。
 * 三条命令描述的是同一批载波，只是各报一部分：^HFREQINFO 给频点与带宽，
 * 另外两条给信号。按下行频点对齐即可合并展示。
 */
export const carrierSignalFor = (
  carrier: { sysMode: 'NR' | 'LTE'; dlFcn: string },
  nr: SecondaryNR[],
  lte: SecondaryLTE[],
): CarrierSignal | null => {
  const arfcn = Number(carrier.dlFcn);
  if (!Number.isFinite(arfcn)) return null;

  if (carrier.sysMode === 'NR') {
    const hit = nr.find((c) => c.arfcn === arfcn);
    return hit
      ? { pci: hit.pci, rsrp: hit.rsrp, rsrq: hit.rsrq, sinr: hit.sinr, measType: hit.measType }
      : null;
  }

  const hit = lte.find((c) => c.dlArfcn === arfcn);
  return hit ? { pci: hit.pci, rsrp: hit.rsrp, rsrq: hit.rsrq, sinr: null, rssi: hit.rssi } : null;
};

/** 找出没能对应到任何载波的辅小区，避免合并之后把数据悄悄丢掉。 */
export const unmatchedSecondaries = (
  carriers: Array<{ sysMode: 'NR' | 'LTE'; dlFcn: string }>,
  nr: SecondaryNR[],
  lte: SecondaryLTE[],
): { nr: SecondaryNR[]; lte: SecondaryLTE[] } => {
  const arfcns = new Set(carriers.map((c) => Number(c.dlFcn)));
  return {
    nr: nr.filter((c) => !arfcns.has(c.arfcn)),
    lte: lte.filter((c) => c.dlArfcn === null || !arfcns.has(c.dlArfcn)),
  };
};
