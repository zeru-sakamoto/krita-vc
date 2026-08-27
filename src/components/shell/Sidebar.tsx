import { useState } from "react";
import {
  ArrowCounterClockwise,
  ArrowsClockwise,
  ArrowUUpLeft,
  CaretDown,
  DotsThreeVertical,
  ListBullets,
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
import { PerformancePanel } from "../vcs/PerformancePanel";
import { errorText } from "../vcs/BranchDialogs";
import { useBranchActions } from "../vcs/useBranchActions";
import {
  PickStashModal,
  SetAsideModal,
  StashConflictModal,
  StashIcon,
  UnstashIcon,
  isStashConflictError,
  type StashScope,
} from "../vcs/StashDialogs";
import { useResize } from "../../lib/useResize";
import { useRepository } from "../../lib/repository";
import { useStashes } from "../../lib/repoData";
import { useArtistMode } from "../../lib/artistMode";
import { useTour } from "../../lib/tour";
import type { Branch, Commit, WorkingChange } from "../../types";

// Tour steps that spotlight one row inside the (normally click-to-open) panel-options
// menu — the tour forces the menu open for the duration of these steps since the
// overlay blocks real clicks.
const PANEL_OPTION_TOUR_IDS = new Set([
  "panel-option-undo",
  "panel-option-discard-all",
  "panel-option-stash-all",
  "panel-option-stash-pop-latest",
  "panel-option-stash-pick",
]);

const PANEL_TITLE: Record<ActivityView, string> = {
  changes: "Changes",
  // The map renders its own full-width panel and never mounts the Sidebar; present only to
  // keep the record exhaustive.
  map: "Version map",
  history: "History",
  branches: "Branches",
  performance: "Performance",
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
  onFocusFile: (path: string | null) => void;
  /** The shell's one scan of the tracked artwork — the Version Map needs the same result, so it
   *  lives in `AppShell` rather than here. */
  workingItems: WorkingChange[];
  workingError: string | null;
  /** Whether the diff currently in the well (working or commit) is still being computed. */
  diffLoading: boolean;
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
  workingItems,
  workingError,
  diffLoading,
  onShowChanges,
}: SidebarProps) {
  const {
    current,
    refreshNonce,
    refresh,
    scanning,
    undoLastCommit,
    discardChanges,
    popStash,
    saving,
  } = useRepository();
  const { artistMode } = useArtistMode();
  const { active: tourActive, step: tourStep } = useTour();
  const forcePanelOptionsOpen = tourActive && PANEL_OPTION_TOUR_IDS.has(tourStep.tourId);
  const [confirmUndo, setConfirmUndo] = useState(false);
  const [undoError, setUndoError] = useState<string | null>(null);
  const [confirmDiscardAll, setConfirmDiscardAll] = useState(false);
  const [discardAllError, setDiscardAllError] = useState<string | null>(null);
  const [setAside, setSetAside] = useState<{ scope: StashScope; paths: string[] | null } | null>(
    null
  );
  const [pickStash, setPickStash] = useState(false);
  const [stashConflict, setStashConflict] = useState<string | null>(null);
  // Branch switch/create plus the save-first prompt and its set-aside-and-retry escape hatch —
  // shared with the Version Map's action bar and the Branches panel.
  const branchActions = useBranchActions({
    currentBranch: currentBranch.name,
    onShowChanges,
  });

  // Either kind of "checking" — the working-tree rescan or the diff being computed — leaves
  // state these actions read in flux, so both block undo/discard/set-aside the same way.
  const checking = scanning || diffLoading;
  const changedPaths = workingItems.map((c) => c.change.path);
  const stashes = useStashes(current?.path ?? null, refreshNonce);

  const onPop = async (id: string) => {
    branchActions.setError(null);
    try {
      await popStash(id);
      setPickStash(false);
    } catch (e) {
      setPickStash(false);
      if (isStashConflictError(e)) setStashConflict(errorText(e));
      else branchActions.setError(errorText(e));
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
      await discardChanges(changedPaths);
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

  // Shared "panel options" menu (history + changes) — undo is common to both, but setting work
  // aside and bringing it back both act on the working tree, so those rows (and the `footer`
  // group's divider) are changes-only; History's menu is just undo.
  const openSetAside = (scope: StashScope, paths: string[] | null) => {
    branchActions.setError(null);
    setSetAside({ scope, paths });
  };
  const setAsideItems = (
    view === "changes"
      ? [
          {
            // One tracked artwork means there is no subset to set aside — the old
            // "staged files" row would have been a second button doing the same thing.
            id: "stash-all",
            label: artistMode ? "Set this aside" : "Stash changes",
            icon: <StashIcon size={14} />,
            separator: true,
            tourId: "panel-option-stash-all",
            // Needs a commit to revert back to, same guard as undo.
            disabled: workingItems.length === 0 || commits.length === 0 || saving,
            onSelect: () => openSetAside("all", null),
          },
        ]
      : []
  ) satisfies MenuItem[];
  const stashFooter = [
    {
      id: "stash-pop-latest",
      label: artistMode ? "Bring back latest" : "Pop latest stash",
      icon: <UnstashIcon size={14} />,
      detail:
        stashes.length === 0
          ? undefined
          : artistMode
            ? `${stashes.length} set aside`
            : `${stashes.length} ${stashes.length === 1 ? "stash" : "stashes"}`,
      tourId: "panel-option-stash-pop-latest",
      disabled: stashes.length === 0 || saving,
      // The list is newest-first, so [0] is the latest.
      onSelect: () => void onPop(stashes[0].id),
    },
    {
      id: "stash-pick",
      label: artistMode ? "Bring back…" : "Pop stash…",
      icon: <ListBullets size={14} />,
      tourId: "panel-option-stash-pick",
      disabled: stashes.length === 0 || saving,
      onSelect: () => setPickStash(true),
    },
  ] satisfies MenuItem[];

  const panelOptions = (
    <Menu
      align="right"
      minWidth={200}
      footer={view === "changes" ? stashFooter : undefined}
      forceOpen={forcePanelOptionsOpen}
      // Blocked mid-check: the working-tree state these actions read (undo/discard/set-aside)
      // could change out from under a click while a rescan or diff is in flight.
      disabled={checking}
      trigger={(open) => (
        <span
          title={checking ? "Checking for changes…" : "Panel options"}
          aria-label="Panel options"
          data-tour-id="panel-options"
          className={[
            "grid h-8 w-8 place-items-center rounded-button text-text-muted",
            checking
              ? "cursor-not-allowed opacity-40"
              : "transition-colors hover:bg-state-hover hover:text-text",
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
          tourId: "panel-option-undo",
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
                tourId: "panel-option-discard-all",
                disabled: changedPaths.length === 0 || saving,
                onSelect: () => {
                  setDiscardAllError(null);
                  setConfirmDiscardAll(true);
                },
              },
            ] satisfies MenuItem[])
          : []),
        ...setAsideItems,
      ]}
    />
  );

  return (
    <div className="relative flex shrink-0" style={{ width }}>
      <DockerPanel
        title={PANEL_TITLE[view]}
        className="flex-1"
        // Performance manages its own internal scroll (pinned recent-ops); others scroll whole.
        scroll={view !== "performance"}
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
                tourId="refresh"
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
                    data-tour-id="history-branch"
                    className="flex items-center gap-1.5 rounded-button px-1 py-0.5 hover:bg-state-hover"
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
                  onSelect: () => branchActions.switchTo(b.name),
                }))}
                footer={[
                  {
                    id: "new-branch",
                    label: artistMode ? "New version line…" : "New branch…",
                    icon: <Plus size={13} />,
                    onSelect: () => branchActions.askCreate(),
                  },
                ]}
              />
              <span className="text-[11px] text-text-muted">
                {commits.length} {artistMode ? "versions" : "commits"}
              </span>
            </div>

            {branchActions.error && (
              <p className="px-3 pt-2 text-[12px] text-danger">{branchActions.error}</p>
            )}

            <div data-tour-id="history-versions">
              <CommitGraph
                commits={commits}
                selectedId={selectedId}
                onSelect={onSelect}
                branches={branches}
              />
            </div>
          </>
        )}

        {view === "changes" && (
          <ChangesPanel
            currentBranch={currentBranch}
            focusedFile={focusedFile}
            onFocusFile={onFocusFile}
            items={workingItems}
            error={workingError}
          />
        )}

        {view === "branches" && <BranchesPanel branches={branches} onShowChanges={onShowChanges} />}

        {view === "performance" && <PerformancePanel />}
      </DockerPanel>

      {/* Resize handle — the bento gutter itself. Fills the 8px gap to the next
          card and stays invisible until reached for, so nothing draws a line
          between cards that the spacing already communicates. */}
      <div
        role="separator"
        aria-orientation="vertical"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        className={`group absolute -right-2 top-0 z-(--z-panel) flex h-full w-2 items-center justify-center ${cursorClass}`}
      >
        <div className="h-10 w-0.5 rounded-full bg-transparent transition-colors group-hover:bg-accent" />
      </div>

      {branchActions.dialogs}

      {/* The panel-options set-aside (the shelf actions). The save-first prompt has its own,
          inside `branchActions` — that one exists to unblock a switch, this one is the action. */}
      {setAside && (
        <SetAsideModal
          scope={setAside.scope}
          paths={setAside.paths}
          onClose={() => setSetAside(null)}
        />
      )}

      {pickStash && (
        <PickStashModal
          stashes={stashes}
          onClose={() => setPickStash(false)}
          onPick={(id) => void onPop(id)}
        />
      )}

      {stashConflict && (
        <StashConflictModal
          message={stashConflict}
          onClose={() => setStashConflict(null)}
          onShowChanges={onShowChanges}
        />
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
            This permanently reverts this artwork to its latest saved version. Everything painted
            since is lost — including Krita's undo history for it.
          </p>
          {discardAllError && <p className="mt-3 text-[12px] text-danger">{discardAllError}</p>}
        </Modal>
      )}
    </div>
  );
}
