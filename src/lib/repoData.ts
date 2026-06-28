import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Commit, FileStatus } from "../types";
import { MOCK_COMMITS } from "../data/mockData";
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
