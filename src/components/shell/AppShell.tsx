import { useEffect, useMemo, useState } from "react";
import { PaintBrush, SidebarSimple } from "@phosphor-icons/react";
import { ActivityBar, type ActivityView } from "./ActivityBar";
import { Sidebar } from "./Sidebar";
import { Inspector } from "./Inspector";
import { StatusBar } from "./StatusBar";
import { TopBar } from "./TopBar";
import { MainPanel } from "../MainPanel";
import { IconButton } from "../ui/IconButton";
import { MOCK_BRANCHES } from "../../data/mockData";
import { useArtistMode } from "../../lib/artistMode";
import { useRepository } from "../../lib/repository";
import { useCommits, useCommitDiff, useWorkingDiff } from "../../lib/repoData";
import { versionLabel, versionNumbers, assetName } from "../../lib/friendly";

/**
 * Root application shell — owns layout + view state, wires the four zones
 * (Activity bar | Sidebar | Main | Inspector) plus the bottom status bar.
 * (DESIGN.md → Layout & App Shell)
 */
export function AppShell() {
  const { artistMode, toggle: toggleArtistMode } = useArtistMode();
  const { current, refreshNonce } = useRepository();
  const commits = useCommits(current.path, refreshNonce);
  const [activeView, setActiveView] = useState<ActivityView>("history");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [focusedFile, setFocusedFile] = useState<string | null>(null);
  const [inspectorOpen, setInspectorOpen] = useState(true);

  // Keep a valid selection as history loads/changes (default to the newest commit).
  useEffect(() => {
    if (commits.length === 0) {
      setSelectedId(null);
    } else if (!commits.some((c) => c.id === selectedId)) {
      setSelectedId(commits[0].id);
    }
  }, [commits, selectedId]);

  const currentBranch = useMemo(
    () => MOCK_BRANCHES.find((b) => b.kind === "current") ?? MOCK_BRANCHES[0],
    []
  );
  const versions = useMemo(() => versionNumbers(commits), [commits]);
  const selectedCommit = useMemo(
    () => commits.find((c) => c.id === selectedId) ?? null,
    [commits, selectedId]
  );
  const selectedVersion = selectedId ? (versions.get(selectedId) ?? 0) : 0;

  // In the Changes view, a clicked file shows its working-tree diff; otherwise the selected
  // commit's diff. Both hooks run (the inactive one gets a null id and stays empty).
  const showWorking = activeView === "changes" && focusedFile != null;
  const commitDiff = useCommitDiff(current.path, selectedId, refreshNonce);
  const workingDiff = useWorkingDiff(current.path, showWorking ? focusedFile : null, refreshNonce);
  const { entries: diff, error: diffError } = showWorking ? workingDiff : commitDiff;
  const activeFile = diff[0]?.path ?? null;

  const emptyHint =
    activeView === "changes"
      ? "Select a changed file to preview."
      : artistMode
        ? "Select a version to view its changes."
        : "Select a commit to view its diff.";

  return (
    <div className="flex h-screen w-screen flex-col overflow-hidden bg-bg text-text">
      <TopBar />

      {/* Horizontal zones */}
      <div className="flex min-h-0 flex-1">
        <ActivityBar active={activeView} onChange={setActiveView} />

        <div className="flex min-w-0 flex-1 border-l border-border">
          <Sidebar
            view={activeView}
            commits={commits}
            currentBranch={currentBranch}
            selectedId={selectedId}
            onSelect={setSelectedId}
            focusedFile={focusedFile}
            onFocusFile={setFocusedFile}
          />

          <div className="flex min-w-0 flex-1 flex-col border-l border-border">
            {/* Toolbar — commit context (left) + inspector toggle (right) */}
            <div className="flex h-9 shrink-0 items-center gap-2 border-b border-border bg-surface-2 pl-3 pr-1">
              {showWorking ? (
                <>
                  <span className="rounded-badge bg-surface-3 px-1.5 py-0.5 text-[11px] text-text-muted">
                    Unsaved changes
                  </span>
                  <span className="min-w-0 flex-1 truncate text-[13px] text-text">
                    {artistMode ? assetName(focusedFile) : focusedFile}
                  </span>
                </>
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
              <button
                type="button"
                onClick={toggleArtistMode}
                title="Toggle between artist-friendly labels and the full technical view"
                aria-pressed={artistMode}
                className={[
                  "flex items-center gap-1.5 rounded-button px-2 py-1 text-[12px] transition-colors",
                  artistMode
                    ? "bg-accent/15 text-accent"
                    : "text-text-muted hover:bg-white/5 hover:text-text",
                ].join(" ")}
              >
                <PaintBrush size={14} weight={artistMode ? "fill" : "regular"} />
                Artist view
              </button>
              <IconButton
                icon={SidebarSimple}
                label={inspectorOpen ? "Hide inspector" : "Show inspector"}
                active={inspectorOpen}
                size={18}
                onClick={() => setInspectorOpen((v) => !v)}
              />
            </div>

            <div className="flex min-h-0 flex-1">
              <MainPanel diff={diff} error={diffError} emptyHint={emptyHint} />
              {inspectorOpen && (
                <Inspector
                  commit={selectedCommit}
                  version={selectedVersion}
                  onClose={() => setInspectorOpen(false)}
                />
              )}
            </div>
          </div>
        </div>
      </div>

      <StatusBar
        activeFile={activeFile}
        dirty
        branch={currentBranch.name}
        commitCount={commits.length}
      />
    </div>
  );
}
