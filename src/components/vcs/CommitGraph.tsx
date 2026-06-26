import { useMemo } from "react";
import type { Commit } from "../../types";
import { CommitCard } from "./CommitCard";
import { CommitGraphRail } from "./CommitGraphRail";
import { buildGraph } from "../../lib/graph";
import { versionNumbers } from "../../lib/friendly";

interface CommitGraphProps {
  commits: Commit[];
  selectedId: string | null;
  onSelect: (id: string) => void;
}

/**
 * History as a git-style graph: each version block is paired with a rail cell
 * that draws its node and the lane lines connecting it to its neighbors, so
 * branch divergence and merges read at a glance. Replaces the flat CommitList.
 */
export function CommitGraph({ commits, selectedId, onSelect }: CommitGraphProps) {
  const versions = useMemo(() => versionNumbers(commits), [commits]);
  const { rows, laneCount } = useMemo(() => buildGraph(commits), [commits]);

  return (
    <div className="flex flex-col">
      {commits.map((commit, i) => (
        <div key={commit.id} className="flex items-stretch">
          <CommitGraphRail
            row={rows[i]}
            laneCount={laneCount}
            selected={commit.id === selectedId}
          />
          <div className="min-w-0 flex-1">
            <CommitCard
              commit={commit}
              version={versions.get(commit.id) ?? 0}
              selected={commit.id === selectedId}
              onSelect={onSelect}
            />
          </div>
        </div>
      ))}
    </div>
  );
}
