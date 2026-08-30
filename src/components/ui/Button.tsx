import type { ButtonHTMLAttributes, ReactNode } from "react";

type Variant = "default" | "primary" | "destructive" | "ghost";
type Size = "sm" | "md";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  /** `md` (default) is the dialog-confirmation metric; `sm` is for dense inline rows. */
  size?: Size;
  children: ReactNode;
}

const VARIANTS: Record<Variant, string> = {
  // Raised surface button — dialog confirmations, never icon-only.
  default: "bg-surface-3 text-text hover:bg-state-hover",
  // One dominant accent action per view (DESIGN.md).
  primary: "bg-accent text-bg hover:brightness-110",
  // Destructive — reveals danger treatment on hover.
  destructive: "bg-surface-3 text-text hover:bg-danger/15 hover:text-danger",
  // Ghost — no background, no elevation. The recessive half of a button pair
  // (e.g. "Back" beside "Next"), where two raised chips would read as equals.
  ghost: "text-text-muted hover:bg-state-hover hover:text-text",
};

const SIZES: Record<Size, string> = {
  // Dense inline actions — matches the 12px/py-1 metric the app's hand-rolled
  // small buttons already use, so folding them in is not a visual change.
  sm: "px-2.5 py-1 text-dense",
  md: "h-7 px-3 text-body",
};

/**
 * Raised text action button (OK / Cancel / Commit / Discard).
 * `.tactile` carries the depth: raised at rest, sinks on press. It also owns
 * the :active box-shadow, so don't add a shadow-* utility here — an unlayered
 * rule beats Tailwind's utilities layer and the two would fight. For the same
 * reason `ghost` (which has no elevation by design) *omits* the class rather
 * than trying to cancel it with `shadow-none`.
 * (DESIGN.md → Tool Button → Text / Destructive button; Motion System → Button `:active`)
 */
export function Button({
  variant = "default",
  size = "md",
  children,
  className = "",
  ...rest
}: ButtonProps) {
  return (
    <button
      type="button"
      className={[
        variant === "ghost" ? "" : "tactile",
        "inline-flex items-center justify-center gap-1.5 rounded-button",
        "font-medium",
        SIZES[size],
        // Tailwind v4 compiles `translate-y-px` to the standalone `translate` CSS
        // property, not `transform` — transitioning `transform` here animates nothing
        // and the press sink snaps. Verified in the built CSS. Don't change it back.
        "transition-[translate,background-color,box-shadow,filter] duration-(--dur-instant) ease-(--ease-out)",
        "active:translate-y-px",
        "disabled:cursor-not-allowed disabled:opacity-40",
        VARIANTS[variant],
        className,
      ]
        .filter(Boolean)
        .join(" ")}
      {...rest}
    >
      {children}
    </button>
  );
}
