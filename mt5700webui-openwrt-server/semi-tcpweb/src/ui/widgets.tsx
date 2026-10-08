import React from 'react';
import {
  Banner,
  Button,
  Card,
  InputNumber,
  Select,
  Space,
  Switch,
  Tag,
  Typography,
} from '@douyinfe/semi-ui';
import { IconRefresh } from '@douyinfe/semi-icons';

const joinClassNames = (...names: Array<string | false | null | undefined>) =>
  names.filter(Boolean).join(' ');

export const PageCard: React.FC<{
  title: React.ReactNode;
  extra?: React.ReactNode;
  hint?: string;
  variant?: 'default' | 'hero' | 'group';
  bodyClassName?: string;
  footer?: React.ReactNode;
  children?: React.ReactNode;
}> = ({ title, extra, hint, variant = 'default', bodyClassName, footer, children }) => (
  <Card
    className={joinClassNames('page-card', variant !== 'default' && `page-card--${variant}`)}
    title={
      <div className="page-card-title">
        <span>{title}</span>
        {hint ? <span className="page-card-hint">{hint}</span> : null}
      </div>
    }
    headerExtraContent={extra}
  >
    {bodyClassName ? <div className={bodyClassName}>{children}</div> : children}
    {footer ? <div className="page-card-footer">{footer}</div> : null}
  </Card>
);

/**
 * `variant="flat"` drops the border/background/elevation so a Panel can sit
 * *inside* a PageCard without producing the three-deep nesting
 * (PageCard 2px border -> Panel 2px border -> .kv-item 1.5px border) that made
 * the information page read as boxes-inside-boxes-inside-boxes. Inside a card,
 * a titled sub-section only needs the heading rule to separate it.
 */
export const Panel: React.FC<{
  title?: React.ReactNode;
  extra?: React.ReactNode;
  accent?: boolean;
  variant?: 'default' | 'flat';
  className?: string;
  children?: React.ReactNode;
}> = ({ title, extra, accent, variant = 'default', className, children }) => (
  <div
    className={joinClassNames(
      'panel',
      variant === 'flat' && 'panel--flat',
      accent && 'panel--accent',
      className,
    )}
  >
    {title ? (
      <div className="panel-head">
        <span>{title}</span>
        {extra ? <span className="panel-head-extra">{extra}</span> : null}
      </div>
    ) : null}
    {children}
  </div>
);

export const SectionHeader: React.FC<{
  title: React.ReactNode;
  desc?: React.ReactNode;
  icon?: React.ReactNode;
  extra?: React.ReactNode;
  id?: string;
}> = ({ title, desc, icon, extra, id }) => (
  <div className="section-header" id={id}>
    <div className="section-header-main">
      {icon ? <span className="section-header-icon">{icon}</span> : <span className="section-header-marker" />}
      <div>
        <div className="section-header-title">{title}</div>
        {desc ? <div className="section-header-desc">{desc}</div> : null}
      </div>
    </div>
    {extra ? <div className="section-header-extra">{extra}</div> : null}
  </div>
);

export const Metric: React.FC<{
  label: string;
  value: React.ReactNode;
  hint?: string;
  color?: string;
  size?: 'lg' | 'md' | 'sm';
  align?: 'left' | 'center';
  tile?: boolean;
}> = ({ label, value, hint, color, size = 'md', align = 'left', tile }) => {
  // 数值变化时给一次极轻的抬升。信号/温度这类每几秒刷新一次的读数，
  // 静默跳变会让人怀疑「这次刷新到底有没有生效」；抬一下就明确了。
  // 用 key 变化重挂载而不是给每个值做 state diff——轮询场景下 diff 成本
  // 与渲染成本相当，而重挂载天然只在值真的变化时发生。
  const text = value === null || value === undefined ? '—' : String(value);
  const [bump, setBump] = React.useState(false);
  const prev = React.useRef(text);
  React.useEffect(() => {
    if (prev.current === text) return;
    prev.current = text;
    setBump(true);
    const t = window.setTimeout(() => setBump(false), 520);
    return () => window.clearTimeout(t);
  }, [text]);

  return (
    <div
      className={joinClassNames(
        'metric',
        size !== 'md' && `metric--${size}`,
        align === 'center' && 'metric--center',
        tile && 'metric-tile',
        bump && 'metric--updated',
      )}
    >
      <div className="metric-value" style={color ? { color } : undefined}>
        {value ?? '—'}
      </div>
      <div className="metric-label">{label}</div>
      {hint ? <div className="metric-hint">{hint}</div> : null}
    </div>
  );
};

export const Kv: React.FC<{
  items: Array<{ label: string; value: React.ReactNode }>;
  columns?: 1 | 2 | 3 | 4;
  dense?: boolean;
}> = ({ items, columns = 2, dense }) => (
  <div
    className={joinClassNames(
      'kv-grid',
      // `columns={1}` used to emit `kv-grid--1`, which no rule defines, so the
      // grid silently stayed at 2 columns. `kv-grid--single` is a real rule.
      columns === 1 && 'kv-grid--single',
      columns > 2 && `kv-grid--${columns}`,
      dense && 'kv-grid--dense',
    )}
  >
    {items.map((item) => (
      <div className="kv-item" key={item.label}>
        <div className="kv-label">{item.label}</div>
        <div className="kv-value">{item.value ?? '—'}</div>
      </div>
    ))}
  </div>
);

export const AutoRefresh: React.FC<{
  enabled: boolean;
  interval: number;
  onChange: (enabled: boolean, interval: number) => void;
}> = ({ enabled, interval, onChange }) => (
  <Space>
    <Switch checked={enabled} onChange={(value) => onChange(value, interval)} />
    <Typography.Text size="small" type="tertiary">
      自动刷新
    </Typography.Text>
    {enabled ? (
      <>
        <InputNumber
          size="small"
          min={1}
          max={60}
          value={interval}
          onChange={(value) => onChange(true, Number(value) || 5)}
          style={{ width: 72 }}
        />
        <Typography.Text size="small">秒</Typography.Text>
      </>
    ) : null}
  </Space>
);

export const RefreshBtn: React.FC<{ onClick: () => void; loading?: boolean; label?: string }> = ({
  onClick,
  loading,
  label = '刷新',
}) => (
  <Button icon={<IconRefresh />} onClick={onClick} loading={loading} size="small">
    {label}
  </Button>
);

export const TwoCol: React.FC<{ children: React.ReactNode; className?: string }> = ({
  children,
  className,
}) => <div className={joinClassNames('two-col', className)}>{children}</div>;

/**
 * `fullWidth` (default true) makes the control span the field. Semi renders
 * `Select`/`Input`/`InputNumber` at an intrinsic width, so a row mixing a
 * `Select` with `width:'100%'` and a bare `Input` rendered visibly ragged.
 * Centralising it here removes the need for per-call-site inline widths.
 */
export const Field: React.FC<{
  label: string;
  extra?: React.ReactNode;
  hint?: React.ReactNode;
  className?: string;
  fullWidth?: boolean;
  children: React.ReactNode;
}> = ({ label, extra, hint, className, fullWidth = true, children }) => (
  <div
    className={joinClassNames(
      'field',
      fullWidth && 'field--full',
      className,
    )}
  >
    <div className="field-label">
      {label}
      {extra}
    </div>
    {children}
    {hint ? <div className="field-hint">{hint}</div> : null}
  </div>
);

export const ConfigSelect: React.FC<{
  label: string;
  current: React.ReactNode;
  currentColor?: string;
  value?: string | number;
  onChange: (value: any) => void;
  options: Array<{ label: React.ReactNode; value: string | number; disabled?: boolean }>;
  placeholder?: string;
  loading?: boolean;
  disabled?: boolean;
  hint?: React.ReactNode;
  warning?: React.ReactNode;
}> = ({
  label,
  current,
  currentColor = 'red',
  value,
  onChange,
  options,
  placeholder,
  loading,
  disabled,
  hint,
  warning,
}) => (
  <div className="config-select">
    <div className="config-select-current">
      <span className="config-select-current-label">当前状态</span>
      <Tag color={currentColor as any}>{current ?? '未知'}</Tag>
    </div>
    <Field label={label} hint={hint}>
      <Select
        value={value}
        optionList={options as any}
        placeholder={placeholder}
        loading={loading}
        disabled={disabled}
        onChange={onChange}
      />
    </Field>
    {warning ? <Banner type="warning" closeIcon={null} description={warning} /> : null}
  </div>
);

/**
 * 加载骨架。
 *
 * 之前页面在数据到位前只渲染「--」或空文本，于是「还在加载」和「加载完成
 * 但确实没数据」在视觉上完全一样——弱信号区尤其误导：等 15 秒看到一片
 * 「--」，用户会以为模组坏了。骨架让这两种状态可区分。
 *
 * aria-busy 由调用方负责（本组件是纯展示），Screen reader 会读
 * 内部文本，故骨架条本身标 aria-hidden。
 */
export const Skeleton: React.FC<{
  /** 行数 */
  lines?: number;
  /** 首行是否加宽（模拟标题） */
  title?: boolean;
  /** 卡片高度 px */
  height?: number;
  className?: string;
}> = ({ lines = 3, title, height, className }) => (
  <div className={joinClassNames('skeleton', className)} aria-hidden="true">
    {title ? <div className="skeleton-line skeleton-line--title" /> : null}
    {Array.from({ length: lines }, (_, i) => (
      <div
        key={i}
        className="skeleton-line"
        style={{ width: `${100 - i * 12}%`, ...(height ? { height } : undefined) }}
      />
    ))}
  </div>
);

/** 卡片级骨架：多张卡片依次闪入，用于页面首屏。 */
export const SkeletonCards: React.FC<{ count?: number; className?: string }> = ({
  count = 4,
  className,
}) => (
  <div className={joinClassNames('skeleton-grid', className)}>
    {Array.from({ length: count }, (_, i) => (
      <div key={i} className="skeleton-card" style={{ ['--i' as string]: i }}>
        <Skeleton lines={3} title />
      </div>
    ))}
  </div>
);
