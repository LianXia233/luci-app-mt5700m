import { ATService } from '@/services/at';

export const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

const at = () => ATService.getInstance();

/**
 * 取出模组给的失败原因。后端初始化时下发了 AT+CMEE=2（手册 3.14），
 * 模组会用 "+CME ERROR: <错误描述>" 代替干巴巴的 ERROR，把它带到界面上，
 * 比一律显示"设置失败"有用得多。
 */
export function atErrorText(res: { success: boolean; data?: unknown; error?: unknown }, fallback: string): string {
  const text = String(res.error ?? res.data ?? '');
  const match = text.match(/\+CM[ES] ERROR:\s*(.+)/i);
  if (match) return `${fallback}：${match[1].trim()}`;
  const trimmed = text.trim();
  if (trimmed && trimmed !== 'ERROR') return `${fallback}：${trimmed}`;
  return fallback;
}
