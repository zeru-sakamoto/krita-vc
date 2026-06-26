import { FilesIcon } from "@phosphor-icons/react";
import { DiffView } from "./vcs/DiffView";
import type { Commit, DiffEntry } from "../types";

interface MainPanelProps {
  commit: Commit | null;
  diff: DiffEntry[];
}

/**
 * The canvas zone — distinct darkest background, no border, fills completely.
 * Hosts the diff for the selected commit.
 * (DESIGN.md → Layout & App Shell → Canvas Area)
 */
export function MainPanel({ commit, diff }: MainPanelProps) {
  return (
    <main className="flex min-w-0 flex-1 flex-col bg-bg">
      {commit && diff.length > 0 ? (
        <DiffView entries={diff} />
      ) : (
        <div className="grid flex-1 place-items-center text-text-muted">
          <div className="flex flex-col items-center gap-2">
            <FilesIcon size={32} weight="regular" />
            <p className="text-[13px]">Select a commit to view its diff.</p>
          </div>
        </div>
      )}
    </main>
  );
}
