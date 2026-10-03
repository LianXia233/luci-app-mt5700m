import React, { useEffect, useState } from 'react';
import { Button, Tag, Typography } from '@douyinfe/semi-ui';
import { ATResponse, ATService, StateSnapshot } from '@/services/at';
import {
  ACT_TYPES,
  parseC5greg,
  parseCgpaddr,
  parseLendc,
  parseNrTxPower,
  parseTxPower,
  REG_STATES,
  type EndcStatus,
  type NrTxPower,
  type PdpAddress,
  type Reg5G,
  type TxPower,
} from '@/modem/status';
import { Kv, Panel, SectionHeader, TwoCol } from '@/ui/widgets';

const at = () => ATService.getInstance();

const dbm = (v: number | null): string => (v == null ? '—' : `${v} dBm`);

export const Diagnostics: React.FC = () => {
  const [endc, setEndc] = useState<EndcStatus | null>(null);
  const [reg, setReg] = useState<Reg5G | null>(null);
  const [tx, setTx] = useState<TxPower | null>(null);
  const [nrTx, setNrTx] = useState<NrTxPower[]>([]);
  const [addrs, setAddrs] = useState<PdpAddress[]>([]);
  const [loading, setLoading] = useState(false);

  // ---- Async Architecture：事件 / SWR 快照 -> 页面状态 ----
  // 后端采集器把 ENDC / 发射功率 / 注册状态写入 StateCache 并推送 *.updated
  // 事件（endc 5s、txpower 5s、registration 3s），页面只消费事件与首屏快照，
  // 不再自己轮询。手动“刷新”按钮保留：点一下立即发命令拿最新值。
  // 采集器在非对应组网下会发布空对象，因此这里只在拿到实际字段时才覆盖状态。

  const applyEndcEvent = (v: Record<string, unknown>) => {
    if (typeof v.available !== 'number') return;
    setEndc({
      available: v.available === 1,
      plmnAvailable: v.plmnAvailable === 1,
      restricted: v.restricted === 1,
      established: v.established === 1,
    });
  };

  const applyRegEvent = (v: Record<string, unknown>) => {
    const stat = Number(v.state);
    if (!Number.isFinite(stat)) return;
    const actNum = Number(v.act);
    setReg((prev) => ({
      stat,
      statText: REG_STATES[stat] ?? `状态 ${stat}`,
      registered: stat === 1 || stat === 5,
      tac: typeof v.tac === 'string' ? v.tac : (prev?.tac ?? ''),
      ci: typeof v.ci === 'string' ? v.ci : (prev?.ci ?? ''),
      act: Number.isFinite(actNum) ? (ACT_TYPES[actNum] ?? '') : (prev?.act ?? ''),
      nssai: typeof v.nssai === 'string' ? v.nssai : (prev?.nssai ?? ''),
    }));
  };

  const applyTxEvent = (v: Record<string, unknown>) => {
    const n = (k: string): number | null => (typeof v[k] === 'number' ? (v[k] as number) : null);
    if (n('pusch') === null && n('pucch') === null && n('total') === null) return;
    setTx({ total: n('total'), pusch: n('pusch'), pucch: n('pucch'), srs: n('srs'), prach: n('prach') });
  };

  const applyNrTxEvent = (v: Record<string, unknown>) => {
    if (!Array.isArray(v.carriers)) return;
    const n = (r: Record<string, unknown>, k: string): number | null =>
      typeof r[k] === 'number' ? (r[k] as number) : null;
    setNrTx(
      (v.carriers as Record<string, unknown>[]).map((c) => ({
        pusch: n(c, 'pusch'),
        pucch: n(c, 'pucch'),
        srs: n(c, 'srs'),
        prach: n(c, 'prach'),
        freq: n(c, 'freq'),
      })),
    );
  };

  // SWR 首屏：先渲染缓存里最近一次后台采集值，事件流随后补齐。
  const applySnapshot = (snap: StateSnapshot) => {
    const pick = (topic: string): Record<string, unknown> | null => {
      const v = snap[topic]?.value;
      return v && typeof v === 'object' ? (v as Record<string, unknown>) : null;
    };
    const endcV = pick('endc');
    if (endcV) applyEndcEvent(endcV);
    const regV = pick('registration');
    if (regV) applyRegEvent(regV);
    const txV = pick('txpower');
    if (txV) applyTxEvent(txV);
    const nrV = pick('nr_txpower');
    if (nrV) applyNrTxEvent(nrV);
  };

  useEffect(() => {
    void at()
      .requestSnapshot()
      .then((snap) => {
        if (snap) applySnapshot(snap);
      });
    const handle = (response: ATResponse) => {
      if (!('type' in response)) return;
      if (response.type === 'endc.updated') {
        applyEndcEvent(response.data as Record<string, unknown>);
        return;
      }
      if (response.type === 'registration.updated') {
        applyRegEvent(response.data as Record<string, unknown>);
        return;
      }
      if (response.type === 'txpower.updated') {
        applyTxEvent(response.data as Record<string, unknown>);
        return;
      }
      if (response.type === 'nr_txpower.updated') {
        applyNrTxEvent(response.data as Record<string, unknown>);
        return;
      }
    };
    at().subscribe(handle);
    return () => {
      at().unsubscribe(handle);
    };
  }, []);

  // 手动刷新：立即发命令拿最新值（事件流之外的即时路径）。
  const refresh = async () => {
    setLoading(true);
    try {
      // 这几条都可能因为“当前不是那个组网”而失败，属于正常情况，静默处理。
      // 手册 11.7.2：仅 LTE 主模且单板支持 NR 时查询才有效。
      const lendc = await at().sendCommand('AT^LENDC?');
      setEndc(lendc.success && lendc.data ? parseLendc(String(lendc.data)) : null);

      // 手册 5.27.2：仅当终端注册在 5G 核心网时上报。
      const c5g = await at().sendCommand('AT+C5GREG?');
      setReg(c5g.success && c5g.data ? parseC5greg(String(c5g.data)) : null);

      // 手册 13.23.2：仅 GUL 下有效，ENDC 场景查的是 LTE 侧。
      const txp = await at().sendCommand('AT^TXPOWER?');
      setTx(txp.success && txp.data ? parseTxPower(String(txp.data)) : null);

      // 手册 13.24.2：仅 NR/L 下有效，ENDC 场景查的是 NR 侧。
      const ntxp = await at().sendCommand('AT^NTXPOWER?');
      setNrTx(ntxp.success && ntxp.data ? parseNrTxPower(String(ntxp.data)) : []);

      // 不带 cid 就返回所有已激活 PDP 上下文的地址（手册 7.8.2）。
      const pdp = await at().sendCommand('AT+CGPADDR');
      setAddrs(pdp.success && pdp.data ? parseCgpaddr(String(pdp.data)) : []);
    } finally {
      setLoading(false);
    }
  };

  // 不再 useATReady(refresh)：挂载即直发 LENDC/TXPOWER/NTXPOWER 等慢命令
  // 会占死串口，数据由后端采集器事件（endc/txpower/nr_txpower.updated）
  // + SWR 快照驱动；WS 连接由宿主页面（Info.tsx 的 useATReady）负责。
  // 下面的“刷新”按钮保留为手动即时路径。

  const endcTag = () => {
    if (!endc) return <Tag color="grey">不适用</Tag>;
    if (endc.established) return <Tag color="green">已建立</Tag>;
    if (!endc.available) return <Tag color="red">小区不支持</Tag>;
    if (!endc.plmnAvailable) return <Tag color="orange">运营商未开通</Tag>;
    if (endc.restricted) return <Tag color="orange">网络侧受限</Tag>;
    return <Tag color="blue">支持但未建立</Tag>;
  };

  return (
    <>
      {/* 分组标题与刷新按钮放在一起，和页面其它区块的结构保持一致，
          不再额外套一层同名卡片 */}
      <SectionHeader
        title="连接诊断"
        desc="ENDC 是否真的建起来、5G 核心网注册状态、上行发射功率与 PDP 地址；发射功率接近上限通常说明信号弱或摆位不佳"
        extra={
          <Button size="small" loading={loading} onClick={refresh}>
            刷新
          </Button>
        }
      />
      <TwoCol>
        <Panel title="双连接与注册">
          <Kv
            items={[
              { label: 'ENDC 双连接', value: endcTag() },
              {
                label: '5G 核心网注册',
                value: reg ? (
                  <span>
                    {reg.statText}
                    {reg.act ? ` · ${reg.act}` : ''}
                  </span>
                ) : (
                  '未注册 5GC'
                ),
              },
              { label: 'TAC / 小区', value: reg && reg.tac ? `${reg.tac} / ${reg.ci || '—'}` : '—' },
              { label: '网络切片', value: reg && reg.nssai ? reg.nssai : '—' },
            ]}
          />
        </Panel>

        <Panel title="发射功率">
          <Kv
            items={[
              { label: 'LTE PUSCH / PUCCH', value: tx ? `${dbm(tx.pusch)} / ${dbm(tx.pucch)}` : '—' },
              { label: 'LTE SRS / PRACH', value: tx ? `${dbm(tx.srs)} / ${dbm(tx.prach)}` : '—' },
              ...(tx && tx.total !== null ? [{ label: '2G/3G 总功率', value: dbm(tx.total) }] : []),
              ...nrTx.map((c, i) => ({
                label: `NR CC${i + 1} PUSCH`,
                value: `${dbm(c.pusch)}${c.freq ? ` · ${(c.freq / 1000).toFixed(1)} MHz` : ''}`,
              })),
            ]}
          />
        </Panel>
      </TwoCol>

      {addrs.length > 0 ? (
        <Panel title="PDP 地址">
          <Kv
            items={addrs.map((a) => ({
              label: `CID ${a.cid} · ${a.family}`,
              value: a.address,
            }))}
          />
        </Panel>
      ) : (
        <Typography.Text type="tertiary" size="small">
          没有已激活的 PDP 上下文地址。
        </Typography.Text>
      )}
    </>
  );
};
