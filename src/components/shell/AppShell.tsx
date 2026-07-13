import { useEffect, useMemo, useState } from "react";
import { FolderOpen, SidebarSimple } from "@phosphor-icons/react";
import { ActivityBar, type ActivityView } from "./ActivityBar";
import { BusyOverlay } from "./BusyOverlay";
import { Sidebar } from "./Sidebar";
import { Inspector } from "./Inspector";
import { StatusBar } from "./StatusBar";
import { TopBar } from "./TopBar";
import { MainPanel } from "../MainPanel";
import { IconButton } from "../ui/IconButton";
import { useArtistMode } from "../../lib/artistMode";
import { useRepository } from "../../lib/repository";
import { useBranches, useCommits, useCommitDiff, useWorkingDiff } from "../../lib/repoData";
import { versionLabel, versionNumbers, assetName } from "../../lib/friendly";
import type { Repository } from "../../types";

/**
 * Root application shell — owns layout + view state, wires the four zones
 * (Activity bar | Sidebar | Main | Inspector) plus the bottom status bar.
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
    <div className="flex h-screen w-screen flex-col overflow-hidden bg-bg text-text">
      <TopBar />
      <div className="grid min-h-0 flex-1 place-items-center">
        <div className="flex max-w-sm flex-col items-center gap-3 text-center">
          <FolderOpen size={40} className="text-text-muted" />
          <h1 className="text-[15px] font-medium">No repository yet</h1>
          <p className="text-[13px] leading-relaxed text-text-muted">
            Use the switcher in the top-left corner to create a repository or open an existing
            folder of artwork. Its version history will appear here.
          </p>
        </div>
      </div>
    </div>
  );
}

function RepoShell({ repo }: { repo: Repository }) {
  const { artistMode } = useArtistMode();
  const { refreshNonce } = useRepository();
  const commits = useCommits(repo.path, refreshNonce);
  const branches = useBranches(repo.path, refreshNonce);
  const [activeView, setActiveView] = useState<ActivityView>("history");
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [focusedFile, setFocusedFile] = useState<string | null>(null);
  const [inspectorOpen, setInspectorOpen] = useState(true);
  // Which layer/composite the diff navigator has selected — mirrored into the Inspector.
  const [focus, setFocus] = useState<{ path: string; id: string } | null>(null);

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

  // In the Changes view, a clicked file shows its working-tree diff; otherwise the selected
  // commit's diff. Both hooks run (the inactive one gets a null id and stays empty).
  const showWorking = activeView === "changes" && focusedFile != null;
  const commitDiff = useCommitDiff(repo.path, selectedId);
  const workingDiff = useWorkingDiff(repo.path, showWorking ? focusedFile : null, refreshNonce);
  const {
    entries: diff,
    error: diffError,
    loading: diffLoading,
  } = showWorking ? workingDiff : commitDiff;
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
            branches={branches}
            currentBranch={currentBranch}
            selectedId={selectedId}
            onSelect={setSelectedId}
            focusedFile={focusedFile}
            onFocusFile={setFocusedFile}
            onShowChanges={() => setActiveView("changes")}
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
              <IconButton
                icon={SidebarSimple}
                label={inspectorOpen ? "Hide inspector" : "Show inspector"}
                active={inspectorOpen}
                size={18}
                onClick={() => setInspectorOpen((v) => !v)}
              />
            </div>

            <div className="flex min-h-0 flex-1">
              <MainPanel
                diff={diff}
                error={diffError}
                loading={diffLoading}
                emptyHint={emptyHint}
                repoPath={repo.path}
                commitId={showWorking ? null : selectedId}
                working={showWorking}
                nonce={refreshNonce}
                onFocus={setFocus}
              />
              {inspectorOpen && (
                <Inspector
                  commit={selectedCommit}
                  version={selectedVersion}
                  entries={diff}
                  focus={focus}
                  isTip={selectedCommit != null && selectedCommit.id === currentBranch.tip}
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
