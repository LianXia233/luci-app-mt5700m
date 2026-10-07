// 扫频（AT^CELLSCAN）的唯一实现现在在后端 modules/cell/scan.rs：命令拼装、行解析
// 和独占任务都在那边。这里只留「调用哪条路由」和展示用的类型，页面不再拼 AT、
// 不再解析 ^CELLSCAN 字段。
import { ATService } from '@/services/at';

/** 扫频结果的制式编号（手册 5.35.3）。 */
export type ScanRat = 0 | 1 | 2 | 3;

/** 一个小区，字段与后端 `cell.scan_*` 的 cells[] 一一对应。 */
export interface ScanCell {
  rat: ScanRat;
  ratName: string;
  plmn: string;
  /** LTE/NR 下为频点（可直接用于锁频），GSM/WCDMA 下为该制式的频点 */
  freq: number | null;
  pci: number | null;
  /** 频段号，后端已从手册的十六进制换算成十进制；解析不出来时为 null */
  band: number | null;
  lac: string;
  cid: string;
  rxlev: number | null;
  bsic: number | null;
  psc: number | null;
  scs: number | null;
  rsrp: number | null;
  rsrq: number | null;
  sinr: number | null;
  raw: string;
}

/** 页面表单里的筛选条件；空串表示不限。 */
export interface ScanFilter {
  /** 空串表示不指定接入技术，由模组扫描所有支持的制式。 */
  rat?: '' | '1' | '2' | '3';
  plmn?: string;
  freq?: string;
  pci?: string;
  band?: string;
  scs?: string;
}

/** 扫频推送：后端跑完（或取消/失败）时发布一次。 */
export interface ScanPush {
  state: 'running' | 'done' | 'aborted' | 'error';
  /** 扫到的小区，结构已解码；进度推送里是当前累计结果。 */
  cells?: ScanCell[];
  count: number;
  error?: string;
}

// 三条路由都是"立刻受理"型：扫频在独占任务里跑，结果走 cellscan 推送，
// 所以命令超时用默认值即可，不需要为几分钟的扫描留长超时。
const at = () => ATService.getInstance();

/** 开始扫频；筛选条件不合法时后端返回 error（提示文案与页面校验一致）。 */
export const scanStart = (filter: ScanFilter) =>
  at().apiCommand<{ started: boolean }>('cell.scan_start', {
    rat: filter.rat || '',
    plmn: filter.plmn || '',
    freq: filter.freq || '',
    pci: filter.pci || '',
    band: filter.band || '',
    scs: filter.scs || '',
  });

/** 服务端是否还有扫频在跑（页面刷新后恢复界面用）。 */
export const scanState = () => at().apiCommand<{ running: boolean }>('cell.scan_state');

/** 取消扫频；没有在跑时后端也返回成功（aborted: false），不是错误。 */
export const scanAbort = () => at().apiCommand<{ aborted: boolean }>('cell.scan_abort');
