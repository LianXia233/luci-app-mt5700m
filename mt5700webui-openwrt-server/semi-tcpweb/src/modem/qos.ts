// QoS / 数据会话参数的**类型**（无解析函数）。
//
// 解码在后端 `modules/qos`（+CGACT? / ^DSAMBR / +CGEQOSRDP），页面通过统一 API
// 读取：
//
//   api.qos.get     -> QosPayload（命中缓存零 AT；冷缓存时读一次）
//   api.qos.cached  -> { value: QosPayload, available, freshness }
//
// 页面只做显示映射（QCI 文案、kbps -> Mbps），不再自己解析 AT 应答。

export interface QosPayload {
  /** 最低的已激活 PDP 上下文（+CGACT?），没有上下文时为 undefined */
  active_cid?: number;
  /** ^DSAMBR 上报的订阅速率，单位 kbps（页面除以 1000 显示 Mbps） */
  ambr_down_kbps?: number;
  ambr_up_kbps?: number;
  /** ^DSAMBR 回显的 APN */
  ambr_apn?: string;
  /** +CGEQOSRDP 的 QoS 等级，原样文本，页面用 qciLabel 映射文案 */
  qci?: string;
}
