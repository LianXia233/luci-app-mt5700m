import React, { useEffect } from 'react';
import { Notification } from '@douyinfe/semi-ui';
import { IconComment, IconPhone, IconAlertTriangle } from '@douyinfe/semi-icons';
import { ATResponse, ATService } from '@/services/at';

const NotificationHandler: React.FC = () => {
  useEffect(() => {
    const atService = ATService.getInstance();

    const handleNotification = (response: ATResponse) => {
      // 先排除命令应答（data 是 string），再把事件 data 收窄为结构化对象，
      // 否则 Record<string, unknown> 联合里的字段取出来是 unknown，没法渲染。
      if (
        !('type' in response) ||
        typeof response.data !== 'object' ||
        response.data === null
      ) {
        return;
      }
      const data = response.data as Record<string, unknown>;
      const str = (v: unknown): string => (typeof v === 'string' ? v : '');

      switch (response.type) {
        case 'incoming_call': {
          const number = str(data.number);
          const time = str(data.time);
          const state = str(data.state);
          if (number || time || state) {
            const stateText =
              state === 'ringing' ? '振铃中' : state === 'ended' ? '已挂机' : state;
            Notification.info({
              title: `来电话啦 - ${stateText}`,
              icon: <IconPhone style={{ color: state === 'ringing' ? 'var(--semi-color-primary)' : '#10b981' }} />,
              content: (
                <>
                  <div>号码：{number}</div>
                  <div>时间：{time}</div>
                </>
              ),
              duration: 0,
              position: 'topRight',
            });
          }
          break;
        }
        case 'new_sms': {
          const sender = str(data.sender);
          const content = str(data.content);
          const time = str(data.time);
          if (sender || content || time) {
            Notification.info({
              title: '来短信啦',
              icon: <IconComment style={{ color: 'var(--semi-color-primary)' }} />,
              content: (
                <>
                  <div>发信人：{sender}</div>
                  <div>时间：{time}</div>
                  <div>内容：{content}</div>
                </>
              ),
              duration: 0,
              position: 'topRight',
            });
          }
          break;
        }
        case 'memory_full': {
          const message = str(data.message);
          if (message) {
            Notification.warning({
              title: '存储空间警告',
              icon: <IconAlertTriangle style={{ color: '#f59e0b' }} />,
              content: message,
              duration: 0,
              position: 'topRight',
            });
          }
          break;
        }
        default:
          break;
      }
    };

    atService.subscribe(handleNotification);
    return () => atService.unsubscribe(handleNotification);
  }, []);

  return null;
};

export default NotificationHandler;
