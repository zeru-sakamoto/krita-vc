import { createContext, useCallback, useContext, useEffect, useState } from "react";

/**
 * "Legacy version history" toggle. Off by default: the Version Map is the only
 * history surface. On, it brings back the original History (commit graph) and
 * Branches activity-bar tabs alongside it — the escape hatch for the technical
 * view and for the branch actions the map doesn't carry.
 *
 * Pure presentation, same context-plus-localStorage shape as `artistMode.tsx`.
 */

const STORAGE_KEY = "krita-vc:legacy-history";

interface LegacyHistoryValue {
  legacy: boolean;
  toggle: () => void;
}

const LegacyHistoryContext = createContext<LegacyHistoryValue | null>(null);

function readInitial(): boolean {
  if (typeof localStorage === "undefined") return false;
  return localStorage.getItem(STORAGE_KEY) === "true";
}

export function LegacyHistoryProvider({ children }: { children: React.ReactNode }) {
  const [legacy, setLegacy] = useState<boolean>(readInitial);

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, String(legacy));
    } catch {
      // ignore (e.g. private mode) — state still works for the session
    }
  }, [legacy]);

  const toggle = useCallback(() => setLegacy((v) => !v), []);

  return (
    <LegacyHistoryContext.Provider value={{ legacy, toggle }}>
      {children}
    </LegacyHistoryContext.Provider>
  );
}

export function useLegacyHistory(): LegacyHistoryValue {
  const ctx = useContext(LegacyHistoryContext);
  if (!ctx) throw new Error("useLegacyHistory must be used within a LegacyHistoryProvider");
  return ctx;
}
