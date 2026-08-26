import type { Branch, Commit } from "../types";

/**
 * Lane/column layout for the **Version Map** — the left→right canvas of version thumbnails.
 *
 * Deliberately separate from `lib/graph.ts`'s `buildGraph`: that one lays a DAG out *vertically*
 * one row per commit for the legacy history rail, where a lane is an x column and lane 0 always
 * means "the mainline". Here a lane is a y offset, a column is a generation, and lane 0 means
 * "the branch you're standing on" — different enough that sharing the algorithm would mean
 * parameterizing both conventions into one function for no gain.
 */

export interface PlacedCommit {
  /** Vertical lane. 0 is always the current branch's own line. */
  lane: number;
  /** Horizontal column — generation depth from the root (1-based). */
  col: number;
  /** "Version N" — the same depth, so shared ancestors read the same on every lane. */
  version: number;
  /** The branch this commit was made on; null on lane 0 and for pre-branching commits. */
  branch: string | null;
  /** Some drawn commit is this one's parent / child — drives the node's spine stubs. */
  hasIncoming: boolean;
  hasOutgoing: boolean;
}

export interface MapLayout {
  placed: Map<string, PlacedCommit>;
  laneCount: number;
  /** Commit id → the branches whose tip it is. A branch with no commits of its own still shows
   *  up here, sharing its parent branch's tip node. */
  tips: Map<string, string[]>;
}

/**
 * Place every commit on a (lane, column) grid.
 *
 * - **Lane 0 is the current branch's first-parent spine**, walked back from its tip — not the
 *   commits stamped with its name. After a merge the folded-in commits still carry *their*
 *   branch and belong on a side lane; and standing on a side branch, its shared ancestors are
 *   stamped `main` and would otherwise jog the line you're on down a lane.
 * - Everything else groups by `commit.branch`, lanes assigned in order of first appearance
 *   (oldest first) so a lane's color doesn't shuffle between refetches.
 * - **Column = generation depth** (`1 + max(depth(parents))`), so parallel work on two branches
 *   lines up in the same column instead of leaving chronological gaps, and a merge lands one
 *   column past the deeper of its parents.
 */
export function buildVersionMap(
  commits: Commit[],
  branches: Branch[],
  currentBranch: string
): MapLayout {
  const byId = new Map(commits.map((c) => [c.id, c]));
  // Oldest first: parents before children, so one pass computes depth and lane order is stable.
  const ordered = [...commits].reverse();

  const tips = new Map<string, string[]>();
  for (const b of branches) {
    if (!b.tip || !byId.has(b.tip)) continue;
    const at = tips.get(b.tip);
    if (at) at.push(b.name);
    else tips.set(b.tip, [b.name]);
  }

  // Lane 0: the current branch's own line, back along first parents.
  const spine = new Set<string>();
  const currentTip = branches.find((b) => b.name === currentBranch)?.tip ?? null;
  let cursor = currentTip;
  while (cursor) {
    if (spine.has(cursor)) break; // cycles are impossible, but never hang the UI on bad data
    spine.add(cursor);
    cursor = byId.get(cursor)?.parents[0] ?? null;
  }

  const laneOf = new Map<string, number>();
  const lanes = new Map<string, number>(); // branch name → lane
  const depth = new Map<string, number>();
  const hasChild = new Set<string>();

  for (const c of ordered) {
    let d = 1;
    for (const p of c.parents) {
      const pd = depth.get(p);
      if (pd === undefined) continue; // parent outside the drawn set (shouldn't happen)
      hasChild.add(p);
      if (pd + 1 > d) d = pd + 1;
    }
    depth.set(c.id, d);

    // `c.branch === currentBranch` is redundant with the spine in every normal case, but it also
    // covers the moment before `useBranches` resolves, when there's no tip to walk back from and
    // every commit would otherwise be pushed off lane 0.
    if (spine.has(c.id) || !c.branch || c.branch === currentBranch) {
      laneOf.set(c.id, 0);
    } else {
      let lane = lanes.get(c.branch);
      if (lane === undefined) {
        lane = lanes.size + 1;
        lanes.set(c.branch, lane);
      }
      laneOf.set(c.id, lane);
    }
  }

  // Two commits should never land on the same cell — same lane means same branch chain, and a
  // chain's depths strictly increase. Guard anyway: an invisibly stacked node is a nasty bug.
  const taken = new Set<string>();
  const placed = new Map<string, PlacedCommit>();
  let laneCount = 1;
  for (const c of ordered) {
    const lane = laneOf.get(c.id) ?? 0;
    let col = depth.get(c.id) ?? 1;
    while (taken.has(`${lane}:${col}`)) col++;
    taken.add(`${lane}:${col}`);
    laneCount = Math.max(laneCount, lane + 1);
    placed.set(c.id, {
      lane,
      col,
      version: depth.get(c.id) ?? 1,
      branch: lane === 0 ? null : (c.branch ?? null),
      hasIncoming: c.parents.some((p) => byId.has(p)),
      hasOutgoing: hasChild.has(c.id),
    });
  }

  return { placed, laneCount, tips };
}
