import { useMemo } from "react";
import type { Branch, Commit } from "../../types";
import { CommitCard } from "./CommitCard";
import { CommitGraphRail } from "./CommitGraphRail";
import { branchColorMap, buildGraph } from "../../lib/graph";
import { versionNumbers } from "../../lib/friendly";

interface CommitGraphProps {
  commits: Commit[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  /** When provided, branch tips get a name badge and nodes are colored per branch. */
  branches?: Branch[];
}

/**
 * History as a git-style graph: each version block is paired with a rail cell
 * that draws its node and the lane lines connecting it to its neighbors, so
 * branch divergence and merges read at a glance. Replaces the flat CommitList.
 */
export function CommitGraph({ commits, selectedId, onSelect, branches }: CommitGraphProps) {
  const versions = useMemo(() => versionNumbers(commits), [commits]);
  const { rows, laneCount } = useMemo(() => buildGraph(commits), [commits]);

  // Stable per-branch node colors (current branch = accent) + tip → branch labels.
  const currentBranch = branches?.find((b) => b.kind === "current")?.name;
  const branchColors = useMemo(
    () => branchColorMap(commits, currentBranch),
    [commits, currentBranch]
  );
  const tipsByCommit = useMemo(() => {
    const map = new Map<string, Branch[]>();
    for (const b of branches ?? []) {
      if (!b.tip) continue;
      map.set(b.tip, [...(map.get(b.tip) ?? []), b]);
    }
    return map;
  }, [branches]);

  return (
    <div className="flex flex-col">
      {commits.map((commit, i) => {
        const row = rows[i];
        const nodeColor = (commit.branch && branchColors.get(commit.branch)) || row.color;
        return (
          <div key={commit.id} className="flex items-stretch">
            <CommitGraphRail
              row={row.color === nodeColor ? row : { ...row, color: nodeColor }}
              laneCount={laneCount}
              selected={commit.id === selectedId}
            />
            <div className="min-w-0 flex-1">
              <CommitCard
                commit={commit}
                version={versions.get(commit.id) ?? 0}
                selected={commit.id === selectedId}
                onSelect={onSelect}
                tips={tipsByCommit.get(commit.id)}
              />
            </div>
          </div>
        );
      })}
    </div>
  );
}
