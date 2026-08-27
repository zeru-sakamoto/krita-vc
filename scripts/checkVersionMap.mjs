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
import { bendFraction, buildVersionMap } from "../src/lib/versionMap.ts";

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

console.log("versionMap layout: 5 cases OK");
