import { useLayoutEffect, useMemo, useRef, useState } from "react";
import type { Branch, Commit } from "../../types";
import { CommitCard } from "./CommitCard";
import { CommitGraphRail } from "./CommitGraphRail";
import {
  branchColorMap,
  buildGraph,
  buildRevertLinks,
  elbowPath,
  laneX,
  REVERT_LINK_X,
} from "../../lib/graph";
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
  const revertLinks = useMemo(() => buildRevertLinks(commits), [commits]);

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

  // Rows are isolated per-row SVGs, so a revert link (which can span non-adjacent rows) needs
  // real pixel centers measured from the DOM — heights vary with wrapped commit messages.
  const containerRef = useRef<HTMLDivElement>(null);
  const rowRefs = useRef<(HTMLDivElement | null)[]>([]);
  const [rowCenters, setRowCenters] = useState<number[]>([]);

  useLayoutEffect(() => {
    if (revertLinks.length === 0) return;
    const container = containerRef.current;
    if (!container) return;
    const measure = () => {
      // Bound to commits.length, not rowRefs.current.length: switching to a branch with fewer
      // rows leaves stale trailing refs that would otherwise desync rowCenters.length.
      const next = commits.map((_, i) => {
        const el = rowRefs.current[i];
        return el ? el.offsetTop + el.offsetHeight / 2 : 0;
      });
      // Skip the render on width-only resizes (e.g. dragging the sidebar), which never move a row.
      setRowCenters((prev) =>
        prev.length === next.length && prev.every((v, i) => v === next[i]) ? prev : next
      );
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(container);
    return () => observer.disconnect();
  }, [commits, revertLinks.length]);

  return (
    <div ref={containerRef} className="relative flex flex-col">
      {commits.map((commit, i) => {
        const row = rows[i];
        const nodeColor = (commit.branch && branchColors.get(commit.branch)) || row.color;
        return (
          <div
            key={commit.id}
            ref={(el) => {
              rowRefs.current[i] = el;
            }}
            className="flex items-stretch"
          >
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

      {/* Elbow connector from a rollback commit back to the version it restored. */}
      {revertLinks.length > 0 && rowCenters.length === commits.length && (
        <svg className="pointer-events-none absolute inset-0 h-full w-full" aria-hidden>
          {revertLinks.map(({ fromIndex, toIndex }) => {
            const x1 = laneX(rows[fromIndex].nodeLane);
            const x2 = laneX(rows[toIndex].nodeLane);
            const y1 = rowCenters[fromIndex];
            const y2 = rowCenters[toIndex];
            const d = elbowPath(x1, y1, REVERT_LINK_X, x2, y2);
            return (
              <path
                key={`${fromIndex}-${toIndex}`}
                d={d}
                fill="none"
                stroke="var(--color-text-muted)"
                strokeWidth={1.5}
                strokeDasharray="3 3"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            );
          })}
        </svg>
      )}
    </div>
  );
}
