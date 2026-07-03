import { useState } from "react";
import { Button } from "../ui/Button";
import { Modal } from "../ui/Modal";
import { useRepository } from "../../lib/repository";
import { useArtistMode } from "../../lib/artistMode";

/**
 * Shared branch dialogs, used by both the History branch switcher (Sidebar) and the
 * Branches panel. Copy follows Artist Mode: plain language on, VCS terms off.
 */

/** The backend's dirty-tree guard — matched on its stable message prefix. */
export function isUnsavedChangesError(e: unknown): boolean {
  return String(e).startsWith("unsaved changes");
}

/** Strip nothing, just stringify — backend errors are already user-readable sentences. */
export function errorText(e: unknown): string {
  return String(e);
}

/** Name-a-branch dialog; creates and switches to it (instant — no files change). */
export function CreateBranchModal({ onClose }: { onClose: () => void }) {
  const { createBranch, saving } = useRepository();
  const { artistMode } = useArtistMode();
  const [name, setName] = useState("");
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    setError(null);
    try {
      await createBranch(name);
      onClose();
    } catch (e) {
      setError(errorText(e));
    }
  };

  return (
    <Modal
      title={artistMode ? "Start a new version line" : "Create a branch"}
      onClose={() => (saving ? undefined : onClose())}
      footer={
        <>
          <Button onClick={onClose} disabled={saving}>
            Cancel
          </Button>
          <Button variant="primary" onClick={submit} disabled={saving || !name.trim()}>
            {saving ? "Creating…" : "Create"}
          </Button>
        </>
      }
    >
      <p className="text-[13px] leading-relaxed text-text-muted">
        {artistMode
          ? "Try an idea without touching your current work. New versions you save will live on this line until you bring them back together."
          : "The new branch starts at the current commit; new commits land on it until you switch back."}
      </p>
      <input
        type="text"
        value={name}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && name.trim() && !saving) void submit();
        }}
        placeholder={artistMode ? "e.g. new-hair-color" : "branch name"}
        autoFocus
        className="mt-3 w-full rounded-button border border-border bg-surface-2 px-2.5 py-1.5 text-[13px] text-text placeholder:text-text-muted focus:border-accent focus:outline-none"
      />
      {error && <p className="mt-3 text-[12px] text-danger">{error}</p>}
    </Modal>
  );
}

/** Shown when a switch/merge is blocked by unsaved working-tree changes. */
export function SaveFirstModal({
  onClose,
  onShowChanges,
}: {
  onClose: () => void;
  onShowChanges?: () => void;
}) {
  const { artistMode } = useArtistMode();
  return (
    <Modal
      title="Unsaved changes"
      onClose={onClose}
      footer={
        <>
          <Button onClick={onClose}>Cancel</Button>
          {onShowChanges && (
            <Button
              variant="primary"
              onClick={() => {
                onClose();
                onShowChanges();
              }}
            >
              Go to Changes
            </Button>
          )}
        </>
      }
    >
      <p className="text-[13px] leading-relaxed text-text-muted">
        {artistMode
          ? "You have work that isn't saved as a version yet. Save it first so nothing gets lost, then switch."
          : "The working tree has uncommitted changes. Commit (or undo) them before switching branches."}
      </p>
    </Modal>
  );
}
