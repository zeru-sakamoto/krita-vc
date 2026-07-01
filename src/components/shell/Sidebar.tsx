import { useState } from "react";
import { ArrowsClockwise, ArrowUUpLeft, CaretDown, DotsThreeVertical } from "@phosphor-icons/react";
import { DockerPanel } from "./DockerPanel";
import type { ActivityView } from "./ActivityBar";
import { IconButton } from "../ui/IconButton";
import { Button } from "../ui/Button";
import { Modal } from "../ui/Modal";
import { BranchBadge } from "../vcs/BranchBadge";
import { CommitGraph } from "../vcs/CommitGraph";
import { ChangesPanel } from "../vcs/ChangesPanel";
import { BranchesPanel } from "../vcs/BranchesPanel";
import { MOCK_BRANCHES } from "../../data/mockData";
import { useResize } from "../../lib/useResize";
import { useRepository } from "../../lib/repository";
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
  currentBranch: Branch;
  selectedId: string | null;
  onSelect: (id: string) => void;
  /** Working-tree file whose diff is shown in the main panel (Changes view). */
  focusedFile: string | null;
  onFocusFile: (path: string) => void;
}

/**
 * Resizable sidebar (240–320px). Hosts a docker panel whose content switches
 * with the active activity-bar view: Changes / History / Branches.
 * (DESIGN.md → Layout & App Shell → Sidebar / Resize handle)
 */
export function Sidebar({
  view,
  commits,
  currentBranch,
  selectedId,
  onSelect,
  focusedFile,
  onFocusFile,
}: SidebarProps) {
  const { refresh, scanning, undoLastCommit, saving } = useRepository();
  const { artistMode } = useArtistMode();
  const [confirmUndo, setConfirmUndo] = useState(false);
  const [undoError, setUndoError] = useState<string | null>(null);

  const onUndo = async () => {
    setUndoError(null);
    try {
      await undoLastCommit();
      setConfirmUndo(false);
    } catch (e) {
      setUndoError(String(e));
    }
  };
  const {
    size: width,
    onPointerDown,
    onPointerMove,
    onPointerUp,
  } = useResize({
    axis: "x",
    min: 240,
    max: 320,
    initial: 280,
    storageKey: "krita-vc:sidebar-width",
  });

  return (
    <div className="relative flex shrink-0" style={{ width }}>
      <DockerPanel
        title={PANEL_TITLE[view]}
        className="flex-1"
        actions={
          view === "changes" ? (
            <IconButton
              icon={ArrowsClockwise}
              label="Rescan for changes"
              size={16}
              spinning={scanning}
              disabled={scanning}
              onClick={refresh}
            />
          ) : (
            <IconButton icon={DotsThreeVertical} label="Panel options" size={16} />
          )
        }
      >
        {view === "history" && (
          <>
            {/* Branch selector */}
            <div className="flex items-center justify-between gap-2 h-8 border-b border-border px-3 py-1.5">
              <button
                type="button"
                className="flex items-center gap-1.5 rounded-button px-1 py-0.5 hover:bg-white/5"
                title="Switch branch (mock)"
              >
                <BranchBadge branch={currentBranch} />
                <CaretDown size={12} className="text-text-muted" />
              </button>
              <div className="flex items-center gap-1">
                <span className="text-[11px] text-text-muted">
                  {commits.length} {artistMode ? "versions" : "commits"}
                </span>
                <IconButton
                  icon={ArrowUUpLeft}
                  label={artistMode ? "Undo the last version" : "Undo the last commit"}
                  size={15}
                  disabled={commits.length === 0 || saving}
                  onClick={() => {
                    setUndoError(null);
                    setConfirmUndo(true);
                  }}
                />
              </div>
            </div>

            <CommitGraph commits={commits} selectedId={selectedId} onSelect={onSelect} />
          </>
        )}

        {view === "changes" && <ChangesPanel focusedFile={focusedFile} onFocusFile={onFocusFile} />}

        {view === "branches" && <BranchesPanel branches={MOCK_BRANCHES} />}
      </DockerPanel>

      {/* Resize handle */}
      <div
        role="separator"
        aria-orientation="vertical"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        className="absolute right-0 top-0 z-(--z-panel) h-full w-1 translate-x-1/2 cursor-col-resize bg-border transition-colors hover:bg-accent"
      />

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
    </div>
  );
}
