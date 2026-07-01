import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Commit, DiffEntry, FileStatus } from "../types";
import { MOCK_COMMITS, MOCK_DIFF_BY_COMMIT } from "../data/mockData";
import { inTauri } from "./tauri";

/** Shape returned by the `list_commits` Tauri command (serde camelCase). */
interface BackendCommit {
  id: string;
  hash: string;
  message: string;
  author: string;
  timestamp: string;
  parents: string[];
  files: { path: string; status: string; content: string | null; isKra: boolean }[];
}

/**
 * Real commit history for `path` via `list_commits`, newest-first and mapped to the
 * frontend `Commit` shape (the graph + inspector consume it). Falls back to mock data
 * in a plain browser. `nonce` lets callers force a refetch (e.g. after committing).
 */
export function useCommits(path: string, nonce = 0): Commit[] {
  const [commits, setCommits] = useState<Commit[]>(inTauri() ? [] : MOCK_COMMITS);

  useEffect(() => {
    if (!inTauri()) {
      setCommits(MOCK_COMMITS);
      return;
    }
    let cancelled = false;
    invoke<BackendCommit[]>("list_commits", { path })
      .then((cs) => {
        if (cancelled) return;
        // Backend stores oldest-first; the graph expects newest-first.
        const mapped = cs
          .map(
            (c): Commit => ({
              id: c.id,
              hash: c.hash,
              message: c.message,
              author: c.author,
              timestamp: c.timestamp,
              parents: c.parents,
              changes: c.files.map((f) => ({ path: f.path, status: f.status as FileStatus })),
            })
          )
          .reverse();
        setCommits(mapped);
      })
      .catch(() => {
        if (!cancelled) setCommits([]);
      });
    return () => {
      cancelled = true;
    };
  }, [path, nonce]);

  return commits;
}

/** A diff result plus any backend error, so callers can show a message instead of a blank panel. */
export interface DiffResult {
  entries: DiffEntry[];
  error: string | null;
}

/**
 * The visual diff for a single commit via `commit_diff` (art files carry real per-layer PNG
 * rasters + a composite; other files get a minimal text entry). Falls back to the mock diff
 * map in a plain browser. `nonce` forces a refetch after a mutating command (rollback/undo).
 * A backend failure surfaces via `error` rather than silently blanking the panel.
 */
export function useCommitDiff(path: string, commitId: string | null, nonce = 0): DiffResult {
  const [result, setResult] = useState<DiffResult>({ entries: [], error: null });

  useEffect(() => {
    if (!commitId) {
      setResult({ entries: [], error: null });
      return;
    }
    if (!inTauri()) {
      setResult({ entries: MOCK_DIFF_BY_COMMIT[commitId] ?? [], error: null });
      return;
    }
    let cancelled = false;
    invoke<DiffEntry[]>("commit_diff", { path, commitId })
      .then((entries) => {
        if (!cancelled) setResult({ entries, error: null });
      })
      .catch((e) => {
        if (!cancelled) setResult({ entries: [], error: String(e) });
      });
    return () => {
      cancelled = true;
    };
  }, [path, commitId, nonce]);

  return result;
}

/**
 * The visual diff for a single working-tree file (working copy vs its last committed version)
 * via `working_diff`. Empty when `file` is null. Browser/mock has no working diff, so it stays
 * empty there. `nonce` forces a refetch after a rescan/commit.
 */
export function useWorkingDiff(path: string, file: string | null, nonce = 0): DiffResult {
  const [result, setResult] = useState<DiffResult>({ entries: [], error: null });

  useEffect(() => {
    if (!file || !inTauri()) {
      setResult({ entries: [], error: null });
      return;
    }
    let cancelled = false;
    invoke<DiffEntry[]>("working_diff", { path, file })
      .then((entries) => {
        if (!cancelled) setResult({ entries, error: null });
      })
      .catch((e) => {
        if (!cancelled) setResult({ entries: [], error: String(e) });
      });
    return () => {
      cancelled = true;
    };
  }, [path, file, nonce]);

  return result;
}
