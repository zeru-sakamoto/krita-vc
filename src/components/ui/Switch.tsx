import type { Icon } from "@phosphor-icons/react";
import { ICON } from "../../lib/iconSize";
import { Tooltip } from "./Tooltip";

export type SwitchSize = "sm" | "md";

/**
 * Track / thumb / travel per size. Two sizes cover every switch in the app:
 * `sm` for toolbar chips, `md` for settings rows. The old one-off 18x32 is gone.
 *
 * The thumb rests 2px (`left-0.5`) inside the track, so travel is
 * `trackWidth - thumbSize - 4` and the thumb ends 2px from the far edge —
 * symmetric in both states. Change one column and the other two must follow.
 */
const SIZES: Record<SwitchSize, { track: string; thumb: string; travel: string }> = {
  // 16 x 28 track, 12px thumb, 12px travel
  sm: { track: "h-4 w-7", thumb: "size-3", travel: "translate-x-3" },
  // 20 x 36 track, 16px thumb, 16px travel
  md: { track: "h-5 w-9", thumb: "size-4", travel: "translate-x-4" },
};

/**
 * The track+thumb itself. The track is **recessed in both states** and never
 * fills with the accent — the housing doesn't move, so it can't carry state.
 * The thumb does: it slides, and it goes `bg-accent` when on. See DESIGN.md
 * § Toggle / Switch and § Notes, "Depth before color".
 *
 * The off-state thumb stays `bg-text` rather than the spec's `--surface`
 * alternative: on the two light themes `--surface` is #ffffff against a
 * `surface-3` of #ddd6c8 / #e2e5ee, i.e. ~1.4:1, and `--edge-light` is
 * transparent there so the `.raised` shadow is all that separates them.
 * `--text` clears 11:1 on light and dark alike.
 */
function SwitchTrack({ active, size }: { active: boolean; size: SwitchSize }) {
  const s = SIZES[size];
  return (
    <span
      aria-hidden
      className={["inset-well relative shrink-0 rounded-full bg-surface-3", s.track].join(" ")}
    >
      <span
        className={[
          "raised absolute left-0.5 top-1/2 -translate-y-1/2 rounded-full",
          // `translate`, not `transform`: Tailwind v4's translate-* utilities set the
          // standalone `translate` property, so `transition-[transform,...]` would
          // silently not animate the thumb at all.
          "transition-[translate,background-color] duration-(--dur-fast) ease-(--ease-out)",
          s.thumb,
          active ? `${s.travel} bg-accent` : "translate-x-0 bg-text",
        ].join(" ")}
      />
    </span>
  );
}

interface SwitchBaseProps {
  active: boolean;
  /** Track/thumb scale. `sm` for toolbar chips, `md` (default) for settings rows. */
  size?: SwitchSize;
}

interface SwitchButtonProps extends SwitchBaseProps {
  asIndicator?: false;
  /** Accessible label — also shown as the tooltip */
  label: string;
  onClick: () => void;
  disabled?: boolean;
  /** Optional visible text next to the track (icon-only when omitted) */
  text?: string;
  icon?: Icon;
}

interface SwitchIndicatorProps extends SwitchBaseProps {
  asIndicator: true;
  /**
   * The parent `<button>` owns interaction, ARIA and any label/icon/tooltip;
   * this variant is presentation only, so none of those may be passed here.
   */
  label?: never;
  onClick?: never;
  disabled?: never;
  text?: never;
  icon?: never;
}

export type SwitchProps = SwitchButtonProps | SwitchIndicatorProps;

/**
 * The app's one track+thumb toggle — the on/off sibling of IconButton.
 *
 * Two modes, because the same control has to work standalone and nested:
 *
 * - default: a `<button role="switch">` wrapped in a Tooltip, with optional
 *   icon and text.
 * - `asIndicator`: just the track+thumb as an `aria-hidden` `<span>`, no button
 *   and no handlers. Settings rows and the diff toolbar's "Show Diff" chip are
 *   themselves `<button role="switch">` elements wrapping a whole label+detail
 *   row, and a button cannot nest inside a button — which is why those two sites
 *   had hand-rolled copies of this markup instead of calling it.
 */
export function Switch(props: SwitchProps) {
  const { active, size = "md" } = props;

  if (props.asIndicator) {
    return <SwitchTrack active={active} size={size} />;
  }

  const { label, onClick, disabled = false, text, icon: IconCmp } = props;
  return (
    <Tooltip label={label}>
      <button
        type="button"
        role="switch"
        aria-label={label}
        aria-checked={active}
        disabled={disabled}
        onClick={onClick}
        className={[
          "flex shrink-0 items-center gap-1.5 rounded-button px-1 py-1",
          "transition-colors duration-(--dur-fast) ease-(--ease-out) hover:bg-surface-3",
          "disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent",
        ].join(" ")}
      >
        {IconCmp && (
          <IconCmp size={ICON.dense} weight="regular" className="shrink-0 text-text-muted" />
        )}
        {text && <span className="text-caption text-text-muted">{text}</span>}
        <SwitchTrack active={active} size={size} />
      </button>
    </Tooltip>
  );
}
