import { createContext, useCallback, useContext, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { inTauri } from "./tauri";

/**
 * Global "how much CPU may the engine take" preference. Same shape as windowChrome.tsx:
 * a localStorage-backed value plus one live side effect — here, pushing the budget to the
 * Rust side, which resizes its rayon pool (see src-tauri/src/cpu.rs).
 *
 * App-global on purpose, not per-repo: the pool is process-wide, so putting this in
 * Repo::save_config would mean two open repositories fighting over one pool.
 */

const STORAGE_KEY = "krita-vc:cpu-budget";

export const CPU_BUDGETS = [
  {
    percent: 50,
    label: "Gentle",
    hint: "Uses at most half your processor, so Krita stays responsive while versions save in the background.",
  },
  {
    percent: 75,
    label: "Balanced",
    hint: "Recommended. Uses most of your processor but leaves some free for Krita and other apps.",
  },
  {
    percent: 100,
    label: "Full speed",
    hint: "Uses your whole processor. Versions save fastest, but Krita and other apps may lag while it runs.",
  },
] as const;

const DEFAULT_BUDGET = 75;

interface CpuBudgetValue {
  budget: number;
  setBudget: (percent: number) => void;
}

const CpuBudgetContext = createContext<CpuBudgetValue | null>(null);

function readInitial(): number {
  if (typeof localStorage === "undefined") return DEFAULT_BUDGET;
  const stored = Number(localStorage.getItem(STORAGE_KEY));
  // Any junk (missing, NaN, an out-of-range value from a future build) falls back rather
  // than being clamped — a bad stored value shouldn't silently pin the machine at 100%.
  return CPU_BUDGETS.some((b) => b.percent === stored) ? stored : DEFAULT_BUDGET;
}

export function CpuBudgetProvider({ children }: { children: React.ReactNode }) {
  const [budget, setBudget] = useState<number>(readInitial);

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, String(budget));
    } catch {
      // ignore (e.g. private mode) — state still works for the session
    }
    if (!inTauri()) return;
    // Debounced, because `set_cpu_budget` is far from free: cpu::set_budget spawns a whole new
    // rayon pool (one OS thread per budgeted core, each doing a SetThreadPriority call) while
    // holding the pool's write lock. Firing that per intermediate value — every step of a slider
    // drag — stalls the app long enough that the WebView drops the Settings modal's
    // backdrop-filter for a frame, which reads as the whole screen dimming as you drag.
    // Rust applies its own default until this lands, so a slow first paint just means the
    // very first operation may run at the default budget rather than the saved one.
    const t = setTimeout(() => {
      invoke("set_cpu_budget", { percent: budget }).catch(() => {});
    }, 250);
    return () => clearTimeout(t);
  }, [budget]);

  const set = useCallback((percent: number) => setBudget(percent), []);

  return (
    <CpuBudgetContext.Provider value={{ budget, setBudget: set }}>
      {children}
    </CpuBudgetContext.Provider>
  );
}

export function useCpuBudget(): CpuBudgetValue {
  const ctx = useContext(CpuBudgetContext);
  if (!ctx) throw new Error("useCpuBudget must be used within a CpuBudgetProvider");
  return ctx;
}
