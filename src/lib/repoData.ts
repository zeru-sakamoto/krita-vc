import { useEffect, useState } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import type { ArtLayer, Branch, Commit, DiffEntry, FileStatus } from "../types";
import { inTauri } from "./tauri";

/** Shape returned by the `list_commits` Tauri command (serde camelCase). */
interface BackendCommit {
  id: string;
  hash: string;
  message: string;
  author: string;
  timestamp: string;
  parents: string[];
  branch?: string;
  files: { path: string; status: string; content: string | null; isKra: boolean }[];
}

/**
 * Real commit history for `path` via `list_commits`, newest-first and mapped to the
 * frontend `Commit` shape (the graph + inspector consume it). Empty in a plain browser
 * (no backend). `nonce` lets callers force a refetch (e.g. after committing).
 */
export function useCommits(path: string, nonce = 0): Commit[] {
  const [commits, setCommits] = useState<Commit[]>([]);

  useEffect(() => {
    // No backend in a plain browser (`npm run dev`) — history stays empty.
    if (!inTauri()) {
      setCommits([]);
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
              // Pre-branching commits are stamped "" — leave those unset.
              branch: c.branch || undefined,
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

/** Shape returned by the branch Tauri commands (serde camelCase). */
interface BackendBranch {
  name: string;
  tip: string | null;
  current: boolean;
}

function mapBranches(bs: BackendBranch[]): Branch[] {
  return bs.map((b) => ({
    name: b.name,
    kind: b.current ? "current" : "local",
    tip: b.tip,
  }));
}

/**
 * Local branches for `path` via `list_branches` (current branch flagged, tips included).
 * Empty in a plain browser (no backend). `nonce` forces a refetch after any
 * branch mutation (create/switch/merge/delete all bump the shared refresh nonce).
 */
export function useBranches(path: string, nonce = 0): Branch[] {
  const [branches, setBranches] = useState<Branch[]>([]);

  useEffect(() => {
    // No backend in a plain browser — a fresh repo always shows "main" via AppShell's fallback.
    if (!inTauri()) {
      setBranches([]);
      return;
    }
    let cancelled = false;
    invoke<BackendBranch[]>("list_branches", { path })
      .then((bs) => {
        if (!cancelled) setBranches(mapBranches(bs));
      })
      .catch(() => {
        if (!cancelled) setBranches([]);
      });
    return () => {
      cancelled = true;
    };
  }, [path, nonce]);

  return branches;
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
 * rasters + a composite; other files get a minimal text entry). Empty in a plain browser
 * (no backend). `nonce` forces a refetch after a mutating command (rollback/undo).
 * A backend failure surfaces via `error` rather than silently blanking the panel.
 */
/**
 * Session cache of `commit_diff` results, so re-clicking a commit renders instantly instead of
 * re-running the backend diff. Commits are immutable by id (and so is their parent tree), so
 * entries never invalidate — mutations don't touch the key, and a commit removed by undo just
 * ages out of the LRU. Same LRU pattern/cap as `layerCache` below.
 */
const diffCache = new Map<string, DiffEntry[]>();
const DIFF_CACHE_MAX = 20;

export function useCommitDiff(path: string, commitId: string | null): DiffResult {
  const [result, setResult] = useState<DiffResult>({ entries: [], error: null, loading: false });

  useEffect(() => {
    if (!commitId) {
      setResult({ entries: [], error: null, loading: false });
      return;
    }
    if (!inTauri()) {
      setResult({ entries: [], error: null, loading: false });
      return;
    }
    const key = `${path}|${commitId}`;
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
  }, [path, commitId]);

  return result;
}

/**
 * The visual diff for a single working-tree file (working copy vs its last committed version)
 * via `working_diff`. Empty when `file` is null or in a plain browser (no backend).
 * `nonce` forces a refetch after a rescan/commit.
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
 * arrived. Returns `null` in a plain browser (no backend) — callers fall back to whatever
 * layers the diff shipped.
 */
/**
 * Loaded layer rasters keyed by request identity, so revisiting a commit/file doesn't
 * re-rasterize on the backend. Committed layers are immutable by commit id, so their keys
 * ignore `nonce` and survive mutations; only the *working* side keys on `nonce`, since the
 * working copy genuinely changes. Capped small — entries are multi-MB data-URL payloads.
 */
const layerCache = new Map<string, Map<string, ArtLayer>>();
const LAYER_CACHE_MAX = 20;

/**
 * Drop the session diff/layer caches. Called on repository switch: keys embed the repo path
 * (so stale hits are impossible), but on the base64 fallback each retained entry is multi-MB —
 * without this a switch keeps the previous repo's payloads resident until LRU eviction.
 */
export function clearSessionCaches(): void {
  diffCache.clear();
  layerCache.clear();
}

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
    const key = working ? `${path}|${file}|working|${nonce}` : `${path}|${file}|${commitId}`;
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
    // Layers arrive one at a time as the backend finishes them. Fast layers can burst far
    // above frame rate, and every setState re-renders the whole diff viewer — coalesce
    // arrivals into one state flush per animation frame so they still pop in progressively
    // without a re-render per message.
    let frame: number | null = null;
    const flush = () => {
      frame = null;
      if (!cancelled) setState({ layers: new Map(received), loading: true });
    };
    const onLayer = new Channel<ArtLayer>();
    onLayer.onmessage = (layer) => {
      if (cancelled) return;
      received.set(layer.id, layer);
      if (frame == null) frame = requestAnimationFrame(flush);
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
        if (frame != null) {
          cancelAnimationFrame(frame);
          frame = null;
        }
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
      if (frame != null) cancelAnimationFrame(frame);
    };
  }, [path, file, commitId, working, nonce]);

  return state;
}
