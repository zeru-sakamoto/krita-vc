import { Check } from "@phosphor-icons/react";
import type { InputHTMLAttributes } from "react";

interface CheckboxProps extends Omit<InputHTMLAttributes<HTMLInputElement>, "type" | "size"> {
  checked: boolean;
}

/**
 * Tactile checkbox: a carved-in well like any other input, with an accent check
 * mark when ticked — depth first, color second (DESIGN.md), never a filled
 * accent background. The native input stays functional (keyboard, label
 * association) but visually transparent; wrap in a `<label>` for click/tap area.
 */
export function Checkbox({ checked, disabled, className = "", ...rest }: CheckboxProps) {
  return (
    <span
      className={[
        "relative inline-grid size-4 shrink-0 place-items-center rounded-badge",
        "inset-well border border-border bg-bg",
        "has-[:focus-visible]:outline has-[:focus-visible]:outline-2",
        "has-[:focus-visible]:outline-accent/50 has-[:focus-visible]:outline-offset-2",
        disabled ? "opacity-40" : "",
        className,
      ].join(" ")}
    >
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        className="absolute inset-0 size-full cursor-pointer opacity-0 disabled:cursor-not-allowed"
        {...rest}
      />
      {checked && <Check size={11} weight="bold" className="text-accent" aria-hidden />}
    </span>
  );
}
