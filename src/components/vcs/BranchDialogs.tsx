import { useState } from "react";
import { Button } from "../ui/Button";
import { Modal } from "../ui/Modal";
import { Select } from "../ui/Menu";
import { useRepository } from "../../lib/repository";
import { useArtistMode } from "../../lib/artistMode";
import { useBranches } from "../../lib/repoData";
import { versionLabel } from "../../lib/friendly";

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

/**
 * Name-a-branch dialog; creates and switches to it. Starting from the current branch is
 * instant; picking another base switches the working files to that branch first.
 *
 * With `commit` set the branch forks from that version instead (`create_branch_at`) — the base
 * picker is hidden, since the backend takes one or the other, not both.
 */
export function CreateBranchModal({
  onClose,
  commit,
  version,
}: {
  onClose: () => void;
  /** Fork from this commit rather than a branch tip — the Version Map's pick-a-version mode. */
  commit?: string;
  /** That commit's version number, for the copy. */
  version?: number;
}) {
  const { createBranch, saving, current, refreshNonce } = useRepository();
  const { artistMode } = useArtistMode();
  const branches = useBranches(current?.path ?? "", refreshNonce);
  const currentName = branches.find((b) => b.kind === "current")?.name;
  const [name, setName] = useState("");
  const [base, setBase] = useState<string | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    setError(null);
    try {
      // Only send a base when it differs from the current branch (the instant path).
      await createBranch(name, base && base !== currentName ? base : undefined, commit);
      onClose();
    } catch (e) {
      setError(
        isUnsavedChangesError(e)
          ? artistMode
            ? "You have work that isn't saved as a version yet. Save it first, then start the new line."
            : "The working tree has uncommitted changes. Commit them before branching from another branch."
          : errorText(e)
      );
    }
  };

  return (
    <Modal
      title={
        commit
          ? artistMode
            ? `Start a new line from ${version ? versionLabel(version) : "this version"}`
            : "Create a branch here"
          : artistMode
            ? "Start a new version line"
            : "Create a branch"
      }
      onClose={() => (saving ? undefined : onClose())}
      footer={(close) => (
        <>
          <Button onClick={close} disabled={saving}>
            Cancel
          </Button>
          <Button variant="primary" onClick={submit} disabled={saving || !name.trim()}>
            {saving ? "Creating…" : "Create"}
          </Button>
        </>
      )}
    >
      <p className="text-body leading-relaxed text-text-muted">
        {commit
          ? artistMode
            ? `Your artwork goes back to ${version ? versionLabel(version) : "that version"} and the new line starts there. Everything after it stays safe on the line you're on now.`
            : `The new branch starts at ${version ? `version ${version}` : "the selected commit"}; the working tree is rewritten to that commit's files.`
          : artistMode
            ? "Try an idea without touching your current work. New versions you save will live on this line until you bring them back together."
            : "The new branch starts at the chosen base branch's latest commit; new commits land on it until you switch back."}
      </p>
      {/* Focus: the sanctioned `.inset-well` exception (DESIGN.md § Focus Ring Spec) — accent on
          the field's own edge instead of the global ring, which would sit 2px outside a carved-in
          edge and read as a halo. `!outline-none` is load-bearing: the global `:focus-visible`
          rule is unlayered and beats Tailwind's utilities layer. `focus-visible:` (not `focus:`)
          keeps it off mouse clicks, and the surface step is the required non-color co-indicator. */}
      <input
        type="text"
        value={name}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && name.trim() && !saving) void submit();
        }}
        placeholder={artistMode ? "e.g. new-hair-color" : "branch name"}
        autoFocus
        className="mt-3 w-full inset-well rounded-button border border-border bg-bg px-2.5 py-1.5 text-body text-text placeholder:text-text-muted focus-visible:border-accent focus-visible:bg-surface-2 !outline-none"
      />
      {/* A base branch and a starting commit are mutually exclusive backend-side. */}
      {!commit && branches.length > 1 && (
        <div className="mt-3">
          <span className="mb-1 block text-dense text-text-muted">
            {artistMode ? "Start from" : "Base branch"}
          </span>
          <Select
            value={base ?? currentName ?? ""}
            onChange={setBase}
            disabled={saving}
            options={branches.map((b) => ({
              value: b.name,
              label:
                b.name +
                (b.kind === "current" ? (artistMode ? " (where you are now)" : " (current)") : ""),
            }))}
          />
        </div>
      )}
      {error && <p className="mt-3 text-dense text-danger">{error}</p>}
    </Modal>
  );
}

/**
 * Shown when a switch/merge is blocked by unsaved working-tree changes. `onSetAside` offers the
 * third way out — park the work and carry on — so this isn't a dead end.
 */
export function SaveFirstModal({
  onClose,
  onShowChanges,
  onSetAside,
}: {
  onClose: () => void;
  onShowChanges?: () => void;
  onSetAside?: () => void;
}) {
  const { artistMode } = useArtistMode();
  return (
    <Modal
      title="Unsaved changes"
      onClose={onClose}
      footer={(close) => (
        <>
          <Button onClick={close}>Cancel</Button>
          {onSetAside && (
            <Button onClick={onSetAside}>{artistMode ? "Set it aside" : "Stash it"}</Button>
          )}
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
      )}
    >
      <p className="text-body leading-relaxed text-text-muted">
        {artistMode
          ? "You have work that isn't saved as a version yet. Save it first so nothing gets lost — or set it aside and pick it up later."
          : "The working tree has uncommitted changes. Commit, stash, or undo them before switching branches."}
      </p>
    </Modal>
  );
}
