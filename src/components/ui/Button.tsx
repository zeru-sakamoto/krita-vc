import type { ButtonHTMLAttributes, ReactNode } from "react";

type Variant = "default" | "primary" | "destructive";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  children: ReactNode;
}

const VARIANTS: Record<Variant, string> = {
  // Bordered surface button — dialog confirmations, never icon-only.
  default: "bg-surface-3 border border-border text-text hover:bg-white/5 active:bg-white/[0.08]",
  // One dominant accent action per view (DESIGN.md).
  primary: "bg-accent border border-transparent text-bg hover:brightness-110 active:brightness-95",
  // Destructive — reveals danger treatment on hover.
  destructive:
    "bg-surface-3 border border-border text-text hover:bg-danger/15 hover:border-danger hover:text-danger",
};

/**
 * Bordered text action button (OK / Cancel / Commit / Discard).
 * (DESIGN.md → Tool Button → Text / Destructive button)
 */
export function Button({ variant = "default", children, className = "", ...rest }: ButtonProps) {
  return (
    <button
      type="button"
      className={[
        "inline-flex h-7 items-center justify-center gap-1.5 rounded-button px-3",
        "text-[13px] font-medium",
        "transition-[transform,background-color,border-color,filter] duration-100 ease-out",
        "active:scale-[0.97]",
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
