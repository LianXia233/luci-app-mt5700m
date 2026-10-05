// SIM 卡状态展示层：后端 `sim.pin_status` / `sim.slot` / `sim.slot_set` /
// `sim.hotplug_set` / `sim.pin_apply` 负责所有 AT 交互与语义（+CPIN、^SIMSQ、
// ^SCICHG、^TDSIMHP、+CLCK、+CPWD 的编解码都在 modules/sim），这里只保留：
//   * 后端返回码 -> 界面文案的映射；
//   * CME 错误文案的翻译（纯展示）。

import { backendMessage, isParameterError } from '@/services/backendError';

export type SimLock = 'ready' | 'pin' | 'puk' | 'pin2' | 'puk2' | 'network' | 'absent' | 'unknown';

/** 后端 `sim.pin_status` 解码出的卡状态（语义由后端给出，文案由这里补）。 */
export interface SimCardState {
  /** +CPIN 原始 <code>，例如 "SIM PIN"；未插卡时为 "ABSENT"。 */
  code: string;
  lock: SimLock;
  /** 是否需要用户输入密码才能继续用卡。 */
  blocked: boolean;
  /** 需要 PUK 时为 true：此时要同时输入 PUK 和新 PIN。 */
  needsNewPin: boolean;
  /** PIN 锁是否启用（+CLCK="SC",2），卡未就绪时为 null。 */
  pinEnabled: boolean | null;
}

/** +CPIN 就绪、未取到密码请求时的初始展示状态。 */
export const READY_CARD: SimCardState = {
  code: 'READY',
  lock: 'ready',
  blocked: false,
  needsNewPin: false,
  pinEnabled: false,
};

/** 后端 `sim.pin_status` 的原始应答。 */
export interface SimPinStatusPayload {
  code?: string;
  lock?: string;
  blocked?: boolean;
  needsNewPin?: boolean;
  card?: { status?: number; dead?: boolean; present?: boolean };
  pinEnabled?: boolean | null;
}

/** 手册 6.3.3 <code> 取值 -> 界面文案。 */
const CPIN_LABELS: Record<string, string> = {
  READY: '无需密码，SIM 卡可用',
  'SIM PIN': '需要输入 PIN 码',
  'SIM PUK': 'PIN 已被锁定，需要 PUK 解锁',
  'SIM PIN2': '需要输入 PIN2 码',
  'SIM PUK2': 'PIN2 已被锁定，需要 PUK2 解锁',
  'PH-NET PIN': '需要网络锁 PIN 码',
  'PH-NET PUK': '需要网络锁 PUK 码',
  'PH-NETSUB PIN': '需要子网锁 PIN 码',
  'PH-NETSUB PUK': '需要子网锁 PUK 码',
  'PH-SP PIN': '需要服务提供商锁 PIN 码',
  'PH-SP PUK': '需要服务提供商锁 PUK 码',
  ABSENT: '未检测到 SIM 卡',
};

/** +CPIN 状态码 -> 文案；后端已把大小写和引号规整好。 */
export const pinLabel = (code: string): string => {
  const key = String(code || '')
    .trim()
    .replace(/"/g, '')
    .toUpperCase();
  return CPIN_LABELS[key] ?? `未知状态：${key || '空'}`;
};

/** 把后端 `sim.pin_status` 应答映射成页面使用的卡状态。 */
export const cardStateOf = (payload: SimPinStatusPayload): SimCardState => {
  const code = String(payload.code || '').trim().toUpperCase();
  return {
    code,
    lock: (payload.lock as SimLock) ?? 'unknown',
    blocked: payload.blocked === true,
    needsNewPin: payload.needsNewPin === true,
    pinEnabled: typeof payload.pinEnabled === 'boolean' ? payload.pinEnabled : null,
  };
};

/** 手册 6.6.3 <sim_status>；语义（失效/在位）由后端给出。 */
export interface SimSlotStatus {
  status: number;
  label: string;
  dead: boolean;
  present: boolean;
}

const SIM_STATUS: Record<number, string> = {
  0: '卡不在位',
  1: '卡已插入',
  2: '卡被 PIN/PUK 锁定',
  3: 'SIMLOCK 锁定',
  10: '卡文件初始化中',
  11: '卡初始化完成，可接入网络',
  12: '卡就绪，短信与电话本可用',
  98: '卡已失效（PUK 锁死或物理损坏）',
  99: '卡已移除',
  100: '卡初始化失败',
};

/** ^SIMSQ 状态码 -> 页面展示对象。 */
export const slotStatusOf = (card: {
  status?: number;
  dead?: boolean;
  present?: boolean;
}): SimSlotStatus | null => {
  const status = card.status;
  if (typeof status !== 'number') return null;
  return {
    status,
    label: SIM_STATUS[status] ?? `状态 ${status}`,
    dead: card.dead === true,
    present: card.present === true,
  };
};

// 手册 20.2 CME ERROR 列表里与 SIM 相关的条目。
const CME_MESSAGES: Record<number, string> = {
  3: '操作不允许（当前没有待输入的密码）',
  5: '需要输入 PH-SIM PIN 码',
  10: '没有检测到 SIM 卡',
  11: '需要先输入 PIN 码',
  12: '需要先用 PUK 解锁',
  13: 'SIM 卡故障',
  14: 'SIM 卡忙，请稍后重试',
  15: 'SIM 卡错误',
  16: '密码错误',
  17: '需要先输入 PIN2 码',
  18: '需要先用 PUK2 解锁',
};

// CMEE=2 时模组回的是描述字符串而不是编号，两种都要认。
const CME_TEXTS: Record<string, number> = {
  'operation not allowed': 3,
  'sim not inserted': 10,
  'sim pin required': 11,
  'sim puk required': 12,
  'sim failure': 13,
  'sim busy': 14,
  'sim wrong': 15,
  'incorrect password': 16,
  'sim pin2 required': 17,
  'sim puk2 required': 18,
};

/** 从应答里取出 CME 错误码；模组回的是描述字符串时反查成编号。 */
export const cmeErrorCode = (raw: string): number | null => {
  const match = String(raw || '').match(/\+CME ERROR:\s*(.+)/i);
  if (!match) return null;
  const body = match[1].trim().replace(/[\r\n].*$/s, '');
  if (/^\d+$/.test(body)) return Number(body);
  return CME_TEXTS[body.toLowerCase()] ?? null;
};

/** 把失败应答翻译成给用户看的话。 */
export const simErrorMessage = (raw: string, fallback: string): string => {
  const code = cmeErrorCode(raw);
  if (code !== null && CME_MESSAGES[code]) return CME_MESSAGES[code];
  const match = String(raw || '').match(/\+CME ERROR:\s*(.+)/i);
  if (match) return `${fallback}：${match[1].trim()}`;
  return fallback;
};

/**
 * PIN 操作的失败文案：后端的参数校验消息包在 "参数无效: …" 里
 * （`BackendError::InvalidParameter`），这里只取规则本身那句话，
 * 让弹窗/提示里的措辞和以前一致。
 */
export const pinErrorMessage = (raw: string, fallback: string): string => {
  const text = String(raw || '').trim();
  if (!text) return fallback;
  const code = cmeErrorCode(text);
  if (code !== null && CME_MESSAGES[code]) return CME_MESSAGES[code];
  // 参数校验类消息（密码规则、操作名）只取规则本身那句话，措辞和以前一致。
  if (isParameterError(text)) return backendMessage(text, fallback);
  return simErrorMessage(text, fallback);
};

export type PinOperation = 'verify' | 'unblock' | 'enable' | 'disable' | 'change';
