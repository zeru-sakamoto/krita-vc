import { useEffect, useRef, useState } from "react";
import { useTour } from "../../lib/tour";
import type { ActivityView } from "./ActivityBar";

const HOLD_MS = 300;
const RING_R = 9;
const RING_CIRC = 2 * Math.PI * RING_R;

/**
 * Full-launch spotlight tour. Renders `null` when inactive. The dim-with-a-hole
 * effect is four plain opaque bands tiling the viewport around the target rect
 * (top/bottom/left/right) — deliberately not a box-shadow spread or an SVG mask,
 * both of which turned out to silently fail to paint in this WebView build.
 * A fifth, transparent div sits over the hole itself so it stays non-interactive
 * without visually darkening it.
 */
export function TourOverlay({ setActiveView }: { setActiveView: (v: ActivityView) => void }) {
  const { active, step, stepIndex, totalSteps, next, back, skip } = useTour();
  const [rect, setRect] = useState<DOMRect | null>(null);
  const [inMenu, setInMenu] = useState(false);

  useEffect(() => {
    if (step.view) setActiveView(step.view);
  }, [step.view, setActiveView]);

  useEffect(() => {
    if (!active) return;
    const measure = () => {
      const el = document.querySelector(`[data-tour-id="${step.tourId}"]`);
      setRect(el ? el.getBoundingClientRect() : null);
      // Spotlighting a row inside an open dropdown (e.g. panel options) — put the
      // callout beside it instead of below, so it doesn't cover the rest of the list.
      setInMenu(!!el?.closest('[role="menu"]'));
    };
    measure();
    // Layout may still be settling right after a view switch.
    const raf = requestAnimationFrame(measure);
    window.addEventListener("resize", measure);
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("resize", measure);
    };
  }, [active, step.tourId]);

  useEffect(() => {
    if (!active) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "ArrowRight") {
        e.preventDefault();
        next();
      } else if (e.key === "ArrowLeft") {
        e.preventDefault();
        back();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [active, next, back]);

  if (!active || !rect) return null;

  const pad = 6;
  // Rounded to whole pixels so all four bands (and the hole) agree on the exact same integer
  // boundary — getBoundingClientRect() returns raw floats, and the four bands are independently
  // positioned `fixed` siblings, so leaving them as floats risked a hairline sub-pixel seam
  // between adjoining bands.
  const holeX = Math.round(rect.left - pad);
  const holeY = Math.round(rect.top - pad);
  const holeW = Math.round(rect.width + pad * 2);
  const holeH = Math.round(rect.height + pad * 2);
  const vw = window.innerWidth;
  const vh = window.innerHeight;

  const dim = "rgba(0,0,0,0.6)";
  const bandStyle = (s: React.CSSProperties): React.CSSProperties => ({
    position: "fixed",
    background: dim,
    ...s,
  });

  // ActivityBar targets sit in the left rail, below the TopBar — put the callout to
  // their right, same as an open-dropdown row (`inMenu`), which would otherwise sit
  // right on top of the rest of the list. Anchor to whichever edge of the target has
  // room to grow into (top half of the screen: align top and grow down; bottom half:
  // align bottom and grow up) rather than centering, so a card near the bottom
  // (Settings, Backup) never clips past the window edge. Anything else (the TopBar
  // switcher itself sits near the same left edge) gets the callout below instead.
  const inActivityBar = rect.left < 64 && rect.top > 40;
  const cardWidth = 288; // matches the callout's `w-72` className below
  const calloutStyle: React.CSSProperties =
    inActivityBar || inMenu
      ? rect.top < vh / 2
        ? { left: rect.right + 16, top: rect.top }
        : { left: rect.right + 16, bottom: vh - rect.bottom }
      : // Clamp so a target near the right edge (e.g. the Inspector toggle) doesn't push
        // the card past the window edge.
        { left: Math.min(rect.left, vw - cardWidth - 16), top: rect.bottom + 12 };

  return (
    <div className="fixed inset-0 z-(--z-tour)">
      {/* pointer-events: none so the custom title bar stays draggable/clickable through the dim */}
      <div
        style={bandStyle({
          left: 0,
          top: 0,
          width: vw,
          height: Math.max(holeY, 0),
          pointerEvents: "none",
        })}
      />
      <div
        style={bandStyle({
          left: 0,
          top: holeY + holeH,
          width: vw,
          height: Math.max(vh - (holeY + holeH), 0),
        })}
      />
      <div style={bandStyle({ left: 0, top: holeY, width: Math.max(holeX, 0), height: holeH })} />
      <div
        style={bandStyle({
          left: holeX + holeW,
          top: holeY,
          width: Math.max(vw - (holeX + holeW), 0),
          height: holeH,
        })}
      />
      <div
        className="fixed"
        style={{ left: holeX, top: holeY, width: holeW, height: holeH }}
        onPointerDown={(e) => e.preventDefault()}
        onClick={(e) => e.preventDefault()}
      />

      <div
        style={calloutStyle}
        className="fixed w-72 rounded-panel border border-border bg-surface-2 p-3 shadow-(--shadow-float)"
      >
        <h2 className="text-[13px] font-medium text-text">{step.title}</h2>
        <p className="mt-1 text-[12px] leading-relaxed text-text-muted">{step.body}</p>
        <div className="mt-3 flex items-center justify-between">
          <span className="text-[11px] text-text-muted">
            Step {stepIndex + 1} of {totalSteps}
          </span>
          <div className="flex gap-1.5">
            {stepIndex > 0 && (
              <button
                type="button"
                onClick={back}
                className="rounded-button px-2 py-1 text-[12px] text-text-muted hover:bg-white/5 hover:text-text"
              >
                Back
              </button>
            )}
            <button
              type="button"
              onClick={next}
              className="rounded-button bg-accent px-2.5 py-1 text-[12px] font-medium text-white hover:bg-accent/90"
            >
              {stepIndex + 1 >= totalSteps ? "Done" : "Next"}
            </button>
          </div>
        </div>
      </div>

      <HoldToSkip onSkip={skip} />
    </div>
  );
}

function HoldToSkip({ onSkip }: { onSkip: () => void }) {
  const [holding, setHolding] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const startHold = () => {
    setHolding(true);
    timer.current = setTimeout(onSkip, HOLD_MS);
  };
  const cancelHold = () => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = null;
    setHolding(false);
  };

  return (
    <button
      type="button"
      onPointerDown={startHold}
      onPointerUp={cancelHold}
      onPointerLeave={cancelHold}
      title="Press and hold to skip the tour"
      className="fixed bottom-4 right-4 flex items-center gap-1.5 rounded-button border border-border bg-surface-2 px-2.5 py-1.5 text-[12px] text-text-muted hover:text-text"
    >
      <svg width="20" height="20" viewBox="0 0 20 20" className="-rotate-90">
        <circle
          cx="10"
          cy="10"
          r={RING_R}
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          opacity="0.25"
        />
        <circle
          cx="10"
          cy="10"
          r={RING_R}
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeDasharray={RING_CIRC}
          strokeDashoffset={holding ? 0 : RING_CIRC}
          style={{
            transition: holding
              ? `stroke-dashoffset ${HOLD_MS}ms linear`
              : "stroke-dashoffset 150ms ease-out",
          }}
        />
      </svg>
      Skip
    </button>
  );
}
