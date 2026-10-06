// 定时锁频的接口层。
//
// 配置的读写都在后端 `modules/network/schedule.rs`：路由 `network.schedule_get`
// / `network.schedule_set` 直接给/收这个对象（UCI 的 `schedule_*` 映射、
// 校验、总开关归 LuCI 这些规则都在那边）。页面只负责渲染与表单草稿。
//
// 早先这条路径是 WebSocket 上的伪 AT 命令（`AT+SCHED?` 里塞 JSON），
// 前端得从 AT 文本里截括号再 JSON.parse —— 页面上最后一处“自己解协议”。

import { ATService } from '@/services/at';
import type { LockLists } from '@/modem/lock';

export type SchedulePeriod = {
  enabled: boolean;
  start?: string;
  end?: string;
  lte: LockLists;
  nr: LockLists;
};

export type ScheduleStatus = {
  current_mode: string;
  next_switch: string;
  switch_count: number;
  applied: boolean;
};

export type ScheduleConfig = {
  enabled: boolean;
  check_interval: number;
  timeout: number;
  unlock_lte: boolean;
  unlock_nr: boolean;
  toggle_airplane: boolean;
  night: SchedulePeriod;
  day: SchedulePeriod;
  status?: ScheduleStatus;
};

/**
 * 读取定时锁频配置。返回 null 表示后端给不出配置（离线/旧版本），
 * 调用方据此隐藏整个面板。
 */
export async function fetchSchedule(): Promise<ScheduleConfig | null> {
  const res = await ATService.getInstance().apiCommand<ScheduleConfig>('network.schedule_get');
  if (!res.success || !res.data || typeof res.data.enabled !== 'boolean') return null;
  return res.data;
}

/** 保存配置。后端校验失败时把提示原样抛出来，界面直接显示原因。 */
export async function saveSchedule(cfg: ScheduleConfig): Promise<void> {
  const { status, ...payload } = cfg;
  void status;
  const res = await ATService.getInstance().apiCommand('network.schedule_set', payload);
  if (!res.success) throw new Error(String(res.error || '保存定时锁频配置失败'));
}

export const modeText = (mode: string) => (mode === '' ? '当前时段不锁频' : mode);
