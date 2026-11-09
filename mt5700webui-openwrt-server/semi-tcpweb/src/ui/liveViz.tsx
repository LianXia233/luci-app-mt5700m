import React, { useId } from 'react';

/*
 * 自绘动态 SVG 视觉层（WebUI 版）
 * ---------------------------------------------
 * 与 LuCI 侧 mt5700m/visuals.js 同一套几何与动效语言：
 *   SignalOrb       信号球：进度弧 + 反向刻度环 + 轨道粒子
 *   LinkTopology    链路拓扑：模组 → 基站 → 互联网，虚线流动
 *   ThroughputWave  吞吐波形：面积 + 描边生长动画
 *
 * 约定：
 * - 颜色只用 global.css 的 --app-* 令牌，暗色/主题切换自动跟随；
 * - 动效由 CSS（.lv-*）驱动，prefers-reduced-motion: reduce 时静止；
 * - 组件不渲染文案，标题与数值仍由页面负责（i18n / 可选中）。
 */

const clamp = (value: number, low: number, high: number): number =>
  Math.max(low, Math.min(high, value));

export type VizTone = 'ok' | 'warn' | 'bad';

const toneColor = (tone: VizTone): string =>
  tone === 'bad' ? 'var(--app-danger)' : tone === 'warn' ? 'var(--app-warning)' : 'var(--app-success)';

export interface SignalOrbProps {
  percent?: number | null;
  tone?: VizTone;
  size?: number;
}

export const SignalOrb: React.FC<SignalOrbProps> = ({ percent, tone = 'ok', size = 148 }) => {
  const uid = useId().replace(/[^a-zA-Z0-9_-]/g, '');
  const pct = clamp(Number(percent) || 0, 0, 100);
  const color = toneColor(tone);
  const center = size / 2;
  const radius = size * 0.36;
  const circumference = 2 * Math.PI * radius;
  const arc = (circumference * pct) / 100;

  return (
    <svg
      className="lv-svg lv-orb"
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      role="img"
      aria-hidden="true"
    >
      <defs>
        <linearGradient id={`lv-orb-${uid}`} x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" stopColor={color} stopOpacity="0.95" />
          <stop offset="100%" stopColor="var(--app-secondary)" stopOpacity="0.55" />
        </linearGradient>
        <filter id={`lv-glow-${uid}`} x="-40%" y="-40%" width="180%" height="180%">
          <feGaussianBlur stdDeviation="3.5" result="blur" />
          <feMerge>
            <feMergeNode in="blur" />
            <feMergeNode in="SourceGraphic" />
          </feMerge>
        </filter>
      </defs>

      <g className="lv-rotor lv-rotor--slow" style={{ transformOrigin: `${center}px ${center}px` }}>
        <circle
          cx={center}
          cy={center}
          r={radius + 14}
          fill="none"
          stroke="var(--app-border)"
          strokeWidth="1"
          strokeDasharray="2 6"
          opacity="0.9"
        />
      </g>

      <g className="lv-rotor" style={{ transformOrigin: `${center}px ${center}px`, animationDuration: '26s' }}>
        <circle
          cx={center}
          cy={center}
          r={radius + 8}
          fill="none"
          stroke="var(--app-border-light)"
          strokeWidth="1"
          opacity="0.75"
        />
        <circle cx={center} cy={center - radius - 8} r="3" fill="var(--app-primary)" opacity="0.9" />
      </g>

      <circle cx={center} cy={center} r={radius} fill="none" stroke="var(--app-border-light)" strokeWidth="7" />
      <circle
        cx={center}
        cy={center}
        r={radius}
        fill="none"
        stroke={`url(#lv-orb-${uid})`}
        strokeWidth="7"
        strokeLinecap="round"
        strokeDasharray={`${arc} ${circumference}`}
        transform={`rotate(-90 ${center} ${center})`}
        filter={`url(#lv-glow-${uid})`}
        className="lv-arc"
      />

      <circle cx={center} cy={center} r={size * 0.18} fill="var(--app-surface-2)" stroke="var(--app-border-light)" />
      <circle cx={center} cy={center} r={size * 0.06} fill={color} className="lv-core" />
      <circle
        cx={center}
        cy={center}
        r={size * 0.105}
        fill="none"
        stroke={color}
        strokeWidth="1"
        opacity="0.35"
        className="lv-core-ring"
      />
    </svg>
  );
};

export interface LinkTopologyProps {
  connected: boolean;
  reachable?: boolean;
  height?: number;
}

export const LinkTopology: React.FC<LinkTopologyProps> = ({ connected, reachable = true, height = 92 }) => {
  const width = 320;
  const y = height / 2;
  const color = connected ? 'var(--app-success)' : reachable ? 'var(--app-warning)' : 'var(--app-danger)';
  const nodes = [
    { x: 46, kind: 'chip' as const },
    { x: 160, kind: 'tower' as const },
    { x: 274, kind: 'cloud' as const },
  ];

  const icon = (kind: 'chip' | 'tower' | 'cloud', x: number): React.ReactNode => {
    if (kind === 'chip') {
      return (
        <g key="chip">
          <rect
            x={x - 11}
            y={y - 11}
            width="22"
            height="22"
            rx="5"
            fill="none"
            stroke="var(--app-text-2)"
            strokeWidth="1.8"
            opacity="0.85"
          />
          <rect x={x - 4} y={y - 4} width="8" height="8" rx="2" fill={color} />
        </g>
      );
    }
    if (kind === 'tower') {
      return (
        <g key="tower">
          <path
            d={`M${x - 8},${y + 12} L${x},${y - 10} L${x + 8},${y + 12}`}
            fill="none"
            stroke="var(--app-text-2)"
            strokeWidth="1.8"
            strokeLinejoin="round"
            opacity="0.85"
          />
          <circle cx={x} cy={y - 13} r="3.2" fill={color} className="lv-core" />
          <path
            d={`M${x - 12},${y - 6} a12 12 0 0 1 24 0`}
            fill="none"
            stroke={color}
            strokeWidth="1.2"
            opacity="0.45"
          />
        </g>
      );
    }
    return (
      <path
        key="cloud"
        d={`M${x - 13},${y + 4} a7 7 0 0 1 13,-6 a6 6 0 0 1 12,2 a6 6 0 0 1 1,10 H${x - 13} Z`}
        fill="none"
        stroke="var(--app-text-2)"
        strokeWidth="1.7"
        strokeLinejoin="round"
        opacity="0.85"
      />
    );
  };

  return (
    <svg
      className="lv-svg lv-topo"
      viewBox={`0 0 ${width} ${height}`}
      preserveAspectRatio="xMidYMid meet"
      role="img"
      aria-hidden="true"
    >
      {nodes.slice(0, -1).map((node, index) => {
        const next = nodes[index + 1];
        const x1 = node.x + 16;
        const x2 = next.x - 18;
        return (
          <g key={`link-${index}`}>
            <line
              x1={x1}
              y1={y}
              x2={x2}
              y2={y}
              stroke={color}
              strokeWidth="2"
              strokeDasharray="6 6"
              className="lv-dash"
              opacity={connected ? 0.85 : 0.4}
            />
            <circle
              cx={(x1 + x2) / 2}
              cy={y}
              r="2.6"
              fill={color}
              className="lv-packet"
              style={{ animationDelay: `${index * 320}ms` }}
            />
          </g>
        );
      })}
      {nodes.map((node) => icon(node.kind, node.x))}
    </svg>
  );
};

export interface ThroughputWaveProps {
  series: number[];
  height?: number;
  color?: string;
  emptyLabel?: string;
}

export const ThroughputWave: React.FC<ThroughputWaveProps> = ({
  series,
  height = 96,
  color = 'var(--app-primary)',
}) => {
  const uid = useId().replace(/[^a-zA-Z0-9_-]/g, '');
  const width = 320;
  const pad = 6;
  const baseY = height - 10;
  const points = series.length ? series : [];
  const max = Math.max(1, ...points.map((v) => Math.abs(Number(v) || 0)));
  const coords = points.map((value, index) => {
    const x = points.length === 1 ? pad + (width - pad * 2) / 2 : pad + ((width - pad * 2) * index) / (points.length - 1);
    const y = baseY - ((baseY - 12) * (Math.abs(Number(value) || 0) / max));
    return [x, y] as const;
  });

  let line = '';
  if (coords.length) {
    line = `M${coords[0][0]},${coords[0][1]}`;
    for (let i = 1; i < coords.length; i++) {
      const prev = coords[i - 1];
      const cur = coords[i];
      const mid = (prev[0] + cur[0]) / 2;
      line += ` C${mid},${prev[1]} ${mid},${cur[1]} ${cur[0]},${cur[1]}`;
    }
  }
  const area = coords.length
    ? `${line} L${coords[coords.length - 1][0]},${baseY} L${coords[0][0]},${baseY} Z`
    : '';

  return (
    <svg
      className="lv-svg lv-wave"
      viewBox={`0 0 ${width} ${height}`}
      preserveAspectRatio="none"
      role="img"
      aria-hidden="true"
    >
      <defs>
        <linearGradient id={`lv-fill-${uid}`} x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={color} stopOpacity="0.28" />
          <stop offset="100%" stopColor={color} stopOpacity="0" />
        </linearGradient>
      </defs>
      <line x1={pad} y1={baseY} x2={width - pad} y2={baseY} stroke="var(--app-border-light)" strokeDasharray="1 5" />
      {coords.length ? (
        <>
          <path d={area} fill={`url(#lv-fill-${uid})`} className="lv-wave-area" />
          <path
            d={line}
            fill="none"
            stroke={color}
            strokeWidth="2.4"
            strokeLinecap="round"
            strokeLinejoin="round"
            pathLength={1}
            className="lv-wave-line"
          />
        </>
      ) : (
        <line
          x1={pad}
          y1={baseY}
          x2={width - pad}
          y2={baseY}
          stroke="var(--app-border)"
          strokeDasharray="6 6"
          opacity="0.7"
        />
      )}
    </svg>
  );
};
