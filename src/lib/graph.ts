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

/**
 * Stable branch → color assignment: the current branch always gets the accent (lane-0)
 * color, other branches take the remaining palette in order of first appearance
 * (newest-first). Used to color graph *nodes* by the branch a commit was made on, so a
 * branch keeps its identity even when lane assignment shifts; segments stay lane-colored
 * (they trace lanes, not branches). Commits from before branching existed have no
 * `branch` and fall back to their lane color.
 */
export function branchColorMap(commits: Commit[], currentBranch?: string): Map<string, string> {
  const map = new Map<string, string>();
  if (currentBranch) map.set(currentBranch, LANE_COLORS[0]);
  for (const c of commits) {
    if (c.branch && !map.has(c.branch)) {
      map.set(c.branch, LANE_COLORS[map.size % LANE_COLORS.length]);
    }
  }
  return map;
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

// Pixel geometry for a lane column — shared by the per-row rail (`CommitGraphRail`) and the
// cross-row revert-link overlay (`CommitGraph`) so both agree on where a lane sits.
export const LANE_W = 16;
// Dedicated gutter left of lane 0 for the revert-link overlay, so it doesn't run on top of the
// lane-0 dots/lines.
export const LINK_GUTTER_W = 10;
export const ORIGIN_X = 12 + LINK_GUTTER_W;
/** X for a revert link's vertical run — middle of the dedicated left gutter. */
export const REVERT_LINK_X = LINK_GUTTER_W / 2;

export function laneX(lane: number): number {
  return ORIGIN_X + lane * LANE_W;
}

export function railWidth(laneCount: number): number {
  return ORIGIN_X + Math.max(1, laneCount) * LANE_W;
}

const REVERT_CORNER_R = 4;

/** SVG path for a revert link: out from `(x1, y1)`, down the gutter at `gutterX`, in to `(x2, y2)`, with rounded corners. */
export function elbowPath(x1: number, y1: number, gutterX: number, x2: number, y2: number): string {
  const r = REVERT_CORNER_R;
  const dx1 = Math.sign(x1 - gutterX) || 1;
  const dy = Math.sign(y2 - y1) || 1;
  const dx2 = Math.sign(x2 - gutterX) || 1;
  return [
    `M ${x1} ${y1}`,
    `L ${gutterX + dx1 * r} ${y1}`,
    `Q ${gutterX} ${y1} ${gutterX} ${y1 + dy * r}`,
    `L ${gutterX} ${y2 - dy * r}`,
    `Q ${gutterX} ${y2} ${gutterX + dx2 * r} ${y2}`,
    `L ${x2} ${y2}`,
  ].join(" ");
}

export interface RevertLink {
  fromIndex: number;
  toIndex: number;
}

/**
 * One link per rollback commit whose restored-from target is also in `commits` — draws the
 * graph line connecting a restored version back to the version it copied from. Silently
 * skipped when the target isn't in the current list (e.g. off-branch, not loaded).
 */
export function buildRevertLinks(commits: Commit[]): RevertLink[] {
  const indexById = new Map(commits.map((c, i) => [c.id, i]));
  const links: RevertLink[] = [];
  commits.forEach((c, fromIndex) => {
    if (!c.restoredFrom) return;
    const toIndex = indexById.get(c.restoredFrom);
    if (toIndex !== undefined) links.push({ fromIndex, toIndex });
  });
  return links;
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
