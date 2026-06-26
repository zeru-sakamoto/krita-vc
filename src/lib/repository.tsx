import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import type { Repository } from "../types";
import { MOCK_REPOSITORIES } from "../data/mockData";

/**
 * Selected local repository. The app is local-only (no remotes); a repository is
 * just a folder the user has designated. There is no native folder picker yet,
 * so the list is seeded from mock data and `addRepository` appends a placeholder
 * entry — this swaps for a real Tauri dialog + per-repo data fetch when the
 * backend lands. Only the *selected id* is persisted (the list is mock).
 */

const STORAGE_KEY = "krita-vc:repository";

interface RepositoryValue {
  repositories: Repository[];
  current: Repository;
  currentId: string;
  setCurrent: (id: string) => void;
  addRepository: () => void;
}

const RepositoryContext = createContext<RepositoryValue | null>(null);

function readInitialId(): string {
  const fallback = MOCK_REPOSITORIES[0]?.id ?? "";
  if (typeof localStorage === "undefined") return fallback;
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored && MOCK_REPOSITORIES.some((r) => r.id === stored)) return stored;
  return fallback;
}

export function RepositoryProvider({ children }: { children: React.ReactNode }) {
  const [repositories, setRepositories] = useState<Repository[]>(MOCK_REPOSITORIES);
  const [currentId, setCurrentId] = useState<string>(readInitialId);

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, currentId);
    } catch {
      // ignore (e.g. private mode) — state still works for the session
    }
  }, [currentId]);

  const setCurrent = useCallback((id: string) => setCurrentId(id), []);

  const addRepository = useCallback(() => {
    // Stub until a native folder picker exists: append a placeholder repo and
    // select it. (Real flow: open a Tauri dialog, then add the chosen folder.)
    setRepositories((prev) => {
      const n = prev.length + 1;
      const repo: Repository = {
        id: `repo-new-${n}`,
        name: `new-repository-${n}`,
        path: `C:/Art/new-repository-${n}`,
      };
      setCurrentId(repo.id);
      return [...prev, repo];
    });
  }, []);

  const current = useMemo(
    () => repositories.find((r) => r.id === currentId) ?? repositories[0],
    [repositories, currentId]
  );

  const value = useMemo<RepositoryValue>(
    () => ({ repositories, current, currentId, setCurrent, addRepository }),
    [repositories, current, currentId, setCurrent, addRepository]
  );

  return <RepositoryContext.Provider value={value}>{children}</RepositoryContext.Provider>;
}

export function useRepository(): RepositoryValue {
  const ctx = useContext(RepositoryContext);
  if (!ctx) throw new Error("useRepository must be used within a RepositoryProvider");
  return ctx;
}
