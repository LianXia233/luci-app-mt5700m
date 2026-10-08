import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  Banner,
  Button,
  Checkbox,
  Input,
  Layout,
  Modal,
  Progress,
  Space,
  TextArea,
  Toast,
  Typography,
} from '@douyinfe/semi-ui';
import { IconSend, IconRefresh, IconPlus, IconDelete } from '@douyinfe/semi-icons';
import { ATService } from '@/services/at';
import type { ATResponse } from '@/services/at';
import { backendMessage } from '@/services/backendError';
import { useATReady } from '@/hooks/useATReady';
import {
  normalizePhoneNumber,
  isValidPhoneNumber,
  getCachedSentMessages,
  saveSentMessageToCache,
  parseMessageTime,
  SMS_CACHE_KEY,
} from '@/modem/sms';
import type { SMS, SmsList, SmsStatus, SmsStorageInfo, SmsStats } from '@/modem/sms';

const { Sider, Content } = Layout;

interface Contact {
  number: string;
  lastMessage: string;
  lastTime: string;
  unreadCount: number;
  messages: SMS[];
}

const at = () => ATService.getInstance();

// 慢路由的命令超时：默认 6s 只够一条 AT，短信读/发/删都要给足预算。
const SMS_READ_TIMEOUT_MS = 15000;
const SMS_SEND_TIMEOUT_MS = 60000;


const formatDetailedTime = (time: string): string => {
  const m = time.match(/(\d{2})\/(\d{2})\/(\d{2}),(\d{2}):(\d{2}):(\d{2})/);
  if (m) return `20${m[1]}年${m[2]}月${m[3]}日 ${m[4]}:${m[5]}:${m[6]}`;
  return time;
};

const formatShortTime = (time: string): string => {
  const m = time.match(/,(\d{2}):(\d{2})/);
  return m ? `${m[1]}:${m[2]}` : time;
};

const nowTimeStr = (): string => {
  const d = new Date();
  const pad = (n: number) => String(n).padStart(2, '0');
  return `${pad(d.getFullYear() % 100)}/${pad(d.getMonth() + 1)}/${pad(d.getDate())},${pad(
    d.getHours(),
  )}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
};

const SMSCenter: React.FC = () => {
  const [contacts, setContacts] = useState<Contact[]>([]);
  const [messages, setMessages] = useState<SMS[]>([]);
  const [selectedContact, setSelectedContact] = useState<string>('');
  const [inputMessage, setInputMessage] = useState('');
  const [newContactNumber, setNewContactNumber] = useState('');
  const [loading, setLoading] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [newMessageModal, setNewMessageModal] = useState(false);
  const [batchDeleteMode, setBatchDeleteMode] = useState(false);
  const [selectedMessages, setSelectedMessages] = useState<number[]>([]);
  const [smsEnabled, setSmsEnabled] = useState(true);
  const [imsEnabled, setImsEnabled] = useState(true);
  const [storageInfo, setStorageInfo] = useState({ used: 0, total: 0 });

  const messagesEndRef = useRef<HTMLDivElement>(null);
  const messagesContainerRef = useRef<HTMLDivElement>(null);
  const shouldAutoScrollRef = useRef(true);
  const refreshInFlight = useRef<Promise<void> | null>(null);
  const selectedContactRef = useRef(selectedContact);
  selectedContactRef.current = selectedContact;

  const buildContacts = useCallback((list: SMS[]) => {
    const map = new Map<string, Contact>();
    list.forEach((msg) => {
      const num = normalizePhoneNumber(msg.number);
      if (!num) return;
      const existing = map.get(num);
      if (existing) {
        existing.messages.push(msg);
        const t = parseMessageTime(msg.time).getTime();
        if (t > parseMessageTime(existing.lastTime).getTime()) {
          existing.lastMessage = msg.content;
          existing.lastTime = msg.time;
        }
      } else {
        map.set(num, {
          number: num,
          lastMessage: msg.content,
          lastTime: msg.time,
          unreadCount: 0,
          messages: [msg],
        });
      }
    });
    const list2 = Array.from(map.values())
      .map((c) => ({
        ...c,
        messages: [...c.messages].sort(
          (a, b) => parseMessageTime(a.time).getTime() - parseMessageTime(b.time).getTime(),
        ),
      }))
      .sort(
        (a, b) => parseMessageTime(b.lastTime).getTime() - parseMessageTime(a.lastTime).getTime(),
      );
    setContacts(list2);
    const sel = selectedContactRef.current;
    if (sel) {
      const contact = list2.find(
        (c) => normalizePhoneNumber(c.number) === normalizePhoneNumber(sel),
      );
      setMessages(contact ? contact.messages : []);
    } else {
      setMessages([]);
    }
  }, []);

  // 存储占用来自后端：+CPMS? 的解析、字段名和去重都在 modules/sms。
  const refreshStorage = useCallback(async () => {
    const res = await at().apiCommand<SmsStorageInfo>('sms.storage', undefined, SMS_READ_TIMEOUT_MS);
    const read = res.success ? res.data?.read : undefined;
    if (read) setStorageInfo({ used: read.used, total: read.total });
  }, []);

  const refresh = useCallback(() => {
    if (refreshInFlight.current) return refreshInFlight.current;

    const request = (async () => {
      setRefreshing(true);
      try {
        const res = await at().apiCommand<SmsList>('sms.list', undefined, SMS_READ_TIMEOUT_MS);
        if (!res.success) {
          Toast.error(backendMessage(res.error, '获取短信列表失败'));
          return;
        }
        const received = res.data?.messages ?? [];
        const cached = getCachedSentMessages();
        buildContacts([...received, ...cached]);
      } catch {
        Toast.error('获取短信列表失败');
      } finally {
        setRefreshing(false);
        refreshInFlight.current = null;
      }
    })();
    refreshInFlight.current = request;
    return request;
  }, [buildContacts]);

  const init = useCallback(async () => {
    const res = await at().apiCommand<SmsStatus>('sms.status', undefined, SMS_READ_TIMEOUT_MS);
    const status = res.success ? res.data : undefined;
    if (typeof status?.imsOn === 'boolean') setImsEnabled(status.imsOn);

    if (status?.enabled !== true) {
      setSmsEnabled(false);
      Toast.warning('请先前往设置页开启短信');
      return;
    }
    setSmsEnabled(true);
    if (status.storage?.read) {
      setStorageInfo({ used: status.storage.read.used, total: status.storage.read.total });
    }
    await refresh();
  }, [refresh]);

  useATReady(init);

  useEffect(() => {
    const atService = at();
    const handler = (response: ATResponse) => {
      if ('type' in response && response.type === 'new_sms') {
        refresh();
        refreshStorage();
      }
    };
    atService.subscribe(handler);
    return () => atService.unsubscribe(handler);
  }, [refresh, refreshStorage]);

  useEffect(() => {
    if (!selectedContact) {
      setMessages([]);
      return;
    }
    const contact = contacts.find(
      (c) => normalizePhoneNumber(c.number) === normalizePhoneNumber(selectedContact),
    );
    setMessages(contact ? contact.messages : []);
  }, [selectedContact, contacts]);

  useEffect(() => {
    if (shouldAutoScrollRef.current) {
      messagesEndRef.current?.scrollIntoView({ behavior: 'smooth' });
    }
  }, [messages]);

  const handleMessagesScroll = () => {
    const container = messagesContainerRef.current;
    if (!container) return;
    shouldAutoScrollRef.current =
      container.scrollHeight - container.scrollTop - container.clientHeight < 80;
  };

  const handleSelectContact = (number: string) => {
    shouldAutoScrollRef.current = true;
    setSelectedContact(number);
  };

  const handleMessageSelect = (msg: SMS, checked: boolean) => {
    setSelectedMessages((prev) =>
      checked ? [...prev, msg.index] : prev.filter((i) => i !== msg.index),
    );
  };

  const handleRefresh = async () => {
    setRefreshing(true);
    try {
      await Promise.all([refresh(), refreshStorage()]);
    } finally {
      setRefreshing(false);
    }
  };

  // 中文走 UCS2 一条只能放 70 字，纯英文数字走 7bit 能放 160 字，
  // 超出就会拆成多条分别计费，这个差别得让用户看得见。
  // 分片数/字符集来自后端编码器（sms.analyze）：提示里承诺的“拆成 N 条”
  // 和真正发送时用的是同一套规则，前端不再自己算。
  const [hintStats, setHintStats] = useState<SmsStats | null>(null);

  useEffect(() => {
    const text = inputMessage.trim();
    if (!text) {
      setHintStats(null);
      return;
    }
    let cancelled = false;
    const timer = window.setTimeout(async () => {
      const res = await at().apiCommand<SmsStats>('sms.analyze', { text });
      if (!cancelled && res.success && res.data) setHintStats(res.data);
    }, 120);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [inputMessage]);

  const composeHint = useMemo(() => {
    const text = inputMessage.trim();
    if (!text || !hintStats) return '';
    const { encoding, parts, chars } = hintStats;
    const base = `${chars} 字 · ${encoding === 'UCS2' ? '含中文等字符，按 UCS2 编码' : '纯 ASCII，按 GSM 7bit 编码'}`;
    return parts > 1 ? `${base} · 将拆成 ${parts} 条发送` : base;
  }, [inputMessage, hintStats]);

  const handleSend = async (explicitTarget?: string) => {
    const content = inputMessage.trim();
    if (!content) {
      Toast.warning('请输入短信内容');
      return;
    }
    const target = (explicitTarget || '').trim() || selectedContact || newContactNumber.trim();
    if (!target) {
      Toast.warning('请输入联系人号码');
      return;
    }
    if (!isValidPhoneNumber(target)) {
      Toast.warning('请输入正确的5-19位手机号码');
      return;
    }

    setLoading(true);
    try {
      // 一条路由发完整条短信：PDU 编码、分片、逐片 AT+CMGS 和“第 i/n 条失败”
      // 的措辞都在 modules/sms，页面只负责提示。
      const res = await at().apiCommand(
        'sms.send',
        { number: target, text: content },
        SMS_SEND_TIMEOUT_MS,
      );
      if (!res.success) {
        throw new Error(backendMessage(res.error, '发送失败'));
      }

      const sent: SMS = {
        index: -1,
        content,
        number: normalizePhoneNumber(target),
        time: nowTimeStr(),
        type: 'sent',
      };
      saveSentMessageToCache(sent);

      setInputMessage('');
      setSelectedContact(sent.number);
      setNewContactNumber('');
      if (explicitTarget) setNewMessageModal(false);

      await Promise.all([refresh(), refreshStorage()]);
      Toast.success('发送成功');
    } catch (e) {
      Toast.error(e instanceof Error ? e.message : '发送失败');
    } finally {
      setLoading(false);
    }
  };

  const handleDeleteMessage = (msg: SMS) => {
    Modal.confirm({
      title: '确认删除',
      content:
        msg.index < 0
          ? '确定删除这条本地缓存消息吗？此操作无法撤销。'
          : '确定删除这条设备短信吗？此操作无法撤销。',
      okText: '删除',
      cancelText: '取消',
      okButtonProps: { theme: 'solid', type: 'danger' },
      onOk: async () => {
        setLoading(true);
        try {
          if (msg.index < 0) {
            const updated = getCachedSentMessages().filter(
              (c) => !(c.content === msg.content && c.number === msg.number && c.time === msg.time),
            );
            localStorage.setItem(SMS_CACHE_KEY, JSON.stringify(updated));
          } else {
            const res = await at().apiCommand(
              'sms.delete',
              { index: msg.index },
              SMS_READ_TIMEOUT_MS,
            );
            if (!res.success) throw new Error(backendMessage(res.error, '删除失败'));
            await new Promise((r) => setTimeout(r, 500));
          }
          await Promise.all([refresh(), refreshStorage()]);
          Toast.success('删除成功');
        } catch (e) {
          Toast.error(e instanceof Error ? e.message : '删除失败');
        } finally {
          setLoading(false);
        }
      },
    });
  };

  const handleBatchDelete = () => {
    if (selectedMessages.length === 0) {
      Toast.warning('请先选择要删除的短信');
      return;
    }
    const selected = messages.filter((m) => selectedMessages.includes(m.index));
    Modal.confirm({
      title: '确认删除',
      content: `确定删除选中的 ${selectedMessages.length} 条短信吗？此操作无法撤销。`,
      okText: '删除',
      cancelText: '取消',
      okButtonProps: { theme: 'solid', type: 'danger' },
      onOk: async () => {
        setLoading(true);
        try {
          const cachedToRemove = selected.filter((m) => m.index < 0);
          const deviceToRemove = selected.filter((m) => m.index >= 0);

          if (cachedToRemove.length > 0) {
            const updated = getCachedSentMessages().filter(
              (c) =>
                !cachedToRemove.some(
                  (m) => m.content === c.content && m.number === c.number && m.time === c.time,
                ),
            );
            localStorage.setItem(SMS_CACHE_KEY, JSON.stringify(updated));
          }

          for (const m of deviceToRemove) {
            const res = await at().apiCommand(
              'sms.delete',
              { index: m.index },
              SMS_READ_TIMEOUT_MS,
            );
            if (!res.success) throw new Error(backendMessage(res.error, '批量删除失败'));
            await new Promise((r) => setTimeout(r, 300));
          }

          setSelectedMessages([]);
          setBatchDeleteMode(false);
          await Promise.all([refresh(), refreshStorage()]);
          Toast.success(`成功删除 ${selectedMessages.length} 条短信`);
        } catch (e) {
          Toast.error(e instanceof Error ? e.message : '批量删除失败');
        } finally {
          setLoading(false);
        }
      },
    });
  };

  const renderMessage = (msg: SMS) => (
    <div
      key={`${msg.index}-${msg.time}-${msg.content.length}`}
      className={`sms-message-row ${msg.type === 'sent' ? 'sent' : ''}`}
    >
      {batchDeleteMode && (
        <Checkbox
          checked={selectedMessages.includes(msg.index)}
          onChange={(e) => handleMessageSelect(msg, !!e.target.checked)}
          style={{ marginTop: 6 }}
        />
      )}
      <div className="sms-message-body">
        <Typography.Text type="tertiary" size="small" className="sms-message-time">
          {formatDetailedTime(msg.time)}
        </Typography.Text>
        <div className={`sms-bubble ${msg.type === 'sent' ? 'sent' : ''}`}>
          {msg.content || '(无内容)'}
        </div>
      </div>
      {!batchDeleteMode && (
        <Button
          type="danger"
          theme="borderless"
          size="small"
          icon={<IconDelete />}
          aria-label="删除该条短信"
          className="sms-message-delete"
          onClick={() => handleDeleteMessage(msg)}
        />
      )}
    </div>
  );

  const storagePct =
    storageInfo.total > 0 ? Math.round((storageInfo.used / storageInfo.total) * 100) : 0;

  return (
    <div className="sms-page">
      <div className="sms-toolbar">
        <div className="sms-toolbar-main">
          <Typography.Title heading={5} style={{ margin: 0 }}>
            短信中心
          </Typography.Title>
          {storageInfo.total > 0 && (
            <Space spacing={4}>
              <Typography.Text type="tertiary" size="small">
                存储空间 {storageInfo.used}/{storageInfo.total}
              </Typography.Text>
              <Progress percent={storagePct} showInfo={false} size="small" style={{ width: 80 }} />
            </Space>
          )}
        </div>
        <div className="sms-toolbar-actions">
          <Button
            theme="solid"
            type="primary"
            icon={<IconPlus />}
            onClick={() => setNewMessageModal(true)}
            disabled={!smsEnabled || !imsEnabled}
          >
            发送新短信
          </Button>
          {selectedContact && (
            <>
              <Button
                theme="light"
                type={batchDeleteMode ? 'tertiary' : 'danger'}
                icon={batchDeleteMode ? undefined : <IconDelete />}
                onClick={() => {
                  setBatchDeleteMode((v) => !v);
                  setSelectedMessages([]);
                }}
              >
                {batchDeleteMode ? '取消批量删除' : '批量删除'}
              </Button>
              {batchDeleteMode && (
                <Button
                  onClick={() => {
                    if (messages.length > 0 && selectedMessages.length === messages.length) {
                      setSelectedMessages([]);
                    } else {
                      setSelectedMessages(messages.map((m) => m.index));
                    }
                  }}
                >
                  {selectedMessages.length === messages.length && messages.length > 0
                    ? '取消全选'
                    : '全选'}
                </Button>
              )}
            </>
          )}
          <Button icon={<IconRefresh />} onClick={handleRefresh} loading={refreshing}>
            刷新
          </Button>
        </div>
      </div>

      {!imsEnabled && (
        <Banner
          type="warning"
          closeIcon={null}
          description="IMS未开启，短信功能不可用，请前往设置页开启。"
        />
      )}
      {!smsEnabled && <Banner type="warning" closeIcon={null} description="请先前往设置页开启短信" />}

      <Layout className="sms-shell">
        <Sider className="sms-contacts">
          {contacts.length === 0 ? (
            <div className="sms-contact-empty">
              <Typography.Text type="tertiary">暂无联系人</Typography.Text>
            </div>
          ) : (
            <div className="sms-contact-list">
              {contacts.map((contact) => {
                const active =
                  normalizePhoneNumber(selectedContact) === normalizePhoneNumber(contact.number);
                return (
                  <div
                    key={contact.number}
                    onClick={() => handleSelectContact(contact.number)}
                    className={`sms-contact ${active ? 'active' : ''}`}
                  >
                    <div className="sms-contact-line">
                      <Typography.Text strong className="sms-contact-name">
                        {contact.number}
                      </Typography.Text>
                      <Typography.Text type="tertiary" size="small" className="sms-contact-time">
                        {formatShortTime(contact.lastTime)}
                      </Typography.Text>
                    </div>
                    <Typography.Text type="tertiary" size="small" className="sms-contact-preview">
                      {contact.lastMessage || '(空)'}
                    </Typography.Text>
                  </div>
                );
              })}
            </div>
          )}
        </Sider>
        <Content className="sms-thread">
          {selectedContact ? (
            <>
              <div className="sms-thread-head">
                <Typography.Text strong>{selectedContact}</Typography.Text>
              </div>
              <div
                className="sms-messages"
                ref={messagesContainerRef}
                onScroll={handleMessagesScroll}
              >
                {messages.length > 0 ? (
                  <>
                    {messages.map(renderMessage)}
                    <div ref={messagesEndRef} />
                  </>
                ) : (
                  <div className="sms-thread-empty">
                    {loading ? (
                      <Typography.Text type="tertiary">加载中...</Typography.Text>
                    ) : (
                      <Typography.Text type="tertiary">没有消息记录</Typography.Text>
                    )}
                  </div>
                )}
              </div>
              {batchDeleteMode ? (
                <div className="sms-batch-actions">
                  <Button
                    theme="solid"
                    type="danger"
                    icon={<IconDelete />}
                    onClick={handleBatchDelete}
                    loading={loading}
                    disabled={selectedMessages.length === 0}
                  >
                    删除选中的 {selectedMessages.length} 条短信
                  </Button>
                </div>
              ) : (
                <div className="sms-composer">
                  <TextArea
                    value={inputMessage}
                    onChange={setInputMessage}
                    placeholder={!imsEnabled ? 'IMS功能未开启，无法发送短信' : '请输入短信内容'}
                    autosize={{ minRows: 2, maxRows: 4 }}
                    disabled={!imsEnabled}
                  />
                  <Button
                    theme="solid"
                    type="primary"
                    icon={<IconSend />}
                    onClick={() => handleSend()}
                    loading={loading}
                    disabled={!imsEnabled}
                  >
                    发送
                  </Button>
                </div>
              )}
              {composeHint ? (
                <Typography.Text type="tertiary" size="small" className="sms-composer-hint">
                  {composeHint}
                </Typography.Text>
              ) : null}
            </>
          ) : (
            <div className="sms-thread-empty">
              <Typography.Text type="tertiary">
                选择联系人或点击"发送新短信"按钮开始新对话
              </Typography.Text>
            </div>
          )}
        </Content>
      </Layout>

      <Modal
        title="发送新短信"
        visible={newMessageModal}
        footer={null}
        onCancel={() => {
          setNewMessageModal(false);
          setNewContactNumber('');
          setInputMessage('');
        }}
      >
        <Space vertical spacing={16} style={{ width: '100%' }}>
          <Input
            placeholder="请输入新联系人号码"
            value={newContactNumber}
            onChange={setNewContactNumber}
          />
          <TextArea
            placeholder="请输入短信内容"
            value={inputMessage}
            onChange={setInputMessage}
            autosize={{ minRows: 4, maxRows: 6 }}
          />
          <Button
            theme="solid"
            type="primary"
            icon={<IconSend />}
            loading={loading}
            block
            onClick={async () => {
              const hasNumber = !!newContactNumber.trim();
              const hasContent = !!inputMessage.trim();
              await handleSend(newContactNumber);
              if (hasNumber && hasContent) setNewMessageModal(false);
            }}
          >
            发送
          </Button>
        </Space>
      </Modal>
    </div>
  );
};

export default SMSCenter;
