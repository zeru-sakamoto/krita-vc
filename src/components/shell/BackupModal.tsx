import { useState } from "react";
import { FileZip } from "@phosphor-icons/react";
import { Modal } from "../ui/Modal";
import { Button } from "../ui/Button";
import { Checkbox } from "../ui/Checkbox";
import { useRepository } from "../../lib/repository";

/**
 * Pick which artworks go into one backup archive.
 *
 * Replaces both of the old affordances — "back up this repository" and "back up all artworks",
 * which wrote one zip *per* artwork into a folder. One archive is one thing for the artist to
 * shepherd to their external drive, and the restore flow reads it back whole.
 *
 * Everything is pre-ticked: this is a safety feature, and more backup is the safer default.
 */
export function BackupModal({ onClose }: { onClose: () => void }) {
  const { repositories, backupRepositories } = useRepository();
  const [selected, setSelected] = useState<string[]>(() => repositories.map((r) => r.id));
  const [phase, setPhase] = useState<"pick" | "running" | "done">("pick");
  const [result, setResult] = useState<{ dest: string; failed: string[] } | null>(null);
  const [error, setError] = useState<string | null>(null);

  const toggle = (id: string) =>
    setSelected((prev) => (prev.includes(id) ? prev.filter((x) => x !== id) : [...prev, id]));

  const run = async () => {
    setPhase("running");
    setError(null);
    try {
      const res = await backupRepositories(selected);
      // Null means the user dismissed the save dialog — stay put rather than claim success.
      if (!res) return setPhase("pick");
      setResult(res);
      setPhase("done");
    } catch (e) {
      setError(String(e));
      setPhase("pick");
    }
  };

  if (phase === "done" && result) {
    const succeeded = selected.length - result.failed.length;
    return (
      <Modal
        title="Backup complete"
        onClose={onClose}
        footer={<Button onClick={onClose}>Close</Button>}
      >
        <p className="text-[13px] text-text">
          {succeeded} of {selected.length} artwork{selected.length === 1 ? "" : "s"} backed up.
        </p>
        <p className="mt-1 break-all text-[12px] text-text-muted">Saved to {result.dest}</p>
        {result.failed.length > 0 && (
          <>
            <p className="mt-3 text-[12px] text-text-muted">These couldn’t be backed up:</p>
            <ul className="mt-1 flex flex-col gap-1 font-mono text-[12px] text-danger">
              {result.failed.map((path) => (
                <li key={path} className="truncate">
                  {path}
                </li>
              ))}
            </ul>
          </>
        )}
      </Modal>
    );
  }

  const busy = phase === "running";
  return (
    <Modal
      title="Back up artworks"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose} disabled={busy}>
            Cancel
          </Button>
          <Button variant="primary" disabled={busy || selected.length === 0} onClick={run}>
            <FileZip size={14} />
            {busy
              ? "Backing up…"
              : `Back up ${selected.length} artwork${selected.length === 1 ? "" : "s"}…`}
          </Button>
        </>
      }
    >
      <p className="text-[13px] text-text-muted">
        Everything you pick goes into one zip file — each artwork with its full version history.
        There’s no cloud sync, so this is the only copy that survives a lost drive.
      </p>

      {repositories.length === 0 ? (
        <p className="mt-3 text-[12px] text-text-muted">
          You aren’t tracking any artworks yet, so there’s nothing to back up.
        </p>
      ) : (
        <>
          <div className="mt-3 flex items-center justify-between">
            <span className="text-[11px] uppercase tracking-wide text-text-muted">Artworks</span>
            <button
              type="button"
              disabled={busy}
              onClick={() =>
                setSelected(
                  selected.length === repositories.length ? [] : repositories.map((r) => r.id)
                )
              }
              className="rounded-button px-1.5 py-0.5 text-[12px] text-text-muted hover:bg-state-hover hover:text-text disabled:opacity-40"
            >
              {selected.length === repositories.length ? "Select none" : "Select all"}
            </button>
          </div>
          <ul className="-mx-1 mt-1 max-h-64 overflow-auto">
            {repositories.map((repo) => (
              <li key={repo.id}>
                <label className="flex cursor-pointer items-start gap-2 rounded-button px-2 py-1.5 hover:bg-state-hover">
                  <Checkbox
                    checked={selected.includes(repo.id)}
                    disabled={busy}
                    onChange={() => toggle(repo.id)}
                    className="mt-0.5"
                  />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-[13px] text-text">{repo.name}</span>
                    <span className="block truncate text-[11px] text-text-muted">
                      {backupAgeLabel(repo.lastBackupAt)}
                    </span>
                  </span>
                </label>
              </li>
            ))}
          </ul>
        </>
      )}

      {error && <p className="mt-3 text-[12px] text-danger">{error}</p>}
    </Modal>
  );
}

function backupAgeLabel(lastBackupAt: string | undefined): string {
  if (!lastBackupAt) return "Never backed up";
  const days = Math.floor((Date.now() - new Date(lastBackupAt).getTime()) / 86_400_000);
  if (days <= 0) return "Backed up today";
  if (days === 1) return "Backed up yesterday";
  return `Backed up ${days} days ago`;
}
