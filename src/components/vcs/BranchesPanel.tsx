import { useState } from "react";
import { Check, GitMerge, Plus, Trash } from "@phosphor-icons/react";
import type { Branch } from "../../types";
import { BranchBadge } from "./BranchBadge";
import { Button } from "../ui/Button";
import { IconButton } from "../ui/IconButton";
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

/**
 * Local branch list with real actions: click to switch, hover a row to merge it into the
 * current branch or delete its label. This is a local-only VCS — there are no remotes.
 */
export function BranchesPanel({
  branches,
  onShowChanges,
}: {
  branches: Branch[];
  onShowChanges?: () => void;
}) {
  const { switchBranch, mergeBranch, deleteBranch, saving } = useRepository();
  const { artistMode } = useArtistMode();
  const current = branches.find((b) => b.kind === "current")?.name ?? "main";

  const [createOpen, setCreateOpen] = useState(false);
  const [saveFirst, setSaveFirst] = useState(false);
  const [setAsideOpen, setSetAsideOpen] = useState(false);
  const [mergeTarget, setMergeTarget] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The action the dirty tree blocked, kept so setting work aside can retry the switch or merge
  // the user actually asked for rather than just dismissing the prompt.
  const [blocked, setBlocked] = useState<(() => Promise<void>) | null>(null);

  // Shared error routing: the dirty-tree guard gets the friendly save-first dialog,
  // everything else shows inline under the list.
  const attempt = async (fn: () => Promise<void>) => {
    setError(null);
    try {
      await fn();
      return true;
    } catch (e) {
      if (isUnsavedChangesError(e)) {
        setBlocked(() => fn);
        setSaveFirst(true);
      } else setError(errorText(e));
      return false;
    }
  };

  const onMerge = async () => {
    if (!mergeTarget) return;
    if (await attempt(() => mergeBranch(mergeTarget))) setMergeTarget(null);
    else setMergeTarget(null);
  };
  const onDelete = async () => {
    if (!deleteTarget) return;
    await attempt(() => deleteBranch(deleteTarget));
    setDeleteTarget(null);
  };

  return (
    <div className="flex flex-col">
      <div>
        <h3 className="flex h-8 shrink-0 items-center px-3 text-[11px] font-medium uppercase tracking-wide text-text-muted">
          Local
        </h3>
        <ul className="flex flex-col">
          {branches.map((b) => {
            const active = b.kind === "current";
            return (
              <li key={b.name} className="group relative">
                <button
                  type="button"
                  onClick={() => {
                    if (!active && !saving) void attempt(() => switchBranch(b.name));
                  }}
                  disabled={saving}
                  title={
                    active
                      ? artistMode
                        ? "You're working here"
                        : "Current branch"
                      : `Switch to ${b.name}`
                  }
                  className={[
                    "flex w-full items-center gap-2 border-l-2 px-3 py-1.5 text-left transition-colors",
                    active ? "border-accent bg-accent/12" : "border-transparent hover:bg-white/5",
                  ].join(" ")}
                >
                  <BranchBadge branch={b} />
                  {active && <Check size={13} className="ml-auto text-accent" />}
                </button>
                {!active && (
                  <span className="absolute right-2 top-1/2 flex -translate-y-1/2 items-center gap-0.5 opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100">
                    <IconButton
                      icon={GitMerge}
                      label={
                        artistMode ? `Bring ${b.name} into ${current}` : `Merge into ${current}`
                      }
                      size={14}
                      disabled={saving || !b.tip}
                      onClick={() => setMergeTarget(b.name)}
                    />
                    {b.name !== "main" && (
                      <IconButton
                        icon={Trash}
                        label={artistMode ? "Remove this version line" : "Delete branch"}
                        size={14}
                        disabled={saving}
                        onClick={() => setDeleteTarget(b.name)}
                      />
                    )}
                  </span>
                )}
              </li>
            );
          })}
        </ul>
      </div>

      <button
        type="button"
        onClick={() => setCreateOpen(true)}
        disabled={saving}
        data-tour-id="branches-new"
        className="mx-3 mt-2 flex items-center gap-1.5 rounded-button px-1 py-1 text-[12px] text-text-muted transition-colors hover:bg-white/5 hover:text-text"
      >
        <Plus size={13} />
        {artistMode ? "New version line" : "New branch"}
      </button>

      {error && <p className="px-3 pt-2 text-[12px] text-danger">{error}</p>}

      {createOpen && <CreateBranchModal onClose={() => setCreateOpen(false)} />}
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
            if (retry) void attempt(retry);
          }}
        />
      )}

      {mergeTarget && (
        <Modal
          title={artistMode ? `Bring ${mergeTarget} into ${current}?` : `Merge ${mergeTarget}?`}
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
            <span className="text-text">{current}</span>. If the same artwork changed in both, the
            version from {mergeTarget} wins and the file is flagged for review.
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
    </div>
  );
}
