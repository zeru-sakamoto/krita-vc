import type { Icon } from "@phosphor-icons/react";

interface SwitchProps {
  /** Accessible label — also shown as the tooltip */
  label: string;
  active: boolean;
  onClick: () => void;
  disabled?: boolean;
  /** Optional visible text next to the track (icon-only when omitted) */
  text?: string;
  icon?: Icon;
}

/** Compact track+thumb toggle for toolbars — the on/off sibling of IconButton. */
export function Switch({
  label,
  active,
  onClick,
  disabled = false,
  text,
  icon: IconCmp,
}: SwitchProps) {
  return (
    <button
      type="button"
      role="switch"
      title={label}
      aria-label={label}
      aria-checked={active}
      disabled={disabled}
      onClick={onClick}
      className={[
        "flex shrink-0 items-center gap-1.5 rounded-button px-1 py-1",
        "transition-colors duration-100 ease-out hover:bg-surface-3",
        "disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent",
      ].join(" ")}
    >
      {IconCmp && <IconCmp size={14} weight="regular" className="shrink-0 text-text-muted" />}
      {text && <span className="text-[11px] text-text-muted">{text}</span>}
      <span
        aria-hidden
        className={[
          "inset-well relative h-[18px] w-8 shrink-0 rounded-full transition-colors duration-200",
          active ? "bg-accent" : "bg-surface-3",
        ].join(" ")}
      >
        <span
          className={[
            "raised absolute left-0.5 top-1/2 size-3.5 -translate-y-1/2 rounded-full bg-text transition-transform duration-200 ease-out",
            active ? "translate-x-[14px]" : "translate-x-0",
          ].join(" ")}
        />
      </span>
    </button>
  );
}
