/**
 * Single source of truth for the responsive breakpoints.
 *
 * The stylesheet and the components used to disagree: CSS only ever reacted at
 * 960px and 767px, while JS added 640px and 520px on its own. The 641-960px
 * band therefore had component-level JS fallbacks (table -> card list) with no
 * matching CSS, and the 521-640px band had neither.
 *
 * Three mobile-first tiers, exactly as agreed:
 *   >= 1024px  desktop  — multi-column grids
 *   768-1023px tablet   — two columns
 *   <  768px  mobile    — single column, tables collapse to stacked rows
 *
 * The 480px tier is kept for the few places that genuinely need a phone-portrait
 * adjustment (e.g. the upgrade Steps indent), not as a general tier.
 *
 * The values are exported both as raw numbers and as ready-made media query
 * strings; use the strings in `useMediaQuery` and mirror the numbers in
 * `global.css` (CSS custom media cannot be shared across the two languages
 * without a build-time plugin, so the comment here is the contract).
 */
export const BREAKPOINTS = {
  /** Tablet ceiling: below this the sider collapses into a drawer. */
  mobile: 768,
  /** Phone-portrait adjustments. */
  compact: 480,
  /** Legacy tablet edge kept so the desktop grid can shed a column. */
  tablet: 1024,
} as const;

/** `true` when the viewport is narrower than the mobile tier. */
export const QUERY_MOBILE = `(max-width: ${BREAKPOINTS.mobile - 1}px)`;

/** `true` on phone-sized viewports only. */
export const QUERY_COMPACT = `(max-width: ${BREAKPOINTS.compact - 1}px)`;

/** `true` below the desktop tier, i.e. tablet portrait and phones. */
export const QUERY_TABLET_DOWN = `(max-width: ${BREAKPOINTS.tablet - 1}px)`;
