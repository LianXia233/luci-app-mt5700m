/**
 * `modem.mcs` payload —— 后端 modules/modem 解析 `AT^MCS=1` / `AT^MCS=0` 的结果。
 *
 * 单一后端（2026-10-05）：页面不再自己发 AT、也不再自己按三值分组解析应答。
 * 后端给的是领域数据（RAT、每载波的 MCS 表索引与两个编码值、平均值），
 * 这里只把 code0 映射成调制方式与等级文案 —— 那是纯展示，属于前端。
 *
 * 后端：
 *   { "downlink": { "rat": "NR", "carriers": [ { index, mcs_table_index, code0, code1 } ], "avg_mcs": 23 },
 *     "uplink":   { ... } }
 */
import { getMCSModulation, getMCSPerformance } from '@/modem/parse';

/** 渲染用的单载波 MCS 行（调制方式/等级/颜色都是显示映射）。 */
interface MCSCarrier {
  index: number;
  mcsTableIndex: number;
  code0: number;
  code1: number;
  modulation: string;
  performance: string;
  color: string;
}

/** 一个方向的渲染模型：原 `modem/parse.ts` 的同名类型，解析已移到后端。 */
export interface MCSInfo {
  rat: 'LTE' | 'NR' | 'UNKNOWN';
  carriers: MCSCarrier[];
  avgMCS: number;
}

/** 后端一个载波行。 */
interface McsCarrierPayload {
  index: number;
  /** 应答里每条 ^MCS 行的分组号；LuCI 用它还原「NR Carrier 1」这类行标签。 */
  group?: number;
  /** 该行所属制式（'NR' / 'LTE' / ''），分组标签用；页面本身按 index 对齐载波。 */
  rat?: string;
  mcs_table_index: number;
  code0: number;
  code1: number;
}

/** 后端一个方向（下行/上行）的读数。 */
interface McsBlockPayload {
  rat?: 'LTE' | 'NR' | 'UNKNOWN' | string;
  carriers?: McsCarrierPayload[];
  avg_mcs?: number;
}

/** `modem.mcs` 的完整应答，方向缺失表示该方向没读到。 */
export interface McsPayload {
  downlink?: McsBlockPayload;
  uplink?: McsBlockPayload;
}

/** 领域数据 -> 页面渲染用的 MCSInfo（补上调制方式、等级与颜色）。 */
export function mcsInfoFromPayload(block: McsBlockPayload | undefined): MCSInfo | null {
  if (!block || !Array.isArray(block.carriers) || block.carriers.length === 0) return null;
  const rat = block.rat === 'LTE' || block.rat === 'NR' ? block.rat : 'UNKNOWN';
  return {
    rat,
    carriers: block.carriers.map((c, i) => {
      const perf = getMCSPerformance(c.code0);
      return {
        index: typeof c.index === 'number' ? c.index : i + 1,
        mcsTableIndex: c.mcs_table_index,
        code0: c.code0,
        code1: c.code1,
        modulation: getMCSModulation(c.code0),
        performance: perf.level,
        color: perf.color,
      };
    }),
    avgMCS: typeof block.avg_mcs === 'number' ? block.avg_mcs : 0,
  };
}
