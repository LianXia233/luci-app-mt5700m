// FOTA 固件升级的 API 层：升级流程（初始化、状态机轮询、断点续传、刷写）的唯一
// 实现现在在后端 modules/system/fota.rs，页面只负责发起、观察和展示。页面因此
// 不再拼 AT 命令、不再自己驱动状态机，也不再解析 ^FOTASTATE / ^FOTADLQ。
//
// 升级是长流程且必须活过页面：发起之后即使页面刷新，任务仍在后端跑，
// `fotaState()` 从任务表 + 快照回答（零 AT 流量），`fota.progress` 推送带来
// 状态变化。
import { ATService, type ApiResponse } from '@/services/at';

/** 流程阶段：idle 未开始 / running 进行中 / done 已进入升级 / error 失败。 */
export type FotaPhase = 'idle' | 'running' | 'done' | 'error';

/** `system.fota` 的应答，字段与后端快照一一对应。 */
export interface FotaState {
  running: boolean;
  phase: FotaPhase;
  /** 页面步骤（0 准备 / 1 初始化 / 2 下载 / 3 升级 / 4 完成），后端负责推进。 */
  step: number;
  progress: number;
  /** 模组的 ^FOTASTATE 原始状态码，未读到时为 null。 */
  state: number | null;
  /** 状态码的文案，两条前端的文案由后端统一给。 */
  stateName: string | null;
  total: number;
  received: number;
  error?: string;
}

const at = () => ATService.getInstance();

/** 读取升级状态 —— 纯任务表 + 快照查询，不压 AT 通道。 */
export async function fotaState(): Promise<FotaState | null> {
  const res = await at().apiCommand<FotaState>('system.fota');
  return res.success && res.data ? res.data : null;
}

/** 发起升级：后端校验地址、初始化 FOTA 并开始下载。 */
export async function fotaStart(url: string): Promise<ApiResponse> {
  return at().apiCommand('system.fota_start', { url });
}

/** 取消升级。页面没有取消按钮（界面不变），这是给终端/脚本的恢复入口。 */
export async function fotaAbort(): Promise<ApiResponse> {
  return at().apiCommand('system.fota_abort');
}
