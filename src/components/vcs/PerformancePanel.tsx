import { useMemo } from "react";
import { useRepository } from "../../lib/repository";
import { useArtistMode } from "../../lib/artistMode";
import { useStorageStats, type VersionRow } from "../../lib/repoData";
import {
  readTimings,
  summarizeTimings,
  timingByCommit,
  type CommitTiming,
  type PerfOp,
} from "../../lib/perf";
import { relativeTime } from "../../lib/format";

/** 1.4 KB, 12.0 MB … (base-1024, one decimal above bytes). */
function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  return `${v.toFixed(1)} ${units[i]}`;
}

/** "840ms" or "1.3s". */
function formatMs(ms: number): string {
  return ms < 1000 ? `${Math.round(ms)}ms` : `${(ms / 1000).toFixed(1)}s`;
}

/**
 * Percent of `fullCopy` saved by only storing `stored`. Rounded to nearest, but never up to a
 * misleading 100% while bytes were actually stored (nor down to 0% while some were saved) — so
 * "1.2 MB stored of 349.6 MB" reads as 99%, not 100%.
 */
function savedPercent(stored: number, fullCopy: number): number {
  if (fullCopy <= 0) return 0;
  const saved = Math.max(0, fullCopy - stored);
  let pct = Math.round((saved / fullCopy) * 100);
  if (stored > 0) pct = Math.min(pct, 99);
  if (saved > 0) pct = Math.max(pct, 1);
  return pct;
}

// The operations the summary always shows a slot for, in order.
const SUMMARY_OPS: PerfOp[] = ["commit", "switch", "merge", "diff"];

function opLabel(op: PerfOp, artistMode: boolean): string {
  if (!artistMode) return op.charAt(0).toUpperCase() + op.slice(1);
  const friendly: Record<string, string> = {
    commit: "Save",
    switch: "Switch",
    merge: "Merge",
    diff: "Compare",
    create: "New line",
    delete: "Delete",
    rollback: "Restore",
  };
  return friendly[op] ?? op;
}

const Empty = ({ children }: { children: React.ReactNode }) => (
  <p className="px-3 py-2 text-[12px] text-text-muted">{children}</p>
);

/** One label/value cell in a card's stat grid. */
function Stat({ label, value }: { label: string; value?: number }) {
  return (
    <div className="flex items-baseline justify-between gap-2">
      <span className="text-[11px] text-text-muted">{label}</span>
      <span className="text-[11px] tabular-nums text-text">
        {value != null ? formatMs(value) : "—"}
      </span>
    </div>
  );
}

/** Detailed stat card for a single version: stored vs full-copy + save/compare time. */
function VersionCard({
  row,
  timing,
  artistMode,
}: {
  row: VersionRow;
  timing?: CommitTiming;
  artistMode: boolean;
}) {
  // Storage numbers need this version's original (uncompressed) sizes, which are forward-only.
  const hasData = row.originalBytes > 0;
  const pct = savedPercent(row.storedBytes, row.originalBytes);

  return (
    <div className="rounded-panel border border-border bg-surface-2 p-2.5">
      <div className="flex items-baseline justify-between gap-2">
        <span className="text-[12px] font-medium text-text">Version {row.version}</span>
        {hasData && (
          <span className="rounded-badge bg-accent/12 px-1.5 py-0.5 text-[10px] font-medium text-accent">
            {pct}% saved
          </span>
        )}
      </div>
      {row.message && (
        <div className="mt-0.5 truncate text-[11px] text-text-muted" title={row.message}>
          {row.message}
        </div>
      )}
      {hasData ? (
        <div className="mt-1.5 text-[11px] text-text-muted">
          <span className="tabular-nums text-text">{formatBytes(row.storedBytes)}</span> stored ·{" "}
          <span className="tabular-nums">{formatBytes(row.originalBytes)}</span> full copy
        </div>
      ) : (
        <div className="mt-1.5 text-[11px] text-text-muted">
          Size not measured for this version.
        </div>
      )}
      <div className="mt-2 grid grid-cols-2 gap-x-4 border-t border-border/50 pt-2">
        <Stat label={artistMode ? "Save" : "Save time"} value={timing?.saveMs} />
        <Stat label={artistMode ? "Compare" : "Compare time"} value={timing?.compareMs} />
      </div>
    </div>
  );
}

/**
 * Performance report: how long operations take (measured client-side, from localStorage) and
 * how much disk the delta store saves versus a full copy of every file per version (from the
 * `repo_storage_stats` backend command). Self-contained — pulls its own repo context.
 *
 * Layout owns its own height (DockerPanel `scroll={false}` for this view): the per-version cards
 * are the only scroll region, so the summary stays on top and Recent operations stays pinned to
 * the bottom no matter how many versions there are.
 */
export function PerformancePanel() {
  const { current, refreshNonce } = useRepository();
  const { artistMode } = useArtistMode();
  const path = current?.path ?? "";

  const stats = useStorageStats(path, refreshNonce);
  const samples = useMemo(() => readTimings(path), [path, refreshNonce]);
  const summary = useMemo(() => summarizeTimings(samples), [samples]);
  const byCommit = useMemo(() => timingByCommit(samples), [samples]);

  // Sizes are recorded forward-only, so an existing repo's older versions count as 0 bytes —
  // until enough new versions accrue, `naive` can trail the real store and "saved" reads 0.
  const hasSavings = !!stats && stats.naiveBytes > stats.actualBytes;
  const savedPct = stats ? savedPercent(stats.actualBytes, stats.naiveBytes) : 0;

  // Newest-first for both the version cards and the recent-operations log.
  const versions = useMemo(() => (stats ? [...stats.perVersion].reverse() : []), [stats]);
  const recent = useMemo(() => [...samples].reverse().slice(0, 5), [samples]);

  if (!current) return <Empty>No repository selected.</Empty>;

  return (
    <div className="flex h-full min-h-0 flex-col">
      {/* Summary card (fixed) */}
      <div className="m-3 mb-2 shrink-0 rounded-panel border border-border bg-surface-2 p-3">
        <div className="text-[11px] uppercase tracking-wide text-text-muted">
          {artistMode ? "Storage saved" : "Storage saved vs full copies"}
        </div>
        {!stats ? (
          <div className="mt-1 text-[12px] text-text-muted">
            {path ? "No versions yet." : "Storage report unavailable in browser preview."}
          </div>
        ) : hasSavings ? (
          <>
            <div className="mt-1 flex items-baseline gap-2">
              <span className="text-[22px] font-semibold text-accent">
                {formatBytes(stats.savedBytes)}
              </span>
              <span className="text-[12px] text-text-muted">saved ({savedPct}%)</span>
            </div>
            <div className="mt-1 text-[11px] text-text-muted">
              {formatBytes(stats.actualBytes)} stored vs {formatBytes(stats.naiveBytes)} for a full
              copy of every {artistMode ? "version" : "commit"}
            </div>
          </>
        ) : (
          <div className="mt-1">
            <div className="text-[13px] text-text">{formatBytes(stats.actualBytes)} stored</div>
            <div className="mt-1 text-[11px] leading-relaxed text-text-muted">
              Savings show up once you record a few new {artistMode ? "versions" : "commits"} —
              versions saved before this report existed aren’t measured.
            </div>
          </div>
        )}

        {/* Average operation times */}
        <div className="mt-3 grid grid-cols-2 gap-x-3 gap-y-1.5">
          {SUMMARY_OPS.map((op) => {
            const s = summary.get(op);
            return (
              <div key={op} className="flex items-baseline justify-between">
                <span className="text-[12px] text-text-muted">{opLabel(op, artistMode)}</span>
                <span className="text-[12px] tabular-nums text-text">
                  {s ? formatMs(s.avgMs) : "—"}
                </span>
              </div>
            );
          })}
        </div>
        <div className="mt-2 text-[10px] text-text-muted">
          Average time · measured on this device
        </div>
      </div>

      {/* Per-version cards — the only scroll region */}
      <h3 className="flex h-8 shrink-0 items-center px-3 text-[11px] font-medium uppercase tracking-wide text-text-muted">
        Per version
      </h3>
      <div className="min-h-0 flex-1 overflow-auto px-3 pb-2">
        {versions.length > 0 ? (
          <div className="flex flex-col gap-2">
            {versions.map((r) => (
              <VersionCard
                key={r.commitId || r.version}
                row={r}
                timing={byCommit.get(r.commitId)}
                artistMode={artistMode}
              />
            ))}
          </div>
        ) : (
          <p className="py-2 text-[12px] text-text-muted">
            {stats ? "No versions recorded yet." : "Unavailable in browser preview."}
          </p>
        )}
      </div>

      {/* Recent operations — pinned, capped at 5 rows */}
      <h3 className="flex h-8 shrink-0 items-center border-t border-border px-3 text-[11px] font-medium uppercase tracking-wide text-text-muted">
        Recent operations
      </h3>
      <div className="shrink-0">
        {recent.length > 0 ? (
          <ul className="flex flex-col">
            {recent.map((s, i) => (
              <li
                key={`${s.ts}-${i}`}
                className="flex items-center justify-between border-t border-border/50 px-3 py-1 text-[12px]"
              >
                <span className="text-text">{opLabel(s.op, artistMode)}</span>
                <span className="flex items-baseline gap-2">
                  <span className="tabular-nums text-text">{formatMs(s.ms)}</span>
                  <span className="w-14 text-right text-[11px] text-text-muted">
                    {relativeTime(new Date(s.ts).toISOString())}
                  </span>
                </span>
              </li>
            ))}
          </ul>
        ) : (
          <Empty>Timings appear here after you save, switch, merge, or compare versions.</Empty>
        )}
      </div>
    </div>
  );
}
