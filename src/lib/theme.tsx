import { createContext, useContext, useEffect, useState } from "react";

/**
 * Global color theme. Themes are just palettes — the actual `--color-*` values
 * live in `styles/global.css` under `html[data-theme="..."]` blocks; this
 * context only tracks the chosen id, persists it, and stamps it on <html> so
 * the CSS cascade does the rest. Charcoal is the default and has no override
 * block (it IS the `@theme` base), so unset themes fall back to it.
 *
 * Pure presentation — never touches the data layer.
 */

export type ThemeId =
  | "charcoal"
  | "charcoal-light"
  | "krita-blue"
  | "studio-light"
  | "electric-cyan"
  | "sunset-coral"
  | "tokyo-night";

/** Swatch metadata for the picker. `bg`/`accent` mirror global.css — kept in
 *  sync by hand (two small values), not worth deriving from CSS at runtime. */
export const THEMES: { id: ThemeId; label: string; bg: string; accent: string }[] = [
  // Dark themes first, then the light ones grouped at the end.
  { id: "charcoal", label: "Charcoal", bg: "#131210", accent: "#e07b39" },
  { id: "krita-blue", label: "Krita Blue", bg: "#1e1e24", accent: "#2e86de" },
  { id: "electric-cyan", label: "Electric Cyan", bg: "#1a1d24", accent: "#00d2d3" },
  { id: "sunset-coral", label: "Sunset Coral", bg: "#201e22", accent: "#ff6b6b" },
  { id: "tokyo-night", label: "Tokyo Night", bg: "#1a1b26", accent: "#7aa2f7" },
  { id: "charcoal-light", label: "Charcoal Light", bg: "#f4f1ea", accent: "#a8511a" },
  { id: "studio-light", label: "Studio Light", bg: "#f5f6fa", accent: "#2e86de" },
];

const STORAGE_KEY = "krita-vc:theme";
const DEFAULT: ThemeId = "charcoal";

const IDS = new Set<string>(THEMES.map((t) => t.id));

/** Read the persisted theme. Safe to call outside React (e.g. in main.tsx
 *  before first paint) — mirrors authorName's `readAuthorName`. */
export function readTheme(): ThemeId {
  if (typeof localStorage === "undefined") return DEFAULT;
  const stored = localStorage.getItem(STORAGE_KEY);
  return stored && IDS.has(stored) ? (stored as ThemeId) : DEFAULT;
}

/** Stamp the theme on <html> so the CSS override blocks apply. */
export function applyTheme(theme: ThemeId): void {
  if (typeof document !== "undefined") {
    document.documentElement.dataset.theme = theme;
  }
}

interface ThemeValue {
  theme: ThemeId;
  setTheme: (t: ThemeId) => void;
}

const ThemeContext = createContext<ThemeValue | null>(null);

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [theme, setTheme] = useState<ThemeId>(readTheme);

  useEffect(() => {
    applyTheme(theme);
    try {
      localStorage.setItem(STORAGE_KEY, theme);
    } catch {
      // ignore (e.g. private mode) — state still works for the session
    }
  }, [theme]);

  return <ThemeContext.Provider value={{ theme, setTheme }}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error("useTheme must be used within a ThemeProvider");
  return ctx;
}
