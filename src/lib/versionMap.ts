import type { Branch, Commit } from "../types";

/**
 * Lane/column layout for the **Version Map** — the left→right canvas of version thumbnails.
 *
 * Deliberately separate from `lib/graph.ts`'s `buildGraph`: that one lays a DAG out *vertically*
 * one row per commit for the legacy history rail, where a lane is an x column. Here a lane is a y
 * offset and a column is a generation — different enough that sharing the algorithm would mean
 * parameterizing both conventions into one function for no gain. Both agree that lane 0 is the
 * mainline.
 */

/** The branch that owns lane 0. The backend refuses to delete it (`DeleteMain`), so it always
 *  exists; `BranchesPanel` and the map's action bar special-case the same name. */
const TRUNK = "main";

export interface PlacedCommit {
  /** Vertical lane. 0 is always the trunk (`main`); every other branch gets its own. */
  lane: number;
  /** Horizontal column — generation depth from the root (1-based). */
  col: number;
  /** "Version N" — the same depth, so shared ancestors read the same on every lane. */
  version: number;
  /** The branch this commit was made on; null on the trunk lane and for pre-branching commits. */
  branch: string | null;
}

export interface MapLayout {
  placed: Map<string, PlacedCommit>;
  laneCount: number;
  /** The lane the current branch sits on — drawn in the accent color, since lane 0 is the trunk
   *  now and no longer says where you're standing. */
  currentLane: number;
  /** Commit id → the branches whose tip it is. A branch with no commits of its own still shows
   *  up here, sharing its parent branch's tip node. */
  tips: Map<string, string[]>;
}

/**
 * Place every commit on a (lane, column) grid.
 *
 * - **Lane 0 is the trunk's first-parent spine**, walked back from `main`'s tip — not the commits
 *   stamped with its name, since after a merge the folded-in commits still carry *their* branch
 *   and belong on a side lane. Anchoring on `main` rather than on the branch you're standing on
 *   is what keeps the trunk still while you work: standing on a side branch used to make *its*
 *   spine lane 0, which drew a fork as one straight line and left `main`'s next commit — same
 *   generation depth, so the same column — to be shunted off to a side lane by the collision
 *   guard below. The trunk must not move when you switch branches.
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

  const currentTip = branches.find((b) => b.name === currentBranch)?.tip ?? null;

  // Lane 0: the trunk's line, back along first parents. `main`'s own tip isn't always drawn —
  // with "show all lines" off, `list_commits` is scoped to *your* tip, so a trunk commit newer
  // than the fork point is missing while the shared ancestors are all present. Falling back to
  // the newest drawn commit stamped `main` picks up that shared run; falling back again to the
  // current tip covers the moment before `useBranches` resolves and any history with no trunk
  // commits drawn at all.
  const trunkTip = branches.find((b) => b.name === TRUNK)?.tip ?? null;
  const anchor =
    (trunkTip && byId.has(trunkTip) ? trunkTip : null) ??
    commits.find((c) => c.branch === TRUNK)?.id ?? // `commits` is newest-first
    currentTip;

  const spine = new Set<string>();
  let cursor = anchor;
  while (cursor) {
    if (spine.has(cursor)) break; // cycles are impossible, but never hang the UI on bad data
    spine.add(cursor);
    cursor = byId.get(cursor)?.parents[0] ?? null;
  }

  const laneOf = new Map<string, number>();
  const lanes = new Map<string, number>(); // branch name → lane
  const depth = new Map<string, number>();

  for (const c of ordered) {
    let d = 1;
    for (const p of c.parents) {
      const pd = depth.get(p);
      if (pd === undefined) continue; // parent outside the drawn set (shouldn't happen)
      if (pd + 1 > d) d = pd + 1;
    }
    depth.set(c.id, d);

    // `c.branch === TRUNK` is redundant with the spine in every normal case, but it also covers
    // the moment before `useBranches` resolves, when there's no tip to walk back from and every
    // commit would otherwise be pushed off lane 0.
    if (spine.has(c.id) || !c.branch || c.branch === TRUNK) {
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
    });
  }

  // Read off the tip commit, not the `lanes` name map: a branch created but not yet committed on
  // has no commits of its own, and shares its parent's tip node — whose lane is the right answer.
  const currentLane = (currentTip ? placed.get(currentTip)?.lane : undefined) ?? 0;

  return { placed, laneCount, tips, currentLane };
}

/**
 * `stepPosition` for a lane-crossing connector: puts its descent in the middle of the gutter
 * beside one end — the source's when the line forks down to a deeper lane, the target's when it
 * climbs back — so a branch leaves the spine at the version it started from and rejoins at the
 * version it merges into, without ever running alongside the spine (which doubles the line).
 *
 * `dx` is the horizontal span between the two connector dots, `pitch` the column pitch, `gap` the
 * path's straight run off each handle (`offset`). The fraction is measured between the two gap
 * points, hence the `gap` terms. Lives here rather than in the panel so the layout check can run
 * it; falls back to the midpoint if the two ends nearly touch.
 */
export function bendFraction(dx: number, fork: boolean, pitch: number, gap: number): number {
  const span = dx - 2 * gap;
  if (span <= 0) return 0.5;
  const s = Math.min(Math.max((pitch / 2 - gap) / span, 0), 1);
  return fork ? s : 1 - s;
}
