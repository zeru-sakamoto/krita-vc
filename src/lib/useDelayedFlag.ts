import { useEffect, useState } from "react";

/**
 * How long work has to run before the UI admits it is waiting. A refresh on a fast machine ends
 * inside this, and a spinner that shows for a few frames reads as a flicker, not as progress.
 */
export const LOADING_DELAY_MS = 180;

/**
 * `flag`, but only once it has been continuously true for `delay` ms. It drops to false at once,
 * so a loading state never lingers after the work is done.
 */
export function useDelayedFlag(flag: boolean, delay = LOADING_DELAY_MS): boolean {
  const [shown, setShown] = useState(false);
  useEffect(() => {
    if (!flag) {
      setShown(false);
      return;
    }
    const t = setTimeout(() => setShown(true), delay);
    return () => clearTimeout(t);
  }, [flag, delay]);
  return flag && shown;
}
