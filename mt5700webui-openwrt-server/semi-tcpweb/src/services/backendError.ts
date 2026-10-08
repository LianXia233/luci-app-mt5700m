// 后端错误信封的展示层。
//
// `services/at.ts` 把统一后端的 `{success:false, error, code, retryable}` 原样交给页面，
// `error` 是 `BackendError::message()`：参数校验类的消息带 "参数无效: " 前缀，
// 模组拒绝类带 "模组拒绝指令: " 前缀。页面要显示的是规则/原因本身，
// 所以在这里做一次前缀剥离——展示文案留在前端，语义留在后端。

/** `参数无效: PIN 码只能是数字` -> `PIN 码只能是数字`；空消息回退到 fallback。 */
export const backendMessage = (raw: unknown, fallback: string): string => {
  const text = String(raw ?? '').trim();
  if (!text) return fallback;
  const stripped = text.replace(/^参数无效:\s*/, '');
  return stripped || fallback;
};

/** 是否是一次参数校验失败（后端 `INVALID_PARAMETER`）。 */
export const isParameterError = (raw: unknown): boolean =>
  String(raw ?? '').startsWith('参数无效');
