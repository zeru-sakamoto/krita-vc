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
row above), or the Version Map, full width, when Legacy is off (no Inspector either, same as the
pure Map tab). See [Version Map](#version-map) for why, and for how the map itself is one
instance shared across both places.

| Zone | Component | Responsibility |
|------|-----------|----------------|
| Top bar | [`TopBar`](../src/components/shell/TopBar.tsx) | Repository switcher (folder the user designated); local-only — no remote affordances. Also doubles as the **custom title bar** — see [Custom title bar](#custom-title-bar). |
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
bar + scroll area) used by the Sidebar and Inspector.

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
(the real `scan_repository` result + a UI-only `staged` flag per file), `useWorkingDiff`
(working-tree visual diffs), `useArtLayers` (streamed per-layer rasters) — all keyed by the
selected repository path and the shared `refreshNonce`. `useCommitDiff` (committed visual
diffs) keys only on path + commit id: a commit's diff is immutable once made, so it never needs
a nonce-driven refetch — only `useWorkingDiff`/the working side of `useArtLayers` do, since the
working copy genuinely changes. Derived per render: `currentBranch` (from `useBranches`),
`selectedCommit`, and `diff`.

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
+ per-file staged flag are the one exception: `useWorkingChanges` (below) is called in `Sidebar`,
not `ChangesPanel`, and passed down as props, since `Sidebar`'s "Discard current changes" action
needs the same staged/unstaged split without a second scan.
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
  `<BranchBadge>`" header (the current branch — a commit always lands on it), then working-tree
  changes grouped Staged / Unstaged, with per-file and **Stage all / Unstage all** toggles.
  Staging is **real**: `commit_snapshot`'s optional `paths` (`commit::commit_selected` in Rust)
  restricts the commit to the staged relative paths, leaving the rest dirty. Hitting "Commit
  version" with nothing staged, or with only some files staged, shows a confirm `Modal` first
  (commit everything anyway / commit only the staged files) before calling through; all-staged
  commits right away. Each row also has a **discard** button (reverts just that file to its last
  saved version, behind a confirm modal); the sidebar's `…` menu adds "Discard current changes"
  (all *unstaged* files at once — staged files are left alone), both backed by the repository
  context's `discardChanges`. A failed commit's error message is local `commitError` state, reset
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
  new branch (refused, with a friendly prompt, on unsaved changes). Shared dialogs live in
  [`BranchDialogs.tsx`](../src/components/vcs/BranchDialogs.tsx); the backend's dirty-tree error
  (stable `"unsaved changes"` prefix) becomes a friendly `SaveFirstModal` offering three ways out:
  save first (jump to Changes), **set it aside** (stash everything, then retry the blocked
  switch/merge automatically), or cancel. This is a local-only VCS — there are no remotes.
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
icon-and-color vocabulary). The node *is* the metadata — clicking one opens the full
`MainPanel`/`DiffView` in place (back button; a `Menu` file-picker stands in for the Inspector's
file list on a multi-file version). Only the current branch is drawn (`list_commits` is already
scoped to its tip).

Built on **React Flow** (`@xyflow/react`, pinned to `12.10.2` — `12.11.4` ships a broken pairing
with `@xyflow/system`, Vite's dep optimizer dies on it). Node positions are computed from the
commit graph (`nodesDraggable={false}`) — nothing is persisted, so a new commit can never leave
the layout stale. Edges come from `parents`, not list adjacency, so a merge commit's second
parent already draws its own line once branches land (`LANE_PITCH`/non-zero `y` are the reserved
seam for that).

- **Branch color.** A small local palette in `VersionMapPanel.tsx`
  (`BRANCH_LANE_COLORS` — `info-fg`, `success-fg`, `warning-fg`, `accent`, cycled by
  `laneColor(lane)`), deliberately separate from `graph.ts`'s `LANE_COLORS` (whose lane-0-is-accent
  is a fixed convention for the legacy History graph and stays untouched). Lane 0 — today's only
  lane, the current/main branch — colors every node's connector dot and, mixed 55% toward
  transparent via `color-mix`, the spine between them. The branch tip's thumbnail additionally
  gets a **detached** `outline`/`outline-offset` ring in that same color, distinct from the flush
  `ring-accent` used for whichever node is open. When divergent branches land, each new lane is
  just `laneColor(laneIndex)`.
- **Grid background.** React Flow's `Lines` variant, colored by a `--color-grid` token
  (`src/styles/global.css`) derived from each theme's own `--color-bg` via
  `color-mix(in srgb, var(--color-bg) 75%, black)` — dark themes render a near-black, barely
  visible grid with no per-theme literals; the two light themes override it back to
  `--color-border`.
- **Minimap and zoom.** Wheel zooms toward the cursor, drag pans (`zoomOnScroll`,
  `panOnScroll={false}`) — the same gesture pair as the diff viewer's `useZoomPan`. Below
  `LOD_ZOOM` the caption and chips drop (a boolean `useStore` selector, so a node only re-renders
  when the threshold is actually crossed). Nodes carry explicit `width`/`height` (`NODE_W`/
  `NODE_H`) since the `MiniMap` sizes from the *user* node object, not the measured box.
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

**Legacy version history.** The old **History** and **Branches** Sidebar views are still there but
hidden behind Settings → Appearance → "Legacy version history"
([`lib/legacyHistory.tsx`](../src/lib/legacyHistory.tsx), default **off**). `ActivityBar` filters
those two icons on it, and `RepoShell` snaps back to the map if the toggle goes off while you're
standing on one, so you can't be stranded on a view with no icon. `CommitGraph`/`BranchesPanel`/
`lib/graph.ts` are untouched — branch create/switch/merge/delete still live only in the Branches
panel. The **Performance** tab rides the same toggle: with Legacy off there's no commit selection
left to drive a diff viewer, so `AppShell`'s `perfShowsMap` flag shows the map beside the stats
Sidebar instead of Main Panel + Inspector, reusing the same persistent `VersionMapPanel` instance
above. Legacy on restores the old diff-viewer layout there, unchanged.

## Stashes — setting work aside

"Set aside" (Artist Mode) / "Stash" parks working-tree changes off to the side of history so
they can be brought back later, without polluting commit history — see the backend model in
[version-control.md](version-control.md#stashes--setting-work-aside). The frontend surfaces it in
two places:

- **`Sidebar`'s panel-options `Menu`** (history + changes) is grouped into three
  divider-separated sections: undo/discard, then set-aside, then bring-back (`Menu` gained a
  `MenuItem.separator` flag — a rule above that row — since one `footer` group can only draw one
  divider and this needs two). The two set-aside rows ("Set aside staged files" / "Set aside
  everything") are **changes-view only**, since they act on the working tree, and are gated on
  `commits.length` — same guard as undo, since there's no committed state to revert to otherwise.
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

- [`src/lib/tour.tsx`](../src/lib/tour.tsx) (`TourProvider`/`useTour()`) holds a flat, linear
  `TOUR_STEPS` array (`{tourId, title, body, view?}`, ~28 steps covering every zone) and a
  `stepIndex` state machine (`next`/`back`/`skip`/`restart`/`beginIfFirstTime`). Completion is a
  `localStorage` flag (`krita-vc:tour-completed`) — the same context-plus-flag pattern as Artist
  Mode and the custom title bar toggle. `RepoShell` calls `beginIfFirstTime()` once on mount; it's
  a no-op once the flag is set. A step with a `view` drives `setActiveView` as a side effect, so
  the tour can walk the user through Changes, History, Branches, and Performance without them
  switching tabs themselves.
- [`TourOverlay`](../src/components/shell/TourOverlay.tsx) renders `null` when inactive. Spotlight
  targets are plain `data-tour-id` attributes: `IconButton` and `Menu`'s `MenuItem` both take an
  optional `tourId` prop that sets it, and a handful of other targets (the repo switcher, the
  branch badge row, staged/unstaged sections, the commit-graph, the commit message/button) carry
  `data-tour-id` directly on a wrapper — so no ref plumbing is needed to locate a step's target.
  The dim-with-a-hole effect is **four plain opaque `fixed` bands** tiling the viewport around the
  target rect (top/bottom/left/right) plus a fifth, transparent, non-interactive div over the hole
  itself so it never intercepts clicks — deliberately not a box-shadow spread or an SVG mask, both
  of which silently failed to paint in this WebView build. Rect coordinates are rounded to whole
  pixels so the four independently-positioned bands agree on the same integer boundary (raw floats
  from `getBoundingClientRect()` risked a hairline seam between adjoining bands). The callout card
  anchors beside the target for ActivityBar rows and rows inside an open dropdown (whichever edge
  of the target has room to grow into, so a card near the bottom of the window never clips past
  it), and below the target otherwise, clamped so it never pushes past the right edge either.
- Steps that spotlight a row **inside** the panel-options `Menu` (undo, discard, and the four
  set-aside/bring-back rows) need that menu open while the overlay is blocking the real click that
  would normally open it — [`Menu`](../src/components/ui/Menu.tsx) gained a `forceOpen` prop (ORed
  with its normal click-toggled state, so it never fights outside-click/Escape handling), driven
  by `Sidebar`'s `PANEL_OPTION_TOUR_IDS` set.
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
│                         ├─ branches* → BranchesPanel ─ BranchBadge + BranchDialogs (create/save-first modals)
│                         └─ performance → PerformancePanel (summary + per-version cards + recent ops)
├─ VersionMapPanel ─ ReactFlowProvider ─┬─ VersionNode (× one per commit — thumbnail, connector dot, layer chips)
│  (mounted once — see Version Map)     └─ CommitDrilldown (open node) → MainPanel/DiffView, same as below
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
[Version Map](#version-map) for the always-mounted/`hidden`-toggle mechanics.

`StashDialogs.tsx` (`SetAsideModal`, `PickStashModal`, `StashConflictModal`) is shared between
`Sidebar`'s panel-options menu and `BranchesPanel`/`Sidebar`'s save-first prompt; `SettingsModal`
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
[`BranchBadge`](../src/components/vcs/BranchBadge.tsx).

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
