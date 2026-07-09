import { createContext, useCallback, useContext, useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { inTauri } from "./tauri";

/**
 * Global "custom title bar" preference (default on). When on, TopBar draws its own
 * drag region + minimize/maximize/close controls and the OS window has no native
 * decorations. When off, the OS draws its native title bar instead.
 *
 * Purely a chrome concern like artistMode.tsx, but also drives one live side effect:
 * telling the actual OS window to show/hide its native decorations.
 */

const STORAGE_KEY = "krita-vc:custom-titlebar";

interface WindowChromeValue {
  customTitleBar: boolean;
  toggle: () => void;
}

const WindowChromeContext = createContext<WindowChromeValue | null>(null);

function readInitial(): boolean {
  if (typeof localStorage === "undefined") return true;
  const stored = localStorage.getItem(STORAGE_KEY);
  return stored === null ? true : stored === "true";
}

export function WindowChromeProvider({ children }: { children: React.ReactNode }) {
  const [customTitleBar, setCustomTitleBar] = useState<boolean>(readInitial);

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, String(customTitleBar));
    } catch {
      // ignore (e.g. private mode) — state still works for the session
    }
    // ponytail: one-frame flash possible on boot before this runs when native chrome
    // is the saved preference (tauri.conf.json's static default is decorations:false).
    // Acceptable, same tradeoff as theme's pre-paint flash guard; revisit only if noticeable.
    if (inTauri()) {
      getCurrentWindow()
        .setDecorations(!customTitleBar)
        .catch(() => {});
    }
  }, [customTitleBar]);

  const toggle = useCallback(() => setCustomTitleBar((v) => !v), []);

  return (
    <WindowChromeContext.Provider value={{ customTitleBar, toggle }}>
      {children}
    </WindowChromeContext.Provider>
  );
}

export function useWindowChrome(): WindowChromeValue {
  const ctx = useContext(WindowChromeContext);
  if (!ctx) throw new Error("useWindowChrome must be used within a WindowChromeProvider");
  return ctx;
}
