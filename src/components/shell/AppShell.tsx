import { useCallback, useEffect, useMemo, useState } from "react";
import { FolderOpen, SidebarSimple } from "@phosphor-icons/react";
import { ActivityBar, type ActivityView } from "./ActivityBar";
import { BusyOverlay } from "./BusyOverlay";
import { Sidebar } from "./Sidebar";
import { Inspector } from "./Inspector";
import { StatusBar } from "./StatusBar";
import { TopBar } from "./TopBar";
import { TourOverlay } from "./TourOverlay";
import { MainPanel } from "../MainPanel";
import { VersionMapPanel } from "../vcs/VersionMapPanel";
import { IconButton } from "../ui/IconButton";
import { useArtistMode } from "../../lib/artistMode";
import { useTour } from "../../lib/tour";
import { useLegacyHistory } from "../../lib/legacyHistory";
import { useRepository } from "../../lib/repository";
import {
  useBranches,
  useCommits,
  useCommitDiff,
  useWorkingChanges,
  useWorkingDiff,
  type DiffResult,
} from "../../lib/repoData";
import { versionLabel, versionNumbers, assetName } from "../../lib/friendly";
import type { Repository } from "../../types";

/**
 * Root application shell — owns layout + view state.
 *
 * Framed bento: the root div is one continuous `bg-surface` chrome frame, and
 * TopBar / ActivityBar / StatusBar are seamless zones within it (no borders
 * between them, none at the window edge). Everything else is a recessed
 * `bg-bg` well, inset 8px on all four sides so the frame closes visually on
 * the right too, holding the Sidebar / Main / Inspector cards.
 * Splits on the selected repository so `RepoShell`'s data hooks always have
 * a real path; with no repository yet, a welcome state points at the switcher.
 * (DESIGN.md → Layout & App Shell)
 */
export function AppShell() {
  const { current } = useRepository();
  return (
    <>
      {current ? <RepoShell repo={current} /> : <WelcomeShell />}
      <BusyOverlay />
    </>
  );
}

/** Fresh install / empty list: just the top bar and a pointer to it. */
function WelcomeShell() {
  return (
    <div className="flex h-screen w-screen flex-col overflow-hidden bg-surface text-text">
      <TopBar />
      {/* mx-2 mb-2 — same closed frame as RepoShell; there's no activity bar or
          status bar here, so the margin supplies all three missing edges. */}
      <div className="mx-2 mb-2 grid min-h-0 flex-1 place-items-center rounded-well bg-bg p-2">
        <div className="raised flex max-w-sm flex-col items-center gap-3 rounded-panel bg-surface px-8 py-10 text-center">
          <FolderOpen size={40} className="text-text-muted" />
          <h1 className="text-[15px] font-medium">No artwork yet</h1>
          <p className="text-[13px] leading-relaxed text-text-muted">
            Use the switcher in the top-left corner to pick a Krita artwork to track. Its version
            history will appear here.
          </p>
        </div>
      </div>
    </div>
  );
}

function RepoShell({ repo }: { repo: Repository }) {
  const { artistMode } = useArtistMode();
  const { beginIfFirstTime, setConditions } = useTour();
  const { legacy } = useLegacyHistory();
  const { refreshNonce, setScanning } = useRepository();
  const commits = useCommits(repo.path, refreshNonce);
  const branches = useBranches(repo.path, refreshNonce);
  // One scan of the tracked artwork for the whole shell. Lifted out of `Sidebar` because the
  // Version Map needs the same dirty flag and the Sidebar isn't mounted in map view — and a
  // second `useWorkingChanges` in the (always-mounted) map would both rescan in every view and
  // race the one `setScanning` boolean.
  const { items: workingItems, error: workingError } = useWorkingChanges(
    repo.path,
    refreshNonce,
    setScanning
  );
  const [activeView, setActiveView] = useState<ActivityView>("map");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [focusedFile, setFocusedFile] = useState<string | null>(null);
  const [inspectorOpen, setInspectorOpen] = useState(true);
  // Which layer/composite the diff navigator has selected — mirrored into the Inspector.
  const [focus, setFocus] = useState<{ path: string; id: string } | null>(null);
  // Which file (among a multi-file commit) the Inspector's file list has selected, and an
  // optional navigator id to seed that file's view with (e.g. jump straight to its palette).
  const [selectedFile, setSelectedFile] = useState<string | null>(null);
  const [selectedFocusId, setSelectedFocusId] = useState<string | undefined>(undefined);

  // eslint-disable-next-line react-hooks/exhaustive-deps -- fire once per RepoShell mount only
  useEffect(() => {
    beginIfFirstTime();
  }, []);

  // Which tour steps have anything to point at. Reported live rather than read once at the start:
  // the tour fires on mount, while `useCommits`/`useBranches` are still in flight.
  useEffect(() => {
    setConditions({
      legacy,
      hasVersions: commits.length > 0,
      hasOtherBranches: branches.length > 1,
      dirty: workingItems.length > 0,
    });
  }, [setConditions, legacy, commits.length, branches.length, workingItems.length]);

  // Turning legacy history off while sitting on History/Branches would strand the user on a view
  // with no icon to leave it by — fall back to the map.
  useEffect(() => {
    if (!legacy && (activeView === "history" || activeView === "branches")) setActiveView("map");
  }, [legacy, activeView]);

  // Keep a valid selection as history loads/changes (default to the newest commit).
  useEffect(() => {
    if (commits.length === 0) {
      setSelectedId(null);
    } else if (!commits.some((c) => c.id === selectedId)) {
      setSelectedId(commits[0].id);
    }
  }, [commits, selectedId]);

  // Placeholder shape while branches load — a fresh repo always has "main".
  const currentBranch = useMemo(
    () =>
      branches.find((b) => b.kind === "current") ??
      branches[0] ?? { name: "main", kind: "current" as const },
    [branches]
  );
  const versions = useMemo(() => versionNumbers(commits), [commits]);
  const selectedCommit = useMemo(
    () => commits.find((c) => c.id === selectedId) ?? null,
    [commits, selectedId]
  );
  const selectedVersion = selectedId ? (versions.get(selectedId) ?? 0) : 0;

  // The Changes view never shows History's selection, focused file or not — a leftover
  // `selectedId`/`selectedCommit` from History must not leak into the toolbar, canvas, or
  // Inspector once the user switches tabs. `showWorking` narrows further: only true once a
  // changed file is actually focused, which is when there's a real working-tree diff to fetch.
  // Hoisted, not an inline arrow at the call sites below: `VersionMapPanel` folds this into a
  // memoized node array, so a fresh identity each render hands React Flow a brand-new array of
  // every node — on every AppShell render, which while the tree is dirty is the normal case.
  const showChanges = useCallback(() => setActiveView("changes"), []);

  const inChanges = activeView === "changes";
  // The map owns the whole well: no sidebar, no inspector — the node carries the metadata.
  const inMap = activeView === "map";
  // Performance keeps its own sidebar, but once Legacy version history is off there's no
  // commit selection driving a diff anymore — show the map beside the stats instead of a
  // diff viewer with nothing meaningful selected.
  const perfShowsMap = activeView === "performance" && !legacy;
  const showMap = inMap || perfShowsMap;
  const showWorking = inChanges && focusedFile != null;
  const commitDiff = useCommitDiff(repo.path, selectedId);
  const workingDiff = useWorkingDiff(repo.path, showWorking ? focusedFile : null, refreshNonce);
  const emptyDiff: DiffResult = { entries: [], error: null, loading: false };
  const {
    entries: diff,
    error: diffError,
    loading: diffLoading,
  } = inChanges ? (showWorking ? workingDiff : emptyDiff) : commitDiff;
  const activeFile = diff[0]?.path ?? null;

  // Keep a valid file selection as the diff loads/changes (default to the first entry).
  // "Top-level" excludes embedded palettes (`<kra>::<palette-file>`), which aren't
  // independently selectable — they're reached via their parent .kra's sub-row instead.
  useEffect(() => {
    const topLevelPaths = new Set(
      diff.filter((e) => !(e.kind === "palette" && e.path.includes("::"))).map((e) => e.path)
    );
    if (!selectedFile || !topLevelPaths.has(selectedFile)) {
      setSelectedFile(diff[0]?.path ?? null);
      setSelectedFocusId(undefined);
    }
  }, [diff, selectedFile]);

  const emptyHint = inChanges
    ? "Select a changed file to preview."
    : artistMode
      ? "Select a version to view its changes."
      : "Select a commit to view its diff.";

  return (
    <div className="flex h-screen w-screen flex-col overflow-hidden bg-surface text-text">
      <TopBar />

      {/* Horizontal zones — ActivityBar is the frame's left edge; everything
          right of it is the recessed well holding the bento cards. */}
      <div className="flex min-h-0 flex-1">
        <ActivityBar active={activeView} onChange={setActiveView} />

        {/* mr-2 leaves an 8px strip of frame down the right edge, so the chrome
            closes into a full ring instead of a C open to the right. */}
        <div className="mr-2 flex min-w-0 flex-1 gap-2 rounded-well bg-bg p-2">
          {!inMap && (
            <Sidebar
              view={activeView}
              commits={commits}
              branches={branches}
              currentBranch={currentBranch}
              selectedId={selectedId}
              onSelect={setSelectedId}
              focusedFile={focusedFile}
              onFocusFile={setFocusedFile}
              workingItems={workingItems}
              workingError={workingError}
              // Whatever diff the well is currently showing — the working diff in Changes, the
              // selected commit's in History. Undo/discard/set-aside all act on state this diff
              // reads, so they're blocked while it's still being computed too, not just mid-scan.
              diffLoading={diffLoading}
              onShowChanges={showChanges}
            />
          )}

          {/* Always mounted, never conditionally rendered — pan/zoom and the open-version
              drilldown live inside VersionMapPanel's own state, so remounting it on every tab
              switch (map <-> changes <-> performance) would silently reset both. Only its
              visibility toggles. */}
          <div className={showMap ? "flex min-h-0 min-w-0 flex-1" : "hidden"}>
            <VersionMapPanel
              repoPath={repo.path}
              commits={commits}
              branches={branches}
              currentBranch={currentBranch}
              nonce={refreshNonce}
              dirty={workingItems.length > 0}
              onShowChanges={showChanges}
            />
          </div>

          {!showMap && (
            <>
              <div className="raised flex min-w-0 flex-1 flex-col overflow-hidden rounded-panel bg-surface">
                {/* Card header — commit context (left) + inspector toggle (right).
                Matches DockerPanel's header so every card reads the same. */}
                <div className="flex h-10 shrink-0 items-center gap-2 border-b border-border bg-surface-2 pl-3 pr-1">
                  {inChanges ? (
                    showWorking ? (
                      <>
                        <span className="rounded-badge bg-surface-3 px-1.5 py-0.5 text-[11px] text-text-muted">
                          Unsaved changes
                        </span>
                        <span className="min-w-0 flex-1 truncate text-[13px] text-text">
                          {artistMode ? assetName(focusedFile) : focusedFile}
                        </span>
                      </>
                    ) : (
                      <span className="flex-1 text-[13px] text-text-muted">No changes to show</span>
                    )
                  ) : selectedCommit ? (
                    <>
                      <span
                        className={[
                          "text-[12px] text-text-muted",
                          artistMode ? "font-medium" : "font-mono",
                        ].join(" ")}
                      >
                        {artistMode ? versionLabel(selectedVersion) : selectedCommit.hash}
                      </span>
                      <span className="min-w-0 flex-1 truncate text-[13px] text-text">
                        {selectedCommit.message}
                      </span>
                    </>
                  ) : (
                    <span className="flex-1 text-[13px] text-text-muted">
                      {artistMode ? "No version selected" : "No commit selected"}
                    </span>
                  )}
                  {!inspectorOpen && (
                    <IconButton
                      icon={SidebarSimple}
                      label="Show inspector"
                      size={18}
                      onClick={() => setInspectorOpen(true)}
                      tourId="inspector"
                      iconClassName="-scale-x-100"
                    />
                  )}
                </div>

                <MainPanel
                  diff={diff}
                  error={diffError}
                  loading={diffLoading}
                  emptyHint={emptyHint}
                  repoPath={repo.path}
                  commitId={inChanges ? null : selectedId}
                  working={showWorking}
                  nonce={refreshNonce}
                  onFocus={setFocus}
                  selectedFile={selectedFile}
                  focusId={selectedFocusId}
                />
              </div>

              {inspectorOpen && (
                <Inspector
                  commit={inChanges ? null : selectedCommit}
                  version={selectedVersion}
                  entries={diff}
                  focus={focus}
                  working={inChanges}
                  focusedFile={focusedFile}
                  isTip={selectedCommit != null && selectedCommit.id === currentBranch.tip}
                  onHide={() => setInspectorOpen(false)}
                  selectedFile={selectedFile}
                  onSelectFile={(path, focusId) => {
                    setSelectedFile(path);
                    setSelectedFocusId(focusId);
                  }}
                />
              )}
            </>
          )}
        </div>
      </div>

      <StatusBar
        activeFile={activeFile}
        dirty
        branch={currentBranch.name}
        commitCount={commits.length}
      />
      <TourOverlay setActiveView={setActiveView} />
    </div>
  );
}
