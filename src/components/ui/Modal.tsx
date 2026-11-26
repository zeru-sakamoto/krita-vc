import { useCallback, useEffect, useRef, useState } from "react";
import { EXIT_MS } from "../../lib/useExitTransition";
import { shiftLayout } from "../../lib/layoutShift";

// Duplicated from `--dur-normal` / `--ease-in-out` in `global.css` (the Web Animations API takes
// literal values), as in `layoutShift.ts`. Keep them in sync.
const RESIZE_MS = 220;
const EASE_IN_OUT = "cubic-bezier(0.77, 0, 0.175, 1)";

/**
 * Eases the panel between heights when its content changes (a Settings tab switch, a confirm
 * field appearing) instead of letting it snap.
 *
 * **This animates `height`, a sanctioned exception to DESIGN.md's transform/opacity-only rule.**
 * The transform-only ways to fake a resize were worse here: clipping the panel to the old size
 * clips its drop shadow too, and the footer has to be slid separately over content it would show
 * through (the panel is translucent glass). What the rule protects against is layout dirtied
 * *every frame under a pointer*; this runs once per content change, for 220 ms, and the panel's
 * width never changes, so no text reflows — each frame is one flex column re-sizing its body.
 *
 * Triggered by DOM mutations inside the body, not by a ResizeObserver on the panel: the
 * animation itself resizes the panel every frame, and a window resize changing the `max-h` cap
 * should track the window rather than trail it. The body scrolls normally again once the glide
 * ends; during it, `overflow: hidden` keeps a scrollbar from flashing on the way to the new size.
 */
function glideHeight(panel: HTMLElement, body: HTMLElement): () => void {
  let last = panel.offsetHeight;
  let anim: Animation | null = null;
  const reduce = window.matchMedia("(prefers-reduced-motion: reduce)");

  const settle = () => {
    anim = null;
    body.style.overflow = "";
    last = panel.offsetHeight;
  };

  const mo = new MutationObserver(() => {
    // Start from the height currently drawn — mid-glide if a second change interrupts the first.
    const from = anim ? parseFloat(getComputedStyle(panel).height) : last;
    anim?.cancel();
    anim = null;
    const to = panel.offsetHeight;
    last = to;
    if (Math.abs(to - from) < 1 || reduce.matches) {
      body.style.overflow = "";
      return;
    }
    body.style.overflow = "hidden";
    const a = panel.animate([{ height: `${from}px` }, { height: `${to}px` }], {
      duration: RESIZE_MS,
      easing: EASE_IN_OUT,
    });
    a.onfinish = settle;
    anim = a;
  });
  mo.observe(body, { childList: true, subtree: true, characterData: true });
  // Keep `last` true to the panel when it resizes without a mutation (the window changing the
  // max-height cap, fonts loading), so the next glide starts from what's actually on screen.
  const ro = new ResizeObserver(() => {
    if (!anim) last = panel.offsetHeight;
  });
  ro.observe(panel);

  return () => {
    mo.disconnect();
    ro.disconnect();
    anim?.cancel();
  };
}

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
  const bodyRef = useRef<HTMLDivElement>(null);

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
    const panel = panelRef.current;
    const body = bodyRef.current;
    if (panel && body) return glideHeight(panel, body);
  }, []);

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
        <div ref={bodyRef} className="min-h-0 flex-1 overflow-auto px-4 py-4">
          {children}
        </div>
        {footer && (
          <div
            ref={shiftLayout}
            className="flex shrink-0 justify-end gap-2 border-t border-border px-4 py-3"
          >
            {typeof footer === "function" ? footer(requestClose) : footer}
          </div>
        )}
      </div>
    </div>
  );
}
