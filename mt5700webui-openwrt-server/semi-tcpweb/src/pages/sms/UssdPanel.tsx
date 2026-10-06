import React, { useEffect, useRef, useState } from 'react';
import { Button, Input, Space, Toast, Typography } from '@douyinfe/semi-ui';
import { ATService, type ATResponse } from '@/services/at';
import { USSD_EVENT_TYPE, ussdCancel, ussdSend, type UssdReply } from '@/services/ussd';
import { Field, PageCard } from '@/ui/widgets';

const at = () => ATService.getInstance();

export const UssdPanel: React.FC = () => {
  const [code, setCode] = useState('*133#');
  const [reply, setReply] = useState<UssdReply | null>(null);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  busyRef.current = busy;

  // 网络的回复通过 +CUSD 主动上报回来，不在命令应答里（手册 5.22）。
  // 后端已经把该行解成 sms.ussd 事件（含解码文本与 <m> 文案），页面只收结果。
  useEffect(() => {
    const handle = (response: ATResponse) => {
      if (!('type' in response) || response.type !== USSD_EVENT_TYPE) return;
      const parsed = response.data as UssdReply;
      if (parsed && typeof parsed.m === 'number') {
        setReply(parsed);
        setBusy(false);
      }
    };
    at().subscribe(handle);
    return () => at().unsubscribe(handle);
  }, []);

  // 网络一直不回时不能让按钮永远转圈，到点就放开并说明情况。
  useEffect(() => {
    if (!busy) return undefined;
    const timer = window.setTimeout(() => {
      setBusy(false);
      Toast.warning('等待运营商回复超时，可重试或取消会话');
    }, 30000);
    return () => window.clearTimeout(timer);
  }, [busy]);

  const send = async () => {
    setBusy(true);
    setReply(null);
    try {
      const { reply: inline } = await ussdSend(code);
      // 有些固件会把结果直接放在应答里，有就先显示，否则等 sms.ussd 推送。
      if (inline && inline.text) {
        setReply(inline);
        setBusy(false);
      }
    } catch (err) {
      setBusy(false);
      Toast.error(err instanceof Error ? err.message : 'USSD 请求失败');
    }
  };

  const cancel = async () => {
    try {
      await ussdCancel();
    } catch {
      // 取消失败不打断界面，超时逻辑会放开按钮
    }
    setBusy(false);
  };

  return (
    <PageCard
      title="USSD 查询"
      hint="向运营商发送 USSD 代码，常用于查话费余额与流量，例如中国移动 *133#。代码会按 GSM 7bit 编码后下发。"
    >
      <Field label="USSD 代码">
        <Space>
          <Input
            style={{ width: 200 }}
            value={code}
            placeholder="*133#"
            onChange={setCode}
            onEnterPress={send}
          />
          <Button theme="solid" type="primary" loading={busy} onClick={send}>
            发送
          </Button>
          {busy ? <Button onClick={cancel}>取消会话</Button> : null}
        </Space>
      </Field>

      {reply ? (
        <Field label="运营商回复">
          <div>
            <Typography.Paragraph style={{ whiteSpace: 'pre-wrap', margin: 0 }}>
              {reply.text || '（无内容）'}
            </Typography.Paragraph>
            <Typography.Text type="tertiary" size="small">
              {reply.mText}
              {reply.needsReply ? '：可继续输入选项后再次发送' : ''}
            </Typography.Text>
          </div>
        </Field>
      ) : null}
    </PageCard>
  );
};
