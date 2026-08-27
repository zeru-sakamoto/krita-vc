import { useState } from "react";
import { Button } from "../ui/Button";
import { Modal } from "../ui/Modal";
import {
  CreateBranchModal,
  SaveFirstModal,
  errorText,
  isUnsavedChangesError,
} from "./BranchDialogs";
import { SetAsideModal } from "./StashDialogs";
import { useRepository } from "../../lib/repository";
import { useArtistMode } from "../../lib/artistMode";

// Its own file rather than living in BranchDialogs.tsx: it needs SetAsideModal, and StashDialogs
// already imports `errorText` from BranchDialogs — putting this there would close the cycle.

export interface BranchActions {
  /** Run a branch mutation through the shared error router. True if it went through. */
  run: (fn: () => Promise<void>) => Promise<boolean>;
  /** Whatever the last action failed with, unless it was the dirty tree (that gets a dialog). */
  error: string | null;
  setError: (message: string | null) => void;
  switchTo: (name: string) => void;
  askMerge: (name: string) => void;
  askDelete: (name: string) => void;
  /** Open the create dialog. With `commit`, the new branch forks from that version. */
  askCreate: (commit?: string, version?: number) => void;
  saving: boolean;
  /** Every dialog the actions above can raise, as one node — render it once per call site. */
  dialogs: React.ReactNode;
}

/**
 * Branch mutations plus the whole dialog stack they can raise: the dirty-tree save-first prompt
 * with its set-aside-and-retry escape hatch, the merge and delete confirms, and the create
 * dialog. Shared by the Version Map's action bar, the legacy Branches panel, and the History
 * sidebar's branch switcher — all three used to carry their own copy of this.
 *
 * ponytail: yes, a hook returning JSX. That's the point — one modal stack, three call sites, and
 * the alternative (a wrapper component with a render prop) is more plumbing for the same thing.
 */
export function useBranchActions({
  currentBranch,
  onShowChanges,
}: {
  /** Name of the branch being merged/deleted *into* — for the confirm copy. */
  currentBranch: string;
  /** Jumps the shell to the Changes tab, offered as a way out of the save-first prompt. */
  onShowChanges?: () => void;
}): BranchActions {
  const { switchBranch, mergeBranch, deleteBranch, saving } = useRepository();
  const { artistMode } = useArtistMode();

  const [saveFirst, setSaveFirst] = useState(false);
  const [setAsideOpen, setSetAsideOpen] = useState(false);
  const [mergeTarget, setMergeTarget] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  const [createFrom, setCreateFrom] = useState<{ commit?: string; version?: number } | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The action the dirty tree blocked, kept so setting work aside can retry the switch or merge
  // the user actually asked for rather than just dismissing the prompt.
  const [blocked, setBlocked] = useState<(() => Promise<void>) | null>(null);

  // Shared error routing: the dirty-tree guard gets the friendly save-first dialog,
  // everything else surfaces as `error` for the caller to render.
  const run = async (fn: () => Promise<void>) => {
    setError(null);
    try {
      await fn();
      return true;
    } catch (e) {
      if (isUnsavedChangesError(e)) {
        setBlocked(() => fn); // functional setState — `fn` is itself a function
        setSaveFirst(true);
      } else setError(errorText(e));
      return false;
    }
  };

  const onMerge = async () => {
    if (!mergeTarget) return;
    await run(() => mergeBranch(mergeTarget));
    setMergeTarget(null);
  };
  const onDelete = async () => {
    if (!deleteTarget) return;
    await run(() => deleteBranch(deleteTarget));
    setDeleteTarget(null);
  };

  const dialogs = (
    <>
      {createFrom && (
        <CreateBranchModal
          commit={createFrom.commit}
          version={createFrom.version}
          onClose={() => setCreateFrom(null)}
        />
      )}

      {saveFirst && (
        <SaveFirstModal
          onClose={() => {
            setSaveFirst(false);
            setBlocked(null);
          }}
          onShowChanges={onShowChanges}
          onSetAside={() => {
            setSaveFirst(false);
            setSetAsideOpen(true);
          }}
        />
      )}

      {setAsideOpen && (
        <SetAsideModal
          scope="all"
          paths={null}
          onClose={() => setSetAsideOpen(false)}
          // The tree is clean now, so retry whatever the dirty tree blocked.
          onDone={() => {
            const retry = blocked;
            setBlocked(null);
            if (retry) void run(retry);
          }}
        />
      )}

      {mergeTarget && (
        <Modal
          title={
            artistMode ? `Bring ${mergeTarget} into ${currentBranch}?` : `Merge ${mergeTarget}?`
          }
          onClose={() => (saving ? undefined : setMergeTarget(null))}
          footer={
            <>
              <Button onClick={() => setMergeTarget(null)} disabled={saving}>
                Cancel
              </Button>
              <Button variant="primary" onClick={onMerge} disabled={saving}>
                {saving ? "Merging…" : artistMode ? "Bring it in" : "Merge"}
              </Button>
            </>
          }
        >
          <p className="text-[13px] leading-relaxed text-text-muted">
            Everything from <span className="text-text">{mergeTarget}</span> comes into{" "}
            <span className="text-text">{currentBranch}</span>. If the same artwork changed in both,
            the version from {mergeTarget} wins and the file is flagged for review.
          </p>
        </Modal>
      )}

      {deleteTarget && (
        <Modal
          title={artistMode ? `Remove ${deleteTarget}?` : `Delete ${deleteTarget}?`}
          onClose={() => (saving ? undefined : setDeleteTarget(null))}
          footer={
            <>
              <Button onClick={() => setDeleteTarget(null)} disabled={saving}>
                Cancel
              </Button>
              <Button variant="destructive" onClick={onDelete} disabled={saving}>
                {saving ? "Removing…" : artistMode ? "Remove" : "Delete"}
              </Button>
            </>
          }
        >
          <p className="text-[13px] leading-relaxed text-text-muted">
            The versions saved on it stay in your history. Only the label goes away.
          </p>
        </Modal>
      )}
    </>
  );

  return {
    run,
    error,
    setError,
    switchTo: (name) => {
      if (name !== currentBranch && !saving) void run(() => switchBranch(name));
    },
    askMerge: setMergeTarget,
    askDelete: setDeleteTarget,
    askCreate: (commit, version) => setCreateFrom({ commit, version }),
    saving,
    dialogs,
  };
}
