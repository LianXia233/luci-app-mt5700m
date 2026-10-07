import React, { useEffect, useRef, useState } from 'react';
import { Banner, Button, Input, Modal, Progress, Steps, Toast, Typography } from '@douyinfe/semi-ui';
import { ATService, type ATResponse } from '@/services/at';
import { fotaStart, fotaState as readFotaState, type FotaState } from '@/services/fota';
import { refreshSharedStateFeed, useSharedStateTopic } from '@/services/stateCache';
import { useATReady } from '@/hooks/useATReady';
import { useMediaQuery } from '@/hooks/useMediaQuery';
import { QUERY_COMPACT } from '@/styles/breakpoints';
import { PageCard, Panel, RefreshBtn } from '@/ui/widgets';

// 升级流程在后端 modules/system/fota.rs（初始化、状态机、续传、刷写）；
// 这里只订阅 fota.progress 推送，页面不再发 AT、不再自己跑状态机。
const at = () => ATService.getInstance();

const SystemUpgrade: React.FC = () => {
  const isNarrow = useMediaQuery(QUERY_COMPACT);
  const modemEntry = useSharedStateTopic('modem');
  const [agreed, setAgreed] = useState(false);
  const [showAgree, setShowAgree] = useState(true);
  const [version, setVersion] = useState('');
  const [loading, setLoading] = useState(false);
  const [upgrading, setUpgrading] = useState(false);
  const [progress, setProgress] = useState(0);
  const [step, setStep] = useState(0);
  const [url, setUrl] = useState('');
  const [fotaState, setFotaState] = useState(10);
  const pollTimer = useRef<number | null>(null);
  const pollInFlight = useRef(false);
  const lastPolledState = useRef<number | null>(null);
  // 首帧只是"对齐"：页面打开时流程可能早就结束了，这时不该再把历史状态
  // 当成新变化弹一遍提示（旧页面重新打开时是安静的）。
  const hydrated = useRef(false);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (pollTimer.current !== null) window.clearInterval(pollTimer.current);
      pollTimer.current = null;
    };
  }, []);

  const fetchVersion = async () => {
    if (!mounted.current) return;
    setLoading(true);
    try {
      // modem.get 就是模组信息路由（ATI/AT+CGSN 的解析在 modules/modem），
      // revision 与本页主用的 modem 主题同源，所以两处显示的版本必然一致。
      const res = await at().apiCommand<{ revision?: string }>('modem.get');
      if (mounted.current && res.success && typeof res.data?.revision === 'string' && res.data.revision) {
        setVersion(res.data.revision);
      }
    } catch {
      if (mounted.current) Toast.error('获取版本失败');
    } finally {
      if (mounted.current) setLoading(false);
    }
  };

  useATReady(async () => {
    const snapshot = await refreshSharedStateFeed();
    if (!mounted.current) return;
    const revision = snapshot?.modem?.value?.revision;
    if (typeof revision === 'string' && revision) setVersion(revision);
    else await fetchVersion();
  });

  useEffect(() => {
    const revision = modemEntry?.value?.revision;
    if (typeof revision === 'string' && revision) setVersion(revision);
  }, [modemEntry]);

  /** 把后端快照映射到界面状态（步骤、进度、状态码），并给出状态变化的提示。 */
  const applySnapshot = (snap: FotaState) => {
    if (!mounted.current) return;
    const first = !hydrated.current;
    hydrated.current = true;
    const previousState = lastPolledState.current;
    if (snap.state !== null) lastPolledState.current = snap.state;
    if (typeof snap.step === 'number') setStep(snap.phase === 'error' ? 0 : snap.step);
    if (snap.state !== null) setFotaState(snap.state);
    if (typeof snap.progress === 'number') setProgress(Math.max(0, Math.min(100, snap.progress)));

    const changed = previousState !== snap.state && !first;
    switch (snap.state) {
      case 11:
        if (changed) Toast.info('正在查询新版本...');
        break;
      case 12:
        if (changed) Toast.info('发现新版本');
        break;
      case 13:
        Toast.error('查询新版本失败');
        break;
      case 14:
        Toast.error('服务器无新版本');
        break;
      case 20:
        Toast.error('固件下载失败');
        break;
      case 31:
        if (changed) Toast.info('下载挂起，尝试续传');
        break;
      case 40:
        if (changed) Toast.success('固件下载完成');
        break;
      case 50:
        if (changed) Toast.info('正在准备升级...');
        break;
      default:
        break;
    }

    if (snap.phase === 'done') {
      stopPolling();
      setUpgrading(false);
      setStep(snap.step || 4);
      if (!first) Toast.success('固件升级已开始，设备即将重启');
    } else if (snap.phase === 'error') {
      stopPolling();
      setUpgrading(false);
      setStep(0);
      if (!first && snap.error && snap.state === null) Toast.error(snap.error);
    } else if (snap.running) {
      setUpgrading(true);
    } else {
      // 流程已结束但没有终态标记：不再假装在跑。
      stopPolling();
      setUpgrading(false);
    }
  };

  const stopPolling = () => {
    if (pollTimer.current !== null) window.clearInterval(pollTimer.current);
    pollTimer.current = null;
  };

  /** 订阅推送：状态变化立即反映，轮询只是兜底。 */
  useEffect(() => {
    const handle = (response: ATResponse) => {
      if (!('type' in response) || response.type !== 'fota.progress') return;
      applySnapshot(response.data as FotaState);
    };
    at().subscribe(handle);
    return () => at().unsubscribe(handle);
  }, []);

  // 页面打开时后端可能还在升级（比如刷新页面）：从任务表恢复界面，
  // 与扫频面板同样的处理，否则用户会看到一个"可以重新开始"的假象。
  useATReady(async () => {
    const snap = await readFotaState();
    if (!mounted.current || !snap) return;
    applySnapshot(snap);
    if (snap.running) startPolling();
  });

  const startPolling = () => {
    if (pollTimer.current !== null) window.clearInterval(pollTimer.current);
    pollInFlight.current = false;
    pollTimer.current = window.setInterval(() => {
      if (pollInFlight.current || !mounted.current) return;
      pollInFlight.current = true;
      void (async () => {
        try {
          const snap = await readFotaState();
          if (!mounted.current) return;
          if (snap) applySnapshot(snap);
        } catch {
          if (mounted.current) Toast.error('读取升级进度失败，将稍后重试');
        } finally {
          pollInFlight.current = false;
        }
      })();
    }, 1000);
  };

  const start = async () => {
    if (upgrading) return;
    setUpgrading(true);
    setProgress(0);
    setStep(1);
    lastPolledState.current = null;
    try {
      const res = await fotaStart(url.trim());
      if (!mounted.current) return;
      if (!res.success) {
        Toast.error(res.error || '设置 FOTA 地址失败');
        setUpgrading(false);
        setStep(0);
        return;
      }
      setStep(2);
      startPolling();
    } catch {
      if (mounted.current) {
        Toast.error('固件升级失败');
        setUpgrading(false);
        setStep(0);
      }
    }
  };

  return (
    <>
      <Modal
        title="免责声明"
        visible={!agreed && showAgree}
        okText="同意并继续"
        cancelText="不同意"
        onOk={() => {
          setAgreed(true);
          setShowAgree(false);
        }}
        onCancel={() => setShowAgree(false)}
      >
        <Typography.Title heading={6}>固件升级免责声明</Typography.Title>
        <ol className="agree-list">
          <li>升级过程中请确保供电稳定，切勿断电。</li>
          <li>升级过程中请勿进行其他操作。</li>
          <li>完成后设备将自动重启，请耐心等待。</li>
          <li>操作不当可能导致设备无法正常使用。</li>
          <li>升级前请备份重要数据。</li>
        </ol>
      </Modal>

      {agreed ? (
        <PageCard title="系统升级">
          <div className="form-stack">
            <Panel title="当前版本" extra={<RefreshBtn onClick={fetchVersion} loading={loading} />}>
              <Typography.Text className="mono">{version || '未知'}</Typography.Text>
            </Panel>

            <Panel title="升级步骤">
              <Steps
                className="upgrade-steps"
                current={step}
                type="basic"
                size="small"
                direction={isNarrow ? 'vertical' : 'horizontal'}
              >
                <Steps.Step title="准备" description="设置升级参数" />
                <Steps.Step title="初始化" description="初始化 FOTA" />
                <Steps.Step title="下载" description="下载固件" />
                <Steps.Step title="升级" description="执行升级" />
                <Steps.Step title="完成" description="升级完成" />
              </Steps>
            </Panel>

            {step === 0 ? (
              <Panel title="升级参数">
                <div className="form-stack">
                  <Input
                    prefix="FOTA"
                    value={url}
                    onChange={setUrl}
                    placeholder="http://fota.example.com/path/"
                  />
                  <Typography.Text type="tertiary">仅支持 http 协议</Typography.Text>
                  <div className="action-row">
                    <Button theme="solid" type="primary" loading={upgrading} onClick={start}>
                      开始升级
                    </Button>
                  </div>
                </div>
              </Panel>
            ) : null}

            {(step === 2 || step === 3) && (
              <Panel title="下载进度">
                <Progress percent={progress} showInfo format={(p) => `${p}%${fotaState === 50 ? ' (正在升级...)' : ''}`} />
              </Panel>
            )}

            {upgrading ? (
              <Banner type="warning" closeIcon={null} description="升级过程中请勿断电或执行其他操作，完成后设备将自动重启。" />
            ) : null}
          </div>
        </PageCard>
      ) : (
        <PageCard title="系统升级">
          <div className="form-stack">
            <Banner type="info" closeIcon={null} description="请先阅读并同意免责声明后再进行固件升级。" />
            <div className="action-row">
              <Button onClick={() => setShowAgree(true)}>
                查看免责声明
              </Button>
            </div>
          </div>
        </PageCard>
      )}
    </>
  );
};

export default SystemUpgrade;
