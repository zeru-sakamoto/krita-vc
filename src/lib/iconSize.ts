/**
 * The app's icon size scale.
 *
 * Phosphor takes its size as a React `size` prop, not as CSS, so unlike every
 * other design token in the system this one cannot live in `@theme` — a number
 * passed to a component is invisible to the stylesheet. Putting it here is what
 * makes it enforceable: a call site either imports `ICON` or it is off-scale,
 * and that is greppable.
 *
 * The five values are what the dense UI actually needs, measured rather than
 * wished for. `DESIGN.md` used to claim 16 / 20 / 24; the app shipped nine
 * distinct sizes and `20` appeared at no explicit call site at all. The spec
 * was amended to this scale rather than 90 icons being edited to match a size
 * the interface had already rejected.
 *
 * Anything not on this scale is a bug. Reach for the nearest step; if none of
 * the five is right, the layout around the icon is usually what's wrong.
 */
export const ICON = {
  inline: 12, // inside a chip, badge, or a line of text
  dense: 14, // dense rows: layer lists, status bar, menu items
  default: 16, // standard control icon — IconButton's default
  toolbar: 24, // toolbar / activity bar
  display: 32, // empty-state and error art only, never a control
} as const;

export type IconSize = (typeof ICON)[keyof typeof ICON];
