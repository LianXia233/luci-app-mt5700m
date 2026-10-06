// 后端主动推送的状态事件：事件名 + 负载类型。
//
// 这些事件由后端解析后推送（transport/urc.rs + daemon.rs 派发到对应 topic），
// 页面直接渲染对象，不再自己解析上报原文：
//   network.reject  modules::network::reject  →  手册 13.14 的拒绝原因释义
//   qos.ambr        modules::qos              →  手册 5.33 ^DSAMBR 签约速率
//   sim.changed     原始行（提示页重读 sim.pin_status，不做正则判断）
// USSD 的 `sms.ussd` 事件在 services/ussd.ts。

/** `+REJINFO`/`^REJINFO` 解码结果（手册 13.14.3）。 */
export interface RejectInfo {
  plmn: string;
  domain: number;
  domainText: string;
  cause: number;
  causeText: string;
  rat: number;
  ratText: string;
  rejectType: number;
  rejectTypeText: string;
  originalCause: number;
  lac: string;
  rac: string;
  cellId: string;
  esmCause?: number;
  raw: string;
  at: number;
}

export const NETWORK_REJECT_EVENT = 'network.reject';
export const QOS_AMBR_EVENT = 'qos.ambr';
export const SIM_CHANGED_EVENT = 'sim.changed';
