import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type { Repository } from "../types";
import { MOCK_REPOSITORIES } from "../data/mockData";
import { inTauri } from "./tauri";

/**
 * Selected local repository. The app is local-only (no remotes); a repository is a
 * folder the user designates. In the Tauri shell "Add repository…" opens a native
 * folder picker, initializes a `.kvc/` store there if needed (`init_repository`),
 * and the real list is persisted to localStorage. In a plain browser (`npm run dev`,
 * no backend) it falls back to the mock list + a placeholder append.
 */

const STORAGE_KEY = "krita-vc:repository";
const LIST_KEY = "krita-vc:repositories";

interface RepositoryValue {
  repositories: Repository[];
  current: Repository;
  currentId: string;
  setCurrent: (id: string) => void;
  /** Open an existing folder as a repository (init `.kvc/` if absent). */
  browseRepository: () => void;
  /** Create a new folder named `name` inside `parentPath`, init it, and select it. */
  createRepository: (parentPath: string, name: string) => Promise<void>;
  /** Drop a repo from the list; if `deleteFolder`, also delete it on disk. */
  removeRepository: (id: string, deleteFolder: boolean) => Promise<void>;
  /** Restore the working tree to `commitId` and record it as a new commit. */
  rollbackToCommit: (commitId: string) => Promise<void>;
  /** Undo the last commit, keeping working-tree changes. */
  undoLastCommit: () => Promise<void>;
  /** Bumped to make data hooks (scan/history) refetch — e.g. after a commit. */
  refreshNonce: number;
  refresh: () => void;
  /** True while a commit is being written — locks staging, drives the StatusBar progress bar. */
  saving: boolean;
  setSaving: (v: boolean) => void;
  /** True while the working tree is being rescanned — spins the refresh button. */
  scanning: boolean;
  setScanning: (v: boolean) => void;
}

function joinPath(parent: string, name: string): string {
  const sep = parent.includes("\\") ? "\\" : "/";
  return `${parent.replace(/[/\\]+$/, "")}${sep}${name}`;
}

const RepositoryContext = createContext<RepositoryValue | null>(null);

function basename(path: string): string {
  return path.split(/[/\\]/).filter(Boolean).pop() ?? path;
}

function readStoredList(): Repository[] {
  if (typeof localStorage === "undefined") return MOCK_REPOSITORIES;
  try {
    const raw = localStorage.getItem(LIST_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as Repository[];
      if (Array.isArray(parsed) && parsed.length) return parsed;
    }
  } catch {
    // fall through to mock seed
  }
  return MOCK_REPOSITORIES;
}

export function RepositoryProvider({ children }: { children: React.ReactNode }) {
  const [repositories, setRepositories] = useState<Repository[]>(readStoredList);
  const [currentId, setCurrentId] = useState<string>(
    () => localStorage?.getItem(STORAGE_KEY) ?? readStoredList()[0]?.id ?? ""
  );

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, currentId);
    } catch {
      // ignore (e.g. private mode) — state still works for the session
    }
  }, [currentId]);

  useEffect(() => {
    try {
      localStorage.setItem(LIST_KEY, JSON.stringify(repositories));
    } catch {
      // ignore
    }
  }, [repositories]);

  const [refreshNonce, setRefreshNonce] = useState(0);
  const refresh = useCallback(() => setRefreshNonce((n) => n + 1), []);
  const [saving, setSaving] = useState(false);
  const [scanning, setScanning] = useState(false);

  const setCurrent = useCallback((id: string) => setCurrentId(id), []);

  // Add (or re-select) a repo at `path`, initializing its `.kvc/` store if absent.
  const addPath = useCallback(async (path: string) => {
    const exists = await invoke<boolean>("is_repository", { path });
    if (!exists) await invoke("init_repository", { path });
    const repo: Repository = { id: path, name: basename(path), path };
    setRepositories((prev) => (prev.some((r) => r.id === repo.id) ? prev : [...prev, repo]));
    setCurrentId(repo.id);
  }, []);

  const browseRepository = useCallback(async () => {
    // Browser fallback: no native picker — append a placeholder.
    if (!inTauri()) {
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
      return;
    }
    const picked = await open({ directory: true, title: "Select a folder to version-control" });
    if (typeof picked === "string") await addPath(picked);
  }, [addPath]);

  const createRepository = useCallback(
    async (parentPath: string, name: string) => {
      const path = joinPath(parentPath, name.trim());
      if (!inTauri()) {
        const repo: Repository = { id: path, name: name.trim(), path };
        setRepositories((prev) => (prev.some((r) => r.id === repo.id) ? prev : [...prev, repo]));
        setCurrentId(repo.id);
        return;
      }
      // init_repository's create_dir_all makes the new folder (and parents) if missing.
      await addPath(path);
    },
    [addPath]
  );

  const removeRepository = useCallback(
    async (id: string, deleteFolder: boolean) => {
      const repo = repositories.find((r) => r.id === id);
      if (!repo) return;
      if (deleteFolder && inTauri()) await invoke("delete_repository", { path: repo.path });
      setRepositories((prev) => {
        const next = prev.filter((r) => r.id !== id);
        setCurrentId((cur) => (cur === id ? (next[0]?.id ?? "") : cur));
        return next;
      });
    },
    [repositories]
  );

  const current = useMemo(
    () => repositories.find((r) => r.id === currentId) ?? repositories[0],
    [repositories, currentId]
  );

  // Roll the working tree back to a commit (records a new commit); history refetches after.
  const rollbackToCommit = useCallback(
    async (commitId: string) => {
      if (!inTauri()) return;
      setSaving(true);
      try {
        await invoke("rollback_to_commit", { path: current.path, commitId, author: "You" });
        refresh();
      } finally {
        setSaving(false);
      }
    },
    [current, refresh]
  );

  const undoLastCommit = useCallback(async () => {
    if (!inTauri()) return;
    setSaving(true);
    try {
      await invoke("undo_last_commit", { path: current.path });
      refresh();
    } finally {
      setSaving(false);
    }
  }, [current, refresh]);

  const value = useMemo<RepositoryValue>(
    () => ({
      repositories,
      current,
      currentId,
      setCurrent,
      browseRepository,
      createRepository,
      removeRepository,
      rollbackToCommit,
      undoLastCommit,
      refreshNonce,
      refresh,
      saving,
      setSaving,
      scanning,
      setScanning,
    }),
    [
      repositories,
      current,
      currentId,
      setCurrent,
      browseRepository,
      createRepository,
      removeRepository,
      rollbackToCommit,
      undoLastCommit,
      refreshNonce,
      refresh,
      saving,
      scanning,
    ]
  );

  return <RepositoryContext.Provider value={value}>{children}</RepositoryContext.Provider>;
}

export function useRepository(): RepositoryValue {
  const ctx = useContext(RepositoryContext);
  if (!ctx) throw new Error("useRepository must be used within a RepositoryProvider");
  return ctx;
}
