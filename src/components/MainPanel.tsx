import { FilesIcon, WarningIcon } from "@phosphor-icons/react";
import { DiffView } from "./vcs/DiffView";
import type { DiffEntry } from "../types";

interface MainPanelProps {
  diff: DiffEntry[];
  /** Backend diff error, shown in place of the empty state so failures aren't silently blank. */
  error?: string | null;
  /** Prompt shown when there's nothing to display and no error (context-dependent). */
  emptyHint?: string;
}

/**
 * The canvas zone — distinct darkest background, no border, fills completely.
 * Hosts the diff for the selected commit or focused working-tree file.
 * (DESIGN.md → Layout & App Shell → Canvas Area)
 */
export function MainPanel({ diff, error, emptyHint }: MainPanelProps) {
  if (diff.length > 0) {
    return (
      <main className="flex min-w-0 flex-1 flex-col bg-bg">
        <DiffView entries={diff} />
      </main>
    );
  }

  return (
    <main className="flex min-w-0 flex-1 flex-col bg-bg">
      <div className="grid flex-1 place-items-center text-text-muted">
        {error ? (
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
