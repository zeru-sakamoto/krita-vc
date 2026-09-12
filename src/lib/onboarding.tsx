import { createContext, useCallback, useContext, useState } from "react";
import { readAuthorName } from "./authorName";

/**
 * First-launch interview (name + theme) — `OnboardingOverlay`. One-time, gated on a
 * localStorage flag; same shape as `tour.tsx`. The answers themselves are saved live through
 * `AuthorNameProvider`/`ThemeProvider`, so this context only tracks whether it's showing.
 */

const STORAGE_KEY = "krita-vc:onboarding-completed";
const TOUR_KEY = "krita-vc:tour-completed";

function hasCompleted(): boolean {
  if (typeof localStorage === "undefined") return true;
  // Installs that predate the interview have already been through setup: finishing the tour or
  // having set a name counts. The name is checked non-empty because `AuthorNameProvider` writes
  // the key (as "") on its first mount, so mere presence would skip every fresh install that
  // closed the window mid-interview.
  return (
    localStorage.getItem(STORAGE_KEY) === "true" ||
    localStorage.getItem(TOUR_KEY) === "true" ||
    readAuthorName() !== ""
  );
}

function markCompleted() {
  try {
    localStorage.setItem(STORAGE_KEY, "true");
  } catch {
    // ignore (e.g. private mode) — the interview just comes back next session
  }
}

interface OnboardingValue {
  active: boolean;
  finish: () => void;
  restart: () => void;
}

const OnboardingContext = createContext<OnboardingValue | null>(null);

export function OnboardingProvider({ children }: { children: React.ReactNode }) {
  const [active, setActive] = useState(() => !hasCompleted());

  const finish = useCallback(() => {
    markCompleted();
    setActive(false);
  }, []);
  const restart = useCallback(() => setActive(true), []);

  return (
    <OnboardingContext.Provider value={{ active, finish, restart }}>
      {children}
    </OnboardingContext.Provider>
  );
}

export function useOnboarding(): OnboardingValue {
  const ctx = useContext(OnboardingContext);
  if (!ctx) throw new Error("useOnboarding must be used within an OnboardingProvider");
  return ctx;
}
