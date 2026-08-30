import type { Icon } from "@phosphor-icons/react";
import { ICON } from "../../lib/iconSize";
import { Tooltip } from "./Tooltip";

interface IconButtonProps {
  icon: Icon;
  /** Tooltip + accessible label (no visible text on flat icon buttons) */
  label: string;
  /** Toggled / checked tool state */
  active?: boolean;
  /**
   * Icon px size — use a step from `ICON` (`src/lib/iconSize.ts`); defaults to
   * `ICON.default` (16).
   * Still typed `number` rather than `IconSize` because call sites pass
   * off-scale literals (18, 13, …); tightening it is Wave 3's job, once those
   * are migrated.
   */
  size?: number;
  disabled?: boolean;
  /** Spin the icon (in-progress feedback, e.g. a rescan). */
  spinning?: boolean;
  onClick?: () => void;
  /** Tour spotlight hook (`data-tour-id`) — see `src/lib/tour.tsx`. */
  tourId?: string;
  /** Extra classes on the icon itself, e.g. to mirror a left-facing glyph. */
  iconClassName?: string;
}

/**
 * Raised icon chip: sits proud of its surface, sinks when held.
 * Toggled-on is the *pressed* look rather than a tint — a switch that stays
 * held down reads as on without needing color to say it.
 * Note this deliberately replaces Krita's flat no-chrome-until-hover button;
 * see DESIGN.md → Krita Design Influence for why that was reversed.
 * The 32×32 chip is from the Tool Button table; the 16px default glyph is from
 * § Icon System, which supersedes that table's stale "Icon: 20px centered" —
 * 20 is no longer a step on the scale.
 * (DESIGN.md → VCS Component Patterns → Tool Button → Icon chip; → Icon System)
 */
export function IconButton({
  icon: IconCmp,
  label,
  active = false,
  size = ICON.default,
  disabled = false,
  spinning = false,
  onClick,
  tourId,
  iconClassName,
}: IconButtonProps) {
  return (
    // Not gated on `disabled` — the button's native `disabled` already blocks clicks;
    // the tooltip should still explain what a greyed-out action is, same as the old
    // native `title=` did regardless of disabled state.
    <Tooltip label={label}>
      <button
        type="button"
        aria-label={label}
        aria-pressed={active}
        disabled={disabled}
        onClick={onClick}
        data-tour-id={tourId}
        data-pressed={active ? "true" : undefined}
        className={[
          "tactile grid h-8 w-8 place-items-center rounded-button bg-surface-2",
          "transition-[translate,background-color,box-shadow,color] duration-(--dur-instant) ease-(--ease-out)",
          "hover:bg-surface-3 hover:text-text active:translate-y-px",
          "disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-surface-2",
          active ? "text-accent" : "text-text-muted",
        ].join(" ")}
      >
        <IconCmp
          size={size}
          weight="regular"
          className={
            [spinning ? "animate-spin" : "", iconClassName ?? ""].filter(Boolean).join(" ") ||
            undefined
          }
        />
      </button>
    </Tooltip>
  );
}
