// 载波聚合的接口层。
//
// 三条命令的解析都在后端 `modules/ca`（`^HFREQINFO?` 载波列表、`^CASCELLINFO?`
// LTE 辅小区、`^MONSSC` NR 辅站），`ca.get` 直接给领域模型：
//
//   { carriers: [{radio, band, source, dl_arfcn, ul_arfcn, dl_frequency_mhz,
//                 ul_frequency_mhz, dl_bandwidth_mhz, ul_bandwidth_mhz}],
//     secondary: [{radio:"NR",  arfcn, pci, rsrp?, rsrq?, sinr?, measType}
//               | {radio:"LTE", index, pci, band, rssi?, rsrp?, rsrq?,
//                  ulArfcn?, dlArfcn?, ulFreq?, dlFreq?, ulBandwidth?, dlBandwidth?}],
//     carrier_count, ca_active, dc_active, nr_carrier_count, ... }
//
// 页面把 carriers 映射成卡片、把 secondary 按 radio 分成两份信号，按下行频点
// 对齐 —— 这些合并规则在 `modem/ca.ts`，前端不再解析任何 AT 应答。

import { ATService } from '@/services/at';

/** `ca.get` 的载波条目（`CaCarrier` 的 JSON 形状）。 */
export interface CaCarrier {
  radio: 'NR' | 'LTE';
  /** 展示用频段：`n41` / `B3`。 */
  band: string;
  source: 'hfreqinfo' | 'lte_scell';
  dl_arfcn: string;
  ul_arfcn: string;
  dl_frequency_mhz: number;
  ul_frequency_mhz: number;
  dl_bandwidth_mhz: number;
  ul_bandwidth_mhz: number;
}

/** `ca.get` 的辅小区条目（`SecondaryCell` 的 JSON 形状）。 */
export interface CaSecondary {
  radio: 'NR' | 'LTE';
  arfcn?: number;
  index?: number;
  pci: number;
  band?: number;
  rssi?: number;
  rsrp?: number;
  rsrq?: number;
  sinr?: number;
  measType?: string;
  ulArfcn?: number;
  dlArfcn?: number;
  ulFreq?: number;
  dlFreq?: number;
  ulBandwidth?: number;
  dlBandwidth?: number;
}

export interface CaPayload {
  carriers: CaCarrier[];
  secondary: CaSecondary[];
  carrier_count: number;
  ca_active: boolean;
  dc_active: boolean;
  secondary_connection_count: number;
}

/** 读取载波聚合（`refresh=true` 强制重查模组，否则优先缓存）。 */
export const fetchCa = async (refresh = false): Promise<CaPayload | null> => {
  const res = await ATService.getInstance().apiCommand<CaPayload>(
    'ca.get',
    refresh ? { refresh: true } : undefined,
  );
  if (!res.success || !res.data || !Array.isArray(res.data.carriers)) return null;
  return res.data;
};
