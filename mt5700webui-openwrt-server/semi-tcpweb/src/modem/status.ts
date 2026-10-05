// 运行状态类数据的**类型与标签表**。
//
// 这里的解析函数（parseLendc / parseTxPower / parseNrTxPower / parseC5greg /
// parseCgpaddr）已经删除：解码只有后端一份实现，页面通过统一 API 读取
//
//   api.modem.endc        -> EndcStatus      （modules/modem/parser.rs）
//   api.registration.get  -> Reg5G           （modules/network/parser.rs）
//   api.modem.txpower     -> TxPower         （modules/modem/parser.rs）
//   api.modem.nr_txpower  -> { carriers: NrTxPower[] }
//   api.network.pdp       -> { addresses: PdpAddress[] }
//
// 保留下面的 interface 是为了给那些 JSON 形状做文档，并与事件推送
// （endc.updated / registration.updated / txpower.updated / nr_txpower.updated）
// 保持同一套字段名；REG_STATES / ACT_TYPES 是 3GPP 枚举 -> 文案的显示映射，
// 属于纯展示层。
//
// 手册出处：
//   11.7  AT^LENDC?     — NSA 下 LTE-NR 双连接是否真的建起来了
//   13.23 AT^TXPOWER?   — GUL 发射功率（ENDC 下查的是 LTE 侧）
//   13.24 AT^NTXPOWER?  — NR 发射功率，支持多 CC
//   5.27  AT+C5GREG?    — 5G 核心网注册状态
//   7.8   AT+CGPADDR    — PDP 上下文实际使用的地址

export interface EndcStatus {
  /** 当前小区是否支持 ENDC（SIB2 upperLayerIndication） */
  available: boolean;
  /** 当前小区所选 PLMN 是否支持 ENDC */
  plmnAvailable: boolean;
  /** 手册 11.7.3：0 表示 restricted，1 表示 not restricted */
  restricted: boolean;
  /** PSCell 是否为 NR，也就是 ENDC 是否真的建立了 */
  established: boolean;
}

export interface TxPower {
  /** 2G/3G 的总发射功率，单位已从 0.1dBm 还原为 dBm；4G 下模组填 999，这里给 null */
  total: number | null;
  pusch: number | null;
  pucch: number | null;
  srs: number | null;
  prach: number | null;
}

export interface NrTxPower {
  pusch: number | null;
  pucch: number | null;
  srs: number | null;
  prach: number | null;
  freq: number | null;
}

// 手册 5.27.3 <stat>
export const REG_STATES: Record<number, string> = {
  0: '未注册，未搜网',
  1: '已注册本地网络',
  2: '未注册，搜网中',
  3: '注册被拒绝',
  4: '未知原因',
  5: '已注册漫游网络',
  8: '仅紧急业务',
};

// 手册 5.27.3 <AcT>
export const ACT_TYPES: Record<number, string> = { 10: 'EUTRAN-5GC', 11: 'NR-5GC' };

export interface Reg5G {
  stat: number;
  statText: string;
  registered: boolean;
  tac: string;
  ci: string;
  act: string;
  nssai: string;
}

export interface PdpAddress {
  cid: number;
  address: string;
  family: 'IPv4' | 'IPv6' | '未知';
}
