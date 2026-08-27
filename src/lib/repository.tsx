import { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import type { Repository } from "../types";
import { inTauri } from "./tauri";
import { clearSessionCaches } from "./repoData";
import { readAuthorName, resolvedAuthor } from "./authorName";
import { markChecked } from "./checkedRepos";
import { timed } from "./perf";
import { useToast } from "./toast";

/** Map a branch Tauri command to the Performance-report op label (unknowns pass through). */
const BRANCH_OP: Record<string, string> = {
  switch_branch: "switch",
  merge_branch: "merge",
  create_branch: "create",
  delete_branch: "delete",
};

/**
 * The selected artwork. The app is local-only (no remotes), and the unit it versions is **one
 * `.kra` document**, not a folder: artists work per painting, and a folder-wide history with a
 * "which files go in this version" step is the single biggest thing standing between them and
 * the app. In the Tauri shell "Track an artwork…" opens a native *file* picker filtered to
 * `.kra`, creates that document's store beside it if absent (`init_repository`), and the list is
 * persisted to localStorage. In a plain browser (`npm run dev`, no backend) the list starts empty
 * and these actions are no-ops.
 *
 * The context is still named `Repository` throughout — the type is the app-wide contract with
 * `repoData.ts` and every panel, and renaming it buys nothing the doc comments don't.
 */

const STORAGE_KEY = "krita-vc:repository";
const LIST_KEY = "krita-vc:repositories";

interface RepositoryValue {
  repositories: Repository[];
  /** Selected repository, or null when the list is empty (fresh install). */
  current: Repository | null;
  currentId: string;
  setCurrent: (id: string) => void;
  /** Pick a `.kra` to track (creating its store if absent) and select it. */
  browseRepository: () => void;
  /**
   * Drop an artwork from the list; if `deleteHistory`, also delete its store on disk (preferring
   * the OS Recycle Bin). Resolves `false` only when that delete fell back to a permanent one.
   * The `.kra` itself is never touched.
   */
  removeRepository: (id: string, deleteHistory: boolean) => Promise<boolean>;
  /** Zip an artwork and its history to a user-chosen path. Null if canceled or in a browser. */
  backupRepository: (id: string) => Promise<string | null>;
  /**
   * Zip every known repository into a user-chosen destination folder. Resolves the list of
   * repo paths that failed (empty = all succeeded), or null if canceled or in a browser.
   */
  backupAllRepositories: () => Promise<string[] | null>;
  /** Restore the working tree to `commitId` and record it as a new commit. */
  rollbackToCommit: (commitId: string) => Promise<void>;
  /** Undo the last commit, keeping working-tree changes. */
  undoLastCommit: () => Promise<void>;
  /**
   * Discard uncommitted working-tree changes — no new commit. Empty `paths` discards
   * everything; otherwise only those relative paths are touched.
   */
  discardChanges: (paths: string[]) => Promise<void>;
  /**
   * Set uncommitted work aside and revert those files to their committed state. `null` paths
   * sets aside everything dirty; otherwise only those relative paths. Needs at least one
   * commit — there's no committed state to revert to otherwise.
   */
  createStash: (label: string, paths: string[] | null) => Promise<void>;
  /**
   * Bring a stash back into the working tree and take it off the shelf. Rejects (with a
   * `"stash conflict"`-prefixed error) if anything it holds has changed since.
   */
  popStash: (id: string) => Promise<void>;
  /** Remove a stash without restoring it. Its storage is reclaimed by the next cleanup. */
  dropStash: (id: string) => Promise<void>;
  /** Empty the shelf. */
  dropAllStashes: () => Promise<void>;
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
  /**
   * Read-only integrity check over the stored history. Writes nothing, so it never raises the
   * busy overlay. Null in a plain browser or with no path to check. `scrub` (default off)
   * additionally re-hashes every live version's content — IO over the whole store. `path`
   * defaults to the current repository, but callers can pass another repo's path (e.g. checking
   * every repo in the local list).
   */
  checkRepository: (scrub?: boolean, path?: string) => Promise<CheckReport | null>;
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
  /** Bytes moved out of the live store this run — quarantined to `.kvc/trash/`, not yet gone. */
  bytesReclaimed: number;
  /** Preview images freed (regenerable — pruned to budget / stale-filter wipe). */
  cacheBytesReclaimed: number;
  /** Bytes permanently freed by aging quarantined trash out past its retention window. */
  trashBytesPruned: number;
}

/** Shape returned by the `check_repository` Tauri command (serde camelCase). */
export interface CheckReport {
  commitsChecked: number;
  objectsChecked: number;
  /** Whether a full content scrub was requested for this run. */
  scrubPerformed: boolean;
  /** Live versions re-hashed (only non-zero when `scrubPerformed`). */
  versionsScrubbed: number;
  problems: { kind: string; detail: string }[];
}

const RepositoryContext = createContext<RepositoryValue | null>(null);

/** Display name for an artwork: its filename without the `.kra` extension. */
function basename(path: string): string {
  const file = path.split(/[/\\]/).filter(Boolean).pop() ?? path;
  return file.replace(/\.kra$/i, "");
}

/**
 * The backend refuses to open an artwork whose store isn't currently reachable (a custom store
 * root on a drive that isn't plugged in) with this stable prefix, rather than "not tracked" —
 * answering that with "start tracking?" would mint an empty store and orphan every saved
 * version. Matched on the message the way `isStashConflictError` matches its own.
 */
export function isStoreUnreachableError(e: unknown): boolean {
  return String(e).includes("history isn't reachable");
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
  const { show } = useToast();

  const setCurrent = useCallback((id: string) => setCurrentId(id), []);

  // Add (or re-select) the artwork at `path`, creating its store if absent.
  const addPath = useCallback(
    async (path: string) => {
      try {
        const exists = await invoke<boolean>("is_repository", { path });
        if (!exists) await invoke("init_repository", { path });
      } catch (e) {
        // Don't leave a rejected invoke as an unhandled rejection, and don't add an artwork we
        // failed to set up — tell the user instead. This is also the path that surfaces
        // `StoreUnreachable`: an artwork whose history lives somewhere not currently mounted
        // must say so, never quietly get a fresh empty store (see `isStoreUnreachableError`).
        show(`Couldn't start tracking that artwork: ${String(e)}`, "error");
        return;
      }
      const repo: Repository = { id: path, name: basename(path), path };
      setRepositories((prev) => (prev.some((r) => r.id === repo.id) ? prev : [...prev, repo]));
      setCurrentId(repo.id);
    },
    [show]
  );

  const browseRepository = useCallback(async () => {
    // No native picker in a plain browser — this needs the desktop shell.
    if (!inTauri()) return;
    const picked = await open({
      title: "Choose a Krita artwork to track",
      filters: [{ name: "Krita artwork", extensions: ["kra"] }],
    });
    if (typeof picked === "string") await addPath(picked);
  }, [addPath]);

  const removeRepository = useCallback(
    async (id: string, deleteHistory: boolean) => {
      const repo = repositories.find((r) => r.id === id);
      if (!repo) return true;
      let usedTrash = true;
      if (deleteHistory && inTauri()) {
        try {
          usedTrash = await invoke<boolean>("delete_repository", { path: repo.path });
        } catch (e) {
          show(`Couldn't delete the version history: ${String(e)}`, "error");
          usedTrash = false;
        }
      }
      setRepositories((prev) => {
        const next = prev.filter((r) => r.id !== id);
        setCurrentId((cur) => (cur === id ? (next[0]?.id ?? "") : cur));
        return next;
      });
      return usedTrash;
    },
    [repositories, show]
  );

  // Manual last-resort backup: zips the whole project folder (art + `.kvc/`) to a user-chosen
  // path/folder. Doesn't touch `saving` (nothing here is mutated) but still drives the
  // BusyOverlay via `busyMessage` since zipping a large `objects/` dir isn't instant.
  const backupRepository = useCallback(
    async (id: string): Promise<string | null> => {
      if (!inTauri()) return null;
      const repo = repositories.find((r) => r.id === id);
      if (!repo) return null;
      const dest = await save({
        defaultPath: `${repo.name}-backup-${new Date().toISOString().slice(0, 10)}.zip`,
        filters: [{ name: "Zip archive", extensions: ["zip"] }],
      });
      if (!dest) return null;
      setBusyMessage("Backing up artwork — please wait…");
      try {
        await invoke("export_repository_zip", { path: repo.path, dest });
        const lastBackupAt = new Date().toISOString();
        setRepositories((prev) => prev.map((r) => (r.id === id ? { ...r, lastBackupAt } : r)));
        return dest;
      } finally {
        setBusyMessage(null);
      }
    },
    [repositories]
  );

  const backupAllRepositories = useCallback(async (): Promise<string[] | null> => {
    if (!inTauri() || repositories.length === 0) return null;
    const destDir = await open({ directory: true, title: "Choose a folder for the backups" });
    if (typeof destDir !== "string") return null;
    setBusyMessage("Backing up artworks — please wait…");
    try {
      return await invoke<string[]>("export_repositories_zip", {
        paths: repositories.map((r) => r.path),
        destDir,
      });
    } finally {
      setBusyMessage(null);
    }
  }, [repositories]);

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
        // Records against the NEW commit the rollback creates (c.id), not the target `commitId`.
        await timed(
          current.path,
          "rollback",
          invoke<{ id: string }>("rollback_to_commit", {
            path: current.path,
            commitId,
            author: resolvedAuthor(readAuthorName()),
          }),
          (c) => ({ commitId: c.id })
        );
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

  const discardChanges = useCallback(
    async (paths: string[]) => {
      if (!inTauri() || !current) return;
      setSaving(true);
      setBusyMessage("Discarding changes — please wait…");
      try {
        await invoke("discard_changes", { path: current.path, paths });
        refresh();
      } finally {
        setSaving(false);
        setBusyMessage(null);
      }
    },
    [current, refresh]
  );

  // Stash actions can't go through `branchMutation` — its args are string-only and these carry
  // a path list. Same shape otherwise; errors rethrow so panels can show the conflict prompt.
  const createStash = useCallback(
    async (label: string, paths: string[] | null) => {
      if (!inTauri() || !current) return;
      setSaving(true);
      setBusyMessage("Setting your work aside — please wait…");
      try {
        await timed(
          current.path,
          "stash",
          invoke("create_stash", {
            path: current.path,
            label,
            author: resolvedAuthor(readAuthorName()),
            paths,
          })
        );
        refresh();
      } finally {
        setSaving(false);
        setBusyMessage(null);
      }
    },
    [current, refresh]
  );

  const popStash = useCallback(
    async (id: string) => {
      if (!inTauri() || !current) return;
      setSaving(true);
      setBusyMessage("Bringing your work back — please wait…");
      try {
        await timed(current.path, "stash", invoke("pop_stash", { path: current.path, id }));
        refresh();
      } finally {
        setSaving(false);
        setBusyMessage(null);
      }
    },
    [current, refresh]
  );

  // Shelf edits skip `busyMessage`: they're metadata-only writes made from inside the Settings
  // modal, and the full-screen overlay would cover the very list being edited (cf. the cleanup
  // dry run). `saving` alone is enough to lock the buttons.
  const dropStash = useCallback(
    async (id: string) => {
      if (!inTauri() || !current) return;
      setSaving(true);
      try {
        await invoke("drop_stash", { path: current.path, id });
        refresh();
      } finally {
        setSaving(false);
      }
    },
    [current, refresh]
  );

  const dropAllStashes = useCallback(async () => {
    if (!inTauri() || !current) return;
    setSaving(true);
    try {
      await invoke("drop_all_stashes", { path: current.path });
      refresh();
    } finally {
      setSaving(false);
    }
  }, [current, refresh]);

  // Branch mutations share one shape: invoke + refresh with the saving flag held (locks
  // staging, drives the StatusBar progress bar, and the full-screen BusyOverlay via `label`).
  // Errors rethrow so panels can show friendly messages (e.g. the dirty-tree save-first
  // prompt). No-ops without a backend/repository.
  const branchMutation = useCallback(
    async (
      command: string,
      args: Record<string, string>,
      label: string,
      // Pull extra fields (e.g. the resulting commit id) off the command's result into the timing
      // sample — used so a merge's timing ties to the version it created.
      meta?: (value: unknown) => { commitId?: string }
    ) => {
      if (!inTauri() || !current) return;
      setSaving(true);
      setBusyMessage(label);
      try {
        await timed(
          current.path,
          BRANCH_OP[command] ?? command,
          invoke(command, { path: current.path, ...args }),
          meta
        );
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
        "Merging branches — please wait…",
        // merge_branch returns the (merge) Commit — tie its save time to that version.
        (c) => ({ commitId: (c as { id?: string }).id })
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

  // No `busyMessage`: the check only reads, so there's nothing for a stray click to race.
  const checkRepository = useCallback(
    async (scrub = false, path = current?.path): Promise<CheckReport | null> => {
      if (!inTauri() || !path) return null;
      setSaving(true);
      try {
        const report = await invoke<CheckReport>("check_repository", { path, scrub });
        markChecked(path);
        return report;
      } finally {
        setSaving(false);
      }
    },
    [current]
  );

  const value = useMemo<RepositoryValue>(
    () => ({
      repositories,
      current,
      currentId,
      setCurrent,
      browseRepository,
      removeRepository,
      backupRepository,
      backupAllRepositories,
      rollbackToCommit,
      undoLastCommit,
      discardChanges,
      createStash,
      popStash,
      dropStash,
      dropAllStashes,
      createBranch,
      switchBranch,
      mergeBranch,
      deleteBranch,
      cleanupRepository,
      checkRepository,
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
      removeRepository,
      backupRepository,
      backupAllRepositories,
      rollbackToCommit,
      undoLastCommit,
      discardChanges,
      createStash,
      popStash,
      dropStash,
      dropAllStashes,
      createBranch,
      switchBranch,
      mergeBranch,
      deleteBranch,
      cleanupRepository,
      checkRepository,
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
