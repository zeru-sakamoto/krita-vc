import type { Commit } from "../types";

/**
 * Lane-assignment layout for the history graph. Turns a list of commits
 * (newest first, each carrying `parents`) into a per-row drawing model: a node
 * column plus ready-to-draw line segments. Lanes are never compacted, so a
 * lane keeps its column for its whole lifetime — pass-through lanes are simple
 * verticals and only branch/merge points produce diagonals.
 *
 * Coordinates are lane-relative: `x` is a lane index, `y` is 0 (top edge),
 * 50 (node anchor, vertically centered in the row) or 100 (bottom edge). The
 * renderer maps those to pixels. Colors are CSS variable references so they
 * track the theme tokens.
 */

// Functional palette for lanes (readability of distinct branches). Lane 0 is the
// mainline → accent; the rest cycle through existing status tokens.
const LANE_COLORS = [
  "var(--color-accent)",
  "var(--color-info-fg)",
  "var(--color-success-fg)",
  "var(--color-warning-fg)",
];

export function laneColor(lane: number): string {
  return LANE_COLORS[lane % LANE_COLORS.length];
}

export interface GraphSegment {
  x1: number;
  y1: number;
  x2: number;
  y2: number;
  color: string;
}

export interface GraphRow {
  commitId: string;
  nodeLane: number;
  color: string;
  /** A merge commit (more than one parent). */
  merge: boolean;
  segments: GraphSegment[];
}

export interface GraphLayout {
  rows: GraphRow[];
  laneCount: number;
}

function firstFree(lanes: (string | null)[]): number {
  const idx = lanes.indexOf(null);
  if (idx !== -1) return idx;
  lanes.push(null);
  return lanes.length - 1;
}

export function buildGraph(commits: Commit[]): GraphLayout {
  const lanes: (string | null)[] = [];
  const rows: GraphRow[] = [];
  let laneCount = 0;

  for (const commit of commits) {
    const before = [...lanes];

    // The node's lane: an existing reservation, else a fresh lane.
    let nodeLane = before.indexOf(commit.id);
    if (nodeLane === -1) nodeLane = firstFree(lanes);

    // Free every lane reserved for this commit (multiple = converging branches).
    for (let i = 0; i < lanes.length; i++) {
      if (lanes[i] === commit.id) lanes[i] = null;
    }

    const parents = commit.parents ?? [];
    if (parents.length > 0) lanes[nodeLane] = parents[0];
    for (let k = 1; k < parents.length; k++) {
      const p = parents[k];
      const idx = lanes.indexOf(p) !== -1 ? lanes.indexOf(p) : firstFree(lanes);
      lanes[idx] = p;
    }

    // Segments: pass-throughs + incoming (from children above) + outgoing (to parents).
    const segments: GraphSegment[] = [];
    for (let i = 0; i < before.length; i++) {
      const id = before[i];
      if (id === null) continue;
      if (id === commit.id) {
        segments.push({ x1: i, y1: 0, x2: nodeLane, y2: 50, color: laneColor(i) });
      } else {
        segments.push({ x1: i, y1: 0, x2: i, y2: 100, color: laneColor(i) });
      }
    }
    if (parents.length > 0) {
      segments.push({ x1: nodeLane, y1: 50, x2: nodeLane, y2: 100, color: laneColor(nodeLane) });
      for (let k = 1; k < parents.length; k++) {
        const idx = lanes.indexOf(parents[k]);
        segments.push({ x1: nodeLane, y1: 50, x2: idx, y2: 100, color: laneColor(idx) });
      }
    }

    rows.push({
      commitId: commit.id,
      nodeLane,
      color: laneColor(nodeLane),
      merge: parents.length > 1,
      segments,
    });
    laneCount = Math.max(laneCount, lanes.length, nodeLane + 1);
  }

  return { rows, laneCount };
}
