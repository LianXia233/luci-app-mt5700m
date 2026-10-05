import React, { useEffect, useMemo, useRef, useState } from 'react';
import {
  Banner,
  Button,
  Collapse,
  Space,
  Spin,
  Table,
  Tag,
  Toast,
  Typography,
} from '@douyinfe/semi-ui';
import { ATService, type ATResponse, type URCData } from '@/services/at';
import { useATReady } from '@/hooks/useATReady';
import { useMediaQuery } from '@/hooks/useMediaQuery';
import { QUERY_MOBILE } from '@/styles/breakpoints';
import {
  checkArfcn,
  checkPci,
  emptyLockItem,
  lockFormFromPayload,
  type LockApplyPayload,
  type LockItem,
  type LockRequest,
  type LockStatePayload,
} from '@/modem/lock';
import { atErrorText, sleep } from '@/modem/atx';
import type { C5gOptionPayload, NeighborsPayload, SsbPayload } from '@/modem/status';
import { type RejectInfo } from '@/modem/reject';
import { AutoRefresh, Field, PageCard, Panel, SectionHeader, TwoCol } from '@/ui/widgets';
import { LockEditor } from '@/ui/LockEditor';
import { SchedulePanel } from './SchedulePanel';
import { ScanPanel } from './ScanPanel';

const at = () => ATService.getInstance();

type Neighbor = { type: string; arfcn: string | number; pci: number; rsrp: string | number; rsrq?: string | number; sinr?: string | number; rxlev?: string; band?: number };

const emptyItem = emptyLockItem;

const NetworkSettings: React.FC = () => {
  const isNarrow = useMediaQuery(QUERY_MOBILE);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [activeKeys, setActiveKeys] = useState<string[]>(['nrLock']);
  const [lteLockType, setLteLockType] = useState(0);
  const [nrLockType, setNrLockType] = useState(0);
  const [lteMobility, setLteMobility] = useState(0);
  const [nrMobility, setNrMobility] = useState(0);
  const [lteItems, setLteItems] = useState<LockItem[]>([emptyItem()]);
  const [nrItems, setNrItems] = useState<LockItem[]>([emptyItem()]);
  const [neighbors, setNeighbors] = useState<Neighbor[]>([]);
  const lockCellRef = useRef<(cell: Neighbor) => void>(() => {});
  const neighborTableData = useMemo(
    () => neighbors.map((cell, index) => ({ ...cell, key: `${cell.type}-${cell.pci}-${index}` })),
    [neighbors],
  );
  const neighborColumns = useMemo(
    () => [
      { title: '制式', dataIndex: 'type' },
      {
        title: '频段',
        dataIndex: 'band',
        render: (value: number | undefined, record: Neighbor) =>
          value ? (record.type === 'NR' ? `n${value}` : `B${value}`) : '—',
      },
      { title: 'ARFCN', dataIndex: 'arfcn' },
      { title: 'PCI', dataIndex: 'pci' },
      { title: 'RSRP', dataIndex: 'rsrp' },
      {
        title: 'RSRQ',
        dataIndex: 'rsrq',
        render: (value: string | number | undefined) => value ?? '—',
      },
      {
        title: 'SINR',
        dataIndex: 'sinr',
        render: (value: string | number | undefined) => value ?? '—',
      },
      {
        title: '操作',
        render: (_value: unknown, record: Neighbor) => (
          <Button size="small" onClick={() => lockCellRef.current(record)}>
            锁定
          </Button>
        ),
      },
    ],
    [],
  );
  const [scanLoading, setScanLoading] = useState(false);
  // 全网扫频期间模组被独占，页面上的轮询和其它命令都要先让路。
  const [scanning, setScanning] = useState(false);
  const scanningRef = useRef(false);
  scanningRef.current = scanning;
  const [reject, setReject] = useState<RejectInfo | null>(null);
  const [auto, setAuto] = useState(false);
  const [interval, setIntervalSec] = useState(5);
  const [option5g, setOption5g] = useState<{ nr_sa_support_flag: number; nr_dc_mode: number; gc_access_mode: number } | null>(null);
  const [ssb, setSsb] = useState<{
    servingCell: { arfcn: string; cid: string; pci: string; rsrp: number; sinr: number; ta: number; ssbs: Array<{ ssbId: number; rsrp: number }> };
    neighborCells: Array<{ pci: string; arfcn: string; rsrp: number; sinr: number; ssbs: Array<{ ssbId: number; rsrp: number }> }>;
  } | null>(null);
  const scanTimer = useRef<number>();

  // 锁频读数来自统一 API：后端 modules/network 解析 ^LTEFREQLOCK?/^NRFREQLOCK?
  // （行布局、十六进制 PCI 都在后端），页面只填表单。
  const fetchCurrent = async () => {
    setLoading(true);
    try {
      const lte = await at().apiCommand<LockStatePayload>('network.lock_get', { rat: 'lte' });
      if (lte.success && lte.data) {
        const parsed = lockFormFromPayload('lte', lte.data);
        setLteLockType(parsed.lockType);
        setLteMobility(parsed.mobility);
        setLteItems(parsed.items);
        if (parsed.lockType !== 0) setActiveKeys((prev) => Array.from(new Set([...prev, 'lteLock'])));
      }
      const nr = await at().apiCommand<LockStatePayload>('network.lock_get', { rat: 'nr' });
      if (nr.success && nr.data) {
        const parsed = lockFormFromPayload('nr', nr.data);
        setNrLockType(parsed.lockType);
        setNrMobility(parsed.mobility);
        setNrItems(parsed.items);
        if (parsed.lockType !== 0) setActiveKeys((prev) => Array.from(new Set([...prev, 'nrLock'])));
      }
      await query5G();
    } catch {
      Toast.error('获取锁频设置失败');
    } finally {
      setLoading(false);
    }
  };

  useATReady(fetchCurrent);

  // 5G 接入模式同样来自后端（modules/network 解析 ^C5GOPTION?），页面只保留
  // "仅 SA / 仅 NSA / SA+NSA / 其他" 的文案映射。
  const query5G = async () => {
    await sleep(200);
    const res = await at().apiCommand<C5gOptionPayload>('network.c5goption');
    if (res.success && res.data && typeof res.data.nr_sa_support_flag === 'number') {
      setOption5g({
        nr_sa_support_flag: res.data.nr_sa_support_flag,
        nr_dc_mode: res.data.nr_dc_mode ?? 0,
        gc_access_mode: res.data.gc_access_mode ?? 0,
      });
    }
  };

  // 锁频写入交给后端：AT 字符串、取值校验、飞行模式循环与两个制式的独立性
  // 都在 modules/network（network.lock_apply）。页面把两个方向放在一次调用里，
  // 后端只循环一次飞行模式——与页面原先的步骤一致。
  const applyLock = async () => {
    if (busy) return;
    setBusy(true);
    setLoading(true);
    try {
      const requests: LockRequest[] = [
        { rat: 'lte', lock_type: lteLockType, mobility: lteMobility, items: lteItems },
        { rat: 'nr', lock_type: nrLockType, mobility: nrMobility, items: nrItems },
      ];
      const res = await at().apiCommand<LockApplyPayload>('network.lock_apply', { locks: requests });
      if (!res.success || !res.data) {
        throw new Error(atErrorText(res as { success: boolean; error?: unknown }, '锁频设置失败'));
      }
      const byRat = new Map((res.data.results || []).map((r) => [r.rat, r]));
      const failed = [
        byRat.get('lte')?.applied === false
          ? `LTE 锁频设置失败${byRat.get('lte')?.error ? `：${byRat.get('lte')?.error}` : ''}`
          : null,
        byRat.get('nr')?.applied === false
          ? `NR 锁频设置失败${byRat.get('nr')?.error ? `：${byRat.get('nr')?.error}` : ''}`
          : null,
      ].filter(Boolean) as string[];
      if (failed.length === 2) throw new Error(failed.join('；'));
      if (failed.length === 1) Toast.warning(`${failed[0]}，另一制式已生效`);
      else Toast.success('锁频设置成功');
      await fetchCurrent();
    } catch (error) {
      Toast.error(error instanceof Error ? error.message : '锁频设置失败');
    } finally {
      setBusy(false);
      setLoading(false);
    }
  };

  // 邻区扫描：AT^MONNC 的行布局、十六进制 PCI、NR 的 1/8 倍率还原和
  // ARFCN->频段表都在后端 modules/cell（cell.neighbors），页面只渲染表格。
  const scanNeighbors = async () => {
    if (busy) return;
    setBusy(true);
    setScanLoading(true);
    try {
      const res = await at().apiCommand<NeighborsPayload>('cell.neighbors');
      if (!res.success || !res.data) {
        Toast.error(atErrorText(res as { success: boolean; error?: unknown }, '扫描邻区失败'));
        return;
      }
      setNeighbors((res.data.cells || []) as Neighbor[]);
    } catch {
      Toast.error('扫描邻区失败');
    } finally {
      setScanLoading(false);
      setBusy(false);
    }
  };

  // 手册 13.14：注册/业务请求被网络拒绝时模组会主动上报原因值。
  useEffect(() => {
    const handle = (response: ATResponse) => {
      if (!('type' in response) || response.type !== 'urc_data') return;
      const urc = response.data as URCData;
      if (urc.type === 'REJINFO') setReject(urc.parsed as RejectInfo);
    };
    at().subscribe(handle);
    return () => at().unsubscribe(handle);
  }, []);

  useEffect(() => {
    if (scanTimer.current) window.clearInterval(scanTimer.current);
    if (!auto) return undefined;
    scanTimer.current = window.setInterval(() => {
      if (scanningRef.current) return;
      scanNeighbors();
    }, interval * 1000);
    return () => {
      if (scanTimer.current) window.clearInterval(scanTimer.current);
    };
  }, [auto, interval]);

  const lockCell = async (cell: Neighbor & { scs?: number }) => {
    if (busy) return;
    setBusy(true);
    let radioOff = false;
    try {
      const kind = cell.type === 'LTE' ? 'lte' : 'nr';
      if (cell.band == null) throw new Error(`无法由频点 ${cell.arfcn} 判断频段，请在上方锁频表单中手动选择`);
      // 前端仍先做一次即时校验（输入框级别的提示），AT 侧的取值范围由后端再验一次。
      const arfcn = checkArfcn(cell.type, String(cell.arfcn).trim());
      const pci = checkPci(kind, String(cell.pci).trim());
      const res = await at().apiCommand<LockApplyPayload>('network.lock_apply', {
        rat: kind,
        lock_type: 2,
        mobility: 0,
        // 扫频结果里带模组实测的子载波间隔时优先用它，缺省由后端按频段补。
        items: [{ band: cell.band, arfcn, pci, scs: cell.scs }],
      });
      const applied = res.data?.results?.every((r) => r.applied) ?? false;
      if (!res.success || !applied) {
        const reason = res.data?.results?.find((r) => r.applied === false)?.error;
        throw new Error(reason || atErrorText(res as { success: boolean; error?: unknown }, '锁定失败'));
      }
      Toast.success(`已锁定 ${cell.type} PCI ${cell.pci}`);
      await fetchCurrent();
    } catch (error) {
      Toast.error(error instanceof Error ? error.message : '锁定失败');
    } finally {
      setBusy(false);
    }
  };

  lockCellRef.current = lockCell;

  // 写入也在后端：AT^C5GOPTION=… 与必要的飞行模式循环由
  // modules/network（network.c5goption_set）完成，页面只负责提示与刷新。
  const set5G = async (option: { nr_sa_support_flag: number; nr_dc_mode: number; gc_access_mode: number }) => {
    if (busy) return;
    setBusy(true);
    try {
      const res = await at().apiCommand('network.c5goption_set', option);
      if (!res.success) throw new Error(atErrorText(res as { success: boolean; error?: unknown }, '设置 5G 接入模式失败'));
      Toast.success('设置成功');
      await query5G();
    } catch (error) {
      Toast.error(error instanceof Error ? error.message : '设置失败');
    } finally {
      setBusy(false);
    }
  };

  // SSB 波束报告来自后端 modules/beam（beam.ssb，解析 AT^NRSSBID? 的固定偏移），
  // 页面只把领域数据放进两组卡片。
  const querySSB = async () => {
    if (busy) return;
    setBusy(true);
    try {
      const res = await at().apiCommand<SsbPayload>('beam.ssb');
      if (!res.success || !res.data || !res.data.servingCell) return;
      setSsb({
        servingCell: {
          arfcn: res.data.servingCell.arfcn ?? '',
          cid: res.data.servingCell.cid ?? '',
          pci: res.data.servingCell.pci ?? '',
          rsrp: res.data.servingCell.rsrp ?? 0,
          sinr: res.data.servingCell.sinr ?? 0,
          ta: res.data.servingCell.ta ?? 0,
          ssbs: res.data.servingCell.ssbs ?? [],
        },
        neighborCells: (res.data.neighborCells ?? []).map((n) => ({
          pci: n.pci ?? '',
          arfcn: n.arfcn ?? '',
          rsrp: n.rsrp ?? 0,
          sinr: n.sinr ?? 0,
          ssbs: n.ssbs ?? [],
        })),
      });
    } catch {
      Toast.error('查询 SSB 失败');
    } finally {
      setBusy(false);
    }
  };

  const optionText = () => {
    if (!option5g) return '未知';
    const { nr_sa_support_flag: sa, nr_dc_mode: dc, gc_access_mode: gc } = option5g;
    if (sa === 1 && dc === 0 && gc === 1) return '仅 SA';
    if (sa === 0 && dc === 1 && gc === 0) return '仅 NSA';
    if (sa === 1 && dc === 1 && gc === 1) return 'SA+NSA';
    return '其他';
  };

  return (
    <Spin spinning={loading}>
      <div className="page-stack">
        <SectionHeader title="锁频与邻区" desc="锁定 LTE / NR 频点、小区或 Band；扫描邻区并查看波束信息" />
        {reject ? (
          <Banner
            type="warning"
            closeIcon
            onClose={() => setReject(null)}
            title={`网络拒绝：${reject.rejectTypeText}（${reject.causeText}）`}
            description={
              <span>
                {reject.ratText} · PLMN {reject.plmn} · {reject.domainText} · 小区 {reject.cellId || '—'}
                {reject.esmCause !== undefined ? ` · ESM 原因 #${reject.esmCause}` : ''}
                <br />
                锁频后掉网时，这里能区分是被网络拒绝还是没有覆盖。
              </span>
            }
          />
        ) : null}
        <PageCard title="锁频设置" extra={<Button theme="solid" type="primary" loading={busy} disabled={scanning} onClick={applyLock}>应用锁频</Button>}>
          <Collapse
            activeKey={activeKeys}
            onChange={(keys) => setActiveKeys(Array.isArray(keys) ? keys : keys === undefined ? [] : [keys])}
          >
            <Collapse.Panel header="4G 锁频设置" itemKey="lteLock">
              <LockEditor
                kind="lte"
                type={lteLockType}
                items={lteItems}
                mobility={lteMobility}
                onTypeChange={setLteLockType}
                onItemsChange={setLteItems}
                onMobilityChange={setLteMobility}
              />
            </Collapse.Panel>
            <Collapse.Panel header="5G 锁频设置" itemKey="nrLock">
              <LockEditor
                kind="nr"
                type={nrLockType}
                items={nrItems}
                mobility={nrMobility}
                onTypeChange={setNrLockType}
                onItemsChange={setNrItems}
                onMobilityChange={setNrMobility}
              />
            </Collapse.Panel>
          </Collapse>
        </PageCard>

        <SchedulePanel />

        <ScanPanel
          disabled={busy}
          onScanningChange={setScanning}
          onLock={(target) =>
            lockCell({
              type: target.type,
              band: target.band,
              arfcn: target.arfcn,
              pci: Number(target.pci),
              rsrp: '',
              scs: target.scs,
            })
          }
        />

        <PageCard
          title="邻区扫描"
          extra={
            <Space>
              <Button loading={scanLoading} disabled={scanning} onClick={scanNeighbors}>
                扫描邻区
              </Button>
              <AutoRefresh
                enabled={auto}
                interval={interval}
                onChange={(e, i) => {
                  setAuto(e);
                  setIntervalSec(i);
                  if (e) scanNeighbors();
                }}
              />
            </Space>
          }
        >
          {/* 窄屏放不下 8 列表格，"锁定"按钮会被裁出屏幕，改成列表逐项展示 */}
          {isNarrow ? (
            <div className="cell-list">
              {neighborTableData.length === 0 ? (
                <div className="cell-list-empty">暂无邻区</div>
              ) : (
                neighborTableData.map((cell) => (
                  <div className="cell-list-item" key={cell.key}>
                    <div className="cell-list-body">
                      <div className="cell-list-main">
                        <Tag size="small" color={cell.type === 'NR' ? 'violet' : 'blue'}>
                          {cell.type}
                        </Tag>
                        <b>
                          {cell.band ? (cell.type === 'NR' ? `n${cell.band}` : `B${cell.band}`) : '—'}
                          {` · ${cell.arfcn}`}
                        </b>
                        <span>PCI {cell.pci}</span>
                      </div>
                      <div className="cell-list-sub">
                        RSRP {cell.rsrp}
                        {cell.rsrq !== undefined ? ` · RSRQ ${cell.rsrq}` : ''}
                        {cell.sinr !== undefined ? ` · SINR ${cell.sinr}` : ''}
                      </div>
                    </div>
                    <Button size="small" onClick={() => lockCellRef.current(cell)}>
                      锁定
                    </Button>
                  </div>
                ))
              )}
            </div>
          ) : (
            <Table
              size="small"
              pagination={false}
              dataSource={neighborTableData}
              empty="暂无邻区"
              columns={neighborColumns}
            />
          )}
        </PageCard>

        <PageCard title="SSB 波束信息" extra={<Button loading={busy} onClick={querySSB}>查询 SSB</Button>}>
          {!ssb ? (
            <Typography.Text type="tertiary">尚未查询。SSB 用于 5G 小区搜索与初始接入。</Typography.Text>
          ) : (
            <TwoCol>
              <Panel title={`服务小区 PCI ${ssb.servingCell.pci}`}>
                <Typography.Text type="tertiary">
                  ARFCN {ssb.servingCell.arfcn} · CID {ssb.servingCell.cid} · RSRP {ssb.servingCell.rsrp} · SINR {ssb.servingCell.sinr} · TA {ssb.servingCell.ta}
                </Typography.Text>
                <Space wrap style={{ marginTop: 8 }}>
                  {ssb.servingCell.ssbs.map((b) => (
                    <Tag key={b.ssbId} color="red">
                      SSB {b.ssbId}: {b.rsrp} dBm
                    </Tag>
                  ))}
                </Space>
              </Panel>
              <Panel title="邻区波束">
                {ssb.neighborCells.length === 0 ? (
                  <Typography.Text type="tertiary">无邻区 SSB</Typography.Text>
                ) : (
                  ssb.neighborCells.map((n) => (
                    <div key={`${n.pci}-${n.arfcn}`} style={{ marginBottom: 8 }}>
                      <Typography.Text>
                        PCI {n.pci} · ARFCN {n.arfcn} · RSRP {n.rsrp}
                      </Typography.Text>
                      <div>
                        {n.ssbs.map((b) => (
                          <Tag key={b.ssbId} size="small">
                            {b.ssbId}:{b.rsrp}
                          </Tag>
                        ))}
                      </div>
                    </div>
                  ))
                )}
              </Panel>
            </TwoCol>
          )}
        </PageCard>

        <SectionHeader
          title="5G 接入模式"
          desc="切换 SA / NSA 会短暂进入飞行模式，设置后自动重新查询"
          extra={<Tag color="red">当前：{optionText()}</Tag>}
        />
        <PageCard title="5G 接入模式设置">
          <Banner type="info" closeIcon={null} description="切换 SA/NSA 会短暂进入飞行模式。当前模式会在设置后重新查询。" />
          <div className="action-row" style={{ marginTop: 12 }}>
            <Button onClick={() => set5G({ nr_sa_support_flag: 1, nr_dc_mode: 0, gc_access_mode: 1 })}>仅 SA</Button>
            <Button onClick={() => set5G({ nr_sa_support_flag: 0, nr_dc_mode: 1, gc_access_mode: 0 })}>仅 NSA</Button>
            <Button onClick={() => set5G({ nr_sa_support_flag: 1, nr_dc_mode: 1, gc_access_mode: 1 })}>SA+NSA</Button>
          </div>
        </PageCard>
      </div>
    </Spin>
  );
};

export default NetworkSettings;
