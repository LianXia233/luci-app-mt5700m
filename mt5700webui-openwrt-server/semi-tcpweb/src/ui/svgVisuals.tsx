import React, { useId } from 'react';

// ============================================================================
// 1. SvgAmbientMesh: 动态柔光背景网格与流动色斑
// 白色毛玻璃需要背景中有色彩流动与深度衬托，才能透出晶莹剔透的磨砂质感
// ============================================================================
export const SvgAmbientMesh: React.FC = () => {
  return (
    <div className="ambient-mesh-canvas" aria-hidden="true">
      <svg
        className="ambient-mesh-svg"
        viewBox="0 0 1440 900"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
        preserveAspectRatio="xMidYMid slice"
      >
        <defs>
          <filter id="ambient-blur-filter" x="-20%" y="-20%" width="140%" height="140%">
            <feGaussianBlur stdDeviation="90" result="blur" />
          </filter>
          <linearGradient id="orb-grad-1" x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" stopColor="#38bdf8" stopOpacity="0.45" />
            <stop offset="50%" stopColor="#818cf8" stopOpacity="0.35" />
            <stop offset="100%" stopColor="#c084fc" stopOpacity="0.15" />
          </linearGradient>
          <linearGradient id="orb-grad-2" x1="100%" y1="0%" x2="0%" y2="100%">
            <stop offset="0%" stopColor="#fb7185" stopOpacity="0.38" />
            <stop offset="50%" stopColor="#f43f5e" stopOpacity="0.22" />
            <stop offset="100%" stopColor="#fbcfe8" stopOpacity="0.1" />
          </linearGradient>
          <linearGradient id="orb-grad-3" x1="0%" y1="100%" x2="100%" y2="0%">
            <stop offset="0%" stopColor="#34d399" stopOpacity="0.3" />
            <stop offset="60%" stopColor="#38bdf8" stopOpacity="0.25" />
            <stop offset="100%" stopColor="#a7f3d0" stopOpacity="0.05" />
          </linearGradient>
          <linearGradient id="orb-grad-4" x1="50%" y1="0%" x2="50%" y2="100%">
            <stop offset="0%" stopColor="#f59e0b" stopOpacity="0.22" />
            <stop offset="100%" stopColor="#fbbf24" stopOpacity="0.06" />
          </linearGradient>

          {/* 细密科技网格纹理 */}
          <pattern id="ambient-grid-pattern" width="48" height="48" patternUnits="userSpaceOnUse">
            <path
              d="M 48 0 L 0 0 0 48"
              fill="none"
              stroke="currentColor"
              strokeWidth="0.8"
              className="ambient-grid-line"
            />
            <circle cx="48" cy="0" r="1.2" fill="currentColor" className="ambient-grid-dot" />
          </pattern>
        </defs>

        {/* 呼吸流动的弥散光球层 */}
        <g filter="url(#ambient-blur-filter)">
          <circle className="ambient-orb ambient-orb--1" cx="240" cy="180" r="280" fill="url(#orb-grad-1)" />
          <circle className="ambient-orb ambient-orb--2" cx="1200" cy="220" r="320" fill="url(#orb-grad-2)" />
          <circle className="ambient-orb ambient-orb--3" cx="680" cy="740" r="340" fill="url(#orb-grad-3)" />
          <circle className="ambient-orb ambient-orb--4" cx="1320" cy="800" r="260" fill="url(#orb-grad-4)" />
        </g>

        {/* 科技微点阵背景 */}
        <rect width="100%" height="100%" fill="url(#ambient-grid-pattern)" opacity="0.32" />
      </svg>
    </div>
  );
};

// ============================================================================
// 2. SvgBrandLogo: 动态 5G 调制解调器天线与信号波徽标
// 侧边栏品牌区域展示，多层向外律动扩散的动态无线电波
// ============================================================================
export const SvgBrandLogo: React.FC<{ size?: number; className?: string }> = ({ size = 32, className = '' }) => {
  const id = useId();
  return (
    <svg
      className={`svg-brand-logo ${className}`}
      width={size}
      height={size}
      viewBox="0 0 48 48"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      role="img"
      aria-label="MT5700 5G CPE"
    >
      <defs>
        <linearGradient id={`${id}-core-grad`} x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" stopColor="#ff4d4f" />
          <stop offset="100%" stopColor="#c7000b" />
        </linearGradient>
        <linearGradient id={`${id}-glow-grad`} x1="0%" y1="0%" x2="100%" y2="100%">
          <stop offset="0%" stopColor="#ff7875" stopOpacity="0.8" />
          <stop offset="100%" stopColor="#c7000b" stopOpacity="0" />
        </linearGradient>
      </defs>

      {/* 外圈脉冲动态扩散波 */}
      <circle className="brand-wave brand-wave--3" cx="24" cy="24" r="20" stroke="url(#orb-grad-1)" strokeWidth="1.2" strokeDasharray="3 3" />
      <circle className="brand-wave brand-wave--2" cx="24" cy="24" r="15" stroke="var(--app-accent-500)" strokeWidth="1.5" />
      <circle className="brand-wave brand-wave--1" cx="24" cy="24" r="10" stroke="var(--app-accent-600)" strokeWidth="1.8" />

      {/* 5G 天线塔基立柱 */}
      <path
        d="M24 10 L28 36 H20 L24 10 Z"
        fill="url(#${id}-core-grad)"
        opacity="0.9"
      />
      {/* 塔顶放射光针 */}
      <line x1="24" y1="6" x2="24" y2="12" stroke="#ff4d4f" strokeWidth="2.5" strokeLinecap="round" />

      {/* 核心信号发光点 */}
      <circle className="brand-core-pulse" cx="24" cy="8" r="3.2" fill="#ffffff" stroke="#c7000b" strokeWidth="1.5" />
      
      {/* 塔身横梁科技装饰 */}
      <line x1="21.5" y1="22" x2="26.5" y2="22" stroke="#ffffff" strokeWidth="1.2" strokeLinecap="round" opacity="0.9" />
      <line x1="20" y1="29" x2="28" y2="29" stroke="#ffffff" strokeWidth="1.2" strokeLinecap="round" opacity="0.9" />

      {/* 底部稳定基座圆环 */}
      <ellipse cx="24" cy="38" rx="8" ry="2.5" fill="none" stroke="var(--app-accent-500)" strokeWidth="1.2" opacity="0.7" />
    </svg>
  );
};

// ============================================================================
// 3. SvgConnectionPulse: 顶栏动态连接状态雷达/脉冲光环
// 根据连接状态展示翡翠绿脉冲、琥珀黄雷达扫掠、警示红心跳
// ============================================================================
export const SvgConnectionPulse: React.FC<{
  tone: 'ok' | 'warn' | 'err';
  busy?: boolean;
}> = ({ tone, busy }) => {
  const colorMap = {
    ok: { main: '#10b981', wave: 'rgba(16, 185, 129, 0.45)', glow: 'rgba(16, 185, 129, 0.2)' },
    warn: { main: '#f59e0b', wave: 'rgba(245, 158, 11, 0.45)', glow: 'rgba(245, 158, 11, 0.2)' },
    err: { main: '#f43f5e', wave: 'rgba(244, 63, 94, 0.45)', glow: 'rgba(244, 63, 94, 0.2)' },
  };
  const theme = colorMap[tone] || colorMap.err;

  return (
    <span className="svg-conn-pulse-wrapper" aria-hidden="true">
      <svg width="18" height="18" viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
        {/* 外圈扩散波纹 */}
        <circle
          className={`conn-pulse-ring ${busy ? 'conn-pulse-ring--busy' : 'conn-pulse-ring--steady'}`}
          cx="12"
          cy="12"
          r="8"
          stroke={theme.wave}
          strokeWidth="1.6"
        />
        {/* 内发光光晕 */}
        <circle cx="12" cy="12" r="5" fill={theme.glow} />
        {/* 核心实体圆点 */}
        <circle cx="12" cy="12" r="3.2" fill={theme.main} />
      </svg>
    </span>
  );
};

// ============================================================================
// 4. SvgSignalTower: 动态 5G 基站与信号强度全息可视化
// 包含 5G 信号塔骨架、向外发射的高频微波束、5 阶动态发光信号阶梯
// ============================================================================
export const SvgSignalTower: React.FC<{
  percent: number | null;
  mode?: string;
  rsrp?: number | null;
  sinr?: number | null;
}> = ({ percent, mode = '5G NR', rsrp, sinr }) => {
  const p = percent ?? 0;
  // 计算亮起的信号格数 (0 ~ 5)
  const activeBars = p >= 80 ? 5 : p >= 60 ? 4 : p >= 40 ? 3 : p >= 20 ? 2 : p > 0 ? 1 : 0;
  const isGood = p >= 60;
  const isWarn = p >= 35 && p < 60;
  const toneColor = isGood ? '#10b981' : isWarn ? '#f59e0b' : '#f43f5e';

  return (
    <div className="svg-signal-tower-card">
      <div className="signal-tower-graphic">
        <svg
          className="signal-tower-svg"
          viewBox="0 0 160 120"
          fill="none"
          xmlns="http://www.w3.org/2000/svg"
        >
          <defs>
            <linearGradient id="tower-beam-grad" x1="0%" y1="0%" x2="100%" y2="100%">
              <stop offset="0%" stopColor={toneColor} stopOpacity="0.8" />
              <stop offset="100%" stopColor={toneColor} stopOpacity="0.05" />
            </linearGradient>
            <linearGradient id="bar-active-grad" x1="0%" y1="100%" x2="0%" y2="0%">
              <stop offset="0%" stopColor="#0ea5e9" />
              <stop offset="100%" stopColor={toneColor} />
            </linearGradient>
          </defs>

          {/* 动态微波束辐射弧线 */}
          <g className="tower-radiation-waves">
            <path
              d="M 45 32 A 20 20 0 0 1 65 32"
              stroke={toneColor}
              strokeWidth="1.8"
              strokeLinecap="round"
              fill="none"
              className="radiation-wave radiation-wave--1"
            />
            <path
              d="M 38 24 A 32 32 0 0 1 72 24"
              stroke={toneColor}
              strokeWidth="1.6"
              strokeLinecap="round"
              fill="none"
              className="radiation-wave radiation-wave--2"
            />
            <path
              d="M 30 16 A 44 44 0 0 1 80 16"
              stroke={toneColor}
              strokeWidth="1.4"
              strokeDasharray="4 2"
              strokeLinecap="round"
              fill="none"
              className="radiation-wave radiation-wave--3"
            />
          </g>

          {/* 基站铁塔骨架 */}
          <g className="tower-structure" stroke="currentColor" opacity="0.85">
            <line x1="55" y1="36" x2="40" y2="105" strokeWidth="2.2" strokeLinecap="round" />
            <line x1="55" y1="36" x2="70" y2="105" strokeWidth="2.2" strokeLinecap="round" />
            {/* 桁架 X 撑 */}
            <line x1="44" y1="88" x2="66" y2="88" strokeWidth="1.5" />
            <line x1="48" y1="68" x2="62" y2="68" strokeWidth="1.4" />
            <line x1="51" y1="50" x2="59" y2="50" strokeWidth="1.2" />
            <line x1="44" y1="88" x2="62" y2="68" strokeWidth="1" strokeDasharray="2 2" opacity="0.6" />
            <line x1="66" y1="88" x2="48" y2="68" strokeWidth="1" strokeDasharray="2 2" opacity="0.6" />
            <line x1="48" y1="68" x2="59" y2="50" strokeWidth="1" strokeDasharray="2 2" opacity="0.6" />
            <line x1="62" y1="68" x2="51" y2="50" strokeWidth="1" strokeDasharray="2 2" opacity="0.6" />
            {/* 基座横杆 */}
            <line x1="34" y1="105" x2="76" y2="105" strokeWidth="2.5" strokeLinecap="round" />
            {/* 避雷针与射频发射机天线顶端 */}
            <line x1="55" y1="28" x2="55" y2="38" strokeWidth="2.5" strokeLinecap="round" stroke={toneColor} />
            <circle cx="55" cy="27" r="3" fill="#ffffff" stroke={toneColor} strokeWidth="2" className="tower-top-pulse" />
          </g>

          {/* 右侧：5 阶动态发光信号阶梯柱 */}
          <g className="signal-level-bars" transform="translate(92, 42)">
            {[
              { x: 0, y: 46, h: 16 },
              { x: 10, y: 38, h: 24 },
              { x: 20, y: 28, h: 34 },
              { x: 30, y: 16, h: 46 },
              { x: 40, y: 2, h: 60 },
            ].map((bar, idx) => {
              const active = idx < activeBars;
              return (
                <g key={idx} className={`signal-bar-group ${active ? 'is-active' : 'is-inactive'}`}>
                  {/* 背景槽位 */}
                  <rect
                    x={bar.x}
                    y={bar.y}
                    width="6.5"
                    height={bar.h}
                    rx="3.25"
                    fill={active ? 'url(#bar-active-grad)' : 'currentColor'}
                    opacity={active ? 1 : 0.15}
                    className="signal-bar-rect"
                  />
                  {active ? (
                    <circle
                      cx={bar.x + 3.25}
                      cy={bar.y - 4}
                      r="1.6"
                      fill={toneColor}
                      className="signal-bar-sparkle"
                    />
                  ) : null}
                </g>
              );
            })}
          </g>

          {/* 动态能量流动粒子 */}
          <circle className="energy-particle energy-particle--1" cx="55" cy="36" r="1.5" fill={toneColor} />
          <circle className="energy-particle energy-particle--2" cx="75" cy="46" r="1.2" fill={toneColor} />
        </svg>
      </div>

      <div className="signal-tower-meta">
        <div className="signal-tower-header">
          <span className="signal-tower-badge">{mode || '5G NR'}</span>
          <span className="signal-tower-percent" style={{ color: toneColor }}>
            {percent !== null ? `${p}%` : '未连接'}
          </span>
        </div>
        <div className="signal-tower-details">
          <span className="tower-desc">
            {p >= 75 ? '超强 5G 信号' : p >= 50 ? '良好蜂窝覆盖' : p >= 25 ? '一般驻留' : '弱信号/边缘'}
          </span>
          <div className="tower-metrics">
            {rsrp !== null && rsrp !== undefined ? <span>RSRP: <b>{rsrp} dBm</b></span> : null}
            {sinr !== null && sinr !== undefined ? <span>SINR: <b>{sinr} dB</b></span> : null}
          </div>
        </div>
      </div>
    </div>
  );
};

// ============================================================================
// 5. SvgDataStream: 动态双向数据流（光子微粒流动）
// 在速率仪表盘中呈现高速下行与上行的双向动态数据粒子流管道
// ============================================================================
export const SvgDataStream: React.FC<{
  dlRate: number; // kbps
  ulRate: number; // kbps
}> = ({ dlRate, ulRate }) => {
  const isDlActive = dlRate > 10;
  const isUlActive = ulRate > 10;
  
  // 流动速度随速率提高而加快
  const dlDuration = Math.max(0.6, 2.8 - Math.min(2.0, (dlRate / 50000)));
  const ulDuration = Math.max(0.6, 2.8 - Math.min(2.0, (ulRate / 20000)));

  return (
    <div className="svg-data-stream-container" aria-hidden="true">
      <svg
        className="data-stream-svg"
        viewBox="0 0 460 36"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
        preserveAspectRatio="none"
      >
        <defs>
          <linearGradient id="dl-stream-grad" x1="0%" y1="0%" x2="100%" y2="0%">
            <stop offset="0%" stopColor="#0284c7" stopOpacity="0.1" />
            <stop offset="50%" stopColor="#38bdf8" stopOpacity="0.8" />
            <stop offset="100%" stopColor="#0284c7" stopOpacity="0.1" />
          </linearGradient>
          <linearGradient id="ul-stream-grad" x1="100%" y1="0%" x2="0%" y2="0%">
            <stop offset="0%" stopColor="#059669" stopOpacity="0.1" />
            <stop offset="50%" stopColor="#34d399" stopOpacity="0.8" />
            <stop offset="100%" stopColor="#059669" stopOpacity="0.1" />
          </linearGradient>
        </defs>

        {/* 下行数据管道 */}
        <path
          d="M 10 10 C 120 10, 160 6, 230 6 C 300 6, 340 10, 450 10"
          stroke="rgba(14, 165, 233, 0.22)"
          strokeWidth="2.5"
          strokeLinecap="round"
        />
        <path
          d="M 10 10 C 120 10, 160 6, 230 6 C 300 6, 340 10, 450 10"
          stroke="url(#dl-stream-grad)"
          strokeWidth="2.5"
          strokeDasharray="16 28"
          strokeLinecap="round"
          className="stream-path-dl"
          style={{
            animationDuration: `${dlDuration}s`,
            opacity: isDlActive ? 1 : 0.35,
          }}
        />

        {/* 上行数据管道 */}
        <path
          d="M 10 26 C 120 26, 160 30, 230 30 C 300 30, 340 26, 450 26"
          stroke="rgba(16, 185, 129, 0.22)"
          strokeWidth="2.5"
          strokeLinecap="round"
        />
        <path
          d="M 10 26 C 120 26, 160 30, 230 30 C 300 30, 340 26, 450 26"
          stroke="url(#ul-stream-grad)"
          strokeWidth="2.5"
          strokeDasharray="14 26"
          strokeLinecap="round"
          className="stream-path-ul"
          style={{
            animationDuration: `${ulDuration}s`,
            opacity: isUlActive ? 1 : 0.35,
          }}
        />
      </svg>
    </div>
  );
};

// ============================================================================
// 6. SvgRadarScanner: 动态全网扫频雷达
// 扫频面板中展现的科技雷达扫描器，旋转扫描锥束与雷达定位点
// ============================================================================
export const SvgRadarScanner: React.FC<{
  scanning: boolean;
  cellCount?: number;
}> = ({ scanning, cellCount = 0 }) => {
  return (
    <div className={`radar-scanner-box ${scanning ? 'is-scanning' : ''}`}>
      <svg
        className="radar-scanner-svg"
        viewBox="0 0 200 200"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
      >
        <defs>
          <radialGradient id="radar-glow-grad" cx="50%" cy="50%" r="50%">
            <stop offset="0%" stopColor="#0ea5e9" stopOpacity="0.25" />
            <stop offset="70%" stopColor="#0ea5e9" stopOpacity="0.05" />
            <stop offset="100%" stopColor="#0ea5e9" stopOpacity="0" />
          </radialGradient>
          <linearGradient id="radar-sweep-grad" x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" stopColor="#38bdf8" stopOpacity="0.6" />
            <stop offset="50%" stopColor="#38bdf8" stopOpacity="0.15" />
            <stop offset="100%" stopColor="#38bdf8" stopOpacity="0" />
          </linearGradient>
        </defs>

        {/* 底层雷达淡蓝微光 */}
        <circle cx="100" cy="100" r="90" fill="url(#radar-glow-grad)" />

        {/* 同心距离圈 */}
        <circle cx="100" cy="100" r="90" stroke="rgba(14, 165, 233, 0.3)" strokeWidth="1.2" />
        <circle cx="100" cy="100" r="66" stroke="rgba(14, 165, 233, 0.25)" strokeWidth="1" strokeDasharray="3 3" />
        <circle cx="100" cy="100" r="42" stroke="rgba(14, 165, 233, 0.25)" strokeWidth="1" />
        <circle cx="100" cy="100" r="18" stroke="rgba(14, 165, 233, 0.3)" strokeWidth="1.2" />

        {/* 十字方位基准线 */}
        <line x1="10" y1="100" x2="190" y2="100" stroke="rgba(14, 165, 233, 0.2)" strokeWidth="1" />
        <line x1="100" y1="10" x2="100" y2="190" stroke="rgba(14, 165, 233, 0.2)" strokeWidth="1" />
        {/* 对角斜线 */}
        <line x1="36" y1="36" x2="164" y2="164" stroke="rgba(14, 165, 233, 0.12)" strokeWidth="0.8" strokeDasharray="2 4" />
        <line x1="164" y1="36" x2="36" y2="164" stroke="rgba(14, 165, 233, 0.12)" strokeWidth="0.8" strokeDasharray="2 4" />

        {/* 旋转扫描扇区 (CSS Keyframes 驱动) */}
        {scanning ? (
          <g className="radar-sweep-beam">
            <path
              d="M 100 100 L 190 100 A 90 90 0 0 0 164 36 Z"
              fill="url(#radar-sweep-grad)"
            />
            <line x1="100" y1="100" x2="190" y2="100" stroke="#38bdf8" strokeWidth="2" strokeLinecap="round" />
          </g>
        ) : null}

        {/* 探测到的小区目标脉冲点 */}
        <g className="radar-blips">
          <circle className="radar-blip radar-blip--1" cx="135" cy="65" r="3" fill="#10b981" />
          <circle className="radar-blip-ring radar-blip-ring--1" cx="135" cy="65" r="6" stroke="#10b981" strokeWidth="1" />
          {cellCount > 1 ? (
            <>
              <circle className="radar-blip radar-blip--2" cx="72" cy="130" r="2.6" fill="#38bdf8" />
              <circle className="radar-blip-ring radar-blip-ring--2" cx="72" cy="130" r="5" stroke="#38bdf8" strokeWidth="1" />
            </>
          ) : null}
          {cellCount > 3 ? (
            <circle className="radar-blip radar-blip--3" cx="150" cy="120" r="2.8" fill="#f59e0b" />
          ) : null}
        </g>

        {/* 核心雷达探针圆心 */}
        <circle cx="100" cy="100" r="3.5" fill="#38bdf8" stroke="#ffffff" strokeWidth="1.5" />
      </svg>
    </div>
  );
};
