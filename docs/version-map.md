# Version Map

The Version Map is the default view and the visual replacement for the History graph
([`VersionMapPanel.tsx`](../src/components/vcs/VersionMapPanel.tsx) and
[`VersionNode.tsx`](../src/components/vcs/VersionNode.tsx)). It draws the current branch's versions
on a pannable, zoomable canvas, left to right and oldest first along a spine. Each node carries the
version's after-composite, a connector dot the spine runs through, a caption, and a two-column grid
of chips for the layers that changed. A chip is a layer-type icon from `friendly.ts`'s
`layerTypeIcon()` plus an A, M or D glyph in `FileStatusChip`'s icon and color vocabulary.

On the map itself the node is the metadata, so the Map tab has no Sidebar and no Inspector. Clicking
a node opens the full `MainPanel`/`DiffView` in place, with a back button and, for a version with
several diff entries, a `Menu` file picker in the header. The opened version gets its own toggleable
[`Inspector`](../src/components/shell/Inspector.tsx), open by default, with the same Restore action
and "Selected" section as the legacy layout. It is shown and hidden with the same `SidebarSimple`
icon button `AppShell` uses, but its state lives in `CommitDrilldown`, not in the shell. So "no
Inspector" only describes the canvas before a version is opened.

For how the map fits into the rest of the shell, see
[frontend-architecture.md](frontend-architecture.md#app-shell).

## Building blocks

The map is built on React Flow (`@xyflow/react`), the one framework-sized frontend dependency in
the app. It was chosen over the in-repo `useZoomPan` because branch lanes need edge routing, a
minimap and fit-view over a real graph, which is most of what React Flow is. It costs about 60 KB
gzipped. The version is pinned exactly, to `12.10.2` with no caret: `12.11.4` ships a broken
pairing in which `@xyflow/react` imports `handleAttributionWarning` from `@xyflow/system@0.0.80`,
which doesn't export it, and Vite's dependency optimizer fails on it. Re-test before widening the
range.

Node positions are computed from the commit graph and `nodesDraggable` is `false`. History is not a
mood board: nothing is persisted, so a new commit can never leave the layout stale. `NODE_PITCH` and
`LANE_PITCH` are the layout constants. Edges come from `parents`, not from list adjacency, so a merge
commit's second parent draws its own line into the lane it came from.

The map adds no backend command. A node calls the same `useCommitDiff` → `commit_diff` the diff
viewer uses, which already returns what a node needs: `afterImage` (the capped, content-addressed
`mergedimage.png` as a `kvcimg://` URL) and `layers[]` with `change` and `layerType`, without the
expensive per-layer rasters (`with_rasters = false`). Opening a node's drilldown is therefore a
`diffCache` hit, not a second round trip. `commit_diff` runs through `run_heavy` (two at a time) and
also builds a changed-pixel mask the node throws away, so the number of heavy calls is bounded by
`onlyRenderVisibleElements`: a node outside the viewport isn't mounted and never fetches. If that
stops being enough, the upgrade is a dedicated `commit_thumbnails(path, ids)` command. Don't reach
for it before then.

## Mounted once, for the shell's lifetime

`AppShell` renders exactly one `VersionMapPanel` and toggles a `hidden` class on its wrapper instead
of mounting it per view. It used to have two JSX call sites, one for the Map tab and one for the
Performance tab, and each remounted the whole `ReactFlowProvider` on every tab switch. The wrapper's
`showMap` flag covers the Map tab and the Performance tab when Legacy version history is off (see
[Legacy version history](#legacy-version-history)). Everywhere else the panel stays mounted under
`display: none`. Losing the mount would reset both the panned and zoomed viewport and the open
drilldown's `openId` on every tab switch, since both are local `VersionMap` state that React Flow
doesn't persist.

Opening a version unmounts the canvas (not the panel), and a canvas that remounts comes back at the
origin, which is the oldest version. So the viewport is stashed in a ref on the way in and handed
back as `defaultViewport` on the way out.

## Branch lanes

By default only the current branch is drawn, which is free: `list_commits` is already scoped to the
current branch tip. A header toggle (a `GitBranch` icon, "All lines" in Artist Mode, shown only once
another branch exists) switches to all branches, each on its own lane. It defaults to off, persists
to `localStorage` (`krita-vc:map-show-all`), and is plain component state rather than another
app-wide context, because it belongs to the map, not to the app. When it's on, the panel makes its
own `useCommits(repoPath, nonce, true)` call for the backend's wider `allBranches` scope, the union
of the commits reachable from every branch tip. When it's off, the path it passes is `""`, which
`useCommits` short-circuits, so the off state costs nothing and draws exactly the commits the shell
already loaded.

The lane and column assignment is a pure function, `buildVersionMap` in
[`lib/versionMap.ts`](../src/lib/versionMap.ts). It deliberately isn't in `lib/graph.ts`:
`buildGraph` lays a DAG out vertically for the legacy rail, where a lane is an x column, while here
a lane is a y offset. Both agree that lane 0 is the mainline. Three rules carry the layout.

1. Lane 0 is the trunk's first-parent spine, walked back from `main`'s tip. It is not "the commits
   stamped `main`": after a merge, the folded-in commits still carry their own branch name and
   belong on a side lane. Anchoring on `main` rather than on the branch you're standing on matters,
   and getting it wrong was a real bug. Lane 0 used to follow your branch, so a fork off main drew as
   one straight line, and main's next commit (same generation depth, so the same column) got pushed
   onto a side lane by the collision guard. The trunk must not move when you switch branches.
   `main`'s tip isn't always in the drawn set (with "show all lines" off, `list_commits` is scoped to
   your tip), so the anchor falls back to the newest drawn commit stamped `main`, then to the current
   tip.
2. Every other commit groups by `commit.branch` into lanes 1 and up, in order of first appearance
   (oldest first), so a lane's color doesn't shuffle between refetches.
3. The column is the generation depth (`1 + max(depth(parents))`). Parallel work on two branches
   lines up in the same column instead of leaving chronological gaps, and a merge lands one column
   past the deeper of its parents. A `(lane, col)` collision guard bumps the column. It shouldn't
   fire, but a node stacked invisibly under another would be a nasty failure.

That same depth is the node's "Version N", so shared ancestors read the same on every lane. For a
linear history it is identical to `friendly.ts`'s positional `versionNumbers()`, which the legacy
graph and the Inspector still use. Two lanes can therefore both show "Version 5"; the lane color and
the branch name in the caption tell them apart.

`buildVersionMap` also returns `currentLane`, the lane the current branch sits on. It is read off the
current tip's placement rather than a branch-name map, because a branch created but not yet
committed on has no commits of its own and shares its parent's tip node, whose lane is the right
answer. Branch color (below) and the pending-version preview use it.

[`scripts/checkVersionMap.mjs`](../scripts/checkVersionMap.mjs) pins these rules: the trunk stays on
lane 0 whichever branch you stand on, main's next commit lands in the same column as the fork
beside it, the fallback when main's tip isn't drawn works, and the bend corner radius (see
[Drawing the line](#drawing-the-line)) holds for a fork and for one- and three-column merges.
`buildVersionMap` is pure and its only import is `import type`, which is erased, so Node's native
TypeScript stripping runs the `.ts` file directly with no test runner or config. The check runs as
part of `npm run build` (after `tsc`, before `vite build`), so a layout regression fails the build
instead of shipping.

## Branch color

Lane 0 is the trunk, so the lane index no longer says where you're standing. The accent color says
it instead. `laneColor(lane, currentLane)` returns `var(--color-accent)` for whichever lane the
current branch is on (`MapLayout.currentLane`), and every other lane cycles a small local palette,
`BRANCH_LANE_COLORS` in `VersionMapPanel.tsx` (`info-fg`, `success-fg`, `warning-fg`). This is the
same idea as `graph.ts`'s `branchColorMap`, but `graph.ts`'s `LANE_COLORS` stays as it is: its
positional "lane 0 is accent" rule is a fixed convention of the legacy History graph.

A lane's color paints the connector dot of every node on it and, mixed 55% toward transparent with
`color-mix`, the spine between them. Every branch tip's thumbnail also gets a detached
`outline`/`outline-offset` ring in its lane color, plus a branch-name chip under its caption. The
node that's open gets a flush `ring-accent` instead. On your own lane those two rings are now the
same color and only the geometry (detached or flush) tells them apart, so don't collapse them into
one. A branch created but not yet committed on shares its parent's tip node, so it shows up as a
second chip on that node for free.

## Drawing the line

The spine is one drawing system: SVG edges, from dot to dot. Both of a node's handles sit on its
connector dot (the node's center at `SPINE_TOP`) instead of on its left and right edges, so one edge
path spans the source dot, the gutter and the target dot. React Flow draws `.react-flow__edges`
beneath `.react-flow__nodes`, and the dot is opaque, so the line reads as passing through it. The
dot row sits in the 8 px gap between the thumbnail card and the caption, so nothing else covers it.
This works because `@xyflow/system`'s `getHandlePosition` uses the handle's own measured x and y and
does not snap to the node's bounding box.

It replaced half-width CSS bars drawn inside each node to bridge React Flow's gutter-only edges to
the centered dot. Two systems could never stay aligned. A 1.5 px box shifted with
`-translate-y-1/2` lands on a half pixel and rasterizes across two device rows, while an SVG stroke
centers cleanly on its path. So the in-node run and the gutter run sat about 1 px apart and stepped
at every node edge, and they could disagree on color, the stub using the node's lane and the edge
using the crossing's. Don't reintroduce an in-node segment.

The line color is opaque (`color-mix(…, var(--color-bg))`, never toward `transparent`). Two
translucent strokes over the same pixels composite into a brighter, two-tone band that reads as a
doubled line.

A connector that crosses lanes carries its own `pathOptions`:

```ts
{ offset: STEP_GAP, stepPosition: bendFraction(dx, fork, NODE_PITCH, STEP_GAP), borderRadius: 16 }
```

The bend has to land in the middle of the gutter next to the end on the shallower lane. A branch
then drops out of the spine at the version it started from and climbs back in at the version it
merges into, clear of the node's caption and chips, and never runs alongside the spine (which would
double the line). `stepPosition` is a fraction of the run between the two gap points, not an
absolute x, so [`bendFraction`](../src/lib/versionMap.ts) solves for the fraction that puts the
descent at that gutter midpoint, given `dx` (the horizontal span between the two connector dots),
`NODE_PITCH`, and `STEP_GAP`, the gap each end runs straight off its handle before it may bend. It
lives in `versionMap.ts` rather than the panel so the layout check can call it.

`STEP_GAP` (20 px) is deliberately small, and deliberately not half the gutter width. An earlier
version aimed the gap point itself at the bend (`offset: NODE_PITCH / 2`, `stepPosition: fork ? 0 :
1`). It looked the same but wasn't equivalent: `getSmoothStepPath` only drops a gap point when it
lands exactly on the bend's x, and the measured handle positions carry float error, so on the far
end it sometimes didn't. The stray point a hair from the corner collapsed that corner's radius to
about 0, so one bend was rounded and the other square. Keeping the gap points well clear of the
bend means both corners always get the full radius whatever the float error, and
`scripts/checkVersionMap.mjs` pins the radius on both ends.

The connector's stroke is a gradient between the two lanes' colors, so a fork fades out of its
parent branch's color and a merge fades back into the color it joins. `LaneGradients` emits one
`<linearGradient>` per lane pair that actually has a connector, into its own zero-size `<svg>`. A
`url(#…)` paint reference resolves document-wide, and there's no hook to inject defs into React
Flow's own `<svg>` short of a custom edge component. Each gradient must use
`gradientUnits="userSpaceOnUse"` and run purely vertically between the two lanes' spine y values.
User space here is React Flow's flow coordinates, the same space those y values are in. That keeps
the whole transition on the descent, so each horizontal run is exactly its own lane's color, which is
also what hides the short stretch the connector shares with the spine before the bend. An
`objectBoundingBox` gradient would smear the transition across the whole path and tint that shared
stretch.

## Grid background

The canvas uses React Flow's `Lines` background, colored by a `--color-grid` token in
[`src/styles/global.css`](../src/styles/global.css) that each theme derives from its own
background: `color-mix(in srgb, var(--color-bg) 88%, var(--color-text-muted))`. Every theme gets a
barely visible grid with no per-theme values, and the two light themes override it to
`--color-border`. It mixes toward `--color-text-muted`, not toward black. Mixing toward black made
the grid vanish on True Black, whose `--color-bg` is already `#000` and can't get darker. Any
replacement has to mix toward a contrasting token for the same reason.

## Zoom, level of detail and the minimap

The wheel zooms toward the cursor and a drag pans (`zoomOnScroll`, `panOnScroll={false}`), the same
gesture pair as the diff viewer's `useZoomPan`, so the app's two canvases agree about what the wheel
does. Below `LOD_ZOOM` the caption and chips are dropped. The `useStore` selector returns a boolean,
so a node re-renders only when the threshold is crossed, not on every frame. The header also has
"Fit all versions", "Jump to the newest version" and a zoom readout that resets to 100%.

Nodes must carry an explicit `width` and `height` (`NODE_W`, `NODE_H`). React Flow's `MiniMap`
sizes nodes from the user node object (`getNodeDimensions` reads it, not the measured box), so
without them the minimap renders empty. Real node height varies with the chip count; `NODE_H` is
nominal and only feeds culling and the minimap.

The minimap's mask and viewport frame are drawn by `MinimapViewport`, not by React Flow. React Flow
paints both as one `fillRule="evenodd"` path, so `maskStrokeColor` also strokes the mask's outer
rectangle (half that stroke falls inside the viewBox and reads as a stray accent line down the
minimap's edge), and an `h`/`v`/`z` path can't take an `rx`. So the `MiniMap` gets
`maskColor="transparent"` and `maskStrokeWidth={0}`, and `MinimapViewport` draws an evenodd path for
the dim and a stroke-only `<rect rx>` for the frame. The hole in the mask is rounded to the frame's
radius, because a square hole under a rounded stroke leaves undimmed slivers in the corners.

Two couplings hold this together. The overlay is a `Panel`, not a plain div, so it gets the same
margin and stacking as the `MiniMap`'s own panel and lands on it exactly. And since React Flow's
minimap geometry isn't exported, `MinimapViewport` re-derives it from `nodes` and the store
`transform`, which is why `MINIMAP_W`, `MINIMAP_H` and `MINIMAP_OFFSET_SCALE` are shared constants:
they must stay the exact values the `MiniMap` is given, or the two SVGs drift apart. `offsetScale`
is 1.5 instead of React Flow's default 5 because this history is much wider than it is tall.
`viewScale` is width-driven, and that padding applied on both axes shows up as a gap on the short
one.

## Branch actions and pick-a-version mode

The map also acts on branches, through a floating `MapActionBar`: a React Flow
`<Panel position="top-left">`, clear of the minimap in the bottom right, marked `nopan` for the same
reason `VersionNode`'s thumbnail is. It is one `Menu` whose `selected`, `detail`, hover-revealed
`action` and `footer` slots are exactly the legacy Branches panel's row model: select a row to
switch, use the row's hover actions to merge ("Bring X into Y" in Artist Mode) or delete ("Remove
this version line"), and "New version line…" in the footer to create. So it needed no new
primitive, only [`useBranchActions`](../src/components/vcs/useBranchActions.tsx) wired to a second
call site. Neither hover action is offered on the current branch, and delete isn't offered on
`main`, which the backend also refuses (`DeleteMain`).

`useBranchActions` is shared by three call sites: this bar, the legacy `BranchesPanel`, and the
History sidebar's branch switcher. It returns `{ run, error, switchTo, askMerge, askDelete,
askCreate, dialogs }`, which is the dirty-tree error routing (an `"unsaved changes"` error opens
`SaveFirstModal`, whose set-aside option stashes the work and retries the blocked action) plus every
confirm dialog as one node. It lives in its own file rather than in `BranchDialogs.tsx` because it
needs `SetAsideModal`, and `StashDialogs.tsx` already imports `errorText` from `BranchDialogs.tsx`,
so putting it there would close an import cycle.

Next to the menu sits the one action only the map can offer: "Start a line here…" enters a
pick-a-version mode where the next node click forks a branch at that version, through
`create_branch_at`. That backend operation was written and tested before anything could reach it,
because nothing else in the app is a version picker. Pick mode is one boolean (`picking`), and
`VersionNode` knows nothing about it: the node always calls whatever it was handed as `data.onOpen`,
so entering pick mode just swaps that callback (`picking ? onPick : onOpen`). Escape and the bar's
Cancel button both leave the mode. `onPick` calls
`actions.askCreate(id, layout.placed.get(id)?.version)`. `askCreate` takes an optional `commit` and
`version` pair, which `CreateBranchModal` uses to fork from that commit instead of from a base
branch. The base-branch picker is hidden when `commit` is set, since the backend takes one or the
other, and the dialog's copy talks about "version N" instead of the current branch's latest.

## Pending-version preview

When the working tree is dirty, the map draws one extra node that isn't a commit: `PreviewNode` in
[`VersionNode.tsx`](../src/components/vcs/VersionNode.tsx), node type `"preview"`, one column past
the end of the current lane. It shows a dashed empty frame where the composite would go, a hollow
connector dot, "Version N+1 · not saved yet", the branch chip, and a single dashed "Unsaved changes"
chip, reached by a dashed edge off the tip. Clicking it jumps to the Changes tab. There is exactly
one, and only ever on the current lane: there is one working tree, so a per-branch preview would be
a fiction the backend can't back.

It shows no composite and no per-layer chips on purpose. Both would need `working_diff`, which runs
through `run_heavy`, isn't cached, and (because the map stays mounted for the shell's lifetime)
would refire on every `refreshNonce` bump in every view. The dirty flag comes from the shell's single
`scan_repository` instead (the `dirty` prop, `workingItems.length > 0`), which costs one `stat`.
That scan is hoisted into `RepoShell` and passed to both `Sidebar` and the map, because `Sidebar`
isn't mounted in map view, and a second `useWorkingChanges` inside the always-mounted map would
rescan in every view and race the one `setScanning` flag.

The preview counts as part of the map for fit-view, the minimap (so its `data` carries a `laneColor`
like any node) and "jump to latest", but not for pick mode. Its callback is always `onShowChanges`,
never the panel's `onNodeClick`, so it can't be forked from; `picking` only dims it. Its column is
the lane's last column plus one, not the tip's, because `buildVersionMap`'s collision guard can push
a node right of its generation depth, so the tip isn't reliably the rightmost node on its lane. At
worst the preview is a column further out than needed, and it never overlaps a node.

## Legacy version history

The old History and Branches sidebar views are still there, hidden behind Settings → Appearance →
"Legacy version history" ([`lib/legacyHistory.tsx`](../src/lib/legacyHistory.tsx), the same
context plus `localStorage` shape as Artist Mode, default off). `ActivityBar` filters those two
icons on it, and `RepoShell` snaps back to the map if the toggle goes off while you're on one of
them, so you can't be stranded on a view with no icon. `CommitGraph` and `lib/graph.ts` are
unchanged. `BranchesPanel` now shares its actions with `MapActionBar` through `useBranchActions`
instead of owning them.

The Performance tab rides the same toggle. With Legacy off there's no commit selection left to
drive a diff viewer, so `AppShell`'s `perfShowsMap` flag (`activeView === "performance" &&
!legacy`) shows the map next to the stats sidebar instead of `MainPanel` and `Inspector`, reusing
the same mounted `VersionMapPanel`. With Legacy on, the Performance tab keeps the old diff-viewer
layout.

## Tour hooks

Three of the map's details exist for the first-launch tour (see
[onboarding-and-tour.md](onboarding-and-tour.md#application-tour)).
`VersionNodeData.tourTarget` marks the current branch's tip as the stand-in for "a saved version":
it is the node `jumpToLatest` centers on, so `onlyRenderVisibleElements` keeps it mounted.
`MapActionBar` force-opens its branch `Menu` for the "New version line…" step. And `VersionMap`
closes any open drilldown while the tour is active, because a drilldown replaces the canvas and
takes every map spotlight target with it.
