import type { InputHTMLAttributes } from "react";

interface RadioProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "type" | "size"> {
  checked: boolean;
  /** Dot color when selected — "danger" for a destructive choice (e.g. delete folder). */
  tone?: "accent" | "danger";
}

/**
 * Tactile radio button: a carved-in well matching Checkbox, with a dot when
 * selected. The native input stays functional but visually transparent; wrap
 * in a `<label>` for click/tap area.
 */
export function Radio({ checked, disabled, tone = "accent", className = "", ...rest }: RadioProps) {
  return (
    <span
      className={[
        "relative inline-grid size-4 shrink-0 place-items-center rounded-full",
        "inset-well border border-border bg-bg",
        "has-[:focus-visible]:outline has-[:focus-visible]:outline-2",
        "has-[:focus-visible]:outline-accent/50 has-[:focus-visible]:outline-offset-2",
        disabled ? "opacity-40" : "",
        className,
      ].join(" ")}
    >
      <input
        type="radio"
        checked={checked}
        disabled={disabled}
        className="absolute inset-0 size-full cursor-pointer opacity-0 disabled:cursor-not-allowed"
        {...rest}
      />
      {checked && (
        <span
          aria-hidden
          className={["size-1.5 rounded-full", tone === "danger" ? "bg-danger" : "bg-accent"].join(
            " "
          )}
        />
      )}
    </span>
  );
}
