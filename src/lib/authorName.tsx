import { createContext, useCallback, useContext, useEffect, useState } from "react";

/**
 * Global author name used on new commits/merges/rollbacks. Empty/unset falls back to
 * "You" at the call site — mirrors artistMode.tsx's storage shape.
 */

const STORAGE_KEY = "krita-vc:author-name";

interface AuthorNameValue {
  authorName: string;
  setAuthorName: (name: string) => void;
}

const AuthorNameContext = createContext<AuthorNameValue | null>(null);

/** Direct localStorage read for call sites outside the provider tree (e.g. repository.tsx,
 * whose callbacks fire outside React's render cycle so a context subscription isn't needed). */
export function readAuthorName(): string {
  if (typeof localStorage === "undefined") return "";
  return localStorage.getItem(STORAGE_KEY) ?? "";
}
const readInitial = readAuthorName;

export function AuthorNameProvider({ children }: { children: React.ReactNode }) {
  const [authorName, setAuthorNameState] = useState<string>(readInitial);

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, authorName);
    } catch {
      // ignore (e.g. private mode) — state still works for the session
    }
  }, [authorName]);

  const setAuthorName = useCallback((name: string) => setAuthorNameState(name), []);

  return (
    <AuthorNameContext.Provider value={{ authorName, setAuthorName }}>
      {children}
    </AuthorNameContext.Provider>
  );
}

export function useAuthorName(): AuthorNameValue {
  const ctx = useContext(AuthorNameContext);
  if (!ctx) throw new Error("useAuthorName must be used within an AuthorNameProvider");
  return ctx;
}

/** Resolved name for commit `author` fields — empty/unset falls back to "You". */
export function resolvedAuthor(name: string): string {
  return name.trim() || "You";
}
