import { useCallback, useRef, useState } from "react";
import type { ArtDiff, ArtLayer } from "../../types";
import { ArtCanvas, type HighlightMode } from "./ArtCanvas";

interface CompareSliderProps {
  diff: ArtDiff;
  layers: ArtLayer[];
  overlay?: boolean;
  highlightMode?: HighlightMode;
}

/**
 * Swipe comparison: "after" fills the frame; "before" is clipped to the left of a
 * draggable divider. Pointer-capture drag mirrors the Sidebar resize handle pattern.
 */
export function CompareSlider({ diff, layers, overlay, highlightMode }: CompareSliderProps) {
  const [pos, setPos] = useState(50); // divider position, 0..100 (% from left)
  const frameRef = useRef<HTMLDivElement>(null);
  const draggingRef = useRef(false);

  const moveTo = useCallback((clientX: number) => {
    const el = frameRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const pct = ((clientX - rect.left) / rect.width) * 100;
    setPos(Math.min(100, Math.max(0, pct)));
  }, []);

  const onPointerDown = useCallback(
    (e: React.PointerEvent) => {
      draggingRef.current = true;
      (e.target as HTMLElement).setPointerCapture(e.pointerId);
      moveTo(e.clientX);
    },
    [moveTo]
  );
  const onPointerMove = useCallback(
    (e: React.PointerEvent) => {
      if (draggingRef.current) moveTo(e.clientX);
    },
    [moveTo]
  );
  const onPointerUp = useCallback((e: React.PointerEvent) => {
    draggingRef.current = false;
    (e.target as HTMLElement).releasePointerCapture(e.pointerId);
  }, []);

  return (
    <div ref={frameRef} className="relative h-full w-full select-none">
      {/* AFTER — fills the frame */}
      <div className="absolute inset-0">
        <ArtCanvas
          diff={diff}
          layers={layers}
          state="after"
          overlay={overlay}
          highlightMode={highlightMode}
        />
      </div>

      {/* BEFORE — clipped to the left of the divider */}
      <div
        className="absolute inset-0 overflow-hidden"
        style={{ clipPath: `inset(0 ${100 - pos}% 0 0)` }}
      >
        <ArtCanvas diff={diff} layers={layers} state="before" />
      </div>

      {/* State labels */}
      <span className="pointer-events-none absolute left-2 top-2 rounded-badge bg-bg/70 px-1.5 py-0.5 text-[11px] font-medium uppercase text-text-muted">
        Before
      </span>
      <span className="pointer-events-none absolute right-2 top-2 rounded-badge bg-bg/70 px-1.5 py-0.5 text-[11px] font-medium uppercase text-text-muted">
        After
      </span>

      {/* Divider + drag handle */}
      <div
        role="slider"
        aria-label="Reveal before/after"
        aria-valuenow={Math.round(pos)}
        aria-valuemin={0}
        aria-valuemax={100}
        tabIndex={0}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onKeyDown={(e) => {
          if (e.key === "ArrowLeft") setPos((p) => Math.max(0, p - 2));
          if (e.key === "ArrowRight") setPos((p) => Math.min(100, p + 2));
        }}
        className="absolute top-0 z-(--z-sticky) h-full w-1 -translate-x-1/2 cursor-col-resize bg-accent"
        style={{ left: `${pos}%` }}
      >
        <span className="absolute top-1/2 left-1/2 grid h-6 w-6 -translate-x-1/2 -translate-y-1/2 place-items-center rounded-full border-2 border-bg bg-accent text-[10px] text-bg">
          ⇆
        </span>
      </div>
    </div>
  );
}
