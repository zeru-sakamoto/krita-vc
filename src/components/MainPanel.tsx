import { CircleNotchIcon, FilesIcon, WarningIcon } from "@phosphor-icons/react";
import { DiffView } from "./vcs/DiffView";
import type { DiffEntry } from "../types";

interface MainPanelProps {
  diff: DiffEntry[];
  /** Backend diff error, shown in place of the empty state so failures aren't silently blank. */
  error?: string | null;
  /** True while the diff is being computed — shows a spinner instead of the empty state. */
  loading?: boolean;
  /** Prompt shown when there's nothing to display and no error (context-dependent). */
  emptyHint?: string;
  /** Diff source, threaded to art views so they can lazily fetch per-layer rasters. */
  repoPath?: string;
  commitId?: string | null;
  working?: boolean;
  nonce?: number;
  /** Forwarded to the diff viewer so the navigator selection reaches the Inspector. */
  onFocus?: (f: { path: string; id: string }) => void;
  /** Which file (among several in the current diff) to show — from the Inspector's file list. */
  selectedFile?: string | null;
  /** Navigator id to seed the selected file's view with, e.g. jump straight to its palette. */
  focusId?: string;
}

/**
 * The canvas zone — distinct darkest background, no border, fills completely.
 * Hosts the diff for the selected commit or focused working-tree file.
 * (DESIGN.md → Layout & App Shell → Canvas Area)
 */
export function MainPanel({
  diff,
  error,
  loading,
  emptyHint,
  repoPath,
  commitId,
  working,
  nonce,
  onFocus,
  selectedFile,
  focusId,
}: MainPanelProps) {
  if (diff.length > 0) {
    return (
      // min-h-0 is load-bearing: without it this flex column can inflate to fit its content
      // instead of the stretched height from its row parent — the Swipe slider's frame sizes
      // itself with `h-full` (a percentage, needing a genuinely resolved ancestor height), so
      // it would silently overflow downward with only its top visible. Split view masks the
      // same missing constraint because each Pane sizes via flex-1/min-h-0 instead.
      <main className="flex min-h-0 min-w-0 flex-1 flex-col bg-bg">
        <DiffView
          entries={diff}
          repoPath={repoPath}
          commitId={commitId}
          working={working}
          nonce={nonce}
          onFocus={onFocus}
          selectedPath={selectedFile}
          focusId={focusId}
        />
      </main>
    );
  }

  return (
    <main className="flex min-h-0 min-w-0 flex-1 flex-col bg-bg">
      <div className="grid flex-1 place-items-center text-text-muted">
        {loading ? (
          <div className="flex flex-col items-center gap-2">
            <CircleNotchIcon size={28} className="animate-spin text-accent" />
            <p className="text-[13px]">Analyzing changes…</p>
          </div>
        ) : error ? (
          <div className="flex max-w-sm flex-col items-center gap-2 px-4 text-center">
            <WarningIcon size={32} weight="regular" className="text-danger" />
            <p className="text-[13px]">Couldn’t build this diff.</p>
            <p className="text-[12px] text-text-muted/80">{error}</p>
          </div>
        ) : (
          <div className="flex flex-col items-center gap-2">
            <FilesIcon size={32} weight="regular" />
            <p className="text-[13px]">{emptyHint ?? "Select a commit to view its diff."}</p>
          </div>
        )}
      </div>
    </main>
  );
}
