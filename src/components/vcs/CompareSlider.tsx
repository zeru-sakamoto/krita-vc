import { memo, useCallback, useEffect, useRef, useState } from "react";
import type { ArtDiff, ArtLayer, ChangeRegion } from "../../types";
import { ArtCanvas, type HighlightMode } from "./ArtCanvas";

interface CompareSliderProps {
  diff: ArtDiff;
  layers: ArtLayer[];
  overlay?: boolean;
  highlightMode?: HighlightMode;
  /** Change-highlight source for the "after" canvas (composite or the selected layer's own). */
  diffImage?: string | null;
  diffOutline?: string | null;
  regions?: ChangeRegion[];
  /** Shared zoom/pan transform, applied identically to both stacked canvases. */
  transform?: string;
}

/**
 * Swipe comparison: "after" fills the frame; "before" is clipped to the left of a
 * draggable divider. Pointer-capture drag mirrors the Sidebar resize handle pattern.
 *
 * Memoized + the divider drag is rAF-throttled: pointermove fires >100x/s, so a raw
 * setPos per event would re-render both stacked canvases every event. We coalesce to
 * at most one state update per animation frame.
 */
export const CompareSlider = memo(function CompareSlider({
  diff,
  layers,
  overlay,
  highlightMode,
  diffImage,
  diffOutline,
  regions,
  transform,
}: CompareSliderProps) {
  const [pos, setPos] = useState(50); // divider position, 0..100 (% from left)
  const frameRef = useRef<HTMLDivElement>(null);
  const draggingRef = useRef(false);
  // rAF coalescing: latest pointer X, and the pending frame handle (0 = none scheduled).
  const latestXRef = useRef(0);
  const rafRef = useRef(0);

  const flush = useCallback(() => {
    rafRef.current = 0;
    const el = frameRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const pct = ((latestXRef.current - rect.left) / rect.width) * 100;
    setPos(Math.min(100, Math.max(0, pct)));
  }, []);

  const moveTo = useCallback(
    (clientX: number) => {
      latestXRef.current = clientX;
      if (rafRef.current === 0) rafRef.current = requestAnimationFrame(flush);
    },
    [flush]
  );

  // Cancel any pending frame on unmount so flush never runs after teardown.
  useEffect(
    () => () => {
      if (rafRef.current !== 0) cancelAnimationFrame(rafRef.current);
    },
    []
  );

  const onPointerDown = useCallback(
    (e: React.PointerEvent) => {
      // Zoom-pan now pans on plain left-drag too; stop the drag from also reaching that
      // handler on the shared canvas container, or the divider and the pan would fight.
      e.stopPropagation();
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
    if (rafRef.current !== 0) {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = 0;
    }
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
          diffImage={diffImage}
          diffOutline={diffOutline}
          regions={regions}
          transform={transform}
        />
      </div>

      {/* BEFORE — clipped to the left of the divider. The clip stays in frame screen
          space (untransformed wrapper), so it cuts at a fixed vertical line at pos%.
          Both canvases carry the *same* transform, so any image pixel lands at the same
          screen coordinate in both — before/after stay registered under zoom+pan. */}
      <div
        className="absolute inset-0 overflow-hidden"
        style={{ clipPath: `inset(0 ${100 - pos}% 0 0)` }}
      >
        <ArtCanvas diff={diff} layers={layers} state="before" transform={transform} />
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
});
