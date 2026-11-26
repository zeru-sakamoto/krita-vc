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

// --- Wrapping ----------------------------------------------------------------------------------
//
// A long history laid out on one spine is a thin ribbon once fit-to-screen zooms out to show it
// all, with most of the canvas empty above and below. Wrapping cuts the column axis into **bands**
// stacked top to bottom, every band reading left→right (not a boustrophedon: alternate rows read
// right-to-left would break "oldest on the left", and every fork/merge bend rule would need
// mirroring). This is a pure pass over `buildVersionMap`'s (col, lane) grid — it never changes
// which lane or column a commit has, only where that cell lands on screen.

/** One thing to place: a commit, or the pending-version preview. `time` is epoch ms, optional. */
export interface WrapCell {
  id: string;
  col: number;
  lane: number;
  time?: number;
}

/** A parent→child connection, by column only — all a cut's cost depends on. */
export interface WrapEdge {
  fromCol: number;
  toCol: number;
}

export interface WrapBand {
  /** First and last column (inclusive) of the band. */
  start: number;
  end: number;
  /** The lanes with something in this band, ascending — so the trunk is always the top row, and
   *  a band with no branches in it is one row tall. */
  lanes: number[];
  /** Absolute row index of this band's first lane. */
  rowStart: number;
}

export interface WrapPlacement {
  band: number;
  /** Column within the band, 0-based. */
  x: number;
  /** Absolute row, counting every band's rows from the top. */
  row: number;
}

export interface WrappedMap {
  bands: WrapBand[];
  at: Map<string, WrapPlacement>;
  /** Total rows across all bands. */
  rows: number;
  /** Width in columns of the widest band. */
  widest: number;
}

/** A quiet stretch at least this long between two columns counts as a session break, which makes
 *  that boundary a nicer place to cut: a band then reads as one sitting at the easel. */
export const SESSION_GAP_MS = 3 * 60 * 60 * 1000;

/**
 * Cut the columns into bands about `targetCols` wide and give every cell a (band, x, row).
 *
 * Bands are cut greedily, left to right. Each cut is picked from a window of widths around the
 * target (0.7×–1.3×) by cost, cheapest first:
 * 1. **Edges crossing the cut.** The trunk's own edge always crosses (cost baseline), but any
 *    other — a branch alive on both sides of the cut — has to be routed through the gap between
 *    bands, which is the one messy case. Most real histories have plenty of trunk-only boundaries
 *    (fork, try something, merge back or abandon), so this usually picks one.
 * 2. **Session breaks** — a cut where the artist stopped for a while is preferred.
 * 3. **Distance from the target width**, so bands come out evenly sized.
 * The tail is left as one band once it fits within 1.3× the target, rather than leaving a stub.
 *
 * `targetCols <= 0` or non-finite means "don't wrap": one band, which is exactly the unwrapped
 * layout (row = lane index among the lanes present).
 */
export function wrapVersionMap(
  cells: WrapCell[],
  edges: WrapEdge[],
  targetCols: number
): WrappedMap {
  if (cells.length === 0) return { bands: [], at: new Map(), rows: 0, widest: 0 };

  let minCol = Infinity;
  let maxCol = -Infinity;
  for (const c of cells) {
    minCol = Math.min(minCol, c.col);
    maxCol = Math.max(maxCol, c.col);
  }

  // crossings[c] = edges spanning the boundary between column c and c+1 (difference array).
  const diff = new Map<number, number>();
  for (const e of edges) {
    if (e.toCol <= e.fromCol) continue;
    diff.set(e.fromCol, (diff.get(e.fromCol) ?? 0) + 1);
    diff.set(e.toCol, (diff.get(e.toCol) ?? 0) - 1);
  }
  const crossings = new Map<number, number>();
  let run = 0;
  for (let c = minCol; c <= maxCol; c++) {
    run += diff.get(c) ?? 0;
    crossings.set(c, run);
  }

  // Newest moment in each column and oldest in each, for the session-break test.
  const latest = new Map<number, number>();
  const earliest = new Map<number, number>();
  for (const c of cells) {
    if (c.time === undefined || !Number.isFinite(c.time)) continue;
    latest.set(c.col, Math.max(latest.get(c.col) ?? -Infinity, c.time));
    earliest.set(c.col, Math.min(earliest.get(c.col) ?? Infinity, c.time));
  }
  const sessionBreak = (c: number) => {
    const a = latest.get(c);
    const b = earliest.get(c + 1);
    return a !== undefined && b !== undefined && b - a >= SESSION_GAP_MS;
  };

  const spans: [number, number][] = [];
  if (!(targetCols > 0) || !Number.isFinite(targetCols)) {
    spans.push([minCol, maxCol]);
  } else {
    const t = Math.max(1, Math.round(targetCols));
    const minW = Math.max(1, Math.ceil(0.7 * t));
    const maxW = Math.max(minW, Math.floor(1.3 * t));
    let start = minCol;
    while (maxCol - start + 1 > maxW) {
      let best = start + t - 1;
      let bestScore = Infinity;
      for (let w = minW; w <= maxW; w++) {
        const c = start + w - 1;
        if (c >= maxCol) break;
        const extra = Math.max(0, (crossings.get(c) ?? 0) - 1);
        const score = extra * 4 + Math.abs(w - t) / t - (sessionBreak(c) ? 0.5 : 0);
        if (score < bestScore) {
          bestScore = score;
          best = c;
        }
      }
      spans.push([start, best]);
      start = best + 1;
    }
    spans.push([start, maxCol]);
  }

  const bandOf = (col: number) => {
    // Few bands, so a scan is fine; columns between cells still resolve.
    for (let i = 0; i < spans.length; i++) if (col <= spans[i][1]) return i;
    return spans.length - 1;
  };

  const laneSets = spans.map(() => new Set<number>());
  for (const c of cells) laneSets[bandOf(c.col)].add(c.lane);

  const bands: WrapBand[] = [];
  let rows = 0;
  let widest = 0;
  spans.forEach(([start, end], i) => {
    const lanes = [...laneSets[i]].sort((a, b) => a - b);
    bands.push({ start, end, lanes, rowStart: rows });
    rows += Math.max(1, lanes.length);
    widest = Math.max(widest, end - start + 1);
  });

  const at = new Map<string, WrapPlacement>();
  for (const c of cells) {
    const band = bandOf(c.col);
    const b = bands[band];
    at.set(c.id, { band, x: c.col - b.start, row: b.rowStart + b.lanes.indexOf(c.lane) });
  }

  return { bands, at, rows, widest };
}
