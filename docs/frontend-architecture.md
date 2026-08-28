# Frontend Architecture

The frontend is a Vite + React 19 + TypeScript app rendered in the Tauri webview. In the desktop
shell it drives the real Rust backend through Tauri `invoke` (commit history, branches,
working-tree scan, per-commit visual diffs, repository lifecycle — see
[version-control.md](version-control.md)). There is **no mock data by default**: in a plain
browser (`npm run dev`, no backend) the data hooks return empty results, repository actions are
no-ops, and the status bar shows a "Browser preview" badge — the browser build is for UI work
only. The one exception is an explicitly opt-in dev fixture,
[`src/lib/mockRepo.ts`](../src/lib/mockRepo.ts): loading `http://localhost:1420/?mock` makes
`useCommits`/`useBranches`/`useCommitDiff` return a hand-written 12-version history with synthetic
composites, so canvas/layout work on the [Version Map](#version-map) can be seen without the
desktop shell. Gated on `import.meta.env.DEV` **and** the query flag, so it's stripped from
production builds.

## Styling

- **Tailwind CSS v4**, configured via `@theme` in [`src/styles/global.css`](../src/styles/global.css).
  Design tokens from [`DESIGN.md`](../DESIGN.md) are mapped to CSS variables and surface as utilities
  (`bg-bg`, `bg-surface-2`, `text-text-muted`, `text-accent`, `rounded-panel`, `font-mono`, …).
- Non-utility tokens (easing curves, durations, z-index scale) live in `:root` and are referenced
  as `z-(--z-sticky)`, `duration-(--dur-normal)`, etc.
- Fonts (Inter, JetBrains Mono) are self-hosted via `@fontsource` for offline use.
- **Color themes** — the `@theme` block's `--color-*` values are Charcoal (the default, no
  override needed); every other theme is an `html[data-theme="…"] { --color-* : … }` block further
  down `global.css` that overrides just the identity tokens (dark themes) or the identity + status/
  diff tokens and `color-scheme` (light themes). See [Theme selector](#theme-selector).

## App shell — the four zones (plus the map's full-width zone)

[`AppShell`](../src/components/shell/AppShell.tsx) splits on the selected repository: with none
selected (fresh install) it renders a welcome state pointing at the top-bar switcher; otherwise
`RepoShell` owns layout and view state and wires the zones. The **Version Map** (default view) is
the odd one out — it drops Sidebar and Inspector entirely and owns the whole well itself, since
the node carries the metadata a Sidebar/Inspector would otherwise show. Every other view uses the
classic four-zone layout:

```
┌─────────────────────────────────────────────────────────────────────────┐
│ TopBar (36px) — repository switcher                                       │
├──────────┬──────────────────────┬──────────────────────┬───────────────┤
│ Activity │ Sidebar              │ Main Panel           │ Inspector     │
│  48px    │  240–320px resizable │  flex: 1             │  280px toggle │
│  fixed   │  changes/history*/   │  diff viewer         │  commit meta  │
│          │  branches*/perf      │                      │               │
└──────────┴──────────────────────┴──────────────────────┴───────────────┘
                        StatusBar (24px, fixed bottom)
             (* history/branches only when Legacy version history is on)
```

Performance is a hybrid: its Sidebar card (`PerformancePanel`) is always present, but the
right-hand content depends on the Legacy toggle — Main Panel + Inspector when Legacy is on (the
row above), or the Version Map, full width, when Legacy is off (no Inspector on the map's own
canvas, same as the pure Map tab — though opening a version there gets its own toggleable
Inspector, see [Version Map](#version-map)). See [Version Map](#version-map) for why, and for
how the map itself is one instance shared across both places.

| Zone | Component | Responsibility |
|------|-----------|----------------|
| Top bar | [`TopBar`](../src/components/shell/TopBar.tsx) | Artwork switcher (one `.kra` the user chose to track — see [per-document-tracking.md](per-document-tracking.md)); local-only — no remote affordances. Also doubles as the **custom title bar** — see [Custom title bar](#custom-title-bar). |
| Activity bar | [`ActivityBar`](../src/components/shell/ActivityBar.tsx) | Icon strip; emits the active view (`changes` \| `map` \| `history` \| `branches` \| `performance`). `history`/`branches` are filtered out unless [Legacy version history](#version-map) is on. The gear opens the [`SettingsModal`](../src/components/shell/SettingsModal.tsx) — Artist-view toggle, a **custom title bar** toggle (see [Custom title bar](#custom-title-bar)), a **Legacy version history** toggle, a **theme selector** (see [Theme selector](#theme-selector)), author name, the **set-aside shelf** (every stash with its origin branch + age, per-row remove and remove-all — see [Stashes](#stashes--setting-work-aside)), and (per repo) preview cache size, compact-storage toggle, a **low-memory diffs** toggle (`lowMemoryDiff` — decodes working-file diff entries one at a time instead of all at once), and "Clean up storage…" (`CleanupModal`: dry-run preview on open, then a confirmed `cleanup_repository` pass, which also reclaims dropped-stash storage). |
| Sidebar | [`Sidebar`](../src/components/shell/Sidebar.tsx) | Resizable; its content **switches on the active view** (see below). Absent on the Map tab. |
| Version Map | [`VersionMapPanel`](../src/components/vcs/VersionMapPanel.tsx) | The default view — see [Version Map](#version-map). Replaces Main Panel + Inspector on the Map tab, and Main Panel + Inspector on Performance when Legacy is off. |
| Main panel | [`MainPanel`](../src/components/MainPanel.tsx) → [`DiffView`](../src/components/vcs/DiffView.tsx) | Renders **one selected file** of the current commit/working diff (art-diff canvas height is drag-resizable), or an empty state. Which file is chosen by the Inspector's file list (`selectedFile`/`onSelectFile`, lifted to `RepoShell`); a multi-file commit no longer stacks every file's diff at once. |
| Inspector | [`Inspector`](../src/components/shell/Inspector.tsx) | Toggleable. On the **History** view: the selected commit's version/hash, author, date, message, and a Restore action. On the **Changes** view it never shows a History commit — a focused changed file gets an "Unsaved changes" header, and a clean tree (nothing focused) gets a neutral "No changes to show" placeholder instead. Either mode's **changed-files list doubles as the main panel's file selector** — click a row to show that file in `DiffView`; a `.kra` row with an embedded document palette gets a palette sub-row that jumps straight to that palette's pane (`focusId`), and standalone palette files get their own row under a separate "Palettes" heading. Also gets a **Selected** section mirroring the diff navigator's pick — a layer's type/visibility/opacity/blend/change/painted bounds, or the composite's size/DPI/color space/layer count. |
| Status bar | [`StatusBar`](../src/components/shell/StatusBar.tsx) | Active file, branch, commit/version count. |

The center toolbar (in `AppShell`) holds the inspector show/hide button. The **Artist view**
toggle now lives in the Settings modal (gear in the activity bar). See [Artist Mode](#artist-mode).

[`BusyOverlay`](../src/components/shell/BusyOverlay.tsx) renders alongside `RepoShell`/
`WelcomeShell` (a sibling in `AppShell`, not inside either zone): a full-screen,
non-dismissible block shown whenever `busyMessage` on the repository context is set — every
write op (commit, branch create/switch/merge/delete, rollback, undo, cleanup) sets a
human-readable label before the call and clears it in a `finally`. Renders nothing when idle.

[`TourOverlay`](../src/components/shell/TourOverlay.tsx) is the last child of `RepoShell`'s root
element (`fixed inset-0`, so it still visually covers all four zones); it renders nothing when the
tour isn't active. See [Application tour](#application-tour).

[`DockerPanel`](../src/components/shell/DockerPanel.tsx) is the reusable panel container (24px title
bar + scroll area) used by the Sidebar and Inspector. Its header's `actions` slot lays out icons
with a small `gap-1` so adjacent buttons (e.g. Changes' rescan + panel-options) don't sit flush
against each other — every panel gets this for free rather than each header spacing its own icons.

## State ownership

State lives in `RepoShell` and flows down via props:

| State | Drives |
|-------|--------|
| `activeView` | Which sidebar panel renders; the active activity-bar icon. Also gates the toolbar header, main-panel diff, and Inspector: switching to `"changes"` immediately drops any History selection from all three (derived `inChanges` flag), regardless of whether a working file happens to be focused yet. Two derived flags built on it decide the map's visibility (see [Version Map](#version-map)): `inMap` (`activeView === "map"`, no Sidebar) and `perfShowsMap` (`activeView === "performance" && !legacy`); `showMap = inMap \|\| perfShowsMap` toggles the map wrapper's `hidden` class. |
| `selectedId` | Selected commit → main-panel diff + inspector, but only while `activeView !== "changes"` and `!showMap`. |
| `inspectorOpen` | Inspector visibility. |
| `focus` | The diff navigator's layer/composite pick (`{ path, id }`), reported up by `ArtDiffView`'s `onFocus` → the Inspector's **Selected** section. |
| `selectedFile` / `selectedFocusId` | Which file (among possibly several in the current diff) `DiffView` renders, and an optional navigator id to seed its view with (e.g. jump straight to an embedded palette). Set by the Inspector's file list; defaults to the diff's first top-level entry and resets when the diff changes and the current selection no longer applies. |

Data comes from the hooks in [`src/lib/repoData.ts`](../src/lib/repoData.ts) — `useCommits`
(branch-scoped history), `useBranches` (local branches + current + tips), `useWorkingChanges`
(the real `scan_repository` result — at most one entry, the tracked artwork), `useWorkingDiff`
(working-tree visual diffs), `useArtLayers` (streamed per-layer rasters) — all keyed by the
selected repository path and the shared `refreshNonce`. `useCommitDiff` (committed visual
diffs) keys only on path + commit id: a commit's diff is immutable once made, so it never needs
a nonce-driven refetch — only `useWorkingDiff`/the working side of `useArtLayers` do, since the
working copy genuinely changes. `useWorkingDiff` still keeps a small stale-while-revalidate cache
(`workingDiffCache`, keyed on path + file, *not* nonce) so re-focusing a file — e.g. tabbing over
to the Version Map and back — repaints the last-known diff immediately instead of blanking to a
spinner, while the real `working_diff` call runs behind it and replaces the paint once it lands.
Derived per render: `currentBranch` (from `useBranches`), `selectedCommit`, and `diff`.

Six pieces of state live **outside** `AppShell`, each in a React context so any component can read
them without prop-drilling: the global Artist Mode flag
([`src/lib/artistMode.tsx`](../src/lib/artistMode.tsx), see [Artist Mode](#artist-mode)), the
Legacy version history flag ([`src/lib/legacyHistory.tsx`](../src/lib/legacyHistory.tsx), same
context-plus-`localStorage` shape as Artist Mode, default **off** — see
[Version Map](#version-map)), the
custom title bar flag ([`src/lib/windowChrome.tsx`](../src/lib/windowChrome.tsx), see
[Custom title bar](#custom-title-bar)), the
selected color theme ([`src/lib/theme.tsx`](../src/lib/theme.tsx), see
[Theme selector](#theme-selector)), the author name
([`src/lib/authorName.tsx`](../src/lib/authorName.tsx) — persisted to `localStorage`,
sent as the `author` on new commits/merges/rollbacks, falling back to `"You"` when unset; also
readable outside React via `readAuthorName()` for `repository.tsx`'s callbacks), and the
selected repository ([`src/lib/repository.tsx`](../src/lib/repository.tsx) — list + `currentId`,
persisted to `localStorage`; the `TopBar` switcher reads it). The repository context also owns
`refreshNonce`/`refresh` (force a scan/history refetch), `discardChanges(paths)` (discard
uncommitted changes — empty `paths` discards everything dirty, otherwise just those relative
paths; used by `ChangesPanel`'s per-file discard and `Sidebar`'s "Discard current changes"), and
the shared `saving` / `busyMessage` / `scanning` busy flags — `saving` locks staging and drives the
`StatusBar` progress bar during a commit, `busyMessage` (a human-readable label, or `null` when
idle) drives the full-screen `BusyOverlay` during any write op, `scanning` spins the Changes
refresh button. All six providers are mounted in [`App.tsx`](../src/App.tsx).

Local, self-contained UI state stays in the leaf components — e.g. the sidebar width
(`Sidebar`), the art-diff canvas height (`ArtDiffView`), modal open/close state (`BranchesPanel`,
`Sidebar`), and the diff view/compare/highlight controls (`ArtDiffView`). The working-tree items
are the one exception: `useWorkingChanges` (below) is called in `Sidebar`, not `ChangesPanel`, and
passed down as props, so the panel and the panel-options actions act on one scan.
Both drag-resizable dimensions use the shared [`useResize`](../src/lib/useResize.ts) hook
(pointer-capture drag, clamped, persisted under a `krita-vc:` key).

## Sidebar views

`Sidebar` is a thin router on `view` (keeping the resizable shell + `DockerPanel` wrapper). It
never mounts for `view === "map"` — the [Version Map](#version-map) owns the whole well instead.
`history` and `branches` only appear in the router (and in `ActivityBar`) when **Legacy version
history** is on; see [Version Map](#version-map) for what replaced them.

- **`history`** — a live **branch switcher** (the `Menu` primitive: pick a branch to switch to it,
  footer row opens the create-branch modal) + [`CommitGraph`](../src/components/vcs/CommitGraph.tsx):
  a git-style graph where each version block (`CommitCard`) is paired with a rail
  ([`CommitGraphRail`](../src/components/vcs/CommitGraphRail.tsx)) drawing its node and the lane lines
  connecting it to its neighbors, so branch divergence and merges read at a glance. History is
  **scoped to the current branch** (`list_commits` returns commits reachable from its tip, so a
  merged branch's versions appear under the target branch). Lane layout is computed by
  [`buildGraph`](../src/lib/graph.ts); node colors are stable **per branch**
  (`branchColorMap` — accent for the current branch, then `info`/`success`/`warning` tokens, a
  deliberate functional exception to the single-accent rule), and branch tips get a `BranchBadge`
  on their commit card. A rollback commit (`Commit.restoredFrom`) gets a dashed **elbow connector**
  back to the version it restored (`buildRevertLinks` + `elbowPath` in `graph.ts`), routed through
  a dedicated gutter left of the lanes so it never overlaps the solid lineage lines — since each
  row is its own isolated rail SVG, `CommitGraph` measures real row pixel centers via a
  `ResizeObserver` to draw this one overlay across non-adjacent rows. Selection drives the main
  panel.
- **`changes`** — [`ChangesPanel`](../src/components/vcs/ChangesPanel.tsx): a "Saving to
  `<BranchBadge>`" header (the current branch — a commit always lands on it), then **the layers
  that changed** since the last version. A store tracks one `.kra`, so the file list this panel
  used to show would always be one row, and staging a subset of a one-file working tree means
  nothing; what the artist wants to know is what moved in the painting. The rows come from
  `useWorkingDiff`'s existing per-layer `change` — **no new backend command** — rolled up so a
  changed group reads as one row with an "+N inside" count rather than spilling its children
  (the backend enumerates layers with `.descendants()`, so children arrive as siblings of their
  group with no parent link; the rollup is honest about being approximate for that reason).
  The rows are **read-only**: choosing which layers go into a version needs a write path that
  synthesizes a `.kra` holding only the ticked ones, and checkboxes that don't bind would be
  worse than none. "Undo all" in the section header, and "Discard current changes" in the
  sidebar's `…` menu, both revert the artwork via the repository context's `discardChanges`. Both
  that button and the whole `…` menu are disabled — dimmed, tooltip swapped to "Checking for
  changes…" — while `scanning` or the diff itself is still `loading`: undo/discard/set-aside all
  read state those two computations are still in the middle of producing, so a click mid-check
  could act on a picture that's about to change. A failed commit's error message is local `commitError` state, reset
  on repo or branch change (an effect keyed on `path`/`currentBranch.name`) — otherwise it outlives
  the commit it belongs to and, since `ChangesPanel` stays mounted across repo/branch switches,
  misleadingly reads as live on an unrelated one. While a commit or discard is in flight the staging controls lock,
  the commit button shows a spinner, the `StatusBar` shows an indeterminate progress bar (shared
  `saving` flag), and `BusyOverlay` blocks the app (`busyMessage`). The same `…` menu also holds
  the **set-aside** actions — see [Stashes](#stashes--setting-work-aside) below.
- **`branches`** — [`BranchesPanel`](../src/components/vcs/BranchesPanel.tsx): the local branch
  list with **real actions** — click a branch to switch, hover (or keyboard-focus) a row for
  "Merge into current" and "Delete" (both behind plain-language confirm modals), "New branch"
  opens the create modal. The create modal's base-branch picker (a plain `<select>`, only shown
  when more than one branch exists) defaults to the current branch — picking another one is passed
  through as `createBranch(name, base)`, which materializes that branch's tree before recording the
  new branch (refused, with a friendly prompt, on unsaved changes). The actions and every dialog
  they raise now live in one hook,
  [`useBranchActions`](../src/components/vcs/useBranchActions.tsx) — `BranchesPanel` used to carry
  its own copy of this state machine, and now just renders `actions.dialogs`. It's shared with the
  Version Map's own action bar (see [Branch actions & pick-a-version](#branch-actions--pick-a-version-mode))
  and the History sidebar's branch switcher, so the dirty-tree routing and confirm copy can't drift
  between the three. It lives in its own file rather than `BranchDialogs.tsx` because it needs
  `SetAsideModal`, and `StashDialogs.tsx` already imports `errorText` from `BranchDialogs.tsx` —
  putting it there would close an import cycle. The backend's dirty-tree error (stable
  `"unsaved changes"` prefix) becomes a friendly `SaveFirstModal` offering three ways out: save
  first (jump to Changes), **set it aside** (stash everything, then retry the blocked switch/merge
  automatically), or cancel. This is a local-only VCS — there are no remotes.
- **`performance`** — [`PerformancePanel`](../src/components/vcs/PerformancePanel.tsx): the
  Performance report — a summary card (average operation times + total storage saved), a
  scrollable per-version card list (stored vs full-copy bytes + % saved + save/compare time), and a
  pinned recent-operations log. Timing is client-side (localStorage); storage comes from the
  `repo_storage_stats` backend command. See [performance-report.md](performance-report.md).
  Always visible (not legacy-gated), but what sits *beside* it in `AppShell` depends on the
  Legacy toggle — the Version Map when Legacy is off, Main Panel + Inspector when it's on. See
  [Version Map](#version-map).

## Version Map

The **default view** ([`VersionMapPanel`](../src/components/vcs/VersionMapPanel.tsx) +
[`VersionNode`](../src/components/vcs/VersionNode.tsx)) and the visual replacement for the
History graph: this branch's line of versions on a pannable, zoomable canvas, laid out
left→right oldest first along a spine. Each node carries the version's after-composite, a
connector dot the spine runs through, and a two-column grid of chips for the layers that changed
(layer-type icon from `friendly.ts`'s `layerTypeIcon()` + an A/M/D glyph in `FileStatusChip`'s
icon-and-color vocabulary). The node *is* the metadata on the map itself — no Sidebar, no
Inspector there. Clicking one opens the full `MainPanel`/`DiffView` in place (back button; a
`Menu` file-picker in the header for a multi-file version), alongside its own toggleable
[`Inspector`](../src/components/shell/Inspector.tsx) — open by default, same restore action and
"Selected" section as the legacy view, hidden/shown with the same icon-button pattern
`AppShell` uses (`SidebarSimple`, toggled in `CommitDrilldown`'s own local state — the map's
"no Inspector" only applies to the un-opened canvas). By default only the current branch is
drawn (`list_commits` is already scoped to its tip); a header toggle draws every branch on its
own lane — see [Branch lanes](#branch-lanes) below.

Built on **React Flow** (`@xyflow/react`, pinned to `12.10.2` — `12.11.4` ships a broken pairing
with `@xyflow/system`, Vite's dep optimizer dies on it). Node positions are computed from the
commit graph (`nodesDraggable={false}`) — nothing is persisted, so a new commit can never leave
the layout stale. Edges come from `parents`, not list adjacency, so a merge commit's second
parent draws its own line into whichever lane it came from.

- **Branch color.** Since lane 0 is now the trunk (see [Branch lanes](#branch-lanes) below), lane
  index alone no longer says where you're standing, so the accent is spent saying it instead:
  `laneColor(lane, currentLane)` returns `var(--color-accent)` for whichever lane the current
  branch is actually on (`MapLayout.currentLane`, read off the current tip's *placement* rather
  than a branch-name map, so a branch with no commits of its own still resolves correctly), and
  every other lane cycles a small local palette (`BRANCH_LANE_COLORS` in `VersionMapPanel.tsx` —
  `info-fg`, `success-fg`, `warning-fg`), deliberately separate from `graph.ts`'s `LANE_COLORS`
  (whose positional lane-0-is-accent is a fixed convention for the legacy History graph and stays
  untouched). A lane's color paints every node's connector dot on it and, mixed 55% toward
  transparent via `color-mix`, the spine between them. Every branch tip's thumbnail additionally
  gets a **detached** `outline`/`outline-offset` ring in its lane color, distinct from the flush
  `ring-accent` used for whichever node is open — on *your* lane those two are now the same color
  and only the geometry (detached vs. flush) tells them apart — plus a branch-name chip under its
  caption.
- **Grid background.** React Flow's `Lines` variant, colored by a `--color-grid` token
  (`src/styles/global.css`) derived from each theme's own `--color-bg` via
  `color-mix(in srgb, var(--color-bg) 75%, black)` — dark themes render a near-black, barely
  visible grid with no per-theme literals; the two light themes override it back to
  `--color-border`.
- **Zoom and LOD.** Wheel zooms toward the cursor, drag pans (`zoomOnScroll`,
  `panOnScroll={false}`) — the same gesture pair as the diff viewer's `useZoomPan`. Below
  `LOD_ZOOM` the caption and chips drop (a boolean `useStore` selector, so a node only re-renders
  when the threshold is actually crossed).
- **Minimap.** Nodes carry explicit `width`/`height` (`NODE_W`/`NODE_H`) since the `MiniMap`
  sizes from the *user* node object, not the measured box. The dim-outside-the-viewport mask and
  the frame around the viewport itself are **ours**, not React Flow's: it paints both as one
  `fillRule="evenodd"` path, so `maskStrokeColor` strokes the mask's *outer* rectangle too — half
  that stroke falls inside the viewBox and reads as a stray accent line down the minimap's edge —
  and a `h`/`v`/`z` path can't take an `rx`. So the `MiniMap` is handed
  `maskColor="transparent"` + `maskStrokeWidth={0}` and `MinimapViewport` draws the two pieces
  separately: an evenodd path for the dim, and a stroke-only `<rect rx>` for the frame. The hole
  in that mask is rounded to the same radius as the frame — a square hole under a rounded stroke
  leaves undimmed slivers in the corners. Two couplings hold it together: the overlay is a
  `Panel` rather than a plain div, so it inherits the same margin and stacking as the `MiniMap`'s
  own panel and lands on it pixel-exactly; and since React Flow's minimap geometry isn't
  exported, `MinimapViewport` re-derives it from `nodes` plus the store `transform`, which is why
  `MINIMAP_W`/`MINIMAP_H`/`MINIMAP_OFFSET_SCALE` are shared constants — they must stay the exact
  values the `MiniMap` itself is passed or the two SVGs drift apart. `offsetScale` is 1.5 rather
  than React Flow's default 5 because this history is far wider than tall: `viewScale` is
  width-driven, and that padding applied in both axes shows up as a gap in the short one.
- **Opening a version** unmounts the *canvas* (not the panel — see below) and swaps in the
  drilldown; the viewport is stashed in a ref beforehand and handed back as `defaultViewport` on
  the way out, so returning to the map doesn't snap back to the oldest version.
- **No backend command of its own.** A node calls the same `useCommitDiff` → `commit_diff` the
  diff viewer does (`afterImage` = the capped, content-addressed composite as a `kvcimg://` URL,
  plus `layers[]` with `change`/`layerType`, `with_rasters = false`), so opening a drilldown is a
  `diffCache` hit, not a second round trip. Off-viewport nodes aren't mounted
  (`onlyRenderVisibleElements`), so they never fetch.
- **Mounted once, for the shell's lifetime.** `AppShell` renders exactly one `VersionMapPanel`
  and toggles a `hidden` class on its wrapper (`showMap`, see [State ownership](#state-ownership))
  rather than conditionally mounting it per view. Unmounting it — as two separate JSX call sites
  (one for the Map tab, one for Performance) used to do — would remount the whole
  `ReactFlowProvider` on every tab switch, silently resetting both the panned/zoomed viewport and
  the open-drilldown `openId`, since neither lives in anything React Flow itself persists.

### Branch lanes

A header toggle (`GitBranch` icon, shown only once another branch exists) switches the map from
the current branch to **all branches, each on its own lane**. It defaults **off**, persists to
`localStorage` (`krita-vc:map-show-all`), and is plain component state rather than another
app-wide context — it is map-local, not a global preference. On, the panel makes its own
`useCommits(repoPath, nonce, true)` call for the backend's wider `allBranches` scope (a union of
the reachable set over every branch tip); off, the path it passes is `""`, which `useCommits`
short-circuits, so the off state costs nothing and draws exactly the commits the shell already
loaded.

The lane/column assignment is a pure function in
[`lib/versionMap.ts`](../src/lib/versionMap.ts) (`buildVersionMap`), deliberately **not** in
`lib/graph.ts`: `buildGraph` lays a DAG out vertically for the legacy rail, where a lane is an x
column; here a lane is a y offset. Both agree lane 0 is the mainline. Three rules carry it:

- **Lane 0 is the trunk's first-parent spine**, walked back from `main`'s tip — *not* the commits
  stamped with its name, since after a merge the folded-in commits still carry *their* branch and
  belong on a side lane. Anchoring on `main` rather than on the branch you're standing on is
  load-bearing and was a real bug: lane 0 used to follow *your* branch, so a fork off main drew as
  one straight line, and main's next commit — same generation depth, so the same column — got
  shunted onto a side lane by the collision guard below. **The trunk must not move when you switch
  branches.** `main`'s tip isn't always in the drawn set (with "show all lines" off, `list_commits`
  is scoped to *your* tip), so the anchor falls back to the newest drawn commit stamped `main`,
  then to the current tip.
- **Everything else groups by `commit.branch`** into lanes 1.. in order of first appearance
  (oldest first), so a lane's color doesn't shuffle between refetches.
- **Column = generation depth** (`1 + max(depth(parents))`), so parallel work on two branches
  lines up in the same column instead of leaving chronological gaps, and a merge lands one column
  past the deeper of its parents. A `(lane, col)` collision guard bumps the column — it shouldn't
  fire, but an invisibly stacked node is a nasty failure.

That same depth is the node's **"Version N"**, so shared ancestors read the same on every lane;
for a linear history it is identical to `friendly.ts`'s positional `versionNumbers()` (still used
by the legacy graph and Inspector). Two lanes can therefore both show "Version 5" — the lane
color and the branch name in the caption disambiguate.

`buildVersionMap` also returns `currentLane` — the lane the current branch sits on, read off the
current tip's *placement* rather than a branch-name map (a branch created but not yet committed on
has no commits of its own and shares its parent's tip node, whose lane is the right answer) — used
by [Branch color](#version-map) above and by the [pending-version preview](#pending-version-preview)
below. [`scripts/checkVersionMap.mjs`](../scripts/checkVersionMap.mjs) pins all of this (trunk stays
on lane 0 standing on either branch, main's next commit lands in the same column as the fork beside
it, the trunk-tip-outside-the-drawn-set fallback, and the gutter-bend corner-radius case below) —
`buildVersionMap` is pure and its only import is `import type` (erased), so Node's native TS
stripping runs the `.ts` file directly with no test runner or config. Wired into `npm run build`
(`node scripts/checkVersionMap.mjs` runs after `tsc`, before `vite build`), so a layout regression
fails the build instead of shipping silently.

### Drawing the line

The spine is **one drawing system — SVG edges, dot to dot**. Both of a node's handles sit on its
connector dot (node center, `SPINE_TOP`) rather than on the node's left/right edges, so a single
edge path spans the source dot, the gutter and the target dot. React Flow draws
`.react-flow__edges` beneath `.react-flow__nodes` and the dot is opaque, so the line reads as
passing through it; the dot row sits in the 8px gap between the thumbnail card and the caption, so
nothing else occludes it. This works because `@xyflow/system`'s `getHandlePosition` uses the
handle's own measured x/y and does **not** snap to the node's bounding box.

It replaced half-width CSS bars drawn inside each node, which bridged React Flow's gutter-only
edges to the centered dot. Two systems could never stay aligned: a 1.5px box shifted
`-translate-y-1/2` lands on a half pixel and rasterizes across two device rows while an SVG stroke
centers cleanly on its path, so the in-node run and the gutter run sat ~1px apart and stepped at
every node edge — and they could disagree on color, the stub using the node's lane and the edge
using the crossing's. Don't reintroduce an in-node segment.

Line color is **opaque** (`color-mix(…, var(--color-bg))`, never `transparent`): two translucent
strokes over the same pixels composite into a brighter, two-tone band that reads as a doubled
line.

A lane-crossing connector carries per-edge `pathOptions`:

```ts
{ offset: STEP_GAP, stepPosition: bendFraction(dx, fork, NODE_PITCH, STEP_GAP), borderRadius: 16 }
```

The bend needs to land in the **middle of the gutter next to the end on the shallower lane** — so
a branch drops out of the spine at the version it started from and climbs back in at the version
it merges into, clear of the node's caption and chips, and never runs alongside the spine (which
doubles the line). `stepPosition` is a *fraction* of the run between the two gap points, not an
absolute x, so [`bendFraction`](../src/lib/versionMap.ts) (in `versionMap.ts` rather than the
panel, so the layout check can call it) solves for the fraction that puts the descent at that
gutter midpoint given `dx` (the horizontal span between the two connector dots), `NODE_PITCH` and
the gap `STEP_GAP` each end runs straight off its handle before it may bend.

`STEP_GAP` (20px) is deliberately small, and deliberately *not* half the gutter width. An earlier
version aimed the gap point itself at the bend (`offset: NODE_PITCH / 2`, `stepPosition: fork ? 0
: 1`) — visually identical, but not equivalent: `getSmoothStepPath` only drops a gap point when it
lands *exactly* on the bend x, and the measured handle positions carry float error, so on the far
end it sometimes didn't. The stray point a hair from the corner collapsed that corner's radius to
~0 — one bend rounded, the other square. Keeping the gap points well clear of the bend (small
`STEP_GAP`) means both corners always get the full radius regardless of that float error;
`scripts/checkVersionMap.mjs` pins the corner radius on both ends for a fork, a one-column merge,
and a three-column merge.

That connector's stroke is a **gradient** between the two lanes' colors, so a fork fades out of
its parent branch's color and a merge fades back into the color it joins. `LaneGradients` emits
one `<linearGradient>` per lane pair that actually has a connector, into its own zero-size `<svg>`
(a `url(#…)` paint reference resolves document-wide, and there's no hook to inject defs into React
Flow's `<svg>` short of a custom edge component). Each must be `gradientUnits="userSpaceOnUse"`
running **purely vertically** between the two lanes' spine y values — user space here is React
Flow's flow coordinates, the same space those y values are already in. That confines the whole
transition to the descent, so the connector's horizontal run at either end is exactly that lane's
own color; which is also what makes the short stretch it shares with the spine before the bend
invisible. An `objectBoundingBox` gradient would smear the transition across the entire path and
tint that shared stretch.

### Branch actions & pick-a-version mode

The map used to only *draw* branches — create/switch/merge/delete lived solely in the legacy
Branches panel. `MapActionBar` (a React Flow `<Panel position="top-left">`, clear of the
bottom-right minimap; `nopan`, same reason `VersionNode`'s thumbnail carries it) now puts those
same actions on the canvas: one `Menu` whose `selected`/`detail`/hover-revealed `action`/`footer`
slots are exactly the legacy Branches panel's row model (switch by selecting, merge/delete via the
row's hover actions, "New branch…" in the footer), so it needed no new primitive — just
[`useBranchActions`](../src/components/vcs/useBranchActions.tsx) (see the [`branches`
view](#sidebar-views) above) wired to a second call site.

Alongside it sits the one action only the map can offer: **"Start a line here…"** enters a
**pick-a-version mode** where the next node click forks a branch at that version, via
`create_branch_at` — written, tested and unreachable in the backend until now, since nothing else
in the app is a version picker. Pick mode is one boolean (`picking`) and `VersionNode` needs no
notion of it: the node always calls whatever it was handed as `data.onOpen`, so entering pick mode
just swaps that callback (`picking ? onPick : onOpen`) rather than teaching the node a new state.
Escape and the bar's own "Cancel" both leave it. `onPick` calls
`actions.askCreate(id, layout.placed.get(id)?.version)` — `useBranchActions`'s `askCreate` takes an
optional `commit`/`version` pair, which `CreateBranchModal` uses to fork from that commit instead
of a base branch (the base-branch picker is hidden when `commit` is set — the backend takes one or
the other, not both) and to word its copy around "version N" rather than "the current branch's
latest".

### Pending-version preview

When the working tree is dirty, the map draws one extra, non-commit node — `PreviewNode` in
[`VersionNode.tsx`](../src/components/vcs/VersionNode.tsx), node type `"preview"` — one column past
the end of the current lane: a dashed empty frame where the composite would go, a **hollow**
connector dot, "Version N+1 · not saved yet", the branch chip, and a single dashed "Unsaved
changes" chip, reached by a dashed edge off the tip. Clicking it jumps to the Changes tab. There is
exactly **one** of these and it only ever sits on the current lane — there is one working tree, so
a per-branch preview would be a fiction the backend can't back.

It shows no composite and no per-layer chips deliberately: both would need `working_diff`, which
is `run_heavy`, uncached, and — since the map is [mounted for the shell's whole
lifetime](#version-map) — would refire on every `refreshNonce` bump in *every* view. The dirty flag
instead comes from the shell's single `scan_repository` (`dirty` prop, `workingItems.length > 0`),
which is one `stat`. It counts as a map citizen for fit-view, the minimap (its node `data` carries
a `laneColor` like any real node) and "jump to latest", but **not** for pick mode — its callback is
always `onShowChanges`, never the panel's `onNodeClick`, so it can't be forked from; `picking` only
dims it. Its column is the lane's **last** column + 1, not the tip's, since `buildVersionMap`'s
collision guard can shift a node right of its own generation depth, so the tip isn't reliably the
rightmost thing on its own lane — off by a column at worst, never overlapping.

**Legacy version history.** The old **History** and **Branches** Sidebar views are still there but
hidden behind Settings → Appearance → "Legacy version history"
([`lib/legacyHistory.tsx`](../src/lib/legacyHistory.tsx), default **off**). `ActivityBar` filters
those two icons on it, and `RepoShell` snaps back to the map if the toggle goes off while you're
standing on one, so you can't be stranded on a view with no icon. `CommitGraph`/`lib/graph.ts` are
untouched; `BranchesPanel` itself now shares its actions with the map's `MapActionBar` via
`useBranchActions` (see [Branch actions & pick-a-version mode](#branch-actions--pick-a-version-mode)
above) rather than owning them outright. The **Performance** tab rides the same toggle: with Legacy off there's no commit selection
left to drive a diff viewer, so `AppShell`'s `perfShowsMap` flag shows the map beside the stats
Sidebar instead of Main Panel + Inspector, reusing the same persistent `VersionMapPanel` instance
above. Legacy on restores the old diff-viewer layout there, unchanged.

**Tour hooks.** Three of the map's own details exist for the first-launch tour (see
[Application tour](#application-tour)): `VersionNodeData.tourTarget` marks the current branch's
tip as the stand-in for "a saved version" (the node `jumpToLatest` centres on, so
`onlyRenderVisibleElements` keeps it mounted), `MapActionBar` force-opens its branch `Menu` for
the "New version line…" step, and `VersionMap` clears any open drilldown while the tour is active
— a drilldown replaces the canvas outright, taking every map spotlight target with it.

## Stashes — setting work aside

"Set aside" (Artist Mode) / "Stash" parks working-tree changes off to the side of history so
they can be brought back later, without polluting commit history — see the backend model in
[version-control.md](version-control.md#stashes--setting-work-aside). The frontend surfaces it in
two places:

- **`Sidebar`'s panel-options `Menu`** (history + changes) is grouped into three
  divider-separated sections: undo/discard, then set-aside, then bring-back (`Menu` gained a
  `MenuItem.separator` flag — a rule above that row — since one `footer` group can only draw one
  divider and this needs two). The set-aside row ("Set this aside") is **changes-view only**,
  since it acts on the working tree, and is gated on `commits.length` — same guard as undo, since
  there's no committed state to revert to otherwise. (There were two rows here, staged and
  everything; with one tracked artwork they became the same action.)
  The two bring-back rows ("Bring back latest" / "Bring back…") are `footer` items shown in
  **both** views, since you might be looking at History when you want a stash back.
- **`SettingsModal`**'s "Set-aside shelf" section lists every stash (label/asset summary +
  origin branch + age) via `useStashes`, with per-row remove and a "Remove all" — a pure
  management view, not a restore path (bringing work back stays in the Sidebar menu above).
  Confirms (`DropStashModal`, `DropAllStashesModal`) render as *sibling* modals next to
  `SettingsModal`, the same pattern `CleanupModal` uses, since `Modal` has no portal.

Dialogs live in [`StashDialogs.tsx`](../src/components/vcs/StashDialogs.tsx): `SetAsideModal`
(label prompt, used by both the Sidebar menu and the save-first prompt's "set it aside" button),
`PickStashModal` (choose which stash to pop), `StashConflictModal` +
`isStashConflictError` (the pop-time `"stash conflict"`-prefixed error, distinct from the
branch/merge `"unsaved changes"` one), plus the `StashIcon`/`UnstashIcon` glyphs and the
`stashTitle`/`stashSummary` label helpers (also reused by `SettingsModal`'s shelf rows). Data
comes from `useStashes` in [`repoData.ts`](../src/lib/repoData.ts) (`list_stashes`, newest first);
mutations (`createStash`, `popStash`, `dropStash`, `dropAllStashes`) live on the repository
context alongside the other write actions.

## Diff viewer

`DiffView` shows **one top-level entry at a time** — `selectedPath` (from the Inspector's file
list, defaulting to the diff's first entry) picks it out of `entries`, and the rest simply aren't
rendered. Embedded palettes (`kind: "palette"`, path `<kra>::<palette-file>`) aren't independently
selectable top-level entries — they're reached via their parent `.kra`'s own selection plus a
`focusId` that seeds the art view's navigator (see below). The selected entry routes by `kind`:

- `"art"` (`.kra`) → [`ArtDiffView`](../src/components/vcs/ArtDiffView.tsx): a visual layer
  diff. The layers + before/after canvas sit in a **drag-resizable region** (handle along its bottom
  edge, height clamped and persisted via `useResize`); when shrunk the layer list and canvas scroll
  internally, so the sections below stay reachable instead of being pushed off-screen. That file's
  embedded palette (if any, matched by the `<kra>::` prefix) is embedded in
  `ArtDiffView`'s `LayerStackPanel` navigator; `initialFocusId` (from `DiffView`'s `focusId`) seeds
  the navigator's initial selection, so clicking a palette sub-row in the Inspector jumps straight
  to that pane instead of defaulting to the composite. Documented in
  [visual-diff-viewer.md](visual-diff-viewer.md).
- `"palette"` (`.gpl`, `.kpl`, `.aco`, `.ase`, selected standalone) →
  [`PaletteDiffView`](../src/components/vcs/PaletteDiffView.tsx) via `StandalonePaletteDiff`
  (defined inline in `DiffView.tsx`): always renders **color swatches** grouped by change
  (Modified / Added / Removed), each swatch showing before/after colors with hex codes. **Not
  gated by Artist Mode.** The `swatches[]` are computed backend-side (`palette.rs`) and rendered
  as-is — no parsing in the frontend. Its header uses `paletteName` (below), not `assetName` —
  Krita's raw palette filenames carry an internal resource-version segment
  (`<name>.<NNNN>.<ext>`, e.g. `sun-set.0006.kpl`) that `assetName` wouldn't strip.
- `kind: "text"` (generic config, settings, …):
  - **Artist Mode on** (default) → `FriendlyFileDiff`: no code, no hunks, no line numbers. A
    one-line friendly summary using `assetKind` + `statusVerb` from
    [`src/lib/friendly.ts`](../src/lib/friendly.ts).
  - **Artist Mode off** → `DiffFileBlock`: the code-style line renderer (line numbers, +/−, hunk
    headers).

## Artist Mode

A single global toggle aimed at the app's audience (artists, not developers). When **on** (the
default), the whole UI swaps technical strings for plain-language labels; when **off**, the original
technical view is shown verbatim. State is persisted to `localStorage`
(`krita-vc:artist-mode`) by the provider in [`src/lib/artistMode.tsx`](../src/lib/artistMode.tsx);
read it with `useArtistMode()`. The toggle lives in the Settings modal (activity-bar gear).
Label helpers live in [`src/lib/friendly.ts`](../src/lib/friendly.ts).

| Surface | Artist Mode on | Artist Mode off |
|---------|----------------|-----------------|
| Non-art diff | Color-swatch / one-line summary (`FriendlyFileDiff`) | Code-style line diff (`DiffFileBlock`) |
| Commit hash (cards, toolbar, Inspector) | `Version N` (`versionLabel`) | Short hash |
| File paths (Inspector, status bar, art header) | Asset name (`assetName`, no folder/extension) | Full path |
| Palette file paths (standalone/embedded palette headers, Inspector palette rows) | Palette name (`paletteName` — also strips Krita's internal `.NNNN` resource-version segment) | Full path |
| Status code (`FileStatusChip`) | Icon + word ("Updated") | Single letter (`M`) |
| Status-bar count | "N versions" | "N commits" |

Layer opacity/blend mode in `LayerStackPanel` are kept as-is in both modes — they're genuine art
concepts, not jargon.

## Custom title bar

The window boots with **no OS-native title bar by default** (`src-tauri/tauri.conf.json`'s
single window sets `decorations: false`). Instead [`TopBar`](../src/components/shell/TopBar.tsx)
doubles as the title bar: when the "Custom title bar" toggle is on and the app is running in the
Tauri shell (`inTauri()`), the `<header>` carries `data-tauri-drag-region` (native drag, no JS
`startDragging()` call needed) and renders right-aligned minimize/maximize/close buttons built on
`@tauri-apps/api/window`'s `getCurrentWindow()`. In browser preview, or with the toggle off,
`TopBar` renders exactly as it does with no window controls at all.

The preference is [`src/lib/windowChrome.tsx`](../src/lib/windowChrome.tsx)
(`WindowChromeProvider`/`useWindowChrome()`), the same localStorage-context shape as Artist Mode
and the theme selector — `krita-vc:custom-titlebar`, default **on**. Unlike those two, flipping it
also drives one live side effect: its effect calls `getCurrentWindow().setDecorations(!customTitleBar)`
whenever the value changes (including on mount, which is what re-applies a previously-saved
"native chrome" choice at boot, since the static Tauri config always starts with decorations
off) — so switching between custom and native chrome takes effect immediately, no restart. The
toggle lives in the Settings modal (activity-bar gear), right under Artist view.

The capabilities needed to drive this from the frontend
(`src-tauri/capabilities/default.json`): `core:window:allow-start-dragging`,
`core:window:allow-minimize`, `core:window:allow-toggle-maximize`, `core:window:allow-close`,
`core:window:allow-set-decorations`.

## Theme selector

Eight color themes — five dark (`charcoal` default, `krita-blue`, `electric-cyan`, `sunset-coral`,
`tokyo-night`, `true-black`) and two light (`charcoal-light`, `studio-light`) — are picked from a
`Menu` in the Settings modal (gear in the activity bar), each option rendered as a `ThemeChip`
(background swatch + accent dot). Themes are **pure CSS palettes**, not component variants:

- [`src/lib/theme.tsx`](../src/lib/theme.tsx) defines the `ThemeId` union and the `THEMES` array
  (id, label, and the `bg`/`accent` swatch colors shown in the picker — kept in sync by hand with
  `global.css`, not derived at runtime). `ThemeProvider` tracks the selected id, persists it to
  `localStorage` (`krita-vc:theme`), and stamps it as `data-theme` on `<html>` so the CSS cascade
  does the rest; `readTheme()`/`applyTheme()` are also called directly (outside React) in
  [`main.tsx`](../src/main.tsx) before first paint, so a saved non-default theme doesn't flash
  Charcoal for a frame.
- [`src/styles/global.css`](../src/styles/global.css) defines Charcoal's colors in the base
  `@theme` block; every other theme is an `html[data-theme="…"]` block overriding the same
  `--color-*` variables (dark themes override just the identity tokens — bg/surface/border/accent/
  text/danger — and inherit status/diff colors from the base; light themes also override status/
  diff colors and flip `color-scheme`). Because Tailwind utilities and the app's own CSS all read
  colors via `var(--color-*)`, switching themes needs no re-render of anything — the browser
  cascade repaints the whole UI instantly.
- **The visual-diff change-highlight is theme-reactive too.** The backend (`raster.rs`) bakes a
  placeholder color into the `diffImage` mask PNG, but only its *alpha* channel is ever used — the
  frontend (`ArtCanvas.tsx`) treats that raster purely as an SVG mask shape and paints the flat
  tint, hatch pattern, dashed outline, and the region-box fallback with `var(--color-accent)`, so
  the diff highlight always matches the active theme's accent (no cache invalidation needed on
  theme switch, since the cached raster's actual color is never displayed). See the `ArtCanvas`
  section of [visual-diff-viewer.md](visual-diff-viewer.md).

## Application tour

A first-launch, one-time spotlight walkthrough of the shell — fired once and never again
automatically.

- [`src/lib/tour.tsx`](../src/lib/tour.tsx) (`TourProvider`/`useTour()`) holds a linear
  `TOUR_STEPS` array (`{tourId, title, body, view?, when?}`, ~33 steps covering every zone) and a
  `stepIndex` state machine (`next`/`back`/`skip`/`restart`/`beginIfFirstTime`). Completion is a
  `localStorage` flag (`krita-vc:tour-completed`) — the same context-plus-flag pattern as Artist
  Mode and the custom title bar toggle. `RepoShell` calls `beginIfFirstTime()` once on mount; it's
  a no-op once the flag is set. A step with a `view` drives `setActiveView` as a side effect, so
  the tour can walk the user through Changes, the Version Map, History, Branches, and Performance
  without them switching tabs themselves.
- The **Version Map group** sits between the Changes steps and the legacy History ones, matching
  the activity bar's own order (changes → map → history → …). It covers every control the map
  offers:

  | `tourId` | gate | what it points at |
  |---|---|---|
  | `map` | — | the activity-bar icon; what the map is, and that drag pans / scroll zooms |
  | `map-empty` | `!hasVersions` | the empty state — the only thing on screen on a fresh install |
  | `map-version` | `hasVersions` | one version card: composite, number + note, changed-layer chips, the tip ring, and that clicking opens the full comparison (which is where non-legacy users hear about the Inspector) |
  | `map-preview` | `dirty` | the dashed pending-version node |
  | `map-branch` | `hasVersions` | the action bar's branch switcher, incl. its hover-revealed merge/delete |
  | `map-branch-new` | `hasVersions` | "New version line…", with the menu force-opened |
  | `map-pick` | `hasVersions` | "Start a line here…" — pick-a-version branching |
  | `map-all-lines` | `hasVersions && hasOtherBranches` | the all-branches lane toggle |
  | `map-view-controls` | `hasVersions` | zoom readout + fit-all + jump-to-newest, as **one** spotlight — three consecutive holes over adjacent 16px buttons read as padding, not as teaching |
  | `map-minimap` | `hasVersions` | the overview, via `MinimapViewport`'s panel |

- **Not every step applies to every shell.** A step whose target isn't in the DOM leaves
  `TourOverlay` with nothing to spotlight, and it renders `null` — no card, no Next, no Skip. So a
  step can carry a `when?: (c: TourConditions) => boolean` predicate over four facts the shell
  knows: `legacy` (the History/Branches tabs exist at all — off by default), `hasVersions` (the
  map draws nodes rather than its empty state), `hasOtherBranches` (the map's "All lines" toggle
  is rendered) and `dirty` (the map draws its pending-version preview node). A plain predicate,
  not a key registry, so negation (`map-empty`) and compound gates (`map-all-lines`) need no
  syntax. `RepoShell` reports the conditions **live** in an effect rather than the provider
  snapshotting them at start: the tour fires from a mount effect while `useCommits`/`useBranches`
  are still in flight, so a snapshot would drop every map step on a repo that does have history.
  The cursor stays an index into `TOUR_STEPS` — not into the filtered list, which resizes
  underneath it — while `stepIndex`/`totalSteps` come from the filtered list, so "Step N of M"
  matches what the user will actually see (19 steps on a fresh install, ~25 on a real repo with
  legacy off, 33 with it on).
- [`TourOverlay`](../src/components/shell/TourOverlay.tsx) renders `null` when inactive. Spotlight
  targets are plain `data-tour-id` attributes: `IconButton` and `Menu`'s `MenuItem` both take an
  optional `tourId` prop that sets it, and a handful of other targets (the repo switcher, the
  branch badge row, the changed-layer list, the commit-graph, the commit message/button, and the
  Version Map's empty state, header controls, action bar and minimap overlay) carry
  `data-tour-id` directly on a wrapper — so no ref plumbing is needed to locate a step's target.
  The one target that isn't literal markup is a map node: `VersionNodeData.tourTarget` flags the
  **current branch's tip**, which is the node `jumpToLatest` centres on and therefore the one
  `onlyRenderVisibleElements` reliably keeps mounted.
  The dim-with-a-hole effect is **four plain opaque `fixed` bands** tiling the viewport around the
  target rect (top/bottom/left/right) plus a fifth, transparent, non-interactive div over the hole
  itself so it never intercepts clicks — deliberately not a box-shadow spread or an SVG mask, both
  of which silently failed to paint in this WebView build. Rect coordinates are rounded to whole
  pixels so the four independently-positioned bands agree on the same integer boundary (raw floats
  from `getBoundingClientRect()` risked a hairline seam between adjoining bands). The callout card
  anchors beside the target for ActivityBar rows and rows inside an open dropdown (whichever edge
  of the target has room to grow into, so a card near the bottom of the window never clips past
  it), and below the target otherwise, clamped on **both** axes — a bottom-right target (the map's
  minimap) or a tall one (a version card) flips the card above the target instead of running off
  the bottom.
- Measurement retries on rAF for ~400ms rather than measuring once. The Version Map is kept
  mounted under `display: none` when another tab is active (see [Version Map](#version-map)) and
  React Flow re-measures its nodes a frame or two after it becomes visible, so a single rAF can
  read a stale or zero-size rect. A target still missing at the deadline makes the tour **step
  over it in the direction of travel** (a `dir` ref, so a Back press doesn't bounce forward);
  that's the backstop that keeps a missing target from blanking the overlay, whatever `when`
  predicates miss.
- Steps that spotlight a row **inside** a `Menu` (the panel-options undo/discard/set-aside rows,
  and the map action bar's "New version line…") need that menu open while the overlay is blocking
  the real click that would normally open it — [`Menu`](../src/components/ui/Menu.tsx) gained a
  `forceOpen` prop (ORed with its normal click-toggled state, so it never fights
  outside-click/Escape handling), driven by `Sidebar`'s `PANEL_OPTION_TOUR_IDS` set and by
  `MapActionBar`'s single-step check.
- `VersionMap` clears any open commit drilldown while the tour is active: a drilldown replaces the
  canvas outright, taking every map spotlight target with it. The first-launch tour can't hit
  that, but "Replay tour" from Settings can.
- Arrow Left/Right step back/forward as well as the card's Back/Next buttons. A press-and-hold
  "Skip" button (`HoldToSkip`, 300ms hold) guards against a single stray click dismissing the
  whole tour.
- Replay anytime via Settings → Appearance → "Replay tour" (`useTour().restart()`, jumps back to
  step 0 without touching the completion flag until the tour reaches its end again).

## Component map

```
AppShell (→ WelcomeShell with no repository, else RepoShell)
├─ TopBar ─ Menu (repository switcher)
├─ ActivityBar ─ SettingsModal (gear) ─┬─ CleanupModal ("Clean up storage…")
│                                       ├─ CheckModal ("Check for problems…", opt-in full scrub)
│                                       └─ set-aside shelf ─ DropStashModal / DropAllStashesModal
├─ Sidebar ─ DockerPanel ─┬─ history*  → Menu (branch switcher) + CommitGraph ─ CommitGraphRail + CommitCard (+ tip BranchBadge)
│  (absent on "map")      ├─ changes   → ChangesPanel ─ FileStatusChip
│                         ├─ branches* → BranchesPanel ─ BranchBadge + useBranchActions (dialogs)
│                         └─ performance → PerformancePanel (summary + per-version cards + recent ops)
├─ VersionMapPanel ─ ReactFlowProvider ─┬─ VersionNode (× one per commit — thumbnail, connector dot, layer chips)
│  (mounted once — see Version Map)     ├─ PreviewNode (pending-changes preview, only while dirty)
│                                       ├─ MapActionBar (switch/merge/delete/create + pick-a-version, via useBranchActions)
│                                       └─ CommitDrilldown (open node) → MainPanel/DiffView, same as below
│                                          + its own toggleable Inspector (open by default)
├─ MainPanel ─ DiffView ──┬─ art     → ArtDiffView ─┬─ LayerStackPanel ─ FileStatusChip
│                         │          (+ 1st palette)  ├─ ArtCanvas        (side-by-side)
│                         │                           └─ CompareSlider ─ ArtCanvas (swipe)
│                         ├─ palette → PaletteDiffView (standalone or via LayerStackPanel)
│                         └─ text  ──┬─ FriendlyFileDiff (Artist Mode on)
│                                    └─ DiffFileBlock     (Artist Mode off)
├─ Inspector ─ DockerPanel ─ FileStatusChip
└─ StatusBar

(* history/branches only when Legacy version history is on)

BusyOverlay (sibling of the above, not nested — renders when `busyMessage` is set)

RepoShell also renders TourOverlay as its own last child (see Application tour)
```

On the Map tab (and Performance without Legacy), `VersionMapPanel` replaces Main Panel + Inspector
(and, on the pure Map tab, Sidebar too) rather than nesting inside them — see
[Version Map](#version-map) for the always-mounted/`hidden`-toggle mechanics. Opening a version
brings the Inspector back, scoped to that drilldown rather than shared with the legacy layout's.

`StashDialogs.tsx` (`SetAsideModal`, `PickStashModal`, `StashConflictModal`) is shared between
`Sidebar`'s panel-options menu and `useBranchActions`'s save-first prompt (in turn shared by
`BranchesPanel`, `Sidebar`'s History branch switcher, and the map's `MapActionBar`); `SettingsModal`
reuses its `stashTitle`/`stashSummary` helpers for the set-aside shelf rows.

The whole tree is wrapped in `ToastProvider` → `RepositoryProvider` → `ThemeProvider` →
`ArtistModeProvider` → `LegacyHistoryProvider` → `AuthorNameProvider` → `WindowChromeProvider` →
`CpuBudgetProvider` → `TourProvider` (see [`App.tsx`](../src/App.tsx) for the exact nesting).
`ToastProvider`/`CpuBudgetProvider` aren't part of the "six pieces of state" above — the toast
queue (`src/lib/toast.tsx`) backs the backup-zip result notification in `ActivityBar`, and the CPU
budget (`src/lib/cpuBudget.tsx`) is the Settings → Storage → "Background CPU use" knob, both
app-global but orthogonal to `AppShell`'s own layout/view state.

Shared primitives: [`IconButton`](../src/components/ui/IconButton.tsx) (tactile icon chip),
[`Button`](../src/components/ui/Button.tsx), [`Menu`](../src/components/ui/Menu.tsx) (dropdown:
outside-click + Esc to close), [`FileStatusChip`](../src/components/vcs/FileStatusChip.tsx),
[`BranchBadge`](../src/components/vcs/BranchBadge.tsx),
[`Tooltip`](../src/components/ui/Tooltip.tsx) — the app's only hover/focus tooltip, replacing every
native `title=` (the ~13 places still using `title` are the unrelated `Modal` heading prop, not a
tooltip). Positioning mirrors `Menu`'s measure-then-place approach (portal to `document.body`, a
`useLayoutEffect` reads the trigger's and the popover's own rects to flip above/below and clamp
horizontally within the viewport), and it animates in per [`DESIGN.md`](../DESIGN.md)'s
state/animation matrix (`--dur-fast`/`--ease-out`, `scale(0.97)→1` + `opacity 0→1`) on the
previously-unused `--z-tooltip` layer. `IconButton` and `Switch` wrap themselves in it internally, so every call site
that already passed a `label` picked it up for free. Two deliberate exceptions: it's never nested
inside another `Tooltip` (e.g. a "Switch branch" trigger wrapping `BranchBadge` scopes its own
label to the non-badge chrome, since `BranchBadge` already tooltips its own truncated name — two
tooltips stacking on hover would be worse than native `title`'s single-innermost-wins behavior),
and `TourOverlay`'s "Skip" button keeps native `title=` since it sits inside the tour's full-screen
overlay, whose `--z-tour` layer sits *above* `--z-tooltip` by design — a portaled Tooltip there
would render invisibly behind the dimming bands.

Cross-cutting libs: [`src/lib/artistMode.tsx`](../src/lib/artistMode.tsx) (the toggle context),
[`src/lib/legacyHistory.tsx`](../src/lib/legacyHistory.tsx) (the Legacy version history toggle
context — see [Version Map](#version-map)),
[`src/lib/repository.tsx`](../src/lib/repository.tsx) (selected-repository context + all
mutating actions: commit/rollback/undo, stash create/pop/drop/drop-all, branch
create/switch/merge/delete),
[`src/lib/repoData.ts`](../src/lib/repoData.ts) (data hooks: commits, branches, diffs, layers,
stashes),
[`src/lib/useResize.ts`](../src/lib/useResize.ts) (shared drag-resize hook),
[`src/lib/graph.ts`](../src/lib/graph.ts) (history-graph lane layout + `branchColorMap` — the
*legacy* History graph; the Version Map's own lane palette is a separate, smaller one local to
`VersionMapPanel.tsx`, see [Version Map](#version-map)),
[`src/lib/svgArt.ts`](../src/lib/svgArt.ts) (SVG layer compositing for the diff canvas),
[`src/lib/friendly.ts`](../src/lib/friendly.ts) (label helpers — `assetName`, `paletteName`,
`assetKind`, `statusVerb`, `layerTypeLabel`, `layerChangeLabel`,
`versionNumbers`/`versionLabel`),
[`src/lib/format.ts`](../src/lib/format.ts) (timestamps),
[`src/lib/tour.tsx`](../src/lib/tour.tsx) (the first-launch tour's step list + state machine —
see [Application tour](#application-tour)),
[`src/lib/mockRepo.ts`](../src/lib/mockRepo.ts) (dev-only `?mock` fixture — see the top of this
document).
