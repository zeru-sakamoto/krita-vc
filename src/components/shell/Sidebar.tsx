import { useState } from "react";
import {
  ArrowCounterClockwise,
  ArrowsClockwise,
  ArrowUUpLeft,
  CaretDown,
  DotsThreeVertical,
  Plus,
} from "@phosphor-icons/react";
import { DockerPanel } from "./DockerPanel";
import type { ActivityView } from "./ActivityBar";
import { IconButton } from "../ui/IconButton";
import { Button } from "../ui/Button";
import { Menu, type MenuItem } from "../ui/Menu";
import { Modal } from "../ui/Modal";
import { BranchBadge } from "../vcs/BranchBadge";
import { CommitGraph } from "../vcs/CommitGraph";
import { ChangesPanel } from "../vcs/ChangesPanel";
import { BranchesPanel } from "../vcs/BranchesPanel";
import {
  CreateBranchModal,
  SaveFirstModal,
  errorText,
  isUnsavedChangesError,
} from "../vcs/BranchDialogs";
import { useResize } from "../../lib/useResize";
import { useRepository } from "../../lib/repository";
import { useWorkingChanges } from "../../lib/repoData";
import { useArtistMode } from "../../lib/artistMode";
import type { Branch, Commit } from "../../types";

const PANEL_TITLE: Record<ActivityView, string> = {
  changes: "Changes",
  history: "History",
  branches: "Branches",
};

interface SidebarProps {
  view: ActivityView;
  commits: Commit[];
  branches: Branch[];
  currentBranch: Branch;
  selectedId: string | null;
  onSelect: (id: string) => void;
  /** Working-tree file whose diff is shown in the main panel (Changes view). */
  focusedFile: string | null;
  onFocusFile: (path: string) => void;
  /** Jump to the Changes view (used by the save-first prompt). */
  onShowChanges: () => void;
}

/**
 * Resizable sidebar (240–320px). Hosts a docker panel whose content switches
 * with the active activity-bar view: Changes / History / Branches.
 * (DESIGN.md → Layout & App Shell → Sidebar / Resize handle)
 */
export function Sidebar({
  view,
  commits,
  branches,
  currentBranch,
  selectedId,
  onSelect,
  focusedFile,
  onFocusFile,
  onShowChanges,
}: SidebarProps) {
  const {
    current,
    refreshNonce,
    refresh,
    scanning,
    setScanning,
    undoLastCommit,
    discardChanges,
    switchBranch,
    saving,
  } = useRepository();
  const { artistMode } = useArtistMode();
  const [confirmUndo, setConfirmUndo] = useState(false);
  const [undoError, setUndoError] = useState<string | null>(null);
  const [createOpen, setCreateOpen] = useState(false);
  const [saveFirst, setSaveFirst] = useState(false);
  const [switchError, setSwitchError] = useState<string | null>(null);
  const [confirmDiscardAll, setConfirmDiscardAll] = useState(false);
  const [discardAllError, setDiscardAllError] = useState<string | null>(null);

  // Lifted here (not local to ChangesPanel) so "Discard current changes" can see the same
  // staged/unstaged split without a second scan — staging has no backend concept of its own.
  const {
    items: workingItems,
    setItems: setWorkingItems,
    error: workingError,
  } = useWorkingChanges(current?.path ?? null, refreshNonce, setScanning);
  const unstagedPaths = workingItems.filter((c) => !c.staged).map((c) => c.change.path);

  const onSwitch = async (name: string) => {
    if (name === currentBranch.name || saving) return;
    setSwitchError(null);
    try {
      await switchBranch(name);
    } catch (e) {
      if (isUnsavedChangesError(e)) setSaveFirst(true);
      else setSwitchError(errorText(e));
    }
  };

  const onUndo = async () => {
    setUndoError(null);
    try {
      await undoLastCommit();
      setConfirmUndo(false);
    } catch (e) {
      setUndoError(String(e));
    }
  };

  const onDiscardAll = async () => {
    setDiscardAllError(null);
    try {
      await discardChanges(unstagedPaths);
      setConfirmDiscardAll(false);
    } catch (e) {
      setDiscardAllError(String(e));
    }
  };
  const {
    size: width,
    onPointerDown,
    onPointerMove,
    onPointerUp,
    cursorClass,
  } = useResize({
    axis: "x",
    min: 240,
    max: 320,
    initial: 280,
    storageKey: "krita-vc:sidebar-width",
  });

  // Shared "panel options" menu (history + changes). Currently just the undo action.
  const panelOptions = (
    <Menu
      align="right"
      minWidth={200}
      trigger={(open) => (
        <span
          title="Panel options"
          aria-label="Panel options"
          className={[
            "grid h-8 w-8 place-items-center rounded-button text-text-muted",
            "transition-colors hover:bg-white/5 hover:text-text",
            open ? "bg-white/5 text-text" : "",
          ].join(" ")}
        >
          <DotsThreeVertical size={16} />
        </span>
      )}
      items={[
        {
          id: "undo",
          label: artistMode ? "Undo the last version" : "Undo the last commit",
          icon: <ArrowUUpLeft size={14} />,
          disabled: commits.length === 0 || saving,
          onSelect: () => {
            setUndoError(null);
            setConfirmUndo(true);
          },
        },
        ...(view === "changes"
          ? ([
              {
                id: "discard-all",
                label: "Discard current changes",
                icon: <ArrowCounterClockwise size={14} />,
                disabled: unstagedPaths.length === 0 || saving,
                onSelect: () => {
                  setDiscardAllError(null);
                  setConfirmDiscardAll(true);
                },
              },
            ] satisfies MenuItem[])
          : []),
      ]}
    />
  );

  return (
    <div className="relative flex shrink-0" style={{ width }}>
      <DockerPanel
        title={PANEL_TITLE[view]}
        className="flex-1"
        actions={
          view === "changes" ? (
            <>
              <IconButton
                icon={ArrowsClockwise}
                label="Rescan for changes"
                size={16}
                spinning={scanning}
                disabled={scanning}
                onClick={refresh}
              />
              {panelOptions}
            </>
          ) : view === "history" ? (
            panelOptions
          ) : null
        }
      >
        {view === "history" && (
          <>
            {/* Branch selector — the history below shows this branch's line of versions */}
            <div className="flex items-center justify-between gap-2 h-8 border-b border-border px-3 py-1.5">
              <Menu
                trigger={() => (
                  <span
                    className="flex items-center gap-1.5 rounded-button px-1 py-0.5 hover:bg-white/5"
                    title={artistMode ? "Choose which version line to view" : "Switch branch"}
                  >
                    <BranchBadge branch={currentBranch} />
                    <CaretDown size={12} className="text-text-muted" />
                  </span>
                )}
                items={branches.map((b) => ({
                  id: b.name,
                  label: b.name,
                  selected: b.kind === "current",
                  onSelect: () => void onSwitch(b.name),
                }))}
                footer={[
                  {
                    id: "new-branch",
                    label: artistMode ? "New version line…" : "New branch…",
                    icon: <Plus size={13} />,
                    onSelect: () => setCreateOpen(true),
                  },
                ]}
              />
              <span className="text-[11px] text-text-muted">
                {commits.length} {artistMode ? "versions" : "commits"}
              </span>
            </div>

            {switchError && <p className="px-3 pt-2 text-[12px] text-danger">{switchError}</p>}

            <CommitGraph
              commits={commits}
              selectedId={selectedId}
              onSelect={onSelect}
              branches={branches}
            />
          </>
        )}

        {view === "changes" && (
          <ChangesPanel
            currentBranch={currentBranch}
            focusedFile={focusedFile}
            onFocusFile={onFocusFile}
            items={workingItems}
            setItems={setWorkingItems}
            error={workingError}
          />
        )}

        {view === "branches" && <BranchesPanel branches={branches} onShowChanges={onShowChanges} />}
      </DockerPanel>

      {/* Resize handle */}
      <div
        role="separator"
        aria-orientation="vertical"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        className={`absolute right-0 top-0 z-(--z-panel) h-full w-1 translate-x-1/2 ${cursorClass} bg-border transition-colors hover:bg-accent`}
      />

      {createOpen && <CreateBranchModal onClose={() => setCreateOpen(false)} />}
      {saveFirst && (
        <SaveFirstModal onClose={() => setSaveFirst(false)} onShowChanges={onShowChanges} />
      )}

      {confirmUndo && (
        <Modal
          title={artistMode ? "Undo the last version?" : "Undo the last commit?"}
          onClose={() => (saving ? undefined : setConfirmUndo(false))}
          footer={
            <>
              <Button onClick={() => setConfirmUndo(false)} disabled={saving}>
                Cancel
              </Button>
              <Button variant="primary" onClick={onUndo} disabled={saving}>
                {saving ? "Undoing…" : "Undo"}
              </Button>
            </>
          }
        >
          <p className="text-[13px] leading-relaxed text-text-muted">
            This removes the most recent {artistMode ? "version" : "commit"} from history. Your
            files are left exactly as they are — the changes reappear as unsaved work, ready to
            re-save.
          </p>
          {undoError && <p className="mt-3 text-[12px] text-danger">{undoError}</p>}
        </Modal>
      )}

      {confirmDiscardAll && (
        <Modal
          title="Discard current changes?"
          onClose={() => (saving ? undefined : setConfirmDiscardAll(false))}
          footer={
            <>
              <Button onClick={() => setConfirmDiscardAll(false)} disabled={saving}>
                Cancel
              </Button>
              <Button variant="destructive" onClick={onDiscardAll} disabled={saving}>
                {saving ? "Discarding…" : "Discard"}
              </Button>
            </>
          }
        >
          <p className="text-[13px] leading-relaxed text-text-muted">
            This permanently reverts {unstagedPaths.length}{" "}
            {unstagedPaths.length === 1 ? "file" : "files"} to their last saved version. Staged
            files aren't touched. Any unsaved edits are lost.
          </p>
          {discardAllError && <p className="mt-3 text-[12px] text-danger">{discardAllError}</p>}
        </Modal>
      )}
    </div>
  );
}
