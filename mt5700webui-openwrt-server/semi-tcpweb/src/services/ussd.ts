// USSD 面板的接口层。
//
// 编解码全在后端 `modules/sms/ussd.rs`：页面只送 `{code}`，拿回来的就是
// `{sent, reply?}`；网络的回复若走 +CUSD 主动上报，后端会解成 `sms.ussd`
// 事件推过来（`data` 就是这个 `UssdReply`）。页面不再碰 GSM 7bit 打包、
// dcs、`<m>` 文案这些规则。

import { ATService } from '@/services/at';

/** 后端 `modules/sms/ussd.rs::Reply` 的形状，与 +CUSD 主动上报同一份。 */
export interface UssdReply {
  /** 手册 5.22.3 `<m>`。 */
  m: number;
  /** `<m>` 对应的中文说明，由后端给出。 */
  mText: string;
  /** 解码后的应答文本（gsm7 / ucs2 / 8bit，按 dcs）。 */
  text: string;
  /** m=1：网络在等进一步输入。 */
  needsReply: boolean;
}

export interface UssdSendResult {
  sent: boolean;
  /** 有些固件把结果直接放在命令应答里，有就先用。 */
  reply?: UssdReply;
}

const at = () => ATService.getInstance();

/** 下发 USSD 代码；参数非法时后端回页面自己的文案（`error`）。 */
export const ussdSend = async (code: string): Promise<UssdSendResult> => {
  const res = await at().apiCommand<UssdSendResult>('sms.ussd_send', { code });
  if (!res.success) throw new Error(res.error || 'USSD 请求失败');
  return res.data ?? { sent: false };
};

/** `AT+CUSD=2`：退出 USSD 会话。 */
export const ussdCancel = async (): Promise<void> => {
  const res = await at().apiCommand<{ cancelled: boolean }>('sms.ussd_cancel');
  if (!res.success) throw new Error(res.error || 'USSD 取消失败');
};

/** 主动上报的 USSD 应答事件名（后端 transport/urc.rs 发布）。 */
export const USSD_EVENT_TYPE = 'sms.ussd';
