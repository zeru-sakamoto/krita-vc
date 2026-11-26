// Layout check for the Version Map's lane/column assignment. `buildVersionMap` is pure and its
// only import is `import type` (erased), so Node's native type stripping runs the .ts directly —
// no test runner, no config.
//
//   node scripts/checkVersionMap.mjs
//
// The cases below are the ones that actually broke: a branch forked off main used to draw ON the
// trunk (because lane 0 followed whichever branch you stood on), which then left main's next
// commit — same generation, same column — to be shunted off to a side lane.

import assert from "node:assert/strict";
import { Position, getSmoothStepPath } from "@xyflow/system";
import { SESSION_GAP_MS, bendFraction, buildVersionMap, wrapVersionMap } from "../src/lib/versionMap.ts";

/** `commits` is newest-first, matching what `useCommits` hands the map. */
const commit = (id, parents, branch) => ({
  id,
  hash: id,
  message: id,
  author: "t",
  timestamp: "2026-01-01T00:00:00Z",
  parents,
  branch,
  changes: [],
});

const V1 = commit("v1", [], "main");
const V2 = commit("v2", ["v1"], "main");
const V3 = commit("v3", ["v2"], "alt-version"); // forked from V2
const V4 = commit("v4", ["v2"], "main"); // main carries on after the fork

const branch = (name, tip, current) => ({ name, tip, kind: current ? "current" : "local" });

// --- 1. Standing on the side branch: the trunk keeps lane 0, the fork drops to lane 1. -------
{
  const l = buildVersionMap(
    [V3, V2, V1],
    [branch("main", "v2"), branch("alt-version", "v3", true)],
    "alt-version"
  );
  assert.equal(l.placed.get("v1").lane, 0, "V1 is on the trunk");
  assert.equal(l.placed.get("v2").lane, 0, "V2 is on the trunk");
  assert.equal(l.placed.get("v3").lane, 1, "the forked commit is off to the side");
  assert.equal(l.currentLane, 1, "you are standing on lane 1");
  assert.equal(l.laneCount, 2);
}

// --- 2. Main carries on: it stays on the trunk, in the same column as the fork. ---------------
{
  const l = buildVersionMap(
    [V4, V3, V2, V1],
    [branch("main", "v4"), branch("alt-version", "v3", true)],
    "alt-version"
  );
  assert.deepEqual(
    { lane: l.placed.get("v4").lane, col: l.placed.get("v4").col },
    { lane: 0, col: 3 },
    "main's next commit runs straight on from V2"
  );
  assert.deepEqual(
    { lane: l.placed.get("v3").lane, col: l.placed.get("v3").col },
    { lane: 1, col: 3 },
    "the fork sits beside it, not in its cell"
  );
}

// --- 3. The trunk does not move when you switch branches. ------------------------------------
{
  const l = buildVersionMap(
    [V4, V3, V2, V1],
    [branch("main", "v4", true), branch("alt-version", "v3")],
    "main"
  );
  assert.equal(l.placed.get("v4").lane, 0);
  assert.equal(l.placed.get("v3").lane, 1, "the fork stays on its own lane either way");
  assert.equal(l.currentLane, 0, "the accent moves to the trunk");
}

// --- 4. Trunk tip outside the drawn set ("show all lines" off, standing on a branch). --------
// `list_commits` is scoped to your tip, so V4 is missing — the shared ancestors still are not.
{
  const l = buildVersionMap(
    [V3, V2, V1],
    [branch("main", "v4"), branch("alt-version", "v3", true)],
    "alt-version"
  );
  assert.equal(l.placed.get("v2").lane, 0, "falls back to the newest drawn trunk commit");
  assert.equal(l.placed.get("v3").lane, 1);
}

// --- 5. Both of a lane-crossing connector's corners are rounded. -----------------------------
// The bend x is picked via `stepPosition`, a fraction of the run between the two gap points.
// Aiming it at a gap point itself (offset = half the gutter, stepPosition 0/1) *looked* right,
// but only because getSmoothStepPath drops a gap point sitting exactly on the bend — and the
// measured handle positions carry float error, so on the far end it sometimes didn't. The stray
// point a hair from the corner collapsed that corner's radius to ~0: one bend round, one square.
{
  const PITCH = 280; // NODE_W + 72
  const GAP = 20; // STEP_GAP
  const RADIUS = 16;
  for (const [label, dx, fork] of [
    ["fork to the next column", PITCH, true],
    ["merge back one column", PITCH, false],
    ["merge back across three", PITCH * 3, false],
  ]) {
    const [path] = getSmoothStepPath({
      sourceX: 0,
      sourceY: 0,
      sourcePosition: Position.Right,
      targetX: dx,
      targetY: fork ? 340 : -340,
      targetPosition: Position.Left,
      borderRadius: RADIUS,
      offset: GAP,
      stepPosition: bendFraction(dx, fork, PITCH, GAP),
    });
    // Each corner is `L <start>Q <corner> <end>`; a corner that kept its radius starts a full
    // RADIUS away from the corner point, a collapsed one starts on top of it.
    const curves = [
      ...path.matchAll(/L ?(-?[\d.]+),(-?[\d.]+)Q ?(-?[\d.]+),(-?[\d.]+)/g),
    ].map((m) => m.slice(1).map(Number));
    assert.equal(curves.length, 2, `${label}: two corners`);
    for (const [sx, sy, cx, cy] of curves) {
      assert.equal(Math.hypot(cx - sx, cy - sy), RADIUS, `${label}: corner keeps its radius`);
    }
  }
}

// --- 6. Wrapping a straight line: even bands, one row each, every band starting at x 0. -------
const line = (n) => {
  const cells = [];
  const edges = [];
  for (let c = 1; c <= n; c++) {
    cells.push({ id: `c${c}`, col: c, lane: 0 });
    if (c > 1) edges.push({ fromCol: c - 1, toCol: c });
  }
  return { cells, edges };
};
{
  const { cells, edges } = line(30);
  const w = wrapVersionMap(cells, edges, 10);
  assert.deepEqual(
    w.bands.map((b) => [b.start, b.end]),
    [
      [1, 10],
      [11, 20],
      [21, 30],
    ]
  );
  assert.equal(w.rows, 3, "a band with no branches is one row");
  assert.deepEqual(w.at.get("c11"), { band: 1, x: 0, row: 1 });
  const flat = wrapVersionMap(cells, edges, 0);
  assert.equal(flat.bands.length, 1, "0 means don't wrap");
  assert.deepEqual(flat.at.get("c30"), { band: 0, x: 29, row: 0 });
}

// --- 7. A cut never lands where a side branch is alive if a trunk-only boundary is in reach. --
// Branch forks off col 9 and merges back at col 12; the natural cut (after col 10) would sever it.
{
  const { cells, edges } = line(24);
  cells.push({ id: "b10", col: 10, lane: 1 }, { id: "b11", col: 11, lane: 1 });
  edges.push({ fromCol: 9, toCol: 10 }, { fromCol: 10, toCol: 11 }, { fromCol: 11, toCol: 12 });
  const w = wrapVersionMap(cells, edges, 10);
  const first = w.bands[0];
  assert.ok(first.end < 9 || first.end >= 12, `cut after ${first.end} severs the branch`);
  const band = w.at.get("b10").band;
  assert.equal(w.at.get("b11").band, band, "the branch stays in one band");
  assert.equal(w.at.get("c9").band, band, "with the version it forked from");
  assert.equal(w.at.get("c12").band, band, "and the version it merged into");
  assert.equal(w.bands[band].lanes.length, 2, "only that band grows a second row");
}

// --- 8. Given two equally good cuts, a session break wins. ------------------------------------
{
  const { cells, edges } = line(20);
  const t0 = Date.parse("2026-01-01T09:00:00Z");
  for (const c of cells) c.time = t0 + c.col * 60_000 + (c.col > 11 ? SESSION_GAP_MS : 0);
  const w = wrapVersionMap(cells, edges, 10);
  assert.equal(w.bands[0].end, 11, "the cut lands on the overnight gap, not at exactly 10");
}

console.log("versionMap layout: 8 cases OK");
