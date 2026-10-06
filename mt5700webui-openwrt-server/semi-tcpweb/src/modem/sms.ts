// 短信的 PDU 编解码、+CMGL 解析、长短信合并都在统一后端（modules/sms）：
// 这个文件只留「本地发送记录缓存」和展示用的格式化函数。
import { isMockModeEnabled, MOCK_SMS_CACHE_KEY } from '@/services/mockAT';

export const SMS_CACHE_KEY = isMockModeEnabled()
  ? MOCK_SMS_CACHE_KEY
  : 'sms_sent_messages_cache';
export const MAX_SMS_CACHE = 1000;

export interface SMS {
  index: number;
  content: string;
  number: string;
  time: string;
  type: 'sent' | 'received';
  isConcatenated?: boolean;
  concatenatedRef?: number;
  concatenatedSeq?: number;
  concatenatedTotal?: number;
}

export function normalizePhoneNumber(phoneNumber: string): string {
  if (!phoneNumber) return '';
  let normalized = phoneNumber.replace(/^\+/, '');
  if (normalized.startsWith('86') && normalized.length > 2) {
    const rest = normalized.substring(2);
    if (/^\d+$/.test(rest)) return rest;
  }
  if (/^\d+$/.test(normalized)) return normalized;
  const digits = normalized.replace(/\D/g, '');
  return digits || normalized;
}

export function isValidPhoneNumber(number: string): boolean {
  return /^\d{5,19}$/.test(normalizePhoneNumber(number));
}

export function formatPDUTime(timestamp: Date): string {
  const date = timestamp instanceof Date ? timestamp : new Date(timestamp);
  if (Number.isNaN(date.getTime())) return new Date().toLocaleString('zh-CN');
  const formatted = date.toLocaleString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  });
  const [datePart, timePart] = formatted.split(' ');
  const [year, month, day] = datePart.split('/');
  return `${year.slice(-2)}/${month}/${day},${timePart}`;
}

export function parseMessageTime(timeStr: string): Date {
  if (!timeStr) return new Date();
  const raw = String(timeStr).trim();
  const match = raw.match(/(\d{2})\/(\d{2})\/(\d{2}),(\d{2}):(\d{2}):(\d{2})/);
  if (match) {
    const year = 2000 + parseInt(match[1], 10);
    return new Date(year, parseInt(match[2], 10) - 1, parseInt(match[3], 10), parseInt(match[4], 10), parseInt(match[5], 10), parseInt(match[6], 10));
  }
  const parsed = new Date(raw);
  return Number.isNaN(parsed.getTime()) ? new Date() : parsed;
}

// ------------------------------------------------------------------ 路由数据
//
// 页面消费的是后端 modules/sms 的域模型（sms.status / sms.storage / sms.list），
// 字段名在这里声明一次，两个页面共用。

/** `sms.storage` 的一个存储面（+CPMS? 的 name,used,total）。 */
export interface SmsStoragePlane {
  name: string;
  used: number;
  total: number;
}

/** `sms.storage`：读取/写入/接收三个存储面。 */
export interface SmsStorageInfo {
  read?: SmsStoragePlane;
  write?: SmsStoragePlane;
  receive?: SmsStoragePlane;
  storages?: string[];
}

/** `sms.status`：页面加载时的一次性快照（未读到的字段保持原值）。 */
export interface SmsStatus {
  enabled?: boolean;
  imsOn?: boolean;
  center?: string;
  storage?: SmsStorageInfo;
}

/** `sms.list`：后端已解码并合并长短信的消息数组。 */
export interface SmsList {
  messages?: SMS[];
}

/** `sms.analyze` 的返回：分片数/字符集/字数。 */
export interface SmsStats {
  encoding: '7bit' | 'UCS2';
  parts: number;
  chars: number;
}

export function getCachedSentMessages(): SMS[] {
  try {
    const cached = localStorage.getItem(SMS_CACHE_KEY);
    if (!cached) return [];
    const messages = JSON.parse(cached);
    return Array.isArray(messages) ? messages : [];
  } catch {
    return [];
  }
}

export function saveSentMessageToCache(message: SMS) {
  const cached = getCachedSentMessages();
  cached.push(message);
  if (cached.length > MAX_SMS_CACHE) cached.splice(0, cached.length - MAX_SMS_CACHE);
  localStorage.setItem(SMS_CACHE_KEY, JSON.stringify(cached));
}

export function clearSentMessageCache() {
  localStorage.removeItem(SMS_CACHE_KEY);
}

export function setCachedSentMessages(messages: SMS[]) {
  localStorage.setItem(SMS_CACHE_KEY, JSON.stringify(messages));
}
