import {
  cloneElement,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactElement,
} from "react";
import { createPortal } from "react-dom";

const SHOW_DELAY_MS = 400;
const GAP = 6;

interface TooltipProps {
  label: string;
  children: ReactElement<{
    onMouseEnter?: (e: React.MouseEvent) => void;
    onMouseLeave?: (e: React.MouseEvent) => void;
    onFocus?: (e: React.FocusEvent) => void;
    onBlur?: (e: React.FocusEvent) => void;
  }>;
  disabled?: boolean;
}

/**
 * Hover/focus tooltip: portals a small themed popover near the wrapped element.
 * Positioning mirrors Menu.tsx's measure-then-place approach (portal to body so
 * it isn't clipped by a scrollable ancestor); placement flips above/below and
 * clamps horizontally so it can't run off the viewport near a screen edge.
 */
export function Tooltip({ label, children, disabled }: TooltipProps) {
  const [open, setOpen] = useState(false);
  const [visible, setVisible] = useState(false);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  const anchorRef = useRef<HTMLElement | null>(null);
  const popoverRef = useRef<HTMLDivElement>(null);
  const showTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const show = () => {
    if (disabled || !label) return;
    clearTimeout(showTimer.current);
    showTimer.current = setTimeout(() => setOpen(true), SHOW_DELAY_MS);
  };
  const hide = () => {
    clearTimeout(showTimer.current);
    setOpen(false);
    setVisible(false);
    setPos(null);
  };
  useEffect(() => () => clearTimeout(showTimer.current), []);

  useLayoutEffect(() => {
    if (!open || !anchorRef.current || !popoverRef.current) return;
    const anchor = anchorRef.current.getBoundingClientRect();
    const tip = popoverRef.current.getBoundingClientRect();
    const top =
      anchor.top - tip.height - GAP < 8 ? anchor.bottom + GAP : anchor.top - tip.height - GAP;
    const left = Math.min(
      Math.max(anchor.left + anchor.width / 2 - tip.width / 2, 8),
      window.innerWidth - tip.width - 8
    );
    setPos({ top, left });
    // Next frame so the opacity/scale transition below has a "from" state to animate out of,
    // rather than committing the visible state in the same layout pass and skipping it.
    const raf = requestAnimationFrame(() => setVisible(true));
    return () => cancelAnimationFrame(raf);
  }, [open, label]);

  // Native DOM elements only (button/span/div) — none carry an existing ref at their
  // call sites, so this doesn't need to merge with one.
  const child = cloneElement(children, {
    ref: (node: HTMLElement | null) => {
      anchorRef.current = node;
    },
    onMouseEnter: (e: React.MouseEvent) => {
      children.props.onMouseEnter?.(e);
      show();
    },
    onMouseLeave: (e: React.MouseEvent) => {
      children.props.onMouseLeave?.(e);
      hide();
    },
    onFocus: (e: React.FocusEvent) => {
      children.props.onFocus?.(e);
      show();
    },
    onBlur: (e: React.FocusEvent) => {
      children.props.onBlur?.(e);
      hide();
    },
  } as Partial<unknown>);

  return (
    <>
      {child}
      {open &&
        createPortal(
          <div
            ref={popoverRef}
            role="tooltip"
            style={{
              position: "fixed",
              top: pos?.top ?? -9999,
              left: pos?.left ?? -9999,
              visibility: pos ? "visible" : "hidden",
            }}
            className={[
              "pointer-events-none z-(--z-tooltip) max-w-[280px] whitespace-normal break-words",
              "rounded-badge bg-surface-3 px-2 py-1 text-xs text-text shadow-(--shadow-float)",
              "transition-[opacity,transform] duration-(--dur-fast) ease-(--ease-out)",
              visible ? "scale-100 opacity-100" : "scale-[0.97] opacity-0",
            ].join(" ")}
          >
            {label}
          </div>,
          document.body
        )}
    </>
  );
}
