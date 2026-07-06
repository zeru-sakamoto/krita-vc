import { memo } from "react";
import type { GraphRow } from "../../lib/graph";

const LANE_W = 16;
const ORIGIN_X = 12;

export function railWidth(laneCount: number): number {
  return ORIGIN_X + Math.max(1, laneCount) * LANE_W;
}

function laneX(lane: number): number {
  return ORIGIN_X + lane * LANE_W;
}

/**
 * The graph rail for one commit row: vertical lane lines + branch/merge
 * diagonals (an SVG that stretches to the row height, with a non-scaling stroke
 * so line weight stays constant), plus the node dot centered on its lane.
 * Memoized — rows are stable (buildGraph is useMemo'd) so a selection change
 * only re-renders the two affected rails, not every SVG in the list.
 */
export const CommitGraphRail = memo(function CommitGraphRail({
  row,
  laneCount,
  selected,
}: {
  row: GraphRow;
  laneCount: number;
  selected: boolean;
}) {
  const width = railWidth(laneCount);

  return (
    <div className="relative shrink-0 self-stretch" style={{ width }} aria-hidden>
      <svg
        className="absolute inset-0 h-full w-full"
        viewBox={`0 0 ${width} 100`}
        preserveAspectRatio="none"
      >
        {row.segments.map((s, i) => {
          const x1 = laneX(s.x1);
          const x2 = laneX(s.x2);
          const d =
            x1 === x2
              ? `M ${x1} ${s.y1} L ${x2} ${s.y2}`
              : `M ${x1} ${s.y1} C ${x1} ${(s.y1 + s.y2) / 2} ${x2} ${(s.y1 + s.y2) / 2} ${x2} ${s.y2}`;
          return (
            <path
              key={i}
              d={d}
              fill="none"
              stroke={s.color}
              strokeWidth={1.5}
              strokeLinecap="round"
              vectorEffect="non-scaling-stroke"
            />
          );
        })}
      </svg>

      {/* Node dot, centered on its lane and row */}
      <span
        className="absolute -translate-x-1/2 -translate-y-1/2 rounded-full border border-bg"
        style={{
          left: laneX(row.nodeLane),
          top: "50%",
          width: row.merge ? 11 : 9,
          height: row.merge ? 11 : 9,
          backgroundColor: row.color,
          boxShadow: selected
            ? "0 0 0 3px color-mix(in oklab, var(--color-accent) 35%, transparent)"
            : undefined,
        }}
      />
      {row.merge && (
        <span
          className="absolute -translate-x-1/2 -translate-y-1/2 rounded-full bg-bg"
          style={{ left: laneX(row.nodeLane), top: "50%", width: 4, height: 4 }}
        />
      )}
    </div>
  );
});
