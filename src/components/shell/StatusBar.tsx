import { GitBranch } from "@phosphor-icons/react";
import { assetName } from "../../lib/friendly";
import { useArtistMode } from "../../lib/artistMode";
import { useRepository } from "../../lib/repository";

interface StatusBarProps {
  /** Currently focused file (left zone) */
  activeFile: string | null;
  /** True if there are unsaved/uncommitted changes — shows a `·` prefix */
  dirty?: boolean;
  branch: string;
  commitCount: number;
}

function Separator() {
  return <span className="h-3 w-px bg-border" aria-hidden />;
}

/**
 * 24px status bar fixed at the bottom of the shell.
 * (DESIGN.md → VCS Component Patterns → Status Bar)
 */
export function StatusBar({ activeFile, dirty, branch, commitCount }: StatusBarProps) {
  const { artistMode } = useArtistMode();
  const { saving } = useRepository();
  return (
    <footer className="relative flex h-6 shrink-0 items-center justify-between border-t border-border bg-surface px-3 text-[11px] text-text-muted">
      {/* Indeterminate save progress — only while a commit is being written */}
      {saving && (
        <div
          role="progressbar"
          aria-label="Saving version"
          className="absolute inset-x-0 -top-px h-0.5 overflow-hidden"
        >
          <div className="h-full w-2/5 animate-indeterminate rounded-full bg-accent" />
        </div>
      )}
      {/* Left zone — active file */}
      <div className="flex min-w-0 items-center gap-1.5">
        {activeFile ? (
          <span className={["truncate", artistMode ? "" : "font-mono"].join(" ")}>
            {dirty && <span className="text-warning-fg">· </span>}
            {artistMode ? assetName(activeFile) : activeFile}
          </span>
        ) : (
          <span>No file</span>
        )}
      </div>

      {/* Right zone — mock badge, branch, count */}
      <div className="flex shrink-0 items-center gap-2.5">
        <span
          className="rounded-badge bg-warning/20 px-1.5 py-px font-medium uppercase tracking-wide text-warning-fg"
          title="The UI is running on mock data — no backend is wired up yet."
        >
          Mock Data
        </span>
        <Separator />
        <span className="flex items-center gap-1">
          <GitBranch size={12} />
          {branch}
        </span>
        <Separator />
        <span>
          {commitCount} {artistMode ? "versions" : "commits"}
        </span>
      </div>
    </footer>
  );
}
