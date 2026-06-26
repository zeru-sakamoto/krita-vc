import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Reusable pointer-drag resize hook with localStorage persistence. Generalizes
 * the pattern that the sidebar used inline: pointer capture on a drag handle,
 * clamped size, persisted under a `krita-vc:` key.
 *
 * - `axis: "x"` → width (handle on a vertical edge, drag left/right).
 * - `axis: "y"` → height (handle on a horizontal edge, drag up/down).
 *
 * The delta is computed from incremental pointer movement (tracked from
 * pointer-down), so the resized element does not need to be anchored to a
 * window edge.
 */
export interface UseResizeOptions {
  axis: "x" | "y";
  min: number;
  max: number;
  initial: number;
  storageKey: string;
}

export interface UseResize {
  size: number;
  onPointerDown: (e: React.PointerEvent) => void;
  onPointerMove: (e: React.PointerEvent) => void;
  onPointerUp: (e: React.PointerEvent) => void;
}

function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

function readInitial({ initial, min, max, storageKey }: UseResizeOptions): number {
  if (typeof localStorage === "undefined") return initial;
  const stored = localStorage.getItem(storageKey);
  if (stored === null) return initial;
  const parsed = Number.parseInt(stored, 10);
  return Number.isFinite(parsed) ? clamp(parsed, min, max) : initial;
}

export function useResize(options: UseResizeOptions): UseResize {
  const { axis, min, max, storageKey } = options;
  const [size, setSize] = useState<number>(() => readInitial(options));

  // Drag state: are we dragging, and the pointer/size at the start of the drag.
  const draggingRef = useRef(false);
  const startPosRef = useRef(0);
  const startSizeRef = useRef(size);

  useEffect(() => {
    try {
      localStorage.setItem(storageKey, String(size));
    } catch {
      // ignore (e.g. private mode) — state still works for the session
    }
  }, [size, storageKey]);

  const onPointerDown = useCallback(
    (e: React.PointerEvent) => {
      draggingRef.current = true;
      startPosRef.current = axis === "x" ? e.clientX : e.clientY;
      startSizeRef.current = size;
      (e.target as HTMLElement).setPointerCapture(e.pointerId);
    },
    [axis, size]
  );

  const onPointerMove = useCallback(
    (e: React.PointerEvent) => {
      if (!draggingRef.current) return;
      const pos = axis === "x" ? e.clientX : e.clientY;
      const next = startSizeRef.current + (pos - startPosRef.current);
      setSize(clamp(next, min, max));
    },
    [axis, min, max]
  );

  const onPointerUp = useCallback((e: React.PointerEvent) => {
    draggingRef.current = false;
    (e.target as HTMLElement).releasePointerCapture(e.pointerId);
  }, []);

  return { size, onPointerDown, onPointerMove, onPointerUp };
}
