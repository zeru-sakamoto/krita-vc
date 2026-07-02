import { useEffect, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import type { ArtLayer, Commit, DiffEntry, FileStatus } from "../types";
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
  /** True while the diff is being fetched, so the UI can show a "diffing…" indicator. */
  loading: boolean;
}

/**
 * The visual diff for a single commit via `commit_diff` (art files carry real per-layer PNG
 * rasters + a composite; other files get a minimal text entry). Falls back to the mock diff
 * map in a plain browser. `nonce` forces a refetch after a mutating command (rollback/undo).
 * A backend failure surfaces via `error` rather than silently blanking the panel.
 */
/**
 * Session cache of `commit_diff` results, so re-clicking a commit renders instantly instead of
 * re-running the backend diff. Commits are immutable, so entries only invalidate via `nonce`
 * (bumped after any mutating command). Same LRU pattern/cap as `layerCache` below.
 */
const diffCache = new Map<string, DiffEntry[]>();
const DIFF_CACHE_MAX = 20;

export function useCommitDiff(path: string, commitId: string | null, nonce = 0): DiffResult {
  const [result, setResult] = useState<DiffResult>({ entries: [], error: null, loading: false });

  useEffect(() => {
    if (!commitId) {
      setResult({ entries: [], error: null, loading: false });
      return;
    }
    if (!inTauri()) {
      setResult({ entries: MOCK_DIFF_BY_COMMIT[commitId] ?? [], error: null, loading: false });
      return;
    }
    const key = `${path}|${commitId}|${nonce}`;
    const cached = diffCache.get(key);
    if (cached) {
      diffCache.delete(key);
      diffCache.set(key, cached);
      setResult({ entries: cached, error: null, loading: false });
      return;
    }
    let cancelled = false;
    setResult({ entries: [], error: null, loading: true });
    invoke<DiffEntry[]>("commit_diff", { path, commitId })
      .then((entries) => {
        diffCache.set(key, entries);
        while (diffCache.size > DIFF_CACHE_MAX) {
          diffCache.delete(diffCache.keys().next().value!);
        }
        if (!cancelled) setResult({ entries, error: null, loading: false });
      })
      .catch((e) => {
        if (!cancelled) setResult({ entries: [], error: String(e), loading: false });
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
  const [result, setResult] = useState<DiffResult>({ entries: [], error: null, loading: false });

  useEffect(() => {
    if (!file || !inTauri()) {
      setResult({ entries: [], error: null, loading: false });
      return;
    }
    let cancelled = false;
    setResult({ entries: [], error: null, loading: true });
    invoke<DiffEntry[]>("working_diff", { path, file })
      .then((entries) => {
        if (!cancelled) setResult({ entries, error: null, loading: false });
      })
      .catch((e) => {
        if (!cancelled) setResult({ entries: [], error: String(e), loading: false });
      });
    return () => {
      cancelled = true;
    };
  }, [path, file, nonce]);

  return result;
}

/**
 * The per-layer rasters (before/after) for one `.kra`, fetched lazily after the diff itself has
 * already rendered the composite + layer list. The backend **streams** each layer over a Tauri
 * `Channel` the moment its rasters finish (out of order), so the returned map grows layer by
 * layer and the UI can paint them progressively; `loading` stays true until the whole set has
 * arrived. Returns `null` in a plain browser (mock diffs already carry their rasters) — callers
 * fall back to whatever layers the diff shipped.
 */
/**
 * Loaded layer rasters keyed by request identity, so revisiting a commit/file doesn't
 * re-rasterize on the backend. `nonce` in the key invalidates after any mutating command
 * (commit/rollback/undo). Capped small — entries are multi-MB data-URL payloads.
 */
const layerCache = new Map<string, Map<string, ArtLayer>>();
const LAYER_CACHE_MAX = 20;

export function useArtLayers(
  path: string,
  file: string,
  opts: { commitId: string | null; working: boolean; nonce?: number }
): { layers: Map<string, ArtLayer> | null; loading: boolean } {
  const { commitId, working, nonce = 0 } = opts;
  const [state, setState] = useState<{ layers: Map<string, ArtLayer> | null; loading: boolean }>({
    layers: null,
    loading: false,
  });

  useEffect(() => {
    if (!inTauri() || !file || (!working && !commitId)) {
      setState({ layers: null, loading: false });
      return;
    }
    const key = `${path}|${file}|${working ? "working" : commitId}|${nonce}`;
    const cached = layerCache.get(key);
    if (cached) {
      // Refresh insertion order so hot entries survive eviction (Map iterates oldest-first).
      layerCache.delete(key);
      layerCache.set(key, cached);
      setState({ layers: cached, loading: false });
      return;
    }
    let cancelled = false;
    const received = new Map<string, ArtLayer>();
    setState({ layers: null, loading: true });
    // Layers arrive one at a time as the backend finishes them; re-render on each so they
    // pop in progressively instead of all at once when the slowest layer completes.
    const onLayer = new Channel<ArtLayer>();
    onLayer.onmessage = (layer) => {
      if (cancelled) return;
      received.set(layer.id, layer);
      setState({ layers: new Map(received), loading: true });
    };
    const req = working
      ? invoke("working_layers", { path, file, onLayer })
      : invoke("commit_layers", { path, commitId, file, onLayer });
    req
      .then(() => {
        // Only a completed, non-cancelled request may seed the cache. A cancelled one has
        // dropped every streamed layer (see onmessage), so its `received` is empty/partial —
        // caching it would poison the key and make a later revisit render zero layers.
        if (cancelled) return;
        layerCache.set(key, received);
        while (layerCache.size > LAYER_CACHE_MAX) {
          layerCache.delete(layerCache.keys().next().value!);
        }
        setState({ layers: new Map(received), loading: false });
      })
      .catch(() => {
        if (!cancelled) setState((s) => ({ ...s, loading: false }));
      });
    return () => {
      cancelled = true;
    };
  }, [path, file, commitId, working, nonce]);

  return state;
}
