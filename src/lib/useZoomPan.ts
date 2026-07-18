import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Shared zoom/pan for the visual diff viewer. Owns `{scale, tx, ty}` and returns a CSS
 * `transform` applied identically to every canvas (both side-by-side panes AND the swipe
 * slider's two stacked layers), so before/after and the slider divider stay pixel-registered
 * under zoom + pan. Transform-origin is the pane's top-left (`0 0`); `tx`/`ty` are in CSS px.
 *
 * Pan works via middle-mouse or plain left-drag; the slider divider stops propagation on its
 * own pointerdown so a drag started on it doesn't also pan the canvas underneath. Wheel zooms
 * toward the cursor.
 *
 * The transform rides on the SVG-wrapping div (see ArtCanvas), never re-serialized into the
 * SVG string — so interaction stays on the compositor and the heavy inline-SVG DOM is untouched.
 */

const MIN_SCALE = 0.75; // fit-to-pane is 1; allow zooming out a bit further than that
const MAX_SCALE = 16;
const ZOOM_SENSITIVITY = 0.0015;

interface ZoomState {
  scale: number;
  tx: number;
  ty: number;
}

const IDENTITY: ZoomState = { scale: 1, tx: 0, ty: 0 };

export interface UseZoomPan {
  /** e.g. "translate(10px,4px) scale(2)" — pass to ArtCanvas `transform`. */
  transform: string;
  scale: number;
  onWheel: (e: React.WheelEvent) => void;
  onPointerDown: (e: React.PointerEvent) => void;
  onPointerMove: (e: React.PointerEvent) => void;
  onPointerUp: (e: React.PointerEvent) => void;
  /** Cursor class for the pannable area. */
  panCursor: string;
  /** Return to the object-contain fit view. */
  reset: () => void;
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}

export function useZoomPan(): UseZoomPan {
  const [z, setZ] = useState<ZoomState>(IDENTITY);
  const [panning, setPanning] = useState(false);

  // Drag state.
  const panningRef = useRef(false);
  const lastRef = useRef({ x: 0, y: 0 });

  // Wheel/pointermove fire well above 60/s; applying each event as its own setZ re-renders
  // the whole diff viewer per event. Accumulate zoom factor + pan deltas in a ref and flush
  // one setZ per animation frame (same pattern as CompareSlider's divider drag).
  const pendingRef = useRef({ factor: 1, cx: 0, cy: 0, dx: 0, dy: 0, zoom: false });
  const frameRef = useRef<number | null>(null);

  const flush = useCallback(() => {
    frameRef.current = null;
    const p = pendingRef.current;
    pendingRef.current = { factor: 1, cx: 0, cy: 0, dx: 0, dy: 0, zoom: false };
    setZ((prev) => {
      let { scale, tx, ty } = prev;
      if (p.zoom) {
        const next = clamp(scale * p.factor, MIN_SCALE, MAX_SCALE);
        if (next !== scale) {
          const k = next / scale;
          // Keep the image point under the cursor fixed: cx = tx' + p*next, p = (cx - tx)/scale.
          tx = p.cx - (p.cx - tx) * k;
          ty = p.cy - (p.cy - ty) * k;
          scale = next;
          // At scale 1 there's nothing to pan to; snap back so the fit view re-centers.
          if (next === 1) {
            tx = 0;
            ty = 0;
          }
        }
      }
      tx += p.dx;
      ty += p.dy;
      if (scale === prev.scale && tx === prev.tx && ty === prev.ty) return prev;
      return { scale, tx, ty };
    });
  }, []);

  const schedule = useCallback(() => {
    if (frameRef.current == null) frameRef.current = requestAnimationFrame(flush);
  }, [flush]);

  useEffect(
    () => () => {
      if (frameRef.current != null) cancelAnimationFrame(frameRef.current);
    },
    []
  );

  const onWheel = useCallback(
    (e: React.WheelEvent) => {
      e.preventDefault();
      const rect = e.currentTarget.getBoundingClientRect();
      const p = pendingRef.current;
      p.factor *= Math.exp(-e.deltaY * ZOOM_SENSITIVITY);
      p.cx = e.clientX - rect.left;
      p.cy = e.clientY - rect.top;
      p.zoom = true;
      schedule();
    },
    [schedule]
  );

  const onPointerDown = useCallback((e: React.PointerEvent) => {
    if (e.button !== 0 && e.button !== 1) return; // left or middle only
    e.preventDefault();
    panningRef.current = true;
    setPanning(true);
    lastRef.current = { x: e.clientX, y: e.clientY };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }, []);

  const onPointerMove = useCallback(
    (e: React.PointerEvent) => {
      if (!panningRef.current) return;
      const p = pendingRef.current;
      p.dx += e.clientX - lastRef.current.x;
      p.dy += e.clientY - lastRef.current.y;
      lastRef.current = { x: e.clientX, y: e.clientY };
      schedule();
    },
    [schedule]
  );

  const onPointerUp = useCallback((e: React.PointerEvent) => {
    if (!panningRef.current) return;
    panningRef.current = false;
    setPanning(false);
    try {
      (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
    } catch {
      // capture may already be released — harmless
    }
  }, []);

  const reset = useCallback(() => {
    // Drop any queued frame so a pending zoom/pan can't land after the reset.
    pendingRef.current = { factor: 1, cx: 0, cy: 0, dx: 0, dy: 0, zoom: false };
    if (frameRef.current != null) {
      cancelAnimationFrame(frameRef.current);
      frameRef.current = null;
    }
    setZ(IDENTITY);
  }, []);

  return {
    transform: `translate(${z.tx}px,${z.ty}px) scale(${z.scale})`,
    scale: z.scale,
    onWheel,
    onPointerDown,
    onPointerMove,
    onPointerUp,
    panCursor: panning ? "cursor-grabbing" : "cursor-grab",
    reset,
  };
}
