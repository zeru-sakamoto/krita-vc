import type { Icon } from "@phosphor-icons/react";

interface IconButtonProps {
  icon: Icon;
  /** Tooltip + accessible label (no visible text on flat icon buttons) */
  label: string;
  /** Toggled / checked tool state */
  active?: boolean;
  /** Icon px size — 20 default, 16 dense (docker headers), 24 toolbar */
  size?: number;
  disabled?: boolean;
  /** Spin the icon (in-progress feedback, e.g. a rescan). */
  spinning?: boolean;
  onClick?: () => void;
}

/**
 * Flat Krita-style icon button: borderless, no chrome until hover.
 * (DESIGN.md → VCS Component Patterns → Tool Button → Flat icon button)
 */
export function IconButton({
  icon: IconCmp,
  label,
  active = false,
  size = 20,
  disabled = false,
  spinning = false,
  onClick,
}: IconButtonProps) {
  return (
    <button
      type="button"
      title={label}
      aria-label={label}
      aria-pressed={active}
      disabled={disabled}
      onClick={onClick}
      className={[
        "grid h-8 w-8 place-items-center rounded-button text-text-muted",
        "transition-[transform,background-color] duration-100 ease-out",
        "hover:bg-white/5 hover:text-text active:scale-[0.97] active:bg-white/8",
        "disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent",
        active ? "bg-accent/12 text-text" : "",
      ].join(" ")}
    >
      <IconCmp size={size} weight="regular" className={spinning ? "animate-spin" : undefined} />
    </button>
  );
}
