import { useEffect, useRef } from "react";
import { useToast } from "./toast";

const REPEAT_WINDOW_MS = 1500;
const REPEAT_THRESHOLD = 3;

/**
 * Right/middle mouse buttons don't fire `click`, but the browser still engages a button's
 * `:active` state on mousedown regardless of which button was pressed — reading as a dead
 * click. Suppressing the mousedown default (Chromium honors this pre-activation) stops the
 * visual trigger; a burst of repeats nudges the user toward left click via the existing toast.
 */
export function RightClickGuard() {
  const { show } = useToast();
  const hits = useRef<number[]>([]);

  useEffect(() => {
    const onMouseDown = (e: MouseEvent) => {
      if (e.button === 0) return;
      if (!(e.target instanceof Element) || !e.target.closest("button, [role='button']")) return;
      e.preventDefault();
      const now = Date.now();
      hits.current = [...hits.current, now].filter((t) => now - t < REPEAT_WINDOW_MS);
      if (hits.current.length >= REPEAT_THRESHOLD) {
        hits.current = [];
        show("Use left click to activate buttons.", "error");
      }
    };
    window.addEventListener("mousedown", onMouseDown);
    return () => window.removeEventListener("mousedown", onMouseDown);
  }, [show]);

  return null;
}
