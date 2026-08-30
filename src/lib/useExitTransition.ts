import { useEffect, useRef, useState } from "react";

/**
 * How long a floating surface takes to animate away.
 *
 * Exits are deliberately one step quicker than the matching enter (menu enters at
 * `--dur-normal`, modal at `--dur-slow`): an element arriving is information, an element
 * leaving is just cleanup, and a slow exit reads as lag on a tool used this often.
 *
 * Duplicated from `--dur-fast` in `global.css` because this value is needed in JS — the
 * unmount timer can't read a CSS custom property without a layout round-trip, and the two
 * must agree or the surface either unmounts mid-fade or lingers invisible. Keep them in sync.
 */
export const EXIT_MS = 160;

/**
 * Keeps a surface mounted for `exitMs` after `open` goes false, so it can play an exit
 * transition instead of vanishing.
 *
 * Render while `mounted`; drive the transition classes off `visible`. The gap between them
 * is the exit. Reopening mid-exit cancels the pending unmount, so a fast toggle doesn't
 * strand the surface or double-fire.
 *
 * `visible` is set on the frame *after* mount so the enter transition has a "from" state to
 * animate out of — committing both in one paint skips the transition entirely. (Same idiom
 * `Tooltip` used before this hook existed.)
 */
export function useExitTransition(open: boolean, exitMs: number = EXIT_MS) {
  const [mounted, setMounted] = useState(open);
  const [visible, setVisible] = useState(false);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  useEffect(() => {
    clearTimeout(timer.current);
    if (open) {
      setMounted(true);
      const raf = requestAnimationFrame(() => setVisible(true));
      return () => cancelAnimationFrame(raf);
    }
    setVisible(false);
    timer.current = setTimeout(() => setMounted(false), exitMs);
    return () => clearTimeout(timer.current);
  }, [open, exitMs]);

  // Unmounting mid-exit must not leave a timer pointing at dead state.
  useEffect(() => () => clearTimeout(timer.current), []);

  return { mounted, visible };
}
