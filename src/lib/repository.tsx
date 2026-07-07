import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type { Repository } from "../types";
import { inTauri } from "./tauri";
import { clearSessionCaches } from "./repoData";
import { readAuthorName, resolvedAuthor } from "./authorName";

/**
 * Selected local repository. The app is local-only (no remotes); a repository is a
 * folder the user designates. In the Tauri shell "Add repository…" opens a native
 * folder picker, initializes a `.kvc/` store there if needed (`init_repository`),
 * and the list is persisted to localStorage. In a plain browser (`npm run dev`,
 * no backend) the list starts empty and repository actions are no-ops.
 */

const STORAGE_KEY = "krita-vc:repository";
const LIST_KEY = "krita-vc:repositories";

interface RepositoryValue {
  repositories: Repository[];
  /** Selected repository, or null when the list is empty (fresh install). */
  current: Repository | null;
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
  /**
   * Create a branch and switch to it. Starts at the current tip (instant, no file I/O)
   * unless `base` names another branch, which switches the working tree to that branch's
   * files first (refused while there are unsaved changes).
   */
  createBranch: (name: string, base?: string) => Promise<void>;
  /** Switch the working tree to a branch (rewrites only files that differ). */
  switchBranch: (name: string) => Promise<void>;
  /** Merge a branch into the current one (fast-forward or merge commit). */
  mergeBranch: (source: string) => Promise<void>;
  /** Remove a branch label (its versions stay in history). */
  deleteBranch: (name: string) => Promise<void>;
  /**
   * Reclaim storage unreachable from any branch (leftovers of undo / deleted branches).
   * `dryRun` reports what a real pass would free without touching anything. Null in a
   * plain browser or with no repository selected.
   */
  cleanupRepository: (dryRun: boolean) => Promise<CleanupReport | null>;
  /** Bumped to make data hooks (scan/history) refetch — e.g. after a commit. */
  refreshNonce: number;
  refresh: () => void;
  /** True while a commit is being written — locks staging, drives the StatusBar progress bar. */
  saving: boolean;
  setSaving: (v: boolean) => void;
  /** Non-null while a write op is in flight — drives the full-screen BusyOverlay. */
  busyMessage: string | null;
  setBusyMessage: (msg: string | null) => void;
  /** True while the working tree is being rescanned — spins the refresh button. */
  scanning: boolean;
  setScanning: (v: boolean) => void;
}

/** Shape returned by the `cleanup_repository` Tauri command (serde camelCase). */
export interface CleanupReport {
  dryRun: boolean;
  commitsRemoved: number;
  versionsRemoved: number;
  objectsDeleted: number;
  bytesReclaimed: number;
  /** Preview images freed (regenerable — pruned to budget / stale-filter wipe). */
  cacheBytesReclaimed: number;
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
  if (typeof localStorage === "undefined") return [];
  try {
    const raw = localStorage.getItem(LIST_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as Repository[];
      if (Array.isArray(parsed)) return parsed;
    }
  } catch {
    // fall through to an empty list
  }
  return [];
}

export function RepositoryProvider({ children }: { children: React.ReactNode }) {
  const [repositories, setRepositories] = useState<Repository[]>(readStoredList);
  const [currentId, setCurrentId] = useState<string>(
    () => localStorage?.getItem(STORAGE_KEY) ?? readStoredList()[0]?.id ?? ""
  );

  useEffect(() => {
    // The previous repo's cached diffs/layers can hold multi-MB base64 payloads (fallback
    // path) — drop them on switch instead of waiting for LRU eviction.
    clearSessionCaches();
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
  const [busyMessage, setBusyMessage] = useState<string | null>(null);
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
    // No native picker in a plain browser — repository management needs the desktop shell.
    if (!inTauri()) return;
    const picked = await open({ directory: true, title: "Select a folder to version-control" });
    if (typeof picked === "string") await addPath(picked);
  }, [addPath]);

  const createRepository = useCallback(
    async (parentPath: string, name: string) => {
      if (!inTauri()) return;
      // init_repository's create_dir_all makes the new folder (and parents) if missing.
      await addPath(joinPath(parentPath, name.trim()));
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
    () => repositories.find((r) => r.id === currentId) ?? repositories[0] ?? null,
    [repositories, currentId]
  );

  // Roll the working tree back to a commit (records a new commit); history refetches after.
  const rollbackToCommit = useCallback(
    async (commitId: string) => {
      if (!inTauri() || !current) return;
      setSaving(true);
      setBusyMessage("Restoring version — please wait…");
      try {
        await invoke("rollback_to_commit", {
          path: current.path,
          commitId,
          author: resolvedAuthor(readAuthorName()),
        });
        refresh();
      } finally {
        setSaving(false);
        setBusyMessage(null);
      }
    },
    [current, refresh]
  );

  const undoLastCommit = useCallback(async () => {
    if (!inTauri() || !current) return;
    setSaving(true);
    setBusyMessage("Undoing last version — please wait…");
    try {
      await invoke("undo_last_commit", { path: current.path });
      refresh();
    } finally {
      setSaving(false);
      setBusyMessage(null);
    }
  }, [current, refresh]);

  // Branch mutations share one shape: invoke + refresh with the saving flag held (locks
  // staging, drives the StatusBar progress bar, and the full-screen BusyOverlay via `label`).
  // Errors rethrow so panels can show friendly messages (e.g. the dirty-tree save-first
  // prompt). No-ops without a backend/repository.
  const branchMutation = useCallback(
    async (command: string, args: Record<string, string>, label: string) => {
      if (!inTauri() || !current) return;
      setSaving(true);
      setBusyMessage(label);
      try {
        await invoke(command, { path: current.path, ...args });
        refresh();
      } finally {
        setSaving(false);
        setBusyMessage(null);
      }
    },
    [current, refresh]
  );

  const createBranch = useCallback(
    (name: string, base?: string) =>
      branchMutation(
        "create_branch",
        base ? { name, base } : { name },
        "Creating branch — please wait…"
      ),
    [branchMutation]
  );
  const switchBranch = useCallback(
    (name: string) =>
      branchMutation("switch_branch", { name }, "Switching branches — please wait…"),
    [branchMutation]
  );
  const mergeBranch = useCallback(
    (source: string) =>
      branchMutation(
        "merge_branch",
        { source, author: resolvedAuthor(readAuthorName()) },
        "Merging branches — please wait…"
      ),
    [branchMutation]
  );
  const deleteBranch = useCallback(
    (name: string) => branchMutation("delete_branch", { name }, "Deleting branch — please wait…"),
    [branchMutation]
  );

  const cleanupRepository = useCallback(
    async (dryRun: boolean): Promise<CleanupReport | null> => {
      if (!inTauri() || !current) return null;
      setSaving(true);
      if (!dryRun) setBusyMessage("Cleaning up storage — please wait…");
      try {
        const report = await invoke<CleanupReport>("cleanup_repository", {
          path: current.path,
          dryRun,
        });
        if (!dryRun) refresh();
        return report;
      } finally {
        setSaving(false);
        if (!dryRun) setBusyMessage(null);
      }
    },
    [current, refresh]
  );

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
      createBranch,
      switchBranch,
      mergeBranch,
      deleteBranch,
      cleanupRepository,
      refreshNonce,
      refresh,
      saving,
      setSaving,
      busyMessage,
      setBusyMessage,
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
      createBranch,
      switchBranch,
      mergeBranch,
      deleteBranch,
      cleanupRepository,
      refreshNonce,
      refresh,
      saving,
      busyMessage,
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
