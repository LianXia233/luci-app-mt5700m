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

/**
 * `api.network.dhcp` —— 数据会话的地址族参数（modules/network 解析
 * `AT^DHCP?` / `AT^DHCPV6?` / `AT^IPV6CAP?`）。
 *
 * 字段名与页面一直渲染的一致；IPv4 那六个字段的十六进制小端解码在后端完成，
 * 页面只做展示（含未知能力值的文案映射）。
 */
interface DhcpLease {
  address?: string;
  netmask?: string;
  gateway?: string;
  dhcp_server?: string;
  primary_dns?: string;
  secondary_dns?: string;
}

export interface NetworkDhcpPayload {
  /** `AT^DHCP?`（IPv4） */
  ipv4?: DhcpLease;
  /** `AT^DHCPV6?` */
  ipv6?: DhcpLease;
  /** `AT^IPV6CAP?` 的能力值，页面映射为说明文案 */
  ipv6_capability?: number;
}

/**
 * `api.network.c5goption` —— 5G 接入模式（modules/network 解析 `AT^C5GOPTION?`）。
 * 页面把三元组映射成「仅 SA / 仅 NSA / SA+NSA / 其他」文案。
 */
export interface C5gOptionPayload {
  nr_sa_support_flag?: number;
  nr_dc_mode?: number;
  gc_access_mode?: number;
}

/**
 * `api.cell.neighbors` —— `AT^MONNC` 的邻区列表。`band` 由后端的
 * ARFCN->频段表算出，NR 的 1/8 倍率也已在后端还原。
 */
export interface NeighborsPayload {
  cells?: Array<{
    type: string;
    arfcn?: number;
    pci?: number;
    rsrp?: string;
    rsrq?: string;
    sinr?: string;
    rxlev?: string;
    band?: number;
  }>;
}

/**
 * `api.beam.ssb` —— NR SSB 波束报告（modules/beam 解析 `AT^NRSSBID?`）。
 * 未测到的波束槽位（255/32767）已在后端剔除。
 */
interface SsbBeam {
  ssbId: number;
  rsrp: number;
}

export interface SsbPayload {
  servingCell?: {
    arfcn?: string;
    cid?: string;
    pci?: string;
    rsrp?: number;
    sinr?: number;
    ta?: number;
    ssbs?: SsbBeam[];
  } | null;
  neighborCells?: Array<{
    pci?: string;
    arfcn?: string;
    rsrp?: number;
    sinr?: number;
    ssbs?: SsbBeam[];
  }>;
}
