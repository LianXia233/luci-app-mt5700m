import React, { useEffect, useState, useId } from 'react';
import { useMediaQuery } from '@/hooks/useMediaQuery';
import { QUERY_MOBILE } from '@/styles/breakpoints';

/**
 * Kawaii Minimal Dynamic SVG Visuals
 * Soft pastel palette:
 * - Primary Soft Pink: #F9A8D4 / #F472B6
 * - Secondary Soft Purple: #A78BFA
 * - Tertiary Soft Cyan: #67E8F9
 * - Accent Soft Yellow: #FDE68A
 * - Warm White: #FFF7ED
 *
 * Micro-interactions: jelly bounce, squishy press, gentle floating.
 * No neon/glow effects, no harsh dark shadows.
 */

/**
 * 装饰动画的使用者授权（Ambient Motion Opt-in）。
 *
 * 2026-10-04 实测（tools/probe_nav_anim.py）：桌面首屏 `getAnimations()` 返回
 * 19 条，其中 **18 条是 infinite**——4 个 bubble + 1 cloud + 3 ripple + 10 个
 * kawaii-p/pr 圆点，全在 `position: fixed` 的 SVG 装饰层里。手机档仍有 11 条。
 *
 * 这些动画不承载任何语义，纯粹是背景装饰，但代价是实打实的：每条 infinite
 * 动画都会让浏览器在合成线程上 perpetually 保持图层，目标设备（MT6880 路由
 * + 低端客户端浏览器）在同时跑数据面轮询时会明显感到卡顿。
 *
 * 判定为「只保留静态装饰」的情况：
 *   - 视口 < 768px（手机档）：装饰层几乎全在视口外，看不见却照样合成；
 *   - prefers-reduced-motion: reduce（无障碍）；
 *   - 低并发设备（hardwareConcurrency <= 4）。
 * 三者任一成立即降级为静态图形：装饰仍然绘制（视觉不缺失），但不再启动
 * 任何动画。这比纯 CSS 的 `animation: none` 更彻底——不创建动画对象，
 * 自然不占合成线程。
 */
export const useAmbientMotion = (): boolean => {
  const isMobile = useMediaQuery(QUERY_MOBILE);
  const [reduced, setReduced] = useState(false);
  const [weak, setWeak] = useState(false);
  const [forced, setForced] = useState<boolean | null>(null);

  useEffect(() => {
    if (typeof window === 'undefined' || !window.matchMedia) return;
    const mq = window.matchMedia('(prefers-reduced-motion: reduce)');
    setReduced(mq.matches);
    const onChange = () => setReduced(mq.matches);
    mq.addEventListener('change', onChange);
    // 低并发设备（如 4 核以下的瘦客户端 / 低端路由 CPU）同样降级。
    const cores = navigator.hardwareConcurrency ?? 8;
    setWeak(cores <= 4);

    // 显式开关：localStorage['ambient-motion'] = 'off' | 'on'。
    // 核数启发式对「浏览器跑在高性能 PC 上但页面服务的是低功耗路由」这种
    // 常见场景判断不出来（实测桌面 16 核仍会在路由器上打开全部装饰动画）。
    // 现场排障时可以在浏览器控制台一键关掉，不用改代码重新构建。
    let stored: boolean | null = null;
    try {
      const v = window.localStorage.getItem('ambient-motion');
      if (v === 'off') stored = false;
      else if (v === 'on') stored = true;
    } catch {
      /* localStorage 不可用（隐私模式）时忽略即可 */
    }
    setForced(stored);

    return () => mq.removeEventListener('change', onChange);
  }, []);

  return !isMobile && !reduced && !weak && forced !== false;
};

// ============================================================================
// 1. SvgAmbientMesh: 暖白与粉彩浮动气泡/柔和云朵背景
// ============================================================================
export const SvgAmbientMesh: React.FC = () => {
  const motion = useAmbientMotion();
  // 关闭动效时把动画类名换成静态类名：DOM 里不再存在 animation 声明，
  // getAnimations() 自然归零，而不是靠 CSS 事后覆盖。
  // base 形如 "kawaii-p kawaii-p-1"：整体是动画类，后面可跟修饰类。
  // 降级时必须把 `--static` 追加到**动画类**上（kawaii-p--static），
  // 而不是拼在整串末尾（kawaii-p-1--static），否则与 CSS 选择器不匹配。
  const anim = (base: string) => {
    if (motion) return base;
    const parts = base.split(/\s+/);
    return `${parts[0]}--static ${parts.slice(1).join(' ')}`;
  };

  return (
    <div
      className={`kawaii-ambient-mesh${motion ? '' : ' kawaii-ambient-mesh--still'}`}
      aria-hidden="true"
    >
      <svg
        className="kawaii-ambient-svg"
        viewBox="0 0 1440 900"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
        preserveAspectRatio="xMidYMid slice"
      >
        <defs>
          <filter id="kawaii-blur-soft" x="-20%" y="-20%" width="140%" height="140%">
            <feGaussianBlur stdDeviation="40" />
          </filter>
        </defs>

        {/* 柔和飘动粉彩色块 */}
        <circle
          cx="220"
          cy="180"
          r="160"
          fill="#FDE68A"
          opacity="0.32"
          filter="url(#kawaii-blur-soft)"
          className={anim("kawaii-bubble-float kawaii-bubble-float--1")}
        />
        <circle
          cx="1260"
          cy="260"
          r="210"
          fill="#F9A8D4"
          opacity="0.28"
          filter="url(#kawaii-blur-soft)"
          className={anim("kawaii-bubble-float kawaii-bubble-float--2")}
        />
        <circle
          cx="880"
          cy="780"
          r="240"
          fill="#A78BFA"
          opacity="0.22"
          filter="url(#kawaii-blur-soft)"
          className={anim("kawaii-bubble-float kawaii-bubble-float--3")}
        />
        <circle
          cx="340"
          cy="720"
          r="180"
          fill="#67E8F9"
          opacity="0.25"
          filter="url(#kawaii-blur-soft)"
          className={anim("kawaii-bubble-float kawaii-bubble-float--4")}
        />
      </svg>
    </div>
  );
};

// ============================================================================
// 2. SvgBrandLogo: 可爱极简风 5G 路由器/信号基站 Logo
// ============================================================================
export const SvgBrandLogo: React.FC<{ size?: number; className?: string }> = ({
  size = 36,
  className = '',
}) => {
  const id = useId().replace(/:/g, '');

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 48 48"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={`kawaii-brand-logo ${className}`}
    >
      <defs>
        <linearGradient id={`${id}-grad`} x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" stopColor="#F9A8D4" />
          <stop offset="100%" stopColor="#A78BFA" />
        </linearGradient>
      </defs>

      {/* 圆润饱满的糖果底座卡片 */}
      <rect
        x="3"
        y="3"
        width="42"
        height="42"
        rx="14"
        fill={`url(#${id}-grad)`}
      />

      {/* 柔和小天线 */}
      <line x1="16" y1="12" x2="16" y2="20" stroke="#FFFFFF" strokeWidth="2.5" strokeLinecap="round" />
      <circle cx="16" cy="11" r="2.2" fill="#FDE68A" />

      <line x1="32" y1="12" x2="32" y2="20" stroke="#FFFFFF" strokeWidth="2.5" strokeLinecap="round" />
      <circle cx="32" cy="11" r="2.2" fill="#67E8F9" />

      {/* 可爱小路由器主体 */}
      <rect x="10" y="20" width="28" height="18" rx="7" fill="#FFFFFF" />

      {/* 路由器笑脸信号点 */}
      <circle cx="17" cy="28" r="2" fill="#F472B6" />
      <circle cx="31" cy="28" r="2" fill="#F472B6" />
      <path
        d="M21 31 Q24 33.5 27 31"
        stroke="#F472B6"
        strokeWidth="1.8"
        strokeLinecap="round"
        fill="none"
      />
    </svg>
  );
};

// ============================================================================
// 3. SvgConnectionPulse: 柔和粉彩果冻弹跳状态指示器
// ============================================================================
export const SvgConnectionPulse: React.FC<{
  tone: 'ok' | 'warn' | 'err';
  busy?: boolean;
}> = ({ tone, busy }) => {
  const colorMap = {
    ok: { main: '#34D399', bg: '#D1FAE5', border: '#A7F3D0' },
    warn: { main: '#FBBF24', bg: '#FEF3C7', border: '#FDE68A' },
    err: { main: '#F87171', bg: '#FEE2E2', border: '#FECACA' },
  };
  const theme = colorMap[tone] || colorMap.err;

  return (
    <span className="kawaii-conn-pulse" aria-hidden="true">
      <svg width="20" height="20" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
        {/* 外圈柔和粉彩波纹 */}
        <circle
          className={`kawaii-pulse-ring ${busy ? 'is-busy' : ''}`}
          cx="12"
          cy="12"
          r="9"
          fill={theme.bg}
          stroke={theme.border}
          strokeWidth="1.5"
        />
        {/* 核心糖果圆点 */}
        <circle
          className="kawaii-pulse-dot"
          cx="12"
          cy="12"
          r="4.5"
          fill={theme.main}
        />
      </svg>
    </span>
  );
};

// ============================================================================
// 4. SvgSignalTower: 可爱极简 5G 基站 & 圆角糖果信号阶梯
// ============================================================================
export const SvgSignalTower: React.FC<{
  percent: number | null;
  is5G?: boolean;
  mode?: string;
  rsrp?: number | null;
  sinr?: number | null;
}> = ({ percent, is5G = true, mode, rsrp, sinr }) => {
  const motion = useAmbientMotion();
  // base 形如 "kawaii-p kawaii-p-1"：整体是动画类，后面可跟修饰类。
  // 降级时必须把 `--static` 追加到**动画类**上（kawaii-p--static），
  // 而不是拼在整串末尾（kawaii-p-1--static），否则与 CSS 选择器不匹配。
  const anim = (base: string) => {
    if (motion) return base;
    const parts = base.split(/\s+/);
    return `${parts[0]}--static ${parts.slice(1).join(' ')}`;
  };
  const p = percent ?? 0;
  const activeBars = p >= 80 ? 5 : p >= 60 ? 4 : p >= 40 ? 3 : p >= 20 ? 2 : p > 0 ? 1 : 0;
  const displayMode = mode || (is5G ? '5G NR' : '4G LTE');

  // 柔和糖果配色
  const barColors = ['#FDE68A', '#67E8F9', '#93C5FD', '#A78BFA', '#F9A8D4'];

  return (
    <div className="kawaii-signal-tower-card">
      <div className="kawaii-tower-graphic">
        <svg
          className="kawaii-tower-svg"
          viewBox="0 0 160 110"
          fill="none"
          xmlns="http://www.w3.org/2000/svg"
        >
          {/* 柔和云朵装饰 */}
          <g className={anim("kawaii-cloud-float")} opacity="0.8">
            <path
              d="M18 42 C18 36 24 33 28 35 C31 30 39 30 42 35 C46 33 51 36 50 42 Z"
              fill="#FFFFFF"
              stroke="#FBCFE8"
              strokeWidth="1.5"
            />
          </g>

          {/* 顶部发射信号波纹弧线 */}
          <g className="kawaii-tower-ripples">
            <path
              d="M 48 30 A 14 14 0 0 1 68 30"
              stroke="#F472B6"
              strokeWidth="2.5"
              strokeLinecap="round"
              fill="none"
              className={anim("kawaii-ripple-1")}
            />
            <path
              d="M 42 22 A 22 22 0 0 1 74 22"
              stroke="#A78BFA"
              strokeWidth="2.5"
              strokeLinecap="round"
              fill="none"
              className={anim("kawaii-ripple-2")}
            />
            <path
              d="M 36 14 A 30 30 0 0 1 80 14"
              stroke="#67E8F9"
              strokeWidth="2"
              strokeLinecap="round"
              strokeDasharray="4 3"
              fill="none"
              className={anim("kawaii-ripple-3")}
            />
          </g>

          {/* 可爱圆润天线塔 */}
          <g className="kawaii-tower-body">
            {/* 塔顶星星/爱心 */}
            <circle cx="58" cy="30" r="5" fill="#FDE68A" stroke="#F59E0B" strokeWidth="1.5" />
            <line x1="58" y1="35" x2="58" y2="44" stroke="#F472B6" strokeWidth="3" strokeLinecap="round" />

            {/* 圆角支架 */}
            <line x1="58" y1="44" x2="42" y2="98" stroke="#94A3B8" strokeWidth="3" strokeLinecap="round" />
            <line x1="58" y1="44" x2="74" y2="98" stroke="#94A3B8" strokeWidth="3" strokeLinecap="round" />
            
            {/* 横向圆润支撑 */}
            <line x1="48" y1="78" x2="68" y2="78" stroke="#CBD5E1" strokeWidth="2.5" strokeLinecap="round" />
            <line x1="52" y1="62" x2="64" y2="62" stroke="#CBD5E1" strokeWidth="2" strokeLinecap="round" />

            {/* 基底圆垫 */}
            <ellipse cx="42" cy="98" rx="4" ry="2" fill="#CBD5E1" />
            <ellipse cx="74" cy="98" rx="4" ry="2" fill="#CBD5E1" />
          </g>

          {/* 右侧：圆润软糖信号格 */}
          <g className="kawaii-signal-bars" transform="translate(94, 30)">
            {[
              { x: 0, y: 52, h: 18, color: barColors[0] },
              { x: 12, y: 42, h: 28, color: barColors[1] },
              { x: 24, y: 30, h: 40, color: barColors[2] },
              { x: 36, y: 18, h: 52, color: barColors[3] },
              { x: 48, y: 4, h: 66, color: barColors[4] },
            ].map((bar, idx) => {
              const active = idx < activeBars;
              return (
                <g key={idx} className={`kawaii-bar-item ${active ? 'is-active' : 'is-inactive'}`}>
                  {/* 软糖圆角信号条 */}
                  <rect
                    x={bar.x}
                    y={bar.y}
                    width="8"
                    height={bar.h}
                    rx="4"
                    fill={active ? bar.color : '#F3F4F6'}
                    stroke={active ? '#FFFFFF' : 'transparent'}
                    strokeWidth="1"
                    className="kawaii-bar-rect"
                  />
                  {/* 顶端可爱小圆点 */}
                  {active && (
                    <circle
                      cx={bar.x + 4}
                      cy={bar.y - 4}
                      r="2"
                      fill={bar.color}
                      className="kawaii-bar-dot"
                    />
                  )}
                </g>
              );
            })}
          </g>
        </svg>
      </div>

      <div className="kawaii-tower-meta">
        <div className="kawaii-tower-header">
          <span className="kawaii-badge-pill">{displayMode}</span>
          <span className="kawaii-tower-percent">
            {percent !== null ? `${p}%` : '未连接'}
          </span>
        </div>
        <div className="kawaii-tower-desc">
          {p >= 75 ? '🌸 信号满格，超棒' : p >= 50 ? '✨ 信号良好稳定' : p >= 25 ? '🌱 信号一般' : '💤 信号较弱'}
        </div>
        {(rsrp !== null && rsrp !== undefined) || (sinr !== null && sinr !== undefined) ? (
          <div className="kawaii-tower-metrics">
            {rsrp !== null && rsrp !== undefined ? <span>RSRP: {rsrp} dBm</span> : null}
            {sinr !== null && sinr !== undefined ? <span>SINR: {sinr} dB</span> : null}
          </div>
        ) : null}
      </div>
    </div>
  );
};

// ============================================================================
// 5. SvgDataStream: 可爱粉彩流动泡泡数据流
// ============================================================================
export const SvgDataStream: React.FC<{
  active?: boolean;
  downMbps?: number;
  upMbps?: number;
  dlRate?: number;
  ulRate?: number;
}> = ({ active, downMbps, upMbps, dlRate, ulRate }) => {
  const motion = useAmbientMotion();
  // base 形如 "kawaii-p kawaii-p-1"：整体是动画类，后面可跟修饰类。
  // 降级时必须把 `--static` 追加到**动画类**上（kawaii-p--static），
  // 而不是拼在整串末尾（kawaii-p-1--static），否则与 CSS 选择器不匹配。
  const anim = (base: string) => {
    if (motion) return base;
    const parts = base.split(/\s+/);
    return `${parts[0]}--static ${parts.slice(1).join(' ')}`;
  };
  const hasDl = (downMbps ?? (dlRate ? dlRate / 1024 : 0)) > 0.05;
  const hasUl = (upMbps ?? (ulRate ? ulRate / 1024 : 0)) > 0.05;
  const isStreaming = active ?? (hasDl || hasUl);

  return (
    <div className={`kawaii-datastream-container ${isStreaming ? 'is-streaming' : ''}`}>
      <svg
        className="kawaii-datastream-svg"
        viewBox="0 0 720 44"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
        preserveAspectRatio="none"
      >
        <defs>
          <linearGradient id="kawaii-track-dl" x1="0%" y1="0%" x2="100%" y2="0%">
            <stop offset="0%" stopColor="#F9A8D4" stopOpacity="0.15" />
            <stop offset="50%" stopColor="#A78BFA" stopOpacity="0.3" />
            <stop offset="100%" stopColor="#F9A8D4" stopOpacity="0.15" />
          </linearGradient>
          <linearGradient id="kawaii-track-ul" x1="0%" y1="0%" x2="100%" y2="0%">
            <stop offset="0%" stopColor="#67E8F9" stopOpacity="0.15" />
            <stop offset="50%" stopColor="#FDE68A" stopOpacity="0.3" />
            <stop offset="100%" stopColor="#67E8F9" stopOpacity="0.15" />
          </linearGradient>
        </defs>

        {/* 下行轨道 (柔和粉紫) */}
        <line
          x1="20"
          y1="14"
          x2="700"
          y2="14"
          stroke="url(#kawaii-track-dl)"
          strokeWidth="6"
          strokeLinecap="round"
        />

        {/* 上行轨道 (柔和青黄) */}
        <line
          x1="20"
          y1="30"
          x2="700"
          y2="30"
          stroke="url(#kawaii-track-ul)"
          strokeWidth="6"
          strokeLinecap="round"
        />

        {/* 动态流动糖果微粒 */}
        <g className="kawaii-stream-particles-dl">
          <circle cx="80" cy="14" r="5" fill="#F472B6" className={anim("kawaii-p kawaii-p-1")} />
          <circle cx="220" cy="14" r="4" fill="#A78BFA" className={anim("kawaii-p kawaii-p-2")} />
          <circle cx="360" cy="14" r="5.5" fill="#F9A8D4" className={anim("kawaii-p kawaii-p-3")} />
          <circle cx="500" cy="14" r="4.5" fill="#C084FC" className={anim("kawaii-p kawaii-p-4")} />
          <circle cx="640" cy="14" r="5" fill="#F472B6" className={anim("kawaii-p kawaii-p-5")} />
        </g>

        <g className="kawaii-stream-particles-ul">
          <circle cx="640" cy="30" r="4.5" fill="#67E8F9" className={anim("kawaii-p-rev kawaii-pr-1")} />
          <circle cx="500" cy="30" r="5" fill="#FDE68A" className={anim("kawaii-p-rev kawaii-pr-2")} />
          <circle cx="360" cy="30" r="4" fill="#38BDF8" className={anim("kawaii-p-rev kawaii-pr-3")} />
          <circle cx="220" cy="30" r="5.5" fill="#FCD34D" className={anim("kawaii-p-rev kawaii-pr-4")} />
          <circle cx="80" cy="30" r="4.5" fill="#67E8F9" className={anim("kawaii-p-rev kawaii-pr-5")} />
        </g>
      </svg>
    </div>
  );
};

// ============================================================================
// 6. SvgRadarScanner: 可爱极简粉彩雷达扫描盘
// ============================================================================
export const SvgRadarScanner: React.FC<{
  scanning: boolean;
  cellsFound?: number;
  cellCount?: number;
}> = ({ scanning, cellsFound, cellCount }) => {
  const count = cellsFound ?? cellCount ?? 0;
  return (
    <div className="kawaii-radar-container">
      <svg
        className={`kawaii-radar-svg ${scanning ? 'is-scanning' : ''}`}
        viewBox="0 0 200 200"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
      >
        <defs>
          <linearGradient id="kawaii-sweep-grad" x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" stopColor="#F9A8D4" stopOpacity="0.45" />
            <stop offset="60%" stopColor="#A78BFA" stopOpacity="0.1" />
            <stop offset="100%" stopColor="#A78BFA" stopOpacity="0" />
          </linearGradient>
        </defs>

        {/* 柔和同心圆环 */}
        <circle cx="100" cy="100" r="85" stroke="#FBCFE8" strokeWidth="2" strokeDasharray="6 4" />
        <circle cx="100" cy="100" r="60" stroke="#FBCFE8" strokeWidth="2" />
        <circle cx="100" cy="100" r="35" stroke="#FBCFE8" strokeWidth="1.5" strokeDasharray="4 3" />
        <circle cx="100" cy="100" r="12" fill="#FDF2F8" stroke="#F472B6" strokeWidth="2" />

        {/* 轴线 */}
        <line x1="100" y1="15" x2="100" y2="185" stroke="#FCE7F3" strokeWidth="1.5" />
        <line x1="15" y1="100" x2="185" y2="100" stroke="#FCE7F3" strokeWidth="1.5" />

        {/* 扇形扫描指针 */}
        <g className="kawaii-radar-sweep-beam">
          <path
            d="M 100 100 L 100 15 A 85 85 0 0 1 170 50 Z"
            fill="url(#kawaii-sweep-grad)"
          />
          <line x1="100" y1="100" x2="100" y2="15" stroke="#F472B6" strokeWidth="2.5" strokeLinecap="round" />
        </g>

        {/* 探测到的可爱小区糖果亮点 */}
        {count > 0 && (
          <g className="kawaii-radar-blips">
            <circle cx="125" cy="65" r="5" fill="#FDE68A" stroke="#F59E0B" strokeWidth="1.5" className="kawaii-blip-1" />
            {count > 1 && (
              <circle cx="70" cy="135" r="5.5" fill="#67E8F9" stroke="#0284C7" strokeWidth="1.5" className="kawaii-blip-2" />
            )}
            {count > 2 && (
              <circle cx="145" cy="115" r="4.5" fill="#A78BFA" stroke="#7C3AED" strokeWidth="1.5" className="kawaii-blip-3" />
            )}
            {count > 4 && (
              <circle cx="60" cy="75" r="5" fill="#F9A8D4" stroke="#DB2777" strokeWidth="1.5" className="kawaii-blip-4" />
            )}
          </g>
        )}
      </svg>
      <div className="kawaii-radar-footer">
        <span className="kawaii-radar-status">
          {scanning ? '🌸 正在温柔搜索周边基站...' : `已发现 ${count} 个蜂窝频段`}
        </span>
      </div>
    </div>
  );
};
