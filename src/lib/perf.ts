// Client-side operation timing for the Performance report. Durations are measured as the
// invoke round-trip (what the user actually waits through, incl. the streamed layer diff) and
// kept in localStorage keyed by repo path — timing is inherently per-machine, so it lives with
// the browser, not the repo. See docs / the Performance tab (PerformancePanel).

export type PerfOp = "commit" | "switch" | "merge" | "diff" | string;

export interface PerfSample {
  op: PerfOp;
  /** Round-trip duration in milliseconds. */
  ms: number;
  /** Unix epoch ms when it completed. */
  ts: number;
  /** Commit this sample belongs to, when known (a commit's own id, or the diffed commit). */
  commitId?: string;
}

// Last 100 samples/repo — bounds localStorage without a real ring buffer.
const CAP = 100;
const keyFor = (repoPath: string) => `krita-vc:perf:${repoPath}`;

export function readTimings(repoPath: string): PerfSample[] {
  if (typeof localStorage === "undefined") return [];
  try {
    const raw = localStorage.getItem(keyFor(repoPath));
    if (!raw) return [];
    const parsed = JSON.parse(raw);
    return Array.isArray(parsed) ? (parsed as PerfSample[]) : [];
  } catch {
    return [];
  }
}

function record(repoPath: string, sample: PerfSample): void {
  if (typeof localStorage === "undefined") return;
  try {
    const next = [...readTimings(repoPath), sample].slice(-CAP);
    localStorage.setItem(keyFor(repoPath), JSON.stringify(next));
  } catch {
    // ignore (quota / private mode) — timing is best-effort telemetry
  }
}

/**
 * Time the round-trip of `p`, recording a sample on success. Failures rethrow without being
 * recorded (a fast error would skew the averages). Returns `p`'s resolved value. `meta` can pull
 * extra fields (e.g. the resulting commit id) off the resolved value into the sample.
 */
export async function timed<T>(
  repoPath: string,
  op: PerfOp,
  p: Promise<T>,
  meta?: (value: T) => { commitId?: string }
): Promise<T> {
  const start = performance.now();
  const value = await p; // rejections propagate, unrecorded
  record(repoPath, { op, ms: performance.now() - start, ts: Date.now(), ...meta?.(value) });
  return value;
}

export interface OpSummary {
  op: PerfOp;
  avgMs: number;
  count: number;
}

/** Mean duration + sample count per op, in a stable order. */
export function summarizeTimings(samples: PerfSample[]): Map<PerfOp, OpSummary> {
  const acc = new Map<PerfOp, { total: number; count: number }>();
  for (const s of samples) {
    const cur = acc.get(s.op) ?? { total: 0, count: 0 };
    cur.total += s.ms;
    cur.count += 1;
    acc.set(s.op, cur);
  }
  const out = new Map<PerfOp, OpSummary>();
  for (const [op, { total, count }] of acc) {
    out.set(op, { op, avgMs: total / count, count });
  }
  return out;
}

export interface CommitTiming {
  /** Latest `commit` sample for this commit. */
  saveMs?: number;
  /** Latest `diff` sample tied to this commit. */
  compareMs?: number;
}

// Ops that create a version, in case a card's "save time" comes from a merge or rollback rather
// than a plain commit. `commit` is authoritative (see below).
const SAVE_OPS = new Set<PerfOp>(["merge", "rollback"]);

/**
 * Save/compare time per commit id. Samples are ordered oldest-first, so a later sample for the
 * same commit overwrites an earlier one (latest wins). Two passes so a plain `commit` always wins
 * the save slot: merge/rollback only fill it where no commit sample exists (e.g. a fast-forward
 * merge must not clobber the real commit time of a pre-existing version).
 */
export function timingByCommit(samples: PerfSample[]): Map<string, CommitTiming> {
  const out = new Map<string, CommitTiming>();
  const get = (id: string) => {
    const cur = out.get(id) ?? {};
    out.set(id, cur);
    return cur;
  };
  for (const s of samples) {
    if (!s.commitId) continue;
    if (s.op === "commit") get(s.commitId).saveMs = s.ms;
    else if (s.op === "diff") get(s.commitId).compareMs = s.ms;
  }
  for (const s of samples) {
    if (!s.commitId || !SAVE_OPS.has(s.op)) continue;
    const cur = get(s.commitId);
    if (cur.saveMs == null) cur.saveMs = s.ms;
  }
  return out;
}
