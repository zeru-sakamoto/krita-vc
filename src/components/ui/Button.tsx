import type { ButtonHTMLAttributes, ReactNode } from "react";

type Variant = "default" | "primary" | "destructive";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  children: ReactNode;
}

const VARIANTS: Record<Variant, string> = {
  // Raised surface button — dialog confirmations, never icon-only.
  default: "bg-surface-3 text-text hover:bg-state-hover",
  // One dominant accent action per view (DESIGN.md).
  primary: "bg-accent text-bg hover:brightness-110",
  // Destructive — reveals danger treatment on hover.
  destructive: "bg-surface-3 text-text hover:bg-danger/15 hover:text-danger",
};

/**
 * Raised text action button (OK / Cancel / Commit / Discard).
 * `.tactile` carries the depth: raised at rest, sinks on press. It also owns
 * the :active box-shadow, so don't add a shadow-* utility here — an unlayered
 * rule beats Tailwind's utilities layer and the two would fight.
 * (DESIGN.md → Tool Button → Text / Destructive button)
 */
export function Button({ variant = "default", children, className = "", ...rest }: ButtonProps) {
  return (
    <button
      type="button"
      className={[
        "tactile inline-flex h-7 items-center justify-center gap-1.5 rounded-button px-3",
        "text-[13px] font-medium",
        "transition-[transform,background-color,box-shadow,filter] duration-100 ease-out",
        "active:translate-y-px",
        "disabled:cursor-not-allowed disabled:opacity-40",
        VARIANTS[variant],
        className,
      ].join(" ")}
      {...rest}
    >
      {children}
    </button>
  );
}
