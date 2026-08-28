import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowCounterClockwise, ShieldCheck } from "@phosphor-icons/react";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/Button";
import { relativeTime } from "../../lib/format";
import { versionLabel } from "../../lib/friendly";

/** One version on either side — `compare_restore_versions`, serde camelCase, newest first. */
interface VersionRow {
  id: string;
  message: string;
  timestamp: string;
  author: string;
  branch: string;
}

interface CompareVersions {
  backup: VersionRow[];
  current: VersionRow[];
}

/**
 * Side-by-side version lists for a restore clash: what's in the backup vs what this computer
 * already tracks at that destination.
 *
 * The clash warning in `RestoreModal` says restoring replaces the artwork *and its history* —
 * but not whether that's a gain or a loss. This shows it. Deliberately text only: no composites,
 * no diffs. Each version needs `commit_diff` for imagery, which is `run_heavy` and uncached, and
 * the question here ("which history is further along?") is answered by numbers and messages.
 *
 * `Modal` has no portal, so this renders as a *sibling* of `RestoreModal` — the `CleanupModal`
 * pattern.
 */
export function RestoreCompareModal({
  archive,
  dir,
  destPath,
  name,
  onUse,
  onKeep,
  onClose,
}: {
  archive: string;
  dir: string;
  destPath: string;
  name: string;
  /** Restore this artwork over the existing one (ticks the row). */
  onUse: () => void;
  /** Leave the existing artwork alone (unticks the row). */
  onKeep: () => void;
  onClose: () => void;
}) {
  const [data, setData] = useState<CompareVersions | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    invoke<CompareVersions>("compare_restore_versions", { archive, dir, destPath })
      .then((d) => !cancelled && setData(d))
      .catch((e) => !cancelled && setError(String(e)));
    return () => {
      cancelled = true;
    };
  }, [archive, dir, destPath]);

  const backupIds = new Set((data?.backup ?? []).map((v) => v.id));
  const currentIds = new Set((data?.current ?? []).map((v) => v.id));
  const onlyInBackup = (data?.backup ?? []).filter((v) => !currentIds.has(v.id)).length;
  const onlyHere = (data?.current ?? []).filter((v) => !backupIds.has(v.id)).length;

  return (
    <Modal
      title={`Compare versions — ${name}`}
      onClose={onClose}
      maxWidthClassName="max-w-3xl"
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          <Button
            onClick={() => {
              onKeep();
              onClose();
            }}
          >
            <ShieldCheck size={14} />
            Keep mine
          </Button>
          <Button
            variant="primary"
            onClick={() => {
              onUse();
              onClose();
            }}
          >
            <ArrowCounterClockwise size={14} />
            Use the backup
          </Button>
        </>
      }
    >
      {error && <p className="text-[12px] text-danger">{error}</p>}
      {!data && !error && <p className="text-[12px] text-text-muted">Reading versions…</p>}

      {data && (
        <>
          {/* The actual answer, before any scrolling: which side is further along. */}
          <p className="text-[13px] text-text">
            The backup has {data.backup.length} version{data.backup.length === 1 ? "" : "s"}. This
            computer has {data.current.length}.
          </p>
          <p className="mt-1 text-[12px] text-text-muted">
            {onlyInBackup === 0 && onlyHere === 0
              ? "Both sides hold the same versions."
              : [
                  onlyInBackup > 0 && `${onlyInBackup} only in the backup`,
                  onlyHere > 0 &&
                    `${onlyHere} only on this computer — restoring would lose ${onlyHere === 1 ? "it" : "them"}`,
                ]
                  .filter(Boolean)
                  .join(" · ")}
          </p>

          <div className="mt-3 grid grid-cols-2 gap-3">
            <VersionColumn
              heading="In this backup"
              rows={data.backup}
              otherIds={currentIds}
              onlyLabel="only in the backup"
            />
            <VersionColumn
              heading="On this computer"
              rows={data.current}
              otherIds={backupIds}
              onlyLabel="only here"
            />
          </div>
        </>
      )}
    </Modal>
  );
}

function VersionColumn({
  heading,
  rows,
  otherIds,
  onlyLabel,
}: {
  heading: string;
  rows: VersionRow[];
  otherIds: Set<string>;
  onlyLabel: string;
}) {
  return (
    <div className="min-w-0">
      <h3 className="mb-1.5 text-[12px] font-medium text-text-muted">
        {heading} · {rows.length}
      </h3>
      {rows.length === 0 ? (
        <p className="text-[12px] text-text-muted">No versions saved yet.</p>
      ) : (
        <ul className="max-h-72 overflow-auto">
          {rows.map((v, i) => {
            const only = !otherIds.has(v.id);
            return (
              <li key={v.id} className="rounded-button px-2 py-1.5 hover:bg-state-hover">
                <span className="flex items-baseline gap-2">
                  {/* Numbered per side — same positional formula as `versionNumbers`. The two
                      columns therefore only line up when one history is a superset of the
                      other, which is exactly the case this dialog exists for. */}
                  <span className="text-[13px] text-text">{versionLabel(rows.length - i)}</span>
                  <span className="text-[11px] text-text-muted">{relativeTime(v.timestamp)}</span>
                  {only && <span className="text-[11px] text-warning-fg">{onlyLabel}</span>}
                </span>
                <span className="block truncate text-[12px] text-text-muted">
                  {v.message || "No description"}
                </span>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
