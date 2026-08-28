import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as pickFolder } from "@tauri-apps/plugin-dialog";
import { ArrowCounterClockwise, FolderOpen, Warning } from "@phosphor-icons/react";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/Button";
import { Checkbox } from "../ui/Checkbox";
import { RestoreCompareModal } from "./RestoreCompareModal";
import { useRepository, type ImportItem, type ImportResult } from "../../lib/repository";
import { inTauri } from "../../lib/tauri";

/**
 * Where one artwork in the archive would land (`plan_restore`, serde camelCase). Resolved in
 * Rust — whether the original folder still exists and whether something is already sitting at
 * the destination are filesystem questions, and platform path joining isn't the UI's job.
 */
interface RestorePlan {
  dir: string;
  relpath: string;
  originalDir: string;
  originalDirExists: boolean;
  destDir: string;
  destPath: string;
  occupied: boolean;
  tracked: boolean;
}

/**
 * Restore artworks out of a backup archive.
 *
 * The important thing this does that unzipping by hand does not: each artwork's **history** goes
 * wherever *this* machine keeps history. If Settings → Storage names a store root, the tracking
 * folders land there and no `.kvc/` is created beside the artwork at all. Extracting the archive
 * manually would drop a `.kvc/` the app then never looks at, and the artwork would read as
 * untracked. So the destination is stated up front rather than left to be discovered.
 */
export function RestoreModal({ archive, onClose }: { archive: string; onClose: () => void }) {
  const { readBackupManifest, importBackup, addRepositoryPath } = useRepository();
  const [plans, setPlans] = useState<RestorePlan[] | null>(null);
  const [fallbackDir, setFallbackDir] = useState<string | null>(null);
  const [storeRoot, setStoreRoot] = useState<string | null>(null);
  /** Rows the user unticked. Anything occupied starts unticked — Skip is the safe default. */
  const [skipped, setSkipped] = useState<string[]>([]);
  const [phase, setPhase] = useState<"loading" | "pick" | "running" | "done">("loading");
  const [results, setResults] = useState<ImportResult[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [timestamp, setTimestamp] = useState<string | null>(null);
  /** Row whose two histories the user is comparing, if any. */
  const [compare, setCompare] = useState<RestorePlan | null>(null);

  const load = useCallback(
    async (fallback: string | null) => {
      try {
        const [rows, root, manifest] = await Promise.all([
          invoke<RestorePlan[]>("plan_restore", { archive, fallbackDir: fallback }),
          invoke<string | null>("get_store_root"),
          readBackupManifest(archive),
        ]);
        setPlans(rows);
        setStoreRoot(root);
        setTimestamp(manifest?.timestamp ?? null);
        // Occupied rows default to Skip: replacing overwrites the artwork *and* its history.
        setSkipped(rows.filter((r) => r.occupied).map((r) => r.dir));
        setPhase("pick");
      } catch (e) {
        setError(String(e));
        setPhase("pick");
      }
    },
    [archive, readBackupManifest]
  );

  useEffect(() => {
    void load(null);
  }, [load]);

  const chooseFallback = async () => {
    if (!inTauri()) return;
    const picked = await pickFolder({ directory: true, title: "Choose where to restore" });
    if (typeof picked !== "string") return;
    setFallbackDir(picked);
    setPhase("loading");
    await load(picked);
  };

  const toggle = (dir: string) =>
    setSkipped((prev) => (prev.includes(dir) ? prev.filter((x) => x !== dir) : [...prev, dir]));

  /** What the compare dialog's Keep-mine / Use-the-backup buttons write — the same tick. */
  const setKeep = (dir: string, keep: boolean) =>
    setSkipped((prev) =>
      keep ? prev.filter((x) => x !== dir) : prev.includes(dir) ? prev : [...prev, dir]
    );

  const kept = (plans ?? []).filter((p) => !skipped.includes(p.dir));
  const needsFallback = (plans ?? []).some((p) => !p.originalDirExists) && !fallbackDir;

  const run = async () => {
    setPhase("running");
    setError(null);
    try {
      const items: ImportItem[] = kept.map((p) => ({ dir: p.dir, destDir: p.destDir }));
      const res = await importBackup(archive, items);
      setResults(res);
      // Put every artwork that came back cleanly into the repository list. `addRepositoryPath`
      // sees an existing store and skips init, so this is just "track what we restored".
      for (const r of res) if (!r.error) await addRepositoryPath(r.path);
      setPhase("done");
    } catch (e) {
      setError(String(e));
      setPhase("pick");
    }
  };

  if (phase === "done") {
    const ok = results.filter((r) => !r.error);
    return (
      <Modal
        title="Restore complete"
        onClose={onClose}
        footer={<Button onClick={onClose}>Close</Button>}
        maxWidthClassName="max-w-lg"
      >
        <p className="text-[13px] text-text">
          {ok.length} of {results.length} artwork{results.length === 1 ? "" : "s"} restored.
        </p>
        <ul className="mt-3 flex flex-col gap-2">
          {results.map((r) => (
            <li key={r.dir} className="rounded-button bg-surface-3 px-2 py-1.5">
              <span className="block truncate text-[13px] text-text">{r.name || r.dir}</span>
              {r.error ? (
                <span className="block text-[11px] text-danger">{r.error}</span>
              ) : (
                <>
                  <span className="block truncate text-[11px] text-text-muted">{r.path}</span>
                  <span className="block truncate text-[11px] text-text-muted">
                    History kept in {r.store}
                  </span>
                  {r.problems.length > 0 && (
                    <ul className="mt-1 flex flex-col gap-0.5 text-[11px] text-warning-fg">
                      {r.problems.map((p) => (
                        <li key={p}>{p}</li>
                      ))}
                    </ul>
                  )}
                </>
              )}
            </li>
          ))}
        </ul>
      </Modal>
    );
  }

  const busy = phase === "running" || phase === "loading";
  return (
    <>
      <Modal
        title="Restore from a backup"
        onClose={onClose}
        maxWidthClassName="max-w-lg"
        footer={
          <>
            <Button onClick={onClose} disabled={phase === "running"}>
              Cancel
            </Button>
            <Button
              variant="primary"
              disabled={busy || kept.length === 0 || needsFallback}
              onClick={run}
            >
              <ArrowCounterClockwise size={14} />
              {phase === "running"
                ? "Restoring…"
                : `Restore ${kept.length} artwork${kept.length === 1 ? "" : "s"}`}
            </Button>
          </>
        }
      >
        <p className="break-all text-[12px] text-text-muted">
          {archive}
          {timestamp && ` · backed up ${new Date(timestamp).toLocaleDateString()}`}
        </p>

        {/* Say where history goes before anything is written — with a store root set, it does not
          go beside the artwork, and that would otherwise be a surprise. */}
        <p className="mt-2 rounded-button bg-surface-3 px-2 py-1.5 text-[12px] text-text-muted">
          {storeRoot ? (
            <>
              Version history will be kept in <span className="text-text">{storeRoot}</span> — the
              folder set in Settings → Storage.
            </>
          ) : (
            <>Version history will be kept in a hidden .kvc folder beside each artwork.</>
          )}
        </p>

        {phase === "loading" && <p className="mt-3 text-[12px] text-text-muted">Reading backup…</p>}

        {plans && (
          <ul className="-mx-1 mt-3 max-h-72 overflow-auto">
            {plans.map((p) => {
              const keep = !skipped.includes(p.dir);
              return (
                <li key={p.dir}>
                  <label className="flex cursor-pointer items-start gap-2 rounded-button px-2 py-1.5 hover:bg-state-hover">
                    <Checkbox
                      checked={keep}
                      disabled={busy}
                      onChange={() => toggle(p.dir)}
                      className="mt-0.5"
                    />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-[13px] text-text">{p.relpath}</span>
                      <span className="block truncate text-[11px] text-text-muted">
                        {p.originalDirExists
                          ? `Restore to ${p.destDir}`
                          : fallbackDir
                            ? `Original folder is gone — restore to ${p.destDir}`
                            : "Original folder is gone — choose a folder below"}
                      </span>
                      {p.occupied && (
                        <span className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px] text-warning-fg">
                          <span className="flex items-center gap-1">
                            <Warning size={12} weight="fill" />
                            {p.tracked
                              ? "Already here — restoring replaces this artwork and its history"
                              : "A file is already here — restoring overwrites it"}
                          </span>
                          {/* Only a tracked artwork has a history to weigh against the backup's. */}
                          {p.tracked && (
                            <button
                              type="button"
                              className="text-accent underline underline-offset-2 hover:brightness-110"
                              onClick={(e) => {
                                e.preventDefault(); // the row is a <label>; don't toggle the tick
                                setCompare(p);
                              }}
                            >
                              Compare versions
                            </button>
                          )}
                        </span>
                      )}
                    </span>
                  </label>
                </li>
              );
            })}
          </ul>
        )}

        {(needsFallback || fallbackDir) && (
          <div className="mt-3 flex items-center gap-2">
            <Button onClick={chooseFallback} disabled={busy}>
              <FolderOpen size={14} />
              {fallbackDir ? "Change folder…" : "Choose a folder…"}
            </Button>
            <span className="min-w-0 flex-1 truncate text-[12px] text-text-muted">
              {fallbackDir ?? "Needed for artworks whose original folder no longer exists."}
            </span>
          </div>
        )}

        {error && <p className="mt-3 text-[12px] text-danger">{error}</p>}
      </Modal>

      {/* Sibling, not a child: `Modal` has no portal (same pattern as `CleanupModal`). */}
      {compare && (
        <RestoreCompareModal
          archive={archive}
          dir={compare.dir}
          destPath={compare.destPath}
          name={compare.relpath}
          onUse={() => setKeep(compare.dir, true)}
          onKeep={() => setKeep(compare.dir, false)}
          onClose={() => setCompare(null)}
        />
      )}
    </>
  );
}
