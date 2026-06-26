import { createContext, useCallback, useContext, useEffect, useState } from "react";

/**
 * Global "Artist Mode" toggle. When on (the default), the whole UI shows
 * artist-friendly labels — color-swatch palette diffs, asset names instead of
 * file paths, "Version N" instead of hashes, words+icons instead of M/A/D.
 * When off, the original technical view is shown verbatim.
 *
 * This is a pure presentation concern — it never touches the (mock) data layer,
 * so it stays valid once a real backend lands.
 */

const STORAGE_KEY = "krita-vc:artist-mode";

interface ArtistModeValue {
  artistMode: boolean;
  toggle: () => void;
}

const ArtistModeContext = createContext<ArtistModeValue | null>(null);

function readInitial(): boolean {
  if (typeof localStorage === "undefined") return true;
  const stored = localStorage.getItem(STORAGE_KEY);
  return stored === null ? true : stored === "true";
}

export function ArtistModeProvider({ children }: { children: React.ReactNode }) {
  const [artistMode, setArtistMode] = useState<boolean>(readInitial);

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, String(artistMode));
    } catch {
      // ignore (e.g. private mode) — state still works for the session
    }
  }, [artistMode]);

  const toggle = useCallback(() => setArtistMode((v) => !v), []);

  return (
    <ArtistModeContext.Provider value={{ artistMode, toggle }}>
      {children}
    </ArtistModeContext.Provider>
  );
}

export function useArtistMode(): ArtistModeValue {
  const ctx = useContext(ArtistModeContext);
  if (!ctx) throw new Error("useArtistMode must be used within an ArtistModeProvider");
  return ctx;
}
