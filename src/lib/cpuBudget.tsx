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
  { percent: 50, label: "Gentle", hint: "Keeps Krita and your browser snappy while versions save" },
  { percent: 75, label: "Balanced", hint: "Recommended — fast, still leaves room for other apps" },
  { percent: 100, label: "Full speed", hint: "Uses the whole machine; other apps may stutter" },
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
    // Rust applies its own default until this lands, so a slow first paint just means the
    // very first operation may run at the default budget rather than the saved one.
    if (inTauri()) {
      invoke("set_cpu_budget", { percent: budget }).catch(() => {});
    }
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
