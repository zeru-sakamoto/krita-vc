import { useCallback, useEffect, useRef, useState } from "react";
import { EXIT_MS } from "../../lib/useExitTransition";

interface ModalProps {
  title: string;
  onClose: () => void;
  children: React.ReactNode;
  /**
   * Action row pinned to the bottom (e.g. Cancel / Confirm buttons).
   *
   * Prefer the function form: it hands you a `close` that plays the exit transition
   * before calling `onClose`. A dismiss button wired straight to the parent's own
   * `onClose` unmounts the dialog instantly and skips the animation — see the note on
   * `requestClose` below for why the component can't intercept that itself.
   */
  footer?: React.ReactNode | ((close: () => void) => React.ReactNode);
  /** Panel width class. Defaults to the compact dialog width most modals want. */
  maxWidthClassName?: string;
}

/**
 * Minimal themed modal: backdrop + centered surface-2 panel. Closes on Esc or
 * backdrop click. Mirrors Menu's outside-click/Esc pattern — no new dependency.
 */
export function Modal({
  title,
  onClose,
  children,
  footer,
  maxWidthClassName = "max-w-md",
}: ModalProps) {
  // Known coarseness: this panel's backdrop-filter sits inside `.scrim`'s own, and a
  // nested backdrop chain re-rasterizes every frame an opacity in it changes — so the
  // dialog's fade resolves in ~3 steps where Menu's (one backdrop, not nested) is smooth.
  // `will-change` on either layer was measured and does not help. The fix, if it ever
  // matters, is to stop blurring the scrim while the panel animates — not another
  // compositor hint.
  const [visible, setVisible] = useState(false);
  const closing = useRef(false);
  const panelRef = useRef<HTMLDivElement>(null);

  /**
   * Start the exit. The unmount itself is driven by the effect below.
   *
   * Unlike Menu and Tooltip — which own their open state and can therefore use
   * `useExitTransition` — a Modal is mounted by its parent (`{show && <Modal/>}`), so it
   * cannot defer its own unmount. It inverts the problem instead: hold `onClose` back
   * until the transition has run. This covers every close path the Modal itself owns
   * (Esc, backdrop) plus any footer built with the render-prop form.
   */
  const requestClose = useCallback(() => {
    if (closing.current) return; // Esc during the exit must not queue a second onClose
    closing.current = true;
    setVisible(false);
  }, []);

  /**
   * Unmount when the fade actually finishes, not on a fixed timer.
   *
   * A timer was the obvious approach and it does not work here. On a large dialog the
   * re-render plus the repaint of the nested backdrop-filter chain (`.scrim`'s blur under
   * the panel's own) delays the transition's *start* by ~85ms — measured — so a timer set
   * to the exit duration fires while the fade is still running and `transitioncancel`s it
   * partway, which reads as the dialog snapping out. Waiting for `transitionend` is
   * correct regardless of how late the transition starts or how big the dialog is.
   *
   * The timer survives only as a fallback: `transitionend` never fires if the transition
   * is suppressed entirely (reduced motion collapsing it, a hidden tab, a browser that
   * drops it), and the dialog must still close.
   */
  useEffect(() => {
    if (!closing.current || visible) return;
    const el = panelRef.current;
    let done = false;
    const finish = () => {
      if (done) return;
      done = true;
      onClose();
    };
    const onEnd = (e: TransitionEvent) => {
      if (e.target === el && e.propertyName === "opacity") finish();
    };
    el?.addEventListener("transitionend", onEnd);
    const fallback = setTimeout(finish, EXIT_MS * 4);
    return () => {
      el?.removeEventListener("transitionend", onEnd);
      clearTimeout(fallback);
    };
  }, [visible, onClose]);

  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") requestClose();
    };
    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [requestClose]);

  useEffect(() => {
    // Next frame so the transitions below have a "from" state to animate out of,
    // rather than committing the visible state on first paint and skipping it.
    // (Same idiom as Tooltip.tsx.)
    const raf = requestAnimationFrame(() => setVisible(true));
    return () => cancelAnimationFrame(raf);
  }, []);

  return (
    <div
      className={[
        "scrim fixed inset-0 z-(--z-modal) grid place-items-center p-4",
        // Enter takes --dur-slow; the exit is quicker and must finish within EXIT_MS,
        // or the unmount timer cuts the fade off partway (it visibly snaps).
        "transition-opacity ease-(--ease-out)",
        visible ? "duration-(--dur-slow) opacity-100" : "duration-(--dur-fast) opacity-0",
      ].join(" ")}
      onPointerDown={requestClose}
    >
      <div
        ref={panelRef}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onPointerDown={(e) => e.stopPropagation()}
        className={[
          `glass flex w-full ${maxWidthClassName} max-h-[calc(100vh-2rem)] flex-col overflow-hidden rounded-modal shadow-(--shadow-modal)`,
          "transition-[opacity,scale] ease-(--ease-out)",
          visible
            ? "duration-(--dur-slow) scale-100 opacity-100"
            : "pointer-events-none duration-(--dur-fast) scale-[0.97] opacity-0",
        ].join(" ")}
      >
        <h2 className="shrink-0 border-b border-border px-4 py-3 text-heading font-medium text-text">
          {title}
        </h2>
        <div className="min-h-0 flex-1 overflow-auto px-4 py-4">{children}</div>
        {footer && (
          <div className="flex shrink-0 justify-end gap-2 border-t border-border px-4 py-3">
            {typeof footer === "function" ? footer(requestClose) : footer}
          </div>
        )}
      </div>
    </div>
  );
}
