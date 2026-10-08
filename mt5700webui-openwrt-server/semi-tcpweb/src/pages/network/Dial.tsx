import React, { useMemo, useState } from 'react';
import { Banner, Table, Tag, Typography } from '@douyinfe/semi-ui';
import { ATService } from '@/services/at';
import { useATReady } from '@/hooks/useATReady';
import { useMediaQuery } from '@/hooks/useMediaQuery';
import { QUERY_MOBILE } from '@/styles/breakpoints';
import { Field, Kv, PageCard, RefreshBtn, SectionHeader, TwoCol } from '@/ui/widgets';

/*
 * MT5700M WebUI — 拨号状态（只读）
 * ---------------------------------------------
 * 拨号使用 LuCI（luci-app-mt5700m · 移动数据页），WebUI 不拨号：
 *   - 本页只做只读展示，数据来自后端的 network.autodial / network.usb_mode /
 *     network.interface_cfg / network.pdp_contexts 四条路由（命令、解析都在
 *     modules/network）；
 *   - 不发起任何拨号 / 挂断 / PDP 激活 / APN 写入等操作，避免与 LuCI
 *     拨号流程互相影响和干扰；
 *   - 如需修改拨号配置，请前往 LuCI：网络 → MT5700M → 移动数据。
 */

const at = () => ATService.getInstance();

type DialSettings = {
  enable: number;
  dialMode?: number;
  protocol: string;
  apn: string;
  username: string;
  password: string;
  authType: number;
  usbMode?: number;
  infcfgMode?: number;
  postRoute?: number;
};

type PDPContext = {
  cid: number;
  type: string;
  apn: string;
  pdp_addr?: string;
  active?: boolean;
};

const getDialModeText = (mode?: number) => {
  const map: Record<number, string> = { 1: 'USB网络接口', 2: '转网口模式' };
  return mode == null ? '未识别' : map[mode] || '未知';
};

const getUSBModeText = (mode?: number) => {
  const map: Record<number, string> = {
    0: 'Linux-ECM正常模式',
    1: 'Windows-NCM正常模式',
    2: 'Linux-ECM调试模式',
    3: 'Windows-NCM调试模式',
    4: 'Linux-NCM正常模式',
    5: 'Linux-NCM调试模式',
    6: 'Windows-RNDIS单端口模式',
    7: 'Windows-MBIM单端口模式(暂不支持)',
    8: 'Windows/Linux-PPP端口模式',
  };
  return mode == null ? '未识别' : map[mode] || '未知模式';
};

const getInfcfgModeText = (mode?: number) => {
  if (mode === 1) return 'USB Stick + 网口 E5 数传模式';
  if (mode === 2) return 'USB E5 + 网口 E5 数传模式';
  if (mode === 3) return '网口直通模式';
  return '未配置';
};

const getAuthTypeText = (type?: number) => {
  switch (type) {
    case 0:
      return '无鉴权';
    case 1:
      return 'PAP鉴权';
    case 2:
      return 'CHAP鉴权';
    default:
      return '未知';
  }
};

const getPdpTypeText = (type: string) => {
  switch (type) {
    case 'IP':
      return 'IPv4';
    case 'IPV6':
      return 'IPv6';
    case 'IPV4V6':
      return 'IPv4/IPv6';
    default:
      return type;
  }
};

const adaptDmz = (dmz: { enabled?: boolean; host?: string }) => ({
  enabled: dmz.enabled === true,
  host: dmz.host || '',
});

const NetworkDial: React.FC = () => {
  const isNarrow = useMediaQuery(QUERY_MOBILE);
  const [loading, setLoading] = useState({
    dial: true,
    usb: true,
    infcfg: true,
    dmz: false,
    pdp: true,
  });

  const [settings, setSettings] = useState<DialSettings>({
    enable: 0,
    protocol: '',
    apn: '',
    username: '',
    password: '',
    authType: 0,
  });
  const [dmzConfig, setDmzConfig] = useState({ enabled: false, host: '' });
  const [pdpList, setPdpList] = useState<PDPContext[]>([]);

  // 四条只读路由：命令拼装、应答解析都在后端 modules/network，页面只读字段。
  // 应答里没有的字段表示"这次没读到"，保留上一次显示的值（与旧的失败保留一致）。
  const fetchDialSettings = async () => {
    setLoading((l) => ({ ...l, dial: true }));
    try {
      const res = await at().apiCommand<Partial<DialSettings>>('network.autodial');
      if (res.success && res.data && typeof res.data.enable === 'number') {
        setSettings((prev) => ({ ...prev, ...res.data }));
      }
    } catch {
      // 只读查询失败不打断页面，保留上次值
    } finally {
      setLoading((l) => ({ ...l, dial: false }));
    }
  };

  const fetchUSBMode = async () => {
    setLoading((l) => ({ ...l, usb: true }));
    try {
      const res = await at().apiCommand<{ mode?: number }>('network.usb_mode');
      if (res.success && typeof res.data?.mode === 'number') {
        setSettings((prev) => ({ ...prev, usbMode: res.data!.mode }));
      }
    } catch {
      // 只读查询失败不打断页面
    } finally {
      setLoading((l) => ({ ...l, usb: false }));
    }
  };

  const fetchInfcfg = async () => {
    setLoading((l) => ({ ...l, infcfg: true }));
    try {
      const res = await at().apiCommand<{
        mode?: number;
        postRoute?: number;
        dmz?: { enabled?: boolean; host?: string };
      }>('network.interface_cfg');
      if (res.success && res.data) {
        if (typeof res.data.mode === 'number') {
          setSettings((prev) => ({ ...prev, infcfgMode: res.data!.mode }));
        }
        if (typeof res.data.postRoute === 'number') {
          setSettings((prev) => ({ ...prev, postRoute: res.data!.postRoute }));
        }
        if (res.data.dmz) setDmzConfig(adaptDmz(res.data.dmz));
      }
    } catch {
      // 只读查询失败不打断页面
    } finally {
      setLoading((l) => ({ ...l, infcfg: false }));
    }
  };

  const fetchDMZ = async () => {
    setLoading((l) => ({ ...l, dmz: true }));
    try {
      const res = await at().apiCommand<{ dmz?: { enabled?: boolean; host?: string } }>(
        'network.interface_cfg',
      );
      if (res.success && res.data?.dmz) setDmzConfig(adaptDmz(res.data.dmz));
    } catch {
      // 只读查询失败不打断页面
    } finally {
      setLoading((l) => ({ ...l, dmz: false }));
    }
  };

  const fetchPDPContexts = async () => {
    setLoading((l) => ({ ...l, pdp: true }));
    try {
      const res = await at().apiCommand<{ contexts?: PDPContext[] }>('network.pdp_contexts');
      if (res.success) setPdpList(res.data?.contexts ?? []);
    } catch {
      // 只读查询失败不打断页面
    } finally {
      setLoading((l) => ({ ...l, pdp: false }));
    }
  };

  const loadAll = async () => {
    await fetchDialSettings();
    await fetchUSBMode();
    await fetchInfcfg();
    await fetchPDPContexts();
  };

  useATReady(() => {
    loadAll();
  });

  const pdpTableData = useMemo(
    () => pdpList.map((context) => ({ ...context, key: context.cid })),
    [pdpList],
  );
  const pdpColumns = useMemo(
    () => [
      { title: 'CID', dataIndex: 'cid', width: 80 },
      {
        title: '协议类型',
        dataIndex: 'type',
        width: 120,
        render: (type: string) => getPdpTypeText(type),
      },
      {
        title: 'APN',
        dataIndex: 'apn',
        width: 180,
        render: (apn: string) => apn || '-',
      },
      {
        title: '状态',
        dataIndex: 'active',
        width: 100,
        render: (active: boolean) => (
          <Tag color={active ? 'green' : 'grey'}>{active ? '已激活' : '未激活'}</Tag>
        ),
      },
    ],
    [],
  );

  return (
    <div className="page-stack">
      <Banner
        type="info"
        closeIcon={null}
        title="拨号管理请使用 LuCI"
        description="拨号（连接 / 断开 / 重拨 / 自动拨号 / APN）由 LuCI「网络 → MT5700M → 移动数据」独占，WebUI 仅只读展示共享状态，不会与 LuCI 拨号流程互相影响。"
      />

      {/* ---------- 自动拨号（只读） ---------- */}
      <SectionHeader title="拨号连接" desc="自动拨号与 APN 设置（只读）" />
      <PageCard
        title="自动拨号"
        hint="拨号与 APN 修改请到 LuCI 操作"
        extra={<RefreshBtn onClick={fetchDialSettings} loading={loading.dial} label="刷新" />}
      >
        <Kv
          columns={2}
          dense
          items={[
            {
              label: '自动拨号',
              value: (
                <Tag color={settings.enable === 1 ? 'green' : 'orange'}>
                  {settings.enable === 1 ? '已开启' : '已关闭'}
                </Tag>
              ),
            },
            { label: '拨号方式', value: getDialModeText(settings.dialMode) },
            { label: '协议', value: settings.protocol || '—' },
            { label: '认证方式', value: getAuthTypeText(settings.authType) },
          ]}
        />
        <Field label="APN">
          <Typography.Text>{settings.apn || '—'}</Typography.Text>
        </Field>
        <TwoCol>
          <Field label="用户名">
            <Typography.Text>{settings.username || '—'}</Typography.Text>
          </Field>
          <Field label="密码">
            <Typography.Text>{settings.password ? '••••••' : '—'}</Typography.Text>
          </Field>
        </TwoCol>
        <Banner
          type="info"
          closeIcon={null}
          style={{ marginTop: 12 }}
          description="APN 设置将影响设备的网络连接方式；如需修改 APN、拨号方式或自动拨号状态，请前往 LuCI「移动数据」页面操作。"
        />
      </PageCard>

      {/* ---------- 拨号方式 + USB 端口模式（只读） ---------- */}
      <SectionHeader title="模式配置" desc="拨号方式与 USB 端口模式（只读）" />
      <TwoCol>
        <PageCard title="拨号方式设置" hint="只读展示，修改请到 LuCI">
          <Field label="拨号方式">
            <Typography.Text>{getDialModeText(settings.dialMode)}</Typography.Text>
          </Field>
          <Banner
            type="info"
            closeIcon={null}
            style={{ marginTop: 12 }}
            description="拨号方式的修改由 LuCI「移动数据」页负责，修改后需重新开启自动拨号才能生效。"
          />
        </PageCard>

        <PageCard title="USB端口模式" hint="只读展示，修改请到 LuCI">
          <Field label="USB 端口模式">
            <Typography.Text>{getUSBModeText(settings.usbMode)}</Typography.Text>
          </Field>
          <Banner
            type="info"
            closeIcon={null}
            style={{ marginTop: 12 }}
            description="修改 USB 端口模式后设备会自动重启；相关操作请在 LuCI 进行。"
          />
        </PageCard>
      </TwoCol>

      {/* ---------- 网口模式 + DMZ（只读） ---------- */}
      <SectionHeader title="网口与 DMZ" desc="网口模式、后路由与 DMZ 主机设置（只读）" />
      <TwoCol>
        <PageCard title="网口模式配置" hint="只读展示，修改请到 LuCI">
          <Field label="网口模式">
            <Typography.Text>{getInfcfgModeText(settings.infcfgMode)}</Typography.Text>
          </Field>
          <Field label="后路由">
            <Tag color={settings.postRoute === 1 ? 'green' : 'grey'}>
              {settings.postRoute === 1 ? '已开启' : '已关闭'}
            </Tag>
          </Field>
          <Banner
            type="info"
            closeIcon={null}
            style={{ marginTop: 12 }}
            description="后路由与 DMZ 互斥；相关修改请在 LuCI「移动数据」→ 高级工具中操作。"
          />
        </PageCard>

        <PageCard
          title="DMZ 主机设置"
          hint="只读展示，修改请到 LuCI"
          extra={<RefreshBtn onClick={fetchDMZ} loading={loading.dmz} label="刷新" />}
        >
          <Field label="当前状态">
            <Tag color={dmzConfig.enabled ? 'red' : 'grey'}>
              {dmzConfig.enabled ? `已开启 (${dmzConfig.host})` : '未配置'}
            </Tag>
          </Field>
          <Banner
            type="info"
            closeIcon={null}
            style={{ marginTop: 12 }}
            description="DMZ 主机将完全暴露在公网中；DMZ 配置请在 LuCI「移动数据」→ 高级工具中操作。"
          />
        </PageCard>
      </TwoCol>

      {/* ---------- PDP 上下文（只读） ---------- */}
      <SectionHeader title="PDP 上下文" desc="PDP 上下文配置与激活状态（只读，管理请到 LuCI）" />
      <PageCard
        title="PDP 上下文管理"
        hint="只读展示，增删改 / 激活请在 LuCI 操作"
        extra={<RefreshBtn onClick={fetchPDPContexts} loading={loading.pdp} label="刷新状态" />}
      >
        {isNarrow ? (
          <div className="cell-list">
            {pdpTableData.length === 0 ? (
              <div className="cell-list-empty">暂无 PDP 上下文</div>
            ) : (
              pdpTableData.map((ctx) => (
                <div className="cell-list-item" key={ctx.key}>
                  <div className="cell-list-body">
                    <div className="cell-list-main">
                      <Tag size="small" color={ctx.active ? 'green' : 'grey'}>
                        {ctx.active ? '已激活' : '未激活'}
                      </Tag>
                      <b>CID {ctx.cid}</b>
                      <span>{getPdpTypeText(ctx.type)}</span>
                    </div>
                    <div className="cell-list-sub">APN {ctx.apn || '—'}</div>
                  </div>
                </div>
              ))
            )}
          </div>
        ) : (
          <Table
            size="small"
            pagination={false}
            dataSource={pdpTableData}
            empty="暂无 PDP 上下文"
            columns={pdpColumns}
          />
        )}
        <Banner
          type="info"
          closeIcon={null}
          style={{ marginTop: 12 }}
          description="PDP 上下文的添加、编辑、删除与激活 / 去激活请在 LuCI「移动数据」→ 高级工具中操作。"
        />
      </PageCard>
    </div>
  );
};

export default NetworkDial;
