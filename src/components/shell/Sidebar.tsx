import { ArrowsClockwise, CaretDown, DotsThreeVertical } from "@phosphor-icons/react";
import { DockerPanel } from "./DockerPanel";
import type { ActivityView } from "./ActivityBar";
import { IconButton } from "../ui/IconButton";
import { BranchBadge } from "../vcs/BranchBadge";
import { CommitGraph } from "../vcs/CommitGraph";
import { ChangesPanel } from "../vcs/ChangesPanel";
import { BranchesPanel } from "../vcs/BranchesPanel";
import { MOCK_BRANCHES } from "../../data/mockData";
import { useResize } from "../../lib/useResize";
import { useRepository } from "../../lib/repository";
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
}

/**
 * Resizable sidebar (240–320px). Hosts a docker panel whose content switches
 * with the active activity-bar view: Changes / History / Branches.
 * (DESIGN.md → Layout & App Shell → Sidebar / Resize handle)
 */
export function Sidebar({ view, commits, currentBranch, selectedId, onSelect }: SidebarProps) {
  const { refresh, scanning } = useRepository();
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
              <span className="text-[11px] text-text-muted">{commits.length} commits</span>
            </div>

            <CommitGraph commits={commits} selectedId={selectedId} onSelect={onSelect} />
          </>
        )}

        {view === "changes" && <ChangesPanel />}

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
    </div>
  );
}
