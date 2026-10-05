import React, { useEffect, useMemo, useState } from 'react';
import {
  Banner,
  Button,
  Input,
  InputNumber,
  Modal,
  Radio,
  RadioGroup,
  Select,
  Space,
  Switch,
  Tag,
  Toast,
  Typography,
} from '@douyinfe/semi-ui';
import { ATService } from '@/services/at';
import { getSharedStateFeed, refreshSharedStateFeed, useSharedStateTopic } from '@/services/stateCache';
import { useATReady } from '@/hooks/useATReady';
import { backendMessage } from '@/services/backendError';
import { sleep } from '@/modem/atx';
import {
  READY_CARD,
  cardStateOf,
  pinErrorMessage,
  pinLabel,
  slotStatusOf,
  type PinOperation,
  type SimCardState,
  type SimPinStatusPayload,
  type SimSlotStatus,
} from '@/modem/sim';
import { Field, Kv, PageCard, Panel, RefreshBtn, SectionHeader, TwoCol } from '@/ui/widgets';

const at = () => ATService.getInstance();

interface DeviceInfo {
  manufacturer: string;
  model: string;
  revision: string;
}

interface TxPowerInfo {
  PPusch: number;
  PPucch: number;
  PSrs: number;
  PPrach: number;
  Freq: number;
}

interface SysCfgInfo {
  acqorder: string;
  band: string;
  roam: number;
  srvdomain: number;
  lteband: string;
}

const PIN_MODAL_TITLES: Record<PinOperation, string> = {
  verify: 'PIN码验证',
  enable: '启用PIN码',
  disable: '关闭PIN码',
  change: '修改PIN码',
  unblock: 'PUK码解锁',
};

const NETWORK_MODES = [
  { value: '02', label: '3G' },
  { value: '03', label: '4G' },
  { value: '08', label: '5G' },
];

const MODE_LABELS: Record<string, string> = {
  '01': '2G',
  '02': '3G',
  '03': '4G',
  '08': '5G',
};

const LTE_BAND_OPTIONS = [
  { value: '1', label: 'B1 (2100 MHz)' },
  { value: '4', label: 'B3 (1800 MHz)' },
  { value: '10', label: 'B5 (850 MHz)' },
  { value: '80', label: 'B8 (900 MHz)' },
  { value: '200000000', label: 'B34 (2010-2025 MHz TDD)' },
  { value: '2000000000', label: 'B38 (2570-2620 MHz TDD)' },
  { value: '4000000000', label: 'B39 (1880-1920 MHz TDD)' },
  { value: '8000000000', label: 'B40 (2300-2400 MHz TDD)' },
  { value: '10000000000', label: 'B41 (2496-2690 MHz TDD)' },
];

const WCDMA_BAND_OPTIONS = [
  { value: '400000', label: 'B1 (2100 MHz) - WCDMA' },
  { value: '2000000000000', label: 'B8 (900 MHz) - WCDMA' },
];

const VONR_OPTIONS = [
  { value: 0, label: '关闭 VoNR' },
  { value: 1, label: 'FR1-VoNR (Sub-6GHz)' },
  { value: 2, label: 'FR2-VoNR (mmWave)' },
  { value: 3, label: 'FR1+FR2-VoNR (全部)' },
];

const VONR_LABELS = ['关闭', 'FR1-VoNR', 'FR2-VoNR', 'FR1+FR2-VoNR'];

const ROAM_OPTIONS = [
  { value: 0, label: '仅使用本地网络' },
  { value: 1, label: '允许使用漫游网络(可能产生额外费用)' },
];

const SRVDOMAIN_OPTIONS = [
  { value: 0, label: '仅支持通话功能' },
  { value: 1, label: '仅支持上网功能' },
  { value: 2, label: '同时支持通话和上网' },
];

const INTERVAL_OPTIONS = [
  { value: 1, label: '1秒' },
  { value: 2, label: '2秒' },
  { value: 5, label: '5秒' },
  { value: 10, label: '10秒' },
  { value: 30, label: '30秒' },
];

const LEVEL_LABELS = ['正常', '一级温保', '二级温保', '三级温保', '四级温保'];

const ipv4Regex = /^(\d{1,3}\.){3}\d{1,3}$/;
const ipv6Regex =
  /^(([0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,7}:|([0-9a-fA-F]{1,4}:){1,6}:[0-9a-fA-F]{1,4}|([0-9a-fA-F]{1,4}:){1,5}(:[0-9a-fA-F]{1,4}){1,2}|([0-9a-fA-F]{1,4}:){1,4}(:[0-9a-fA-F]{1,4}){1,3}|([0-9a-fA-F]{1,4}:){1,3}(:[0-9a-fA-F]{1,4}){1,4}|([0-9a-fA-F]{1,4}:){1,2}(:[0-9a-fA-F]{1,4}){1,5}|[0-9a-fA-F]{1,4}:((:[0-9a-fA-F]{1,4}){1,6})|:((:[0-9a-fA-F]{1,4}){1,7}|:)|fe80:(:[0-9a-fA-F]{0,4}){0,4}%[0-9a-zA-Z]+|::(ffff(:0{1,4})?:)?((25[0-5]|(2[0-4]|1?[0-9])?[0-9])\.){3}(25[0-5]|(2[0-4]|1?[0-9])?[0-9])|([0-9a-fA-F]{1,4}:){1,4}:((25[0-5]|(2[0-4]|1?[0-9])?[0-9])\.){3}(25[0-5]|(2[0-4]|1?[0-9])?[0-9]))$/;

// 将十六进制频段掩码解析为选中的频段选项
const parseBandHex = (hexValue: string, options: { value: string; label: string }[]): string[] => {
  if (!hexValue) return options.map((o) => o.value);
  const valueBigInt = BigInt('0x' + hexValue);
  return options
    .filter((o) => (valueBigInt & BigInt('0x' + o.value)) !== BigInt(0))
    .map((o) => o.value);
};

// 将选中的频段合并为十六进制掩码
const combineBandHex = (selected: string[], allHex: string): string => {
  if (selected.length === 0) return allHex;
  let result = BigInt(0);
  selected.forEach((s) => {
    result = result | BigInt('0x' + s);
  });
  return result.toString(16).toUpperCase();
};

const SystemInfo: React.FC = () => {
  const modemEntry = useSharedStateTopic('modem');
  const nrTxpowerEntry = useSharedStateTopic('nr_txpower');
  const sharedSnapshot = useMemo(
    () => ({ modem: modemEntry, nr_txpower: nrTxpowerEntry }),
    [modemEntry, nrTxpowerEntry],
  );
  // ---- AT服务器配置 ----
  const [host, setHost] = useState(() => ATService.getInstance().getHost());
  const [port, setPort] = useState(() => ATService.getInstance().getPort());
  const [configLocked, setConfigLocked] = useState(() => ATService.getInstance().isConfigLocked());
  const [atConfigLoading, setAtConfigLoading] = useState(false);
  const [serviceMode, setServiceMode] = useState('获取中...');
  const [serviceModeLoading, setServiceModeLoading] = useState(false);

  // ---- 系统信息 ----
  const [deviceInfo, setDeviceInfo] = useState<DeviceInfo>({ manufacturer: '', model: '', revision: '' });
  const [imei, setImei] = useState('');
  const [systemInfoLoading, setSystemInfoLoading] = useState(false);

  // ---- SIM卡配置 ----
  const [simSlot, setSimSlot] = useState(0);
  const [simSwitching, setSimSwitching] = useState(false);
  const [simHotPlug, setSimHotPlug] = useState(true);
  const [simHotPlugLoading, setSimHotPlugLoading] = useState(false);
  const [simCard, setSimCard] = useState<SimCardState>(READY_CARD);
  const [simSlotStatus, setSimSlotStatus] = useState<SimSlotStatus | null>(null);
  const [pinLoading, setPinLoading] = useState(false);
  const [showPinModal, setShowPinModal] = useState(false);
  const [pinOperation, setPinOperation] = useState<PinOperation>('verify');
  const [pinInput, setPinInput] = useState('');
  const [newPinInput, setNewPinInput] = useState('');
  const [airplaneMode, setAirplaneMode] = useState(false);
  const [airplaneLoading, setAirplaneLoading] = useState(false);

  // ---- 设备控制 ----
  const [nicRate, setNicRate] = useState<number | null>(null);
  const [nicLoading, setNicLoading] = useState(false);
  const [powerControl, setPowerControl] = useState<boolean | null>(null);
  const [powerLoading, setPowerLoading] = useState(false);
  const [factoryLoading, setFactoryLoading] = useState(false);
  const [rebootLoading, setRebootLoading] = useState(false);

  // ---- 5G网络能力控制 ----
  const [nrLoading, setNrLoading] = useState(false);
  const [nrCa, setNrCa] = useState(true);
  const [nrVonr, setNrVonr] = useState(3);
  const [nrDss, setNrDss] = useState({ rateMatchingLTE: 0, additionalDMRS: 0 });

  // ---- 网络系统配置 ----
  const [sysCfg, setSysCfg] = useState<SysCfgInfo>({ acqorder: '', band: '', roam: 1, srvdomain: 2, lteband: '' });
  const [originalSysCfg, setOriginalSysCfg] = useState<SysCfgInfo>({ acqorder: '', band: '', roam: 1, srvdomain: 2, lteband: '' });
  const [sysCfgLoading, setSysCfgLoading] = useState(false);

  // ---- 发射功率信息 ----
  const [txPower, setTxPower] = useState<TxPowerInfo[]>([]);
  const [txPowerLoading, setTxPowerLoading] = useState(false);

  // ---- 温度保护控制 ----
  const [therm, setTherm] = useState({
    enabled: true,
    caMimoSwitch: false,
    interval: 2,
    logSwitch: { consoleLog: true, fileLog: true },
    currentLevel: 0,
    thresholds: [] as number[],
  });
  const [thermLoading, setThermLoading] = useState(false);

  // ---- IMEI修改 ----
  const [showImeiModify, setShowImeiModify] = useState(false);
  const [clickCount, setClickCount] = useState(0);
  const [newImei, setNewImei] = useState('');
  const [imeiLoading, setImeiLoading] = useState(false);

  // ---------- AT服务器配置 ----------
  const fetchConnectionMode = async () => {
    setServiceModeLoading(true);
    try {
      // 链路类型由后端在启动时记录一次（core::modem），不再用 AT+CONNECT? 探测。
      const res = await at().apiCommand<{ mode?: string }>('system.service_mode');
      const mode = res.success ? res.data?.mode : undefined;
      if (mode === 'network') setServiceMode('网络AT');
      else if (mode === 'serial') setServiceMode('串口AT');
      else {
        setServiceMode((previous) =>
          previous === '获取中...' || previous === '获取失败' ? '获取失败' : previous,
        );
        Toast.warning('读取 AT 连接模式失败，已保留上次状态');
      }
    } finally {
      setServiceModeLoading(false);
    }
  };

  const handleSaveATConfig = async () => {
    if (!ipv4Regex.test(host) && !ipv6Regex.test(host)) {
      Toast.error('请输入有效的 IPv4 或 IPv6 地址');
      return;
    }
    setAtConfigLoading(true);
    try {
      await at().setConnection(host, port);
      Toast.success('AT服务器配置已更新并已重新连接');
      await fetchDeviceInfo();
    } catch (error) {
      const message = error instanceof Error ? error.message : '重新连接失败';
      if (message === 'REQUIRE_AUTH_KEY') {
        Toast.warning('AT服务器配置已更新，请完成密钥认证');
      } else {
        Toast.error(`AT服务器配置已更新，但${message}`);
      }
    } finally {
      setAtConfigLoading(false);
    }
  };

  // ---------- 系统信息 ----------
  const fetchDeviceInfo = async () => {
    setSystemInfoLoading(true);
    try {
      // 身份块（ATI + AT+CGSN 的解码）由 modules/modem 负责，页面只渲染。
      const res = await at().apiCommand<{
        manufacturer?: string;
        model?: string;
        revision?: string;
        imei?: string;
      }>('modem.get');
      if (!res.success || !res.data) return;
      const info = res.data;
      setDeviceInfo((prev) => ({
        manufacturer: info.manufacturer || prev.manufacturer,
        model: info.model || prev.model,
        revision: info.revision || prev.revision,
      }));
      if (info.imei) setImei(info.imei);
    } finally {
      setSystemInfoLoading(false);
    }
  };

  // 连续点击5次（3秒内）显示IMEI修改卡片
  const handleImeiClick = () => {
    setClickCount((prev) => {
      const newCount = prev + 1;
      if (newCount >= 5) {
        setShowImeiModify(true);
        return 0;
      }
      setTimeout(() => setClickCount(0), 3000);
      return newCount;
    });
  };

  // ---------- SIM卡配置 ----------
  const fetchSimConfig = async () => {
    // 卡槽与热插拔开关都在后端 sim 主题里解码（modules/sim）。
    const res = await at().apiCommand<{ slot?: number; hotplug?: boolean }>('sim.slot');
    if (!res.success || !res.data) return;
    if (typeof res.data.slot === 'number') setSimSlot(res.data.slot);
    if (typeof res.data.hotplug === 'boolean') setSimHotPlug(res.data.hotplug);
  };

  const handleSwitchSim = (target: number) => {
    Modal.confirm({
      title: `确认切换到${target === 0 ? '外置' : '内置'}SIM卡`,
      content: '切换SIM卡需要执行去激活、切换卡槽、激活并重启模组，此过程可能需要1-2分钟，期间设备将无法使用。',
      okText: '确认切换',
      cancelText: '取消',
      onOk: async () => {
        setSimSwitching(true);
        try {
          // 去激活 -> 切槽 -> 激活 -> 重启射频这套顺序在 modules/sim 里，
          // 页面只声明"切到哪个槽"。
          const res = await at().apiCommand<{ slot?: number }>('sim.slot_set', { slot: target });
          if (!res.success) throw new Error(String(res.error || '切换SIM卡槽失败'));
          setSimSlot(typeof res.data?.slot === 'number' ? res.data.slot : target);
          Toast.success(`正在切换到${target === 0 ? '外置' : '内置'}SIM卡，请等待设备重启...`);
          setTimeout(() => {
            setSimSwitching(false);
            fetchSimConfig();
          }, 30000);
        } catch {
          Toast.error('切换SIM卡失败');
          setSimSwitching(false);
        }
      },
    });
  };

  const handleSimHotPlug = async (checked: boolean) => {
    setSimHotPlugLoading(true);
    try {
      const res = await at().apiCommand<{ hotplug?: boolean }>('sim.hotplug_set', {
        hotplug: checked,
      });
      if (res.success) {
        setSimHotPlug(checked);
        Toast.success(`${checked ? '开启' : '关闭'}SIM卡热插拔成功`);
      } else {
        Toast.error(`${checked ? '开启' : '关闭'}SIM卡热插拔失败`);
      }
    } finally {
      setSimHotPlugLoading(false);
    }
  };

  const fetchPinStatus = async () => {
    // +CPIN 的状态（含未插卡时的 CME 错误）、^SIMSQ 的卡状态和 +CLCK 的
    // PIN 锁开关都由后端 sim.pin_status 一并解码，页面只负责渲染。
    const res = await at().apiCommand<SimPinStatusPayload>('sim.pin_status');
    if (!res.success || !res.data || !res.data.code) return;
    setSimCard(cardStateOf(res.data));
    setSimSlotStatus(res.data.card ? slotStatusOf(res.data.card) : null);
  };

  const openPinModal = (op: PinOperation) => {
    setPinOperation(op);
    setPinInput('');
    setNewPinInput('');
    setShowPinModal(true);
  };

  const closePinModal = () => {
    setShowPinModal(false);
    setPinInput('');
    setNewPinInput('');
  };

  const handlePinOperation = async () => {
    // 启用 PIN 时第二个输入框是"再输一次"，两次不一致直接挡下。
    if (pinOperation === 'enable' && pinInput !== newPinInput) {
      Toast.error('两次输入的PIN码不一致');
      return;
    }

    const successMessage: Record<PinOperation, string> = {
      verify: 'PIN码验证成功',
      enable: 'PIN码启用成功',
      disable: 'PIN码已关闭',
      change: 'PIN码修改成功',
      // 手册 6.3.2：用 PUK 解锁后需要重启模组才会生效。
      unblock: 'PUK码解锁成功，需重启模组后生效',
    };

    setPinLoading(true);
    try {
      // 密码规则与 AT 拼装都在后端 modules/sim；这里把后端拒绝的话原样展示。
      const res = await at().apiCommand('sim.pin_apply', {
        operation: pinOperation,
        pin: pinInput,
        newPin: pinOperation === 'enable' ? undefined : newPinInput,
      });
      if (res.success) {
        Toast.success(successMessage[pinOperation]);
        closePinModal();
        setTimeout(() => void fetchPinStatus(), 1500);
      } else {
        Toast.error(pinErrorMessage(String(res.error || ''), 'PIN码操作失败'));
      }
    } finally {
      setPinLoading(false);
    }
  };

  const fetchAirplaneMode = async () => {
    // +CFUN? 的解码在 modules/network（network.radio）。
    const res = await at().apiCommand<{ airplane?: boolean }>('network.radio');
    if (res.success && typeof res.data?.airplane === 'boolean') {
      setAirplaneMode(res.data.airplane);
    }
  };

  const handleAirplane = async (checked: boolean) => {
    setAirplaneLoading(true);
    try {
      const res = await at().apiCommand('network.radio_set', { airplane: checked });
      if (res.success) {
        setAirplaneMode(checked);
        Toast.success(`${checked ? '开启' : '关闭'}飞行模式成功`);
      } else {
        Toast.error(`${checked ? '开启' : '关闭'}飞行模式失败`);
      }
    } finally {
      setAirplaneLoading(false);
    }
  };

  // ---------- 设备控制 ----------
  const fetchDeviceControl = async () => {
    // 网卡速率与 PCIe 电源开关：两个读都在 modules/system 解码（缺失的字段
    // 表示这次没读到，控件保持原值）。
    const res = await at().apiCommand<{ nic_rate?: number; power_control?: boolean }>(
      'system.device_control',
    );
    if (!res.success || !res.data) return;
    if (typeof res.data.nic_rate === 'number') setNicRate(res.data.nic_rate);
    if (typeof res.data.power_control === 'boolean') setPowerControl(res.data.power_control);
  };

  const handleSetNicRate = (value: number) => {
    const label = value === 1 ? 'RTL8111 (1G)' : 'RTL8125 (2.5G)';
    Modal.confirm({
      title: '确认修改网卡速率',
      content: `您将网卡速率修改为 ${label}，需要重启设备才能生效。`,
      okText: '确认',
      cancelText: '取消',
      onOk: async () => {
        setNicLoading(true);
        try {
          const res = await at().apiCommand('system.nic_rate_set', { rate: value });
          if (res.success) {
            setNicRate(value);
            Toast.success('网卡速率设置成功');
            Modal.confirm({
              title: '是否立即重启设备？',
              content: '需要重启设备才能使新的网卡速率生效',
              okText: '立即重启',
              cancelText: '稍后重启',
              okButtonProps: { theme: 'solid', type: 'danger' },
              onOk: async () => {
                setRebootLoading(true);
                const resetRes = await at().apiCommand('modem.reset');
                if (resetRes.success) {
                  Toast.success('重启指令已发送');
                  setTimeout(() => {
                    setRebootLoading(false);
                    fetchDeviceInfo();
                  }, 30000);
                } else {
                  Toast.error('重启指令发送失败');
                  setRebootLoading(false);
                }
              },
            });
          } else {
            Toast.error('网卡速率设置失败');
          }
        } finally {
          setNicLoading(false);
        }
      },
    });
  };

  const handleSetPower = async (checked: boolean) => {
    setPowerLoading(true);
    try {
      const res = await at().apiCommand('system.power_control_set', { enabled: checked });
      if (res.success) {
        setPowerControl(checked);
        Toast.success(`${checked ? '开启' : '关闭'}电源管理成功`);
      } else {
        Toast.error(`${checked ? '开启' : '关闭'}电源管理失败`);
      }
    } finally {
      setPowerLoading(false);
    }
  };

  const handleFactoryReset = () => {
    Modal.confirm({
      title: '确认恢复出厂设置',
      content: '确定要恢复出厂设置吗？此操作将清除所有设置并重启设备。',
      okText: '确认',
      cancelText: '取消',
      okButtonProps: { theme: 'solid', type: 'danger' },
      onOk: async () => {
        setFactoryLoading(true);
        try {
          const res = await at().apiCommand('system.factory_reset');
          if (res.success) {
            Toast.success('恢复出厂设置指令已发送');
            setTimeout(() => {
              setFactoryLoading(false);
              fetchDeviceInfo();
            }, 30000);
          } else {
            Toast.error('恢复出厂设置指令发送失败');
          }
        } finally {
          setFactoryLoading(false);
        }
      },
    });
  };

  const handleReboot = () => {
    Modal.confirm({
      title: '确认重启',
      content: '确定要重启模组吗？重启过程中设备将暂时无法使用。',
      okText: '确认重启',
      cancelText: '取消',
      okButtonProps: { theme: 'solid', type: 'danger' },
      onOk: async () => {
        setRebootLoading(true);
        const res = await at().apiCommand('modem.reset');
        if (res.success) {
          Toast.success('重启指令已发送');
          setTimeout(() => {
            setRebootLoading(false);
            fetchDeviceInfo();
          }, 30000);
        } else {
          Toast.error('重启指令发送失败');
          setRebootLoading(false);
        }
      },
    });
  };

  // ---------- 5G网络能力控制 ----------
  const fetchNRCapability = async () => {
    setNrLoading(true);
    try {
      // 三个 ^NRRCCAPQRY 查询（按 kind 匹配应答）在 modules/modem 解码；
      // 没读到的项不会出现在返回里，卡片保持原值。
      const res = await at().apiCommand<{
        ca?: boolean;
        vonr?: number;
        dss?: { rateMatchingLTE?: number; additionalDMRS?: number };
      }>('modem.nr_capability');
      if (!res.success || !res.data) return;
      if (typeof res.data.ca === 'boolean') setNrCa(res.data.ca);
      if (typeof res.data.vonr === 'number') setNrVonr(res.data.vonr);
      const dss = res.data.dss;
      if (typeof dss?.rateMatchingLTE === 'number' && typeof dss?.additionalDMRS === 'number') {
        setNrDss({ rateMatchingLTE: dss.rateMatchingLTE, additionalDMRS: dss.additionalDMRS });
      }
    } finally {
      setNrLoading(false);
    }
  };

  const handleSetCA = async (checked: boolean) => {
    setNrLoading(true);
    try {
      const res = await at().apiCommand('modem.nr_capability_set', { ca: checked });
      if (res.success) {
        setNrCa(checked);
        Toast.success(`${checked ? '开启' : '关闭'}载波聚合成功`);
        Modal.info({
          title: '配置已更改',
          content: '载波聚合设置已更改，需要软关软开（飞行模式切换）后生效',
          okText: '知道了',
        });
      } else {
        Toast.error(`${checked ? '开启' : '关闭'}载波聚合失败`);
      }
    } finally {
      setNrLoading(false);
    }
  };

  const handleSetVoNR = async (value: number) => {
    setNrLoading(true);
    try {
      const res = await at().apiCommand('modem.nr_capability_set', { vonr: value });
      if (res.success) {
        setNrVonr(value);
        Toast.success(`VoNR 配置成功：${VONR_LABELS[value] || '未知'}`);
        Modal.info({
          title: '配置已更改',
          content: 'VoNR 设置已更改，需要软关软开（飞行模式切换）后生效',
          okText: '知道了',
        });
      } else {
        Toast.error('VoNR 配置失败');
      }
    } finally {
      setNrLoading(false);
    }
  };

  const handleSetDSS = async (rateMatchingLTE: number, additionalDMRS: number) => {
    setNrLoading(true);
    try {
      const res = await at().apiCommand('modem.nr_capability_set', {
        dss: { rateMatchingLTE, additionalDMRS },
      });
      if (res.success) {
        setNrDss({ rateMatchingLTE, additionalDMRS });
        Toast.success('DSS 配置成功');
        Modal.info({
          title: '配置已更改',
          content: 'DSS 设置已更改，需要软关软开（飞行模式切换）后生效',
          okText: '知道了',
        });
      } else {
        Toast.error('DSS 配置失败');
      }
    } finally {
      setNrLoading(false);
    }
  };

  // ---------- 网络系统配置 ----------
  const fetchSysCfg = async () => {
    setSysCfgLoading(true);
    try {
      // ^SYSCFGEX 的字段顺序（引号可有可无）在 modules/network 解码。
      const res = await at().apiCommand<SysCfgInfo>('network.syscfg');
      if (
        res.success &&
        res.data?.acqorder &&
        typeof res.data.roam === 'number' &&
        typeof res.data.srvdomain === 'number'
      ) {
        const cfg: SysCfgInfo = {
          acqorder: res.data.acqorder,
          band: res.data.band ?? '',
          roam: res.data.roam,
          srvdomain: res.data.srvdomain,
          lteband: res.data.lteband ?? '',
        };
        setSysCfg(cfg);
        setOriginalSysCfg(cfg);
      }
    } finally {
      setSysCfgLoading(false);
    }
  };

  const handleApplySysCfg = () => {
    Modal.confirm({
      title: '确认修改网络配置',
      content: '修改网络配置可能会导致设备短暂断网，是否继续？',
      okText: '确认',
      cancelText: '取消',
      onOk: async () => {
        setSysCfgLoading(true);
        try {
          // 七个参数的拼接与取值范围校验都在 modules/network。
          const res = await at().apiCommand('network.syscfg_set', {
            acqorder: sysCfg.acqorder,
            band: sysCfg.band,
            roam: sysCfg.roam,
            srvdomain: sysCfg.srvdomain,
            lteband: sysCfg.lteband,
          });
          if (res.success) {
            Toast.success('网络系统配置已更新');
            await fetchSysCfg();
          } else {
            Toast.error('网络系统配置更新失败');
          }
        } finally {
          setSysCfgLoading(false);
        }
      },
    });
  };

  const isSysCfgUnchanged =
    sysCfg.acqorder === originalSysCfg.acqorder &&
    sysCfg.band === originalSysCfg.band &&
    sysCfg.roam === originalSysCfg.roam &&
    sysCfg.srvdomain === originalSysCfg.srvdomain &&
    sysCfg.lteband === originalSysCfg.lteband;

  const acqOrderDisplay = sysCfg.acqorder
    ? sysCfg.acqorder.match(/.{2}/g)?.map((m) => MODE_LABELS[m] || m).join(' → ') || '未知'
    : '未知';

  // ---------- 发射功率信息 ----------
  const fetchTxPower = async () => {
    setTxPowerLoading(true);
    try {
      // ^NTXPOWER 的分组（每载波 5 个字段）与 999=不适用 的映射都在
      // modules/modem（modem.nr_txpower），这里只把字段名对到卡片上。
      const res = await at().apiCommand<{ carriers?: Record<string, unknown>[] }>(
        'modem.nr_txpower',
      );
      if (res.success && Array.isArray(res.data?.carriers)) {
        setTxPower(
          res.data.carriers.map((carrier) => ({
            PPusch: typeof carrier.pusch === 'number' ? carrier.pusch : 999,
            PPucch: typeof carrier.pucch === 'number' ? carrier.pucch : 999,
            PSrs: typeof carrier.srs === 'number' ? carrier.srs : 999,
            PPrach: typeof carrier.prach === 'number' ? carrier.prach : 999,
            Freq: typeof carrier.freq === 'number' ? carrier.freq : 0,
          })),
        );
      }
    } finally {
      setTxPowerLoading(false);
    }
  };

  // ---------- 温度保护控制 ----------
  const fetchThermConfig = async () => {
    setThermLoading(true);
    try {
      // 四条温保查询的字段顺序（含 status 第 6 个字段是当前等级）都在
      // modules/system；返回里没有的字段表示这次没读到，保持原值。
      const res = await at().apiCommand<{
        enabled?: boolean;
        caMimoSwitch?: boolean;
        interval?: number;
        logSwitch?: { consoleLog?: boolean; fileLog?: boolean };
        thresholds?: number[];
        currentLevel?: number;
      }>('system.thermal');
      if (!res.success || !res.data) return;
      const t = res.data;
      setTherm((p) => ({
        enabled: typeof t.enabled === 'boolean' ? t.enabled : p.enabled,
        caMimoSwitch: typeof t.caMimoSwitch === 'boolean' ? t.caMimoSwitch : p.caMimoSwitch,
        interval: typeof t.interval === 'number' ? t.interval : p.interval,
        logSwitch: {
          consoleLog:
            typeof t.logSwitch?.consoleLog === 'boolean'
              ? t.logSwitch.consoleLog
              : p.logSwitch.consoleLog,
          fileLog:
            typeof t.logSwitch?.fileLog === 'boolean' ? t.logSwitch.fileLog : p.logSwitch.fileLog,
        },
        thresholds: Array.isArray(t.thresholds) ? t.thresholds : p.thresholds,
        currentLevel: typeof t.currentLevel === 'number' ? t.currentLevel : p.currentLevel,
      }));
    } finally {
      setThermLoading(false);
    }
  };

  const handleSetThermEnabled = async (checked: boolean) => {
    setThermLoading(true);
    try {
      const res = await at().apiCommand('system.thermal_set', {
        enabled: checked,
        caMimoSwitch: therm.caMimoSwitch,
        interval: therm.interval,
      });
      if (res.success) {
        setTherm((p) => ({ ...p, enabled: checked }));
        Toast.success(`${checked ? '开启' : '关闭'}温度保护功能成功`);
      } else {
        Toast.error(`${checked ? '开启' : '关闭'}温度保护功能失败`);
      }
    } finally {
      setThermLoading(false);
    }
  };

  const handleSetThermInterval = async (interval: number) => {
    setThermLoading(true);
    try {
      const res = await at().apiCommand('system.thermal_set', {
        enabled: therm.enabled,
        caMimoSwitch: therm.caMimoSwitch,
        interval,
      });
      if (res.success) {
        setTherm((p) => ({ ...p, interval }));
        Toast.success('温度检测间隔设置成功');
      } else {
        Toast.error('温度检测间隔设置失败');
      }
    } finally {
      setThermLoading(false);
    }
  };

  // ---------- IMEI修改 ----------
  const handleModifyImei = () => {
    Modal.confirm({
      title: '高风险操作警告',
      content: `修改IMEI是高风险操作，可能违反相关法律法规并导致设备无法正常使用。您即将把IMEI修改为 ${newImei}，请确认已了解相关风险。`,
      okText: '确认修改',
      cancelText: '取消',
      okButtonProps: { theme: 'solid', type: 'danger' },
      onOk: async () => {
        setImeiLoading(true);
        try {
          // 15 位数字的规则在后端（modules/modem::set_imei），这里显示它拒绝的原因。
          const res = await at().apiCommand('modem.imei_set', { imei: newImei.trim() });
          if (res.success) {
            Toast.success('IMEI修改成功');
            setNewImei('');
            fetchDeviceInfo();
          } else {
            Toast.error(backendMessage(res.error, 'IMEI修改失败'));
          }
        } finally {
          setImeiLoading(false);
        }
      },
    });
  };

  // ---------- 初始化加载 ----------
  const loadAll = async () => {
    try {
      // Prefer the shared modem.info cache (same value LuCI reads). Keep the
      // existing ATI/IMEI read as a cold-cache fallback and manual refresh path.
      await refreshSharedStateFeed();
      const modem = getSharedStateFeed().snapshot.modem?.value;
      const hasIdentity = !!modem && ['manufacturer', 'model', 'revision', 'imei'].every(
        (key) => typeof modem[key] === 'string' && !!modem[key],
      );
      if (!hasIdentity) await fetchDeviceInfo();
      await sleep(100);
      await fetchConnectionMode();
      await sleep(100);
      await fetchSimConfig();
      await fetchPinStatus();
      await fetchAirplaneMode();
      await sleep(100);
      await fetchDeviceControl();
      await sleep(200);
      await fetchNRCapability();
      await sleep(100);
      await fetchSysCfg();
      await sleep(100);
      await fetchThermConfig();
    } catch {
      Toast.error('部分数据加载失败，请刷新重试');
    }
  };

  useATReady(loadAll);

  // The app-wide StateCache feed is the same snapshot/EventBus consumed by LuCI.
  // Use it for shared identity/NR diagnostics, retaining existing manual refresh
  // handlers for values that are intentionally outside the common cache topics.
  useEffect(() => {
    const modem = sharedSnapshot.modem?.value;
    if (modem && typeof modem === 'object') {
      const m = modem as Record<string, unknown>;
      setDeviceInfo((prev) => ({
        manufacturer: typeof m.manufacturer === 'string' && m.manufacturer ? m.manufacturer : prev.manufacturer,
        model: typeof m.model === 'string' && m.model ? m.model : prev.model,
        revision: typeof m.revision === 'string' && m.revision ? m.revision : prev.revision,
      }));
      if (typeof m.imei === 'string' && m.imei) setImei(m.imei);
    }

    const power = sharedSnapshot.nr_txpower?.value;
    if (power && Array.isArray(power.carriers) && power.carriers.length > 0) {
      setTxPower(
        (power.carriers as Record<string, unknown>[]).map((carrier) => ({
          PPusch: typeof carrier.pusch === 'number' ? carrier.pusch : 999,
          PPucch: typeof carrier.pucch === 'number' ? carrier.pucch : 999,
          PSrs: typeof carrier.srs === 'number' ? carrier.srs : 999,
          PPrach: typeof carrier.prach === 'number' ? carrier.prach : 999,
          Freq: typeof carrier.freq === 'number' ? carrier.freq : 0,
        })),
      );
    }
  }, [sharedSnapshot]);

  const pinPlaceholder =
    pinOperation === 'verify'
      ? '请输入PIN码'
      : pinOperation === 'enable'
      ? '请设置PIN码'
      : pinOperation === 'disable'
      ? '请输入当前PIN码'
      : pinOperation === 'change'
      ? '请输入当前PIN码'
      : '请输入PUK码';

  const newPinPlaceholder =
    pinOperation === 'enable' ? '请再次输入PIN码' : pinOperation === 'change' ? '请输入新的PIN码' : '请设置新的PIN码';

  const pinBannerText =
    pinOperation === 'unblock'
      ? 'PUK码输入错误次数过多将永久锁卡，若没有PUK码请联系运营商。'
      : 'PIN码必须为4-8位数字。';

  return (
    <div className="page-stack">
      <SectionHeader
        title="连接与设备"
        desc="AT 服务器连接配置与基础设备信息"
        id="connection-device"
      />
      <TwoCol>
        <PageCard title="AT服务器配置">
          <div className="form-stack">
            <Field label="当前服务模式">
              <Space>
                <Tag color={serviceMode === '网络AT' ? 'red' : serviceMode === '串口AT' ? 'blue' : 'grey'}>
                  {serviceMode}
                </Tag>
                <RefreshBtn onClick={fetchConnectionMode} loading={serviceModeLoading} />
              </Space>
              <Typography.Text type="tertiary" size="small">
                通过网络连接与设备通信，支持远程控制
              </Typography.Text>
            </Field>
            <Field label="服务器地址">
              <Input
                value={host}
                onChange={(v) => setHost(v)}
                placeholder="IPv4 或 IPv6"
                disabled={configLocked}
              />
            </Field>
            <Field label="端口">
              <InputNumber
                value={port}
                onChange={(v) => setPort(Number(v) || 0)}
                min={1}
                max={65535}
                style={{ width: '100%' }}
                disabled={configLocked}
              />
            </Field>
            {configLocked ? (
              <Banner type="info" closeIcon={null} description="当前 AT 地址由配置文件锁定，无法在此修改。" />
            ) : (
              <div className="card-actions">
                <Button theme="solid" type="primary" loading={atConfigLoading} onClick={handleSaveATConfig}>
                  保存
                </Button>
              </div>
            )}
          </div>
        </PageCard>

        <PageCard
          title="系统信息"
          bodyClassName="system-summary-body"
          extra={<RefreshBtn onClick={fetchDeviceInfo} loading={systemInfoLoading} />}
        >
          <Kv
            items={[
              { label: '制造商', value: deviceInfo.manufacturer || '未知' },
              { label: '设备型号', value: deviceInfo.model || '未知' },
              { label: '固件版本', value: deviceInfo.revision || '未知' },
              {
                label: 'IMEI',
                value: (
                  <span
                    className="mono"
                    onClick={handleImeiClick}
                    style={{ cursor: 'pointer', display: 'block', padding: '4px 0' }}
                  >
                    {imei || '未知'}
                  </span>
                ),
              },
            ]}
          />
        </PageCard>
      </TwoCol>

      {showImeiModify && (
        <PageCard
          title="IMEI修改"
          extra={
            <Button type="tertiary" theme="borderless" size="small" onClick={() => setShowImeiModify(false)}>
              隐藏
            </Button>
          }
        >
          <div className="form-stack">
            <Banner
              type="warning"
              closeIcon={null}
              description="修改IMEI是高风险操作，可能违反相关法律法规并导致设备无法正常使用。请确保您完全了解此操作的风险。"
            />
            <div className="action-row">
              <Input
                value={newImei}
                onChange={(v) => setNewImei(v.replace(/\D/g, '').slice(0, 15))}
                placeholder="请输入新的IMEI（15位数字）"
                maxLength={15}
                style={{ flex: 1 }}
              />
              <Button type="danger" theme="solid" loading={imeiLoading} onClick={handleModifyImei}>
                修改IMEI
              </Button>
            </div>
          </div>
        </PageCard>
      )}

      <SectionHeader
        title="SIM 卡与设备控制"
        desc="SIM 卡槽切换、PIN 管理、飞行模式与设备电源控制"
        id="sim-device"
      />
      <TwoCol>
        <PageCard title="SIM卡配置">
          <div className="form-stack">
            <Field label="SIM卡切换">
              <RadioGroup
                type="button"
                value={simSlot}
                onChange={(e) => handleSwitchSim(Number(e.target.value))}
                disabled={simSwitching}
              >
                <Radio value={0}>外置SIM卡</Radio>
                <Radio value={1}>内置SIM卡</Radio>
              </RadioGroup>
              {simSwitching ? (
                <Banner
                  type="warning"
                  closeIcon={null}
                  description="正在切换SIM卡，请耐心等待设备重启完成..."
                  style={{ marginTop: 8 }}
                />
              ) : null}
              <Typography.Text type="tertiary" size="small">
                切换SIM卡需要重启设备，请确保没有重要的网络操作正在进行
              </Typography.Text>
            </Field>
            <Field label="SIM卡热插拔">
              <Space>
                <Switch checked={simHotPlug} onChange={handleSimHotPlug} loading={simHotPlugLoading} />
                <Typography.Text>{simHotPlug ? '已开启' : '已关闭'}</Typography.Text>
              </Space>
              <Typography.Text type="tertiary" size="small">
                开启后可以在设备运行时插拔外置SIM卡
              </Typography.Text>
            </Field>
            <Field label="PIN码管理">
              <div>
                <Typography.Text type="tertiary">
                  状态：{pinLabel(simCard.code)}
                  {simSlotStatus ? ` · ${simSlotStatus.label}` : ''}
                </Typography.Text>
                {simSlotStatus?.dead ? (
                  <Banner
                    type="danger"
                    closeIcon={null}
                    description="卡已失效：PUK 输错次数用尽或卡片物理损坏，需联系运营商补卡。"
                    style={{ marginTop: 8 }}
                  />
                ) : null}
                <Space style={{ marginTop: 8 }}>
                  {simCard.lock === 'ready' && (
                    <>
                      <Button
                        size="small"
                        type={simCard.pinEnabled ? 'secondary' : 'primary'}
                        onClick={() => openPinModal(simCard.pinEnabled ? 'disable' : 'enable')}
                      >
                        {simCard.pinEnabled ? '关闭PIN码' : '启用PIN码'}
                      </Button>
                      {simCard.pinEnabled && (
                        <Button size="small" onClick={() => openPinModal('change')}>
                          修改PIN码
                        </Button>
                      )}
                    </>
                  )}
                  {['pin', 'pin2', 'network'].includes(simCard.lock) && (
                    <Button size="small" type="primary" onClick={() => openPinModal('verify')}>
                      验证PIN码
                    </Button>
                  )}
                  {simCard.needsNewPin && (
                    <Button size="small" type="danger" onClick={() => openPinModal('unblock')}>
                      解锁PUK
                    </Button>
                  )}
                </Space>
              </div>
            </Field>
            <Field label="飞行模式">
              <Space>
                <Switch checked={airplaneMode} onChange={handleAirplane} loading={airplaneLoading} />
                <Typography.Text>{airplaneMode ? '已开启' : '已关闭'}</Typography.Text>
              </Space>
              <Typography.Text type="tertiary" size="small">
                开启飞行模式将断开所有网络
              </Typography.Text>
            </Field>
          </div>
        </PageCard>

        <PageCard title="设备控制" bodyClassName="device-control-body">
          <div className="device-control-grid">
            <Field className="device-control-item device-control-item--wide" label="网卡速率">
              <Typography.Text type="tertiary">
                当前配置：
                {nicRate === 1 ? 'RTL8111 (1G)' : nicRate === 2 ? 'RTL8125 (2.5G)' : '获取中...'}
              </Typography.Text>
              <Select
                style={{ width: '100%' }}
                value={nicRate ?? undefined}
                onChange={(v) => handleSetNicRate(Number(v))}
                loading={nicLoading || nicRate === null}
                optionList={[
                  { value: 1, label: 'RTL8111 (1G)' },
                  { value: 2, label: 'RTL8125 (2.5G)' },
                ]}
                placeholder="请选择网卡速率"
              />
            </Field>
            {powerControl !== null && (
              <Field className="device-control-item" label="电源管理">
                <Space>
                  <Switch checked={powerControl} onChange={handleSetPower} loading={powerLoading} />
                  <Typography.Text>{powerControl ? '已开启' : '已关闭'}</Typography.Text>
                </Space>
              </Field>
            )}
            <Field
              className={`device-control-item ${powerControl === null ? 'device-control-item--wide' : ''}`}
              label="设备维护"
              hint="执行前请确认当前没有重要网络业务。"
            >
              <div className="device-maintenance-actions">
                <Button type="danger" theme="light" loading={factoryLoading} onClick={handleFactoryReset}>
                  恢复出厂设置
                </Button>
                <Button type="danger" theme="light" loading={rebootLoading} onClick={handleReboot}>
                  重启模组
                </Button>
              </div>
            </Field>
          </div>
        </PageCard>
      </TwoCol>

      <SectionHeader
        title="5G 网络能力"
        desc="载波聚合、VoNR 与 DSS 动态频谱共享"
        id="nr-capability"
      />
      <PageCard
        title="5G网络能力控制"
        extra={<RefreshBtn onClick={fetchNRCapability} loading={nrLoading} />}
      >
        <div className="form-stack">
          <Banner
            type="info"
            closeIcon={null}
            description="修改以下配置需要软关软开（通过飞行模式切换）才能生效。修改后请切换飞行模式，等待3秒后再关闭飞行模式。"
          />
          <div className="carrier-grid capability-grid">
            <Panel title="载波聚合 (CA)">
              <Kv items={[{ label: '当前状态', value: nrCa ? '已开启' : '已关闭' }]} />
              <div className="capability-switch-row">
                <span className="capability-control-label">配置开关</span>
                <div className="capability-switch-control">
                  <Typography.Text size="small" type="tertiary">
                    {nrCa ? '开启' : '关闭'}
                  </Typography.Text>
                  <Switch
                    size="small"
                    checked={nrCa}
                    onChange={handleSetCA}
                    loading={nrLoading}
                    aria-label={nrCa ? '关闭载波聚合' : '开启载波聚合'}
                  />
                </div>
              </div>
            </Panel>
            <Panel title="VoNR (5G语音)">
              <Kv items={[{ label: '当前状态', value: VONR_LABELS[nrVonr] || '未知' }]} />
              <Select
                style={{ width: '100%' }}
                value={nrVonr}
                onChange={(v) => handleSetVoNR(Number(v))}
                optionList={VONR_OPTIONS}
                loading={nrLoading}
              />
            </Panel>
            <Panel title="DSS (动态频谱共享)">
              <Kv
                items={[
                  { label: 'LTE-CRS', value: nrDss.rateMatchingLTE === 0 ? '支持' : '不支持' },
                  { label: 'DMRS-DL-Alt', value: nrDss.additionalDMRS === 0 ? '支持' : '不支持' },
                ]}
              />
              <Space>
                <Button size="small" onClick={() => handleSetDSS(0, 0)} loading={nrLoading}>
                  恢复默认
                </Button>
                <Button size="small" type="danger" onClick={() => handleSetDSS(1, 1)} loading={nrLoading}>
                  禁用DSS
                </Button>
              </Space>
            </Panel>
          </div>
        </div>
      </PageCard>

      <PageCard
        title="网络系统配置"
        extra={<RefreshBtn onClick={fetchSysCfg} loading={sysCfgLoading} />}
      >
        <div className="form-stack">
          <Field label="网络制式优先级">
            <div>
              <Typography.Text type="tertiary">{acqOrderDisplay}</Typography.Text>
              <Select
                multiple
                style={{ width: '100%', marginTop: 8 }}
                value={sysCfg.acqorder.match(/.{2}/g) || []}
                onChange={(v) => {
                  const values = Array.isArray(v) ? (v as string[]) : [];
                  setSysCfg((prev) => ({ ...prev, acqorder: values.join('') }));
                }}
                optionList={NETWORK_MODES}
                placeholder="选择网络制式优先级"
              />
            </div>
          </Field>
          <TwoCol>
            <Field label="漫游设置">
              <Select
                style={{ width: '100%' }}
                value={sysCfg.roam}
                onChange={(v) => setSysCfg((prev) => ({ ...prev, roam: Number(v) }))}
                optionList={ROAM_OPTIONS}
              />
            </Field>
            <Field label="服务类型">
              <Select
                style={{ width: '100%' }}
                value={sysCfg.srvdomain}
                onChange={(v) => setSysCfg((prev) => ({ ...prev, srvdomain: Number(v) }))}
                optionList={SRVDOMAIN_OPTIONS}
              />
            </Field>
          </TwoCol>
          {sysCfg.acqorder.includes('03') && (
            <Field label="4G LTE 频段选择">
              <Select
                multiple
                style={{ width: '100%' }}
                value={parseBandHex(sysCfg.lteband, LTE_BAND_OPTIONS)}
                onChange={(v) => {
                  const values = Array.isArray(v) ? (v as string[]) : [];
                  setSysCfg((prev) => ({ ...prev, lteband: combineBandHex(values, '7FFFFFFFFFFFFFFF') }));
                }}
                optionList={LTE_BAND_OPTIONS}
                placeholder="选择4G LTE频段（不选则默认全频段）"
              />
            </Field>
          )}
          {sysCfg.acqorder.includes('02') && (
            <Field label="3G WCDMA 频段选择">
              <Select
                multiple
                style={{ width: '100%' }}
                value={parseBandHex(sysCfg.band, WCDMA_BAND_OPTIONS)}
                onChange={(v) => {
                  const values = Array.isArray(v) ? (v as string[]) : [];
                  setSysCfg((prev) => ({ ...prev, band: combineBandHex(values, '3FFFFFFF') }));
                }}
                optionList={WCDMA_BAND_OPTIONS}
                placeholder="选择3G WCDMA频段（不选则默认全频段）"
              />
            </Field>
          )}
          <div className="action-row">
            <Button
              theme="solid"
              type="primary"
              onClick={handleApplySysCfg}
              loading={sysCfgLoading}
              disabled={isSysCfgUnchanged}
            >
              应用配置
            </Button>
          </div>
        </div>
      </PageCard>

      <SectionHeader
        title="射频与温控"
        desc="发射功率信息与温度保护控制"
        id="rf-thermal"
      />
      <PageCard
        title="发射功率信息"
        extra={<RefreshBtn onClick={fetchTxPower} loading={txPowerLoading} />}
      >
        {txPower.length === 0 ? (
          <Typography.Text type="tertiary">暂无发射功率数据</Typography.Text>
        ) : (
          <div className="carrier-grid">
            {txPower.map((c, i) => (
              <Panel title={`载波 ${i + 1}`} key={i}>
                <Kv
                  items={[
                    { label: 'PUSCH 发射功率', value: c.PPusch === 999 ? '不适用' : `${c.PPusch} dBm` },
                    { label: 'PUCCH 发射功率', value: c.PPucch === 999 ? '不适用' : `${c.PPucch} dBm` },
                    { label: 'SRS 发射功率', value: c.PSrs === 999 ? '不适用' : `${c.PSrs} dBm` },
                    { label: 'PRACH 发射功率', value: c.PPrach === 999 ? '不适用' : `${c.PPrach} dBm` },
                    { label: '频率', value: c.Freq === 0 ? '不适用' : `${c.Freq} KHz` },
                  ]}
                />
              </Panel>
            ))}
          </div>
        )}
      </PageCard>

      <PageCard
        title="温度保护控制"
        extra={<RefreshBtn onClick={fetchThermConfig} loading={thermLoading} />}
      >
        <div className="form-stack">
          <Banner
            type="info"
            closeIcon={null}
            description="温度保护功能可以防止设备过热，当设备温度升高到危险水平时，系统会自动降低工作性能以保护硬件安全。"
          />
          <TwoCol>
            <Panel title="温保功能开关">
              <Kv items={[{ label: '当前状态', value: therm.enabled ? '已开启' : '已关闭' }]} />
              <Switch
                checked={therm.enabled}
                onChange={handleSetThermEnabled}
                loading={thermLoading}
              />
            </Panel>
            <Panel title="温度检测间隔">
              <Kv items={[{ label: '当前间隔', value: `${therm.interval} 秒` }]} />
              <Select
                style={{ width: '100%' }}
                value={therm.interval}
                onChange={(v) => handleSetThermInterval(Number(v))}
                optionList={INTERVAL_OPTIONS}
                loading={thermLoading}
              />
            </Panel>
          </TwoCol>
          <Panel title="温度保护状态">
            <Kv
              items={[
                { label: '当前温保等级', value: LEVEL_LABELS[therm.currentLevel] || '未知' },
                { label: '一级温保阈值', value: therm.thresholds[1] != null ? `${therm.thresholds[1]}°C` : '—' },
                { label: '二级温保阈值', value: therm.thresholds[3] != null ? `${therm.thresholds[3]}°C` : '—' },
                { label: '三级温保阈值', value: therm.thresholds[5] != null ? `${therm.thresholds[5]}°C` : '—' },
                {
                  label: '日志开关',
                  value: `控制台 ${therm.logSwitch.consoleLog ? '开' : '关'} / 文件 ${therm.logSwitch.fileLog ? '开' : '关'}`,
                },
              ]}
            />
          </Panel>
        </div>
      </PageCard>

      <Modal
        title={PIN_MODAL_TITLES[pinOperation]}
        visible={showPinModal}
        onCancel={closePinModal}
        onOk={handlePinOperation}
        confirmLoading={pinLoading}
        okText="确认"
        cancelText="取消"
      >
        <div className="form-stack">
          <Input
            mode="password"
            value={pinInput}
            onChange={(v) => setPinInput(v.replace(/\D/g, ''))}
            placeholder={pinPlaceholder}
            maxLength={8}
          />
          {(pinOperation === 'enable' || pinOperation === 'change' || pinOperation === 'unblock') && (
            <Input
              mode="password"
              value={newPinInput}
              onChange={(v) => setNewPinInput(v.replace(/\D/g, ''))}
              placeholder={newPinPlaceholder}
              maxLength={8}
            />
          )}
          <Banner type="warning" closeIcon={null} description={pinBannerText} />
        </div>
      </Modal>
    </div>
  );
};

export default SystemInfo;
