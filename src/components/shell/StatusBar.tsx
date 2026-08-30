import { GitBranch } from "@phosphor-icons/react";
import { ICON } from "../../lib/iconSize";
import { assetName } from "../../lib/friendly";
import { useArtistMode } from "../../lib/artistMode";
import { useRepository } from "../../lib/repository";
import { inTauri } from "../../lib/tauri";
import { Tooltip } from "../ui/Tooltip";

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
    <footer className="relative flex h-6 shrink-0 items-center justify-between bg-surface px-3 text-caption text-text-muted">
      {/* Indeterminate save progress — only while a commit is being written */}
      {saving && (
        <div
          role="progressbar"
          aria-label="Saving version"
          // top-0, not -top-px: the border this used to straddle is gone now
          // that the status bar is a seamless edge of the chrome frame.
          className="absolute inset-x-0 top-0 h-0.5 overflow-hidden"
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

      {/* Right zone — browser-preview badge (no backend outside the desktop shell), branch, count */}
      <div className="flex shrink-0 items-center gap-2.5">
        {!inTauri() && (
          <>
            <Tooltip label="No backend in the browser. Run the desktop app to work with real repositories.">
              {/* The border, not the fill, is what makes this read as a chip on the two light
                  themes: there `--warning` is already a pale #f6ecc9, so a 20% wash of it over a
                  cream page is invisible and the badge degraded to bare text. `--warning-fg` is
                  the one warning token defined to contrast with its own theme's background, so
                  the outline holds in all eight. */}
              <span className="rounded-badge border border-warning-fg/40 bg-warning/20 px-1.5 py-px font-medium uppercase tracking-wide text-warning-fg">
                Browser preview
              </span>
            </Tooltip>
            <Separator />
          </>
        )}
        <span className="flex items-center gap-1">
          <GitBranch size={ICON.inline} />
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
