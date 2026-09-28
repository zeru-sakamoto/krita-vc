# Frontend architecture

The frontend is a Vite + React 19 + TypeScript app rendered in the Tauri webview. In the desktop
shell it drives the Rust backend through Tauri `invoke`: history, branches, the working-tree scan,
visual diffs and the painting lifecycle (see [version-control.md](version-control.md)).

There is no mock data by default. In a plain browser (`npm run dev`, no backend) the data hooks
return empty results, repository actions do nothing, and the status bar shows a "Browser preview"
badge; the browser build is for UI work only. The one exception is an opt-in dev fixture,
[`src/lib/mockRepo.ts`](../src/lib/mockRepo.ts). Loading `http://localhost:1420/?mock` makes
`useCommits`, `useBranches` and `useCommitDiff` return a hand-written 12-version history with
synthetic composites, so canvas and layout work on the [Version Map](version-map.md) can be seen
without the desktop shell. It is gated on `import.meta.env.DEV` and on the query flag, so it is
stripped from production builds and never fires by accident.

Features with their own pages: the [Version Map](version-map.md), [layer-subset
staging](layer-staging.md) in the Changes panel, [setting work aside](stashes.md), [backup and
restore](backup-and-restore.md), the [first-launch welcome and tour](onboarding-and-tour.md), and
the [visual diff viewer](visual-diff-viewer.md).

## Styling

- Tailwind CSS v4, configured through `@theme` in
  [`src/styles/global.css`](../src/styles/global.css). Design tokens from
  [`DESIGN.md`](../DESIGN.md) become CSS variables and surface as utilities (`bg-bg`,
  `bg-surface-2`, `text-text-muted`, `text-accent`, `rounded-panel`, `font-mono` and so on).
- Tokens that aren't utilities (easing curves, durations, the z-index scale) live in `:root` and are
  referenced as `z-(--z-sticky)`, `duration-(--dur-normal)` and so on.
- **Type scale.** Six `@theme` steps generate `text-title` (20) · `text-heading` (15) ·
  `text-body` (13) · `text-dense` (12) · `text-caption` (11) · `text-micro` (10). There is no
  separate mono size; mono content is `text-dense font-mono`. An arbitrary `text-[Npx]` anywhere is a
  bug, and the whole app was migrated off them.
- **Icon scale.** Phosphor takes its size as a React prop, not CSS, so this one scale can't live in
  `@theme`. It lives in [`src/lib/iconSize.ts`](../src/lib/iconSize.ts) as `ICON`: `inline` 12 ·
  `dense` 14 · `default` 16 · `toolbar` 24 · `display` 32 (empty-state art only, never a control).
  A literal `size={N}` on an icon is a bug.
- **Motion is always explicit.** Every `transition-*` carries a `duration-(--dur-*)` and an
  `ease-(--ease-out)`. A bare `transition-colors` silently inherits Tailwind's own 150 ms
  `cubic-bezier(0.4, 0, 0.2, 1)`, which isn't one of the app's curves.
- **Transition the property Tailwind actually emits.** v4 compiles `translate-*`, `scale-*` and
  `rotate-*` to the standalone `translate`, `scale` and `rotate` CSS properties, not to
  `transform`. So `transition-transform` on an element moved by `translate-x-*` animates nothing and
  the element snaps. Keep `transition-transform` for elements that write an explicit `transform:`
  string (`Slider`'s thumb, `ArtCanvas`'s zoom wrapper). This fails silently, with no warning.
- Fonts (Inter, JetBrains Mono) are self-hosted through `@fontsource` so the app works offline.
- **Color themes.** The `@theme` block's `--color-*` values are Charcoal, the default. Every other
  theme is an `html[data-theme="…"] { --color-*: … }` block further down `global.css` that overrides
  the identity tokens (dark themes) or the identity, status and diff tokens plus `color-scheme`
  (light themes). See [Theme selector](#theme-selector).

## App shell

[`AppShell`](../src/components/shell/AppShell.tsx) splits on the selected painting. With none
selected (a fresh install) it renders a start screen (`WelcomeShell`) that points at the switcher
and at "Restore from a backup…". Otherwise `RepoShell` owns layout and view state and wires up the zones. The
Version Map, the default view, is the odd one out: it drops the Sidebar and Inspector and owns the
whole well, because each node carries the metadata they would otherwise show. Every other view uses
the classic four-zone layout:

```
┌─────────────────────────────────────────────────────────────────────────┐
│ TopBar (44px): artwork switcher · window controls                         │
├──────────┬──────────────────────┬──────────────────────┬───────────────┤
│ Activity │ Sidebar              │ Main panel           │ Inspector     │
│  48px    │  240-320px resizable │  flex: 1             │  280px toggle │
│  fixed   │  changes/history*/   │  diff viewer         │  commit meta  │
│          │  branches*/perf      │                      │               │
└──────────┴──────────────────────┴──────────────────────┴───────────────┘
                        StatusBar (24px, fixed bottom)
             (* history and branches only when Legacy version history is on)
```

Performance is a hybrid. Its Sidebar card (`PerformancePanel`) is always there, but what sits beside
it depends on the Legacy toggle: the main panel and Inspector when Legacy is on (as in the diagram),
or the Version Map, full width, when it's off. See
[version-map.md](version-map.md#legacy-version-history) for why, and for how the same map instance
serves both places.

| Zone | Component | What it does |
|------|-----------|--------------|
| Top bar | [`TopBar`](../src/components/shell/TopBar.tsx) | The artwork switcher: a searchable dialog (`SwitchArtworkModal`) listing the paintings you track, with "Track an artwork…", "Restore from a backup…" ([backup-and-restore.md](backup-and-restore.md)) and a per-row "Stop tracking". Local-only, with no remote affordances. It doubles as the [custom title bar](#custom-title-bar). |
| Activity bar | [`ActivityBar`](../src/components/shell/ActivityBar.tsx) | The icon strip. It emits the active view (`changes` \| `map` \| `history` \| `branches` \| `performance`), and filters out `history` and `branches` unless Legacy version history is on. A zip button above the gear opens [`BackupModal`](../src/components/shell/BackupModal.tsx). The gear opens [`SettingsModal`](../src/components/shell/SettingsModal.tsx); see [Settings](#settings). |
| Sidebar | [`Sidebar`](../src/components/shell/Sidebar.tsx) | Resizable. Its content switches on the active view (see [Sidebar views](#sidebar-views)). Absent on the Map tab. |
| Version Map | [`VersionMapPanel`](../src/components/vcs/VersionMapPanel.tsx) | The default view ([version-map.md](version-map.md)). Replaces the main panel and Inspector on the Map tab, and on Performance when Legacy is off. |
| Main panel | [`MainPanel`](../src/components/MainPanel.tsx) → [`DiffView`](../src/components/vcs/DiffView.tsx) | Renders one selected entry of the current version or working diff (the art-diff canvas height can be dragged), or an empty state. The entry is chosen in the Inspector's file list (`selectedFile` and `onSelectFile`, lifted to `RepoShell`); a version with several entries no longer stacks them all. Shows "Analyzing changes…" while the diff loads. |
| Inspector | [`Inspector`](../src/components/shell/Inspector.tsx) | Toggleable. On History: the selected version's number or hash, author, date, note, and "Restore this version". On Changes it never shows a History version; a focused working file gets an "Unsaved changes" header, and a clean tree gets a neutral "No changes to show" placeholder. In both modes its changed-files list doubles as the main panel's selector: click a row to show that entry, and a `.kra` row with an embedded palette gets a sub-row that jumps straight to that palette (`focusId`). A **Selected** section mirrors the diff navigator's pick: a layer's type, visibility, opacity, blend mode, change and painted bounds, or the composite's size, DPI, color space and layer count. |
| Status bar | [`StatusBar`](../src/components/shell/StatusBar.tsx) | Active file, branch, and version count, plus a progress bar while a version saves. |

The center toolbar (in `AppShell`) holds the Inspector's show and hide button. The Artist view toggle
lives in Settings (see [Artist Mode](#artist-mode)).

[`BusyOverlay`](../src/components/shell/BusyOverlay.tsx) renders next to `RepoShell` and the start
screen (a sibling in `AppShell`, not inside either): a full-screen block that can't be dismissed,
shown whenever `busyMessage` on the repository context is set. Every write operation (commit,
branch create, switch, merge and delete, rollback, undo, cleanup) sets a readable label before the
call and clears it in a `finally`. It renders nothing when idle.
[`OnboardingOverlay`](../src/components/shell/OnboardingOverlay.tsx) renders beside it while the
welcome is showing, and [`TourOverlay`](../src/components/shell/TourOverlay.tsx) is the last child
of `RepoShell`'s root while the tour runs (`fixed inset-0`, so it still covers all four zones). Both
are described in [onboarding-and-tour.md](onboarding-and-tour.md). They load on first use
(`React.lazy`, behind a `Suspense` with no fallback), and so do `SettingsModal` and `RestoreModal`:
all four open rarely, so they stay out of the startup chunk, which is 54 kB smaller for it.

`RepoShell` listens for the DOM `window` `"focus"` event (a plain `addEventListener`, no Tauri
capability needed) and calls the repository context's `refresh()`, the same bump the "Rescan for
changes" button uses. So switching back from Krita after a save shows the change without a click.
DOM `window` focus tracks the OS window regaining focus, not focus moving between elements, so
clicking between panels doesn't fire it. It is throttled to `FOCUS_REFRESH_THROTTLE_MS` (30 s, in
`AppShell.tsx`). The scan itself is cheap (a `stat`, plus one read of the painting per save it
hasn't seen yet, since it remembers the hash of a saved-but-unversioned file), so the throttle is
there to avoid spinner churn in a quick save-and-switch-back loop, not for backend cost, and it's
skipped while `scanning`
is already true (read through a ref, so the listener is created once and never goes stale).

[`DockerPanel`](../src/components/shell/DockerPanel.tsx) is the reusable bento card (a 40 px title
bar plus a scroll area) the Sidebar and Inspector use. Its header's `actions` slot spaces its icons
with a small `gap-1`, so adjacent buttons (Changes' rescan and panel options, for example) never sit
flush and no panel spaces its own icons.

The header is also exported on its own as `PanelHeader` (`title`, `leading`, `meta`, `actions`,
`pad`), because five other places had hand-rolled copies of the same `h-10 border-b bg-surface-2`
bar and had already drifted apart on padding: `AppShell`'s main card, `Inspector`, the Version Map's
header and its drilldown, and `ArtDiffView`'s diff toolbar. The last of those has no card around it
at all (it sits in a `bg-bg` column), which is why the header is exported separately instead of
forcing every caller through the full `<section>` wrapper. `title` is deliberately a `string`: it
carries the reserved uppercase caption style, so richer content goes into `leading` or `meta`. That
caption style belongs to the card-header level only. Headings inside a card use the `subheading`
step (`text-body font-medium`), which keeps the two levels readable as different.

## State ownership

State lives in `RepoShell` and flows down through props:

| State | Drives |
|-------|--------|
| `activeView` | Which Sidebar panel renders and which activity-bar icon is active. It also gates the toolbar header, the main-panel diff and the Inspector: switching to `"changes"` immediately drops any History selection from all three (the derived `inChanges` flag), whether or not a working file is focused yet. Two derived flags decide the map's visibility: `inMap` (`activeView === "map"`, no Sidebar) and `perfShowsMap` (`activeView === "performance" && !legacy`). `showMap = inMap \|\| perfShowsMap` toggles the map wrapper's `hidden` class. |
| `selectedId` | The selected version, which drives the main-panel diff and the Inspector, but only while `activeView !== "changes"` and `!showMap`. |
| `inspectorOpen` | Inspector visibility. |
| `focus` | The diff navigator's layer or composite pick (`{ path, id }`), reported up by `ArtDiffView`'s `onFocus` to the Inspector's Selected section. |
| `selectedFile`, `selectedFocusId` | Which entry of the current diff `DiffView` renders, and an optional navigator id to open it on (for example, jump straight to an embedded palette). Set by the Inspector's file list; defaults to the diff's first top-level entry and resets when the diff changes and the selection no longer applies. |

Data comes from the hooks in [`src/lib/repoData.ts`](../src/lib/repoData.ts): `useCommits`
(history scoped to the current branch), `useBranches` (local branches, the current one and their
tips), `useWorkingChanges` (the real `scan_repository` result, which has at most one entry, the
tracked painting), `useWorkingDiff` (visual diffs of the working tree) and `useArtLayers` (streamed
per-layer rasters). All of them key on the selected painting's path and the shared `refreshNonce`.
`useCommitDiff` keys only on the path and the commit id: a version's diff never changes once it
exists, so it never needs a nonce-driven refetch. Its session cache (`diffCache`) keeps 300 results,
a few KB each now that rasters travel as `kvcimg` URLs, so panning the Version Map back and forth
doesn't refetch nodes it just drew, and a call already in flight is shared (`diffInflight`): opening
a node whose thumbnail is still loading asks for the very same diff. Only `useWorkingDiff` and the working side of
`useArtLayers` do, because the working copy really changes. `useWorkingDiff` also keeps a small
stale-while-revalidate cache (`workingDiffCache`, keyed on path and file, not the nonce), so
refocusing a file, for example after a trip to the Version Map and back, repaints the last known diff
immediately instead of blanking to a spinner while the real `working_diff` call runs. Derived on each
render: `currentBranch` (from `useBranches`), `selectedCommit` and `diff`.

Several pieces of state live outside `AppShell`, each in a React context so any component can read
them without prop drilling. `App.tsx` nests the providers as `ToastProvider` → `RepositoryProvider`
→ `ThemeProvider` → `ArtistModeProvider` → `LegacyHistoryProvider` → `AuthorNameProvider` →
`WindowChromeProvider` → `CpuBudgetProvider` → `OnboardingProvider` → `TourProvider`.

- The selected painting, [`src/lib/repository.tsx`](../src/lib/repository.tsx): the list and
  `currentId`, persisted to `localStorage`, which the `TopBar` switcher reads. It also owns
  `refreshNonce` and `refresh` (force a rescan and history refetch), all the write actions, and the
  shared busy flags. `saving` locks the layer ticks and drives the `StatusBar` progress bar during a
  commit, `busyMessage` (a readable label, or `null` when idle) drives `BusyOverlay` during any write,
  and `scanning` spins the Changes rescan button. `discardChanges(paths)` discards uncommitted
  changes (empty `paths` discards everything dirty) and backs both "Undo all" in Changes and
  "Discard current changes" in the panel menu.
- Artist Mode, [`src/lib/artistMode.tsx`](../src/lib/artistMode.tsx), see [Artist Mode](#artist-mode).
- Legacy version history, [`src/lib/legacyHistory.tsx`](../src/lib/legacyHistory.tsx), the same
  context plus `localStorage` shape as Artist Mode, default off. See
  [version-map.md](version-map.md#legacy-version-history).
- The custom title bar, [`src/lib/windowChrome.tsx`](../src/lib/windowChrome.tsx), see
  [Custom title bar](#custom-title-bar).
- The color theme, [`src/lib/theme.tsx`](../src/lib/theme.tsx), see [Theme selector](#theme-selector).
- The author name, [`src/lib/authorName.tsx`](../src/lib/authorName.tsx), persisted to
  `localStorage` and sent as the `author` of new commits, merges and rollbacks, falling back to
  `"You"`. `readAuthorName()` reads it outside React for `repository.tsx`'s callbacks.
- The CPU budget, [`src/lib/cpuBudget.tsx`](../src/lib/cpuBudget.tsx), see
  [cpu-headroom.md](cpu-headroom.md).
- The welcome and the tour, [`src/lib/onboarding.tsx`](../src/lib/onboarding.tsx) and
  [`src/lib/tour.tsx`](../src/lib/tour.tsx), see [onboarding-and-tour.md](onboarding-and-tour.md).
- The toast, [`src/lib/toast.tsx`](../src/lib/toast.tsx), a single global slot (`useToast().show`;
  a new message replaces the old one). It carries the repository context's failure notices
  (couldn't start tracking, couldn't delete a history) and the right-click guard's nudge.

Small, self-contained UI state stays in the leaf components: the Sidebar's width, the art-diff
canvas height (`ArtDiffView`), modal open and close state, and the diff view's compare and highlight
controls. Two data hooks are exceptions, called in `RepoShell` instead of the component that renders
their result, and passed down as props:

- `useWorkingChanges`. The Changes panel and the Version Map both need the one dirty-tree scan, and
  `Sidebar` unmounts in map view, so a second call anywhere below would rescan on every view switch
  and race the one `scanning` flag.
- `useStorageStats`. `PerformancePanel` only mounts while `activeView === "performance"`, so the
  hook lives above it to keep its last answer across view switches. It only fetches while that view
  is showing (its `active` argument), and only when the painting or `refreshNonce` moved since the
  answer it holds, so switching back is instant. It used to fetch at startup and after every write
  and focus refresh in every view, and on a long history of a large painting the report is a heavy
  job (see [performance-report.md](performance-report.md)).

Both drag-resizable sizes use the shared [`useResize`](../src/lib/useResize.ts) hook (a
pointer-capture drag, clamped, persisted under a `krita-vc:` key).

## Sidebar views

`Sidebar` is a thin router on `view` that keeps the resizable shell and the `DockerPanel` wrapper.
It never mounts for `view === "map"`, where the [Version Map](version-map.md) owns the whole well.
`history` and `branches` only appear (in the router and in `ActivityBar`) when Legacy version
history is on.

The panel header's ⋮ button opens the panel-options `Menu`. In Changes it has three groups: "Undo
the last version" and "Discard current changes"; "Set this aside"; then "Bring back latest" and
"Bring back…". In History it only has undo. See [stashes.md](stashes.md#in-the-desktop-app) for the
set-aside rows. The menu, and Changes' "Undo all", are disabled (dimmed, with the tooltip changed to
"Checking for changes…") while `scanning` or the working diff is still loading: undo, discard and
set aside all read state those two are still producing, so a click in the middle could act on a
picture that's about to change.

- **`changes`**: [`ChangesPanel`](../src/components/vcs/ChangesPanel.tsx). A "Saving to
  `<BranchBadge>`" header (a version always lands on the current branch), then the layers that
  changed since the last version, each a checkbox, and a note field with the save button. How the
  rows are built and what a partial save stores is in [layer-staging.md](layer-staging.md). "Undo
  all" in the section header reverts the painting through the repository context's
  `discardChanges`, behind a confirm. A failed commit's message is local `commitError` state, reset
  when the painting or branch changes (an effect keyed on `path` and `currentBranch.name`);
  otherwise it would outlive the commit it belongs to and, since `ChangesPanel` stays mounted across
  switches, read as current on an unrelated painting. While a commit or discard runs, the ticks lock,
  the button spins, the `StatusBar` shows an indeterminate progress bar (the shared `saving` flag),
  and `BusyOverlay` blocks the app (`busyMessage`).
- **`history`** (legacy): a live branch switcher (a `Menu`: pick a branch to switch to it, the
  footer row opens the create dialog) above [`CommitGraph`](../src/components/vcs/CommitGraph.tsx), a
  git-style graph where each version (`CommitCard`) is paired with a rail
  ([`CommitGraphRail`](../src/components/vcs/CommitGraphRail.tsx)) that draws its node and the lane
  lines to its neighbors, so divergence and merges read at a glance. History is scoped to the current
  branch (`list_commits` returns what's reachable from its tip, so a merged branch's versions appear
  under the target). [`buildGraph`](../src/lib/graph.ts) computes the lanes. Node colors are stable
  per branch (`branchColorMap`: accent for the current branch, then the `info`, `success` and
  `warning` tokens, a deliberate exception to the single-accent rule), and branch tips get a
  `BranchBadge` on their card. A rollback version (`Commit.restoredFrom`) gets a dashed elbow
  connector back to the version it restored (`buildRevertLinks` and `elbowPath` in `graph.ts`),
  routed through a gutter left of the lanes so it never overlaps the solid lineage lines. Each row is
  its own rail SVG, so `CommitGraph` measures the real row centers with a `ResizeObserver` to draw
  that one overlay across rows that aren't adjacent. Selecting a version drives the main panel.
- **`branches`** (legacy): [`BranchesPanel`](../src/components/vcs/BranchesPanel.tsx), the local
  branch list with working actions. Click a branch to switch; hover (or focus) a row for "Merge into
  current" and "Delete", both behind plain-language confirms; "New branch" opens the create dialog.
  Its base-branch picker (a plain `<select>`, shown only when more than one branch exists) defaults
  to the current branch, and picking another passes `createBranch(name, base)`, which materializes
  that branch's tree before recording the new one (refused, with a friendly prompt, on unsaved
  changes). The actions and every dialog they raise live in one hook,
  [`useBranchActions`](../src/components/vcs/useBranchActions.tsx) (the panel just renders
  `actions.dialogs`), shared with the Version Map's
  action bar and the History switcher so the dirty-tree routing and confirm copy can't drift between
  the three (see [version-map.md](version-map.md#branch-actions-and-pick-a-version-mode)). The
  backend's dirty-tree error (the stable `"unsaved changes"` prefix) becomes `SaveFirstModal`, with
  three ways out: save first (go to Changes), set it aside (stash everything, then retry the blocked
  switch or merge), or cancel.
- **`performance`**: [`PerformancePanel`](../src/components/vcs/PerformancePanel.tsx), a summary
  card (average operation times and total storage saved), a scrolling list of per-version cards
  (stored size against a full copy, the share saved, save and compare time) and a pinned
  recent-operations log. Timing is client-side and recomputed from `localStorage` on every mount,
  which is cheap. Storage figures come from `useStorageStats`, hoisted into `RepoShell` (see above).
  See [performance-report.md](performance-report.md). Always visible, not legacy-gated.

## Settings

The gear opens `SettingsModal`, the single home for preferences. It has four tabs on the left, a
fixed list whether or not a painting is selected. A tab whose settings need a painting shows "Open
an artwork to see these settings." instead of disappearing, so the tabs never jump around.

- **Appearance**: the Artist view toggle, the custom title bar toggle, Legacy version history, your
  name, the theme picker, "Replay tour" and "Replay welcome".
- **Performance**: "Background CPU use" (app-global, so it renders outside the painting gate and
  says it applies everywhere; see [cpu-headroom.md](cpu-headroom.md)) and the per-painting
  "Low-memory diffs" toggle (`lowMemoryDiff`), which decodes a working-file diff one archive entry
  at a time.
- **Storage**: "Where version history is kept" (app-global, `get_store_root` and `set_store_root`;
  changing it moves nothing and only decides where the next painting's store is created, which the
  copy says plainly), the per-painting "Preview cache size" (`cacheMaxBytes`, 128 MB to 2 GB) and
  "Compact storage for heavily-revised art" (`tilePixelDeltas`) through
  `get_repo_config`/`set_repo_config`, "Clean up storage…" (`CleanupModal`: a dry run on open, then
  a confirmed `cleanup_repository` pass; on a `"version history is damaged"` refusal,
  `isDamagedHistoryError`, it says cleanup is unavailable and offers "Check for problems…" in place
  of Clean up, which swaps it for the check dialog), "Check for problems…" (`CheckModal`, over this painting,
  every tracked painting, or only the ones never checked, with an optional full read-back), and a
  line saying when the last backup was made.
- **Set-Aside** ("Stashes" with Artist Mode off): the shelf, see [stashes.md](stashes.md).

Confirm dialogs opened from Settings (`CleanupModal`, `CheckModal`, `DropStashModal`,
`DropAllStashesModal`) render as siblings of `SettingsModal`, because `Modal` has no portal.

## Diff viewer

`DiffView` shows one top-level entry at a time. `selectedPath` (from the Inspector's file list,
defaulting to the diff's first entry) picks it out of `entries`, and the rest aren't rendered.
Embedded palettes (`kind: "palette"`, path `<kra>::<palette-file>`) aren't selectable as top-level
entries; they're reached through their parent `.kra` plus a `focusId` that opens the art view's
navigator on them. The selected entry routes by `kind`:

- `"art"` (`.kra`) → [`ArtDiffView`](../src/components/vcs/ArtDiffView.tsx), a visual layer diff.
  The layer list and before and after canvas sit in a drag-resizable region (a handle along its
  bottom edge, height clamped and persisted through `useResize`). When it's shrunk, the layer list
  and canvas scroll inside it, so the sections below stay reachable. The file's first changed
  embedded palette (the first entry matching the `<kra>::` prefix; any others in the same version
  aren't shown) appears in `LayerStackPanel`'s navigator, and
  `initialFocusId` (from `DiffView`'s `focusId`) opens the navigator on it, so clicking a palette
  sub-row in the Inspector jumps straight to that pane. See
  [visual-diff-viewer.md](visual-diff-viewer.md).
- `"palette"` → [`PaletteDiffView`](../src/components/vcs/PaletteDiffView.tsx): color swatches
  grouped by change (Modified, Added, Removed), each showing before and after colors with hex codes.
  Not gated by Artist Mode. The `swatches[]` are computed in the backend (`palette.rs`) and rendered
  as they arrive. Every palette entry is one embedded in a `.kra`, since standalone palette files
  aren't tracked, so `DiffView` never selects one on its own. The header uses `paletteName`, not
  `assetName`: Krita's raw palette filenames carry an internal resource-version segment
  (`<name>.<NNNN>.<ext>`, for example `sun-set.0006.kpl`) that `assetName` wouldn't strip.
- `kind: "text"`, which is only ever a deleted `.kra` or one that couldn't be rasterized →
  `FriendlyFileDiff` in both modes: no code, no hunks, no line numbers, just a one-line summary built
  from `assetKind` and `statusVerb` in [`src/lib/friendly.ts`](../src/lib/friendly.ts). The backend
  sends no lines for it, so the code-style renderer that Artist Mode off used to pick
  (`DiffFileBlock`) only ever drew an empty list, and is gone. An embedded palette that won't parse
  on either side is left out of the diff rather than degraded to text.

## Artist Mode

A single global toggle aimed at the app's audience, artists rather than developers. When it's on
(the default), the whole UI swaps technical strings for plain-language labels; when it's off, the
technical view is shown as-is. The provider in [`src/lib/artistMode.tsx`](../src/lib/artistMode.tsx)
persists it to `localStorage` (`krita-vc:artist-mode`); read it with `useArtistMode()`. The toggle
is "Artist view" in Settings → Appearance. The label helpers live in
[`src/lib/friendly.ts`](../src/lib/friendly.ts).

| Surface | Artist Mode on | Artist Mode off |
|---------|----------------|-----------------|
| Commit hash (cards, toolbar, Inspector) | `Version N` (`versionLabel`) | Short hash |
| File paths (Inspector, status bar, art header) | Asset name (`assetName`, no folder or extension) | Full path |
| Palette paths (palette headers, Inspector palette rows) | Palette name (`paletteName`, which also strips Krita's `.NNNN` resource-version segment) | Full path |
| Status code (`FileStatusChip`) | Icon and word ("Updated") | Single letter (`M`) |
| Status-bar count | "N versions" | "N commits" |
| Branch actions | "version line", "Bring it in", "Remove" | "branch", "Merge", "Delete" |

Layer opacity and blend mode in `LayerStackPanel` look the same in both modes; they're real art
concepts, not jargon. New UI should prefer the friendly wording and put any unavoidable technical
detail behind Artist Mode being off.

## Custom title bar

The window starts with no OS title bar by default (the single window in
`src-tauri/tauri.conf.json` sets `decorations: false`). [`TopBar`](../src/components/shell/TopBar.tsx)
doubles as the title bar instead. When the "Custom title bar" toggle is on and the app is running in
the Tauri shell (`inTauri()`), its `<header>` carries `data-tauri-drag-region` (native dragging, with
no JavaScript `startDragging()` call) and renders minimize, maximize and close buttons on the right,
built on `@tauri-apps/api/window`'s `getCurrentWindow()`. In browser preview, or with the toggle
off, `TopBar` renders without window controls.

The preference is [`src/lib/windowChrome.tsx`](../src/lib/windowChrome.tsx)
(`WindowChromeProvider`, `useWindowChrome()`), the same `localStorage` context shape as Artist Mode
and the theme (`krita-vc:custom-titlebar`, default on). Unlike those, flipping it has a live side
effect: its effect calls `getCurrentWindow().setDecorations(!customTitleBar)` whenever the value
changes, including on mount, which is what re-applies a saved "native frame" choice at startup,
since the static config always starts with decorations off. So switching between the custom and the
native frame takes effect immediately, with no restart. The toggle is in Settings → Appearance,
under Artist view.

The capabilities this needs, in `src-tauri/capabilities/default.json`:
`core:window:allow-start-dragging`, `core:window:allow-minimize`,
`core:window:allow-toggle-maximize`, `core:window:allow-close` and
`core:window:allow-set-decorations`.

## Theme selector

There are eight themes: six dark (`charcoal`, the default, `krita-blue`, `electric-cyan`,
`sunset-coral`, `tokyo-night`, `true-black`) and two light (`gallery`, `overcast`), each a palette in
its own right rather than an inverted dark one. They're picked from a `Menu` in Settings →
Appearance, each option drawn as a `ThemeChip` (a background swatch with an accent dot). Themes are
pure CSS palettes, not component variants.

- [`src/lib/theme.tsx`](../src/lib/theme.tsx) defines the `ThemeId` union and the `THEMES` array
  (id, label, and the `bg` and `accent` swatch colors the picker shows, kept in sync with
  `global.css` by hand rather than derived at runtime). `ThemeProvider` tracks the selected id,
  persists it to `localStorage` (`krita-vc:theme`) and stamps it as `data-theme` on `<html>`, and
  the CSS cascade does the rest. `readTheme()` and `applyTheme()` are also called directly in
  [`main.tsx`](../src/main.tsx), outside React and before the first paint, so a saved theme doesn't
  flash Charcoal for a frame.
- [`src/styles/global.css`](../src/styles/global.css) defines Charcoal's colors in the base
  `@theme` block. Every other theme is an `html[data-theme="…"]` block that overrides the same
  `--color-*` variables. Dark themes override only the identity tokens (background, surfaces,
  border, accent, text, danger) and inherit the status and diff colors; light themes also override
  status and diff colors and flip `color-scheme`. Tailwind utilities and the app's own CSS all read
  colors through `var(--color-*)`, so switching themes re-renders nothing; the browser repaints the
  whole UI from the cascade. The same blocks also match `[data-theme-preview="…"]`, which is how the
  welcome screen's cards show themes that aren't active (see
  [onboarding-and-tour.md](onboarding-and-tour.md#first-launch-welcome)).
- The diff highlight follows the theme too. The backend (`raster.rs`) bakes a placeholder color into
  the `diffImage` mask, but only its alpha channel is used: `ArtCanvas.tsx` treats the raster as an
  SVG mask and paints the tint, hatch, dashed outline and region-box fallback with
  `var(--color-accent)`. So the highlight always matches the active accent, and switching themes
  needs no cache invalidation, because the cached raster's own color is never shown. See
  [visual-diff-viewer.md](visual-diff-viewer.md#artcanvas).

## Component map

```
AppShell (the start screen with no painting, otherwise RepoShell)
├─ TopBar ─ SwitchArtworkModal (artwork switcher) ─ RestoreModal ("Restore from a backup…")
│                                                  └─ RestoreCompareModal (sibling, "Compare versions")
├─ ActivityBar ─┬─ BackupModal (zip button: several paintings, one archive)
│               └─ SettingsModal (gear) ─┬─ CleanupModal ("Clean up storage…")
│                                        ├─ CheckModal ("Check for problems…")
│                                        └─ Set-Aside tab ─ DropStashModal / DropAllStashesModal
├─ Sidebar ─ DockerPanel ─┬─ history*    → Menu (branch switcher) + CommitGraph ─ CommitGraphRail + CommitCard
│  (absent on "map")      ├─ changes     → ChangesPanel ─ FileStatusChip
│                         ├─ branches*   → BranchesPanel ─ BranchBadge + useBranchActions (dialogs)
│                         └─ performance → PerformancePanel
├─ VersionMapPanel ─ ReactFlowProvider ─┬─ VersionNode (one per version)
│  (mounted once, see version-map.md)   ├─ PreviewNode (only while there are unsaved changes)
│                                       ├─ MapActionBar (branch actions, pick-a-version)
│                                       └─ CommitDrilldown (an opened version) → MainPanel/DiffView
│                                          + its own toggleable Inspector
├─ MainPanel ─ DiffView ──┬─ art     → ArtDiffView ─┬─ LayerStackPanel ─ FileStatusChip
│                         │          (+ palettes)   ├─ ArtCanvas        (side by side)
│                         │                         └─ CompareSlider ─ ArtCanvas (swipe)
│                         ├─ palette → PaletteDiffView
│                         └─ text    → FriendlyFileDiff
├─ Inspector ─ DockerPanel ─ FileStatusChip
└─ StatusBar

(* history and branches only when Legacy version history is on)

BusyOverlay and OnboardingOverlay: siblings of the above, not nested
TourOverlay: RepoShell's last child
(SettingsModal, RestoreModal, OnboardingOverlay and TourOverlay load on first use)
```

On the Map tab, and on Performance without Legacy, `VersionMapPanel` replaces the main panel and
Inspector (and on the Map tab the Sidebar too) instead of nesting inside them. Opening a version
brings an Inspector back, scoped to that drilldown.

`StashDialogs.tsx` (`SetAsideModal`, `PickStashModal`, `StashConflictModal`) is shared between the
panel-options menu and `useBranchActions`'s save-first prompt, and `SettingsModal` reuses its
`stashTitle` and `stashSummary` helpers for the shelf rows.

## Shared UI primitives

These live in `src/components/ui/` and are the only button and toggle types in the app; a
hand-rolled one is a bug. Before the redesign the app had nine separate button treatments.

- [`Button`](../src/components/ui/Button.tsx): `default`, `primary`, `destructive` and `ghost`, in
  `sm` and `md`.
- [`IconButton`](../src/components/ui/IconButton.tsx): a raised, tactile chip that sinks when
  pressed (not the old flat button with no chrome until hover; see `DESIGN.md` → Krita Design
  Influence for why that was reversed). Uses `ICON.default` unless told otherwise.
- [`Switch`](../src/components/ui/Switch.tsx), [`Slider`](../src/components/ui/Slider.tsx),
  [`Checkbox`](../src/components/ui/Checkbox.tsx), [`Radio`](../src/components/ui/Radio.tsx).
- [`Modal`](../src/components/ui/Modal.tsx) and [`Menu`](../src/components/ui/Menu.tsx) (a
  dropdown that closes on an outside click or Escape). `Menu.tsx` also exports `Select`, the same
  surface driven by a value instead of by actions, which is why there's no `Select.tsx`.
- [`Tooltip`](../src/components/ui/Tooltip.tsx), the app's only hover and focus tooltip, which
  replaced every native `title=` (the few remaining `title` props are `Modal`'s heading, not
  tooltips). It positions itself the way `Menu` does: it portals to `document.body`, and a
  `useLayoutEffect` reads the trigger's and its own rectangles to flip above or below and clamp
  horizontally to the viewport. It animates in per `DESIGN.md`'s state and animation matrix
  (`--dur-fast`, `--ease-out`, `scale(0.97)` to 1 with opacity 0 to 1) on the `--z-tooltip` layer.
  `IconButton` and `Switch` wrap themselves in it, so every call site that passed a `label` got it for
  free. Two exceptions: it's never nested inside another `Tooltip` (a "Switch branch" trigger that
  wraps `BranchBadge` gives its own label to the non-badge chrome, since `BranchBadge` already
  tooltips its truncated name), and the tour's Skip button keeps a native `title=`, because the
  tour's layer sits above `--z-tooltip` and a portaled tooltip would render behind the dimming.
- [`FileStatusChip`](../src/components/vcs/FileStatusChip.tsx) and
  [`BranchBadge`](../src/components/vcs/BranchBadge.tsx), shared by the shell and the VCS panels.

Type sizes come from the six `text-*` tokens and icon sizes from `ICON`, as described under
[Styling](#styling).

Two app-wide pieces live in `src/lib/` instead, because neither is a control. `toast.tsx` is the
single-slot toast described above. [`rightClickGuard.tsx`](../src/lib/rightClickGuard.tsx)
suppresses the `mousedown` default for non-left clicks over a button: the browser applies `:active`
for any button, and a tactile chip visibly sinking on a right-click reads as a dead click. A burst of
right-clicks nudges the user toward the left button through the toast.

### Floating surfaces animate away, too

`Tooltip`, `Menu` and `Modal` fade and scale out at `--dur-fast` and `--ease-out` on every way of
dismissing them, and while they exit they're `pointer-events-none`, so a surface halfway through
fading can't swallow a click meant for what's behind it. `Menu` and `Tooltip` own their open state
and use [`useExitTransition`](../src/lib/useExitTransition.ts), which keeps them mounted through the
exit and then drops them.

`Modal` can't do that, because its parent mounts it (`{show && <Modal/>}`). So it inverts the
problem and holds `onClose` back until the transition has run. Two consequences:

- Its `footer` takes a render prop, `footer={(close) => …}`, and a dismiss button must call that
  `close`, not the parent's `onClose`, which would unmount instantly and skip the exit. An action
  that completes (Delete, Merge, Restore) still unmounts immediately, as it should: the dialog is
  finished, not dismissed.
- The unmount is driven by `transitionend`, not a timer. A timer was tried and doesn't work. On a
  large dialog, the re-render plus the repaint of the nested `backdrop-filter` chain (`.scrim`'s blur
  under the panel's own) delays the transition's start by about 85 ms, so a timer set to the exit
  duration fires halfway through the fade and triggers a `transitioncancel`, which looks like the
  dialog snapping shut.
  A fallback timer is kept only for the case where the transition is suppressed and `transitionend`
  never fires. The same nested blur is why the dialog's fade resolves in about three steps while
  `Menu`'s is smooth; `will-change` on either layer was measured and doesn't help.

## Cross-cutting libraries

- [`artistMode.tsx`](../src/lib/artistMode.tsx), [`legacyHistory.tsx`](../src/lib/legacyHistory.tsx),
  [`windowChrome.tsx`](../src/lib/windowChrome.tsx), [`theme.tsx`](../src/lib/theme.tsx),
  [`authorName.tsx`](../src/lib/authorName.tsx), [`cpuBudget.tsx`](../src/lib/cpuBudget.tsx),
  [`onboarding.tsx`](../src/lib/onboarding.tsx), [`tour.tsx`](../src/lib/tour.tsx),
  [`toast.tsx`](../src/lib/toast.tsx): the contexts listed under [State ownership](#state-ownership).
- [`repository.tsx`](../src/lib/repository.tsx): the selected-painting context and every write
  action (commit, rollback, undo, discard, set aside and bring back, branch create, switch, merge and
  delete, backup and restore).
- [`repoData.ts`](../src/lib/repoData.ts): the data hooks for commits, branches, diffs, layers and
  stashes.
- [`checkedRepos.ts`](../src/lib/checkedRepos.ts): the `localStorage` set of paintings that have
  been through "Check for problems", used by the check's "never checked" scope.
- [`useResize.ts`](../src/lib/useResize.ts) (the shared drag-resize hook),
  [`useZoomPan.ts`](../src/lib/useZoomPan.ts) (the diff viewer's wheel zoom and drag pan),
  [`useExitTransition.ts`](../src/lib/useExitTransition.ts) (keeps a floating surface mounted
  through its exit; also exports `EXIT_MS`).
- [`iconSize.ts`](../src/lib/iconSize.ts): the `ICON` scale.
- [`graph.ts`](../src/lib/graph.ts): the legacy History graph's lanes and `branchColorMap`. The
  Version Map's lane layout is [`versionMap.ts`](../src/lib/versionMap.ts), and its lane palette is a
  separate, smaller one in `VersionMapPanel.tsx`.
- [`svgArt.ts`](../src/lib/svgArt.ts): SVG layer compositing for the diff canvas.
- [`friendly.ts`](../src/lib/friendly.ts): label helpers (`assetName`, `paletteName`, `assetKind`,
  `statusVerb`, `layerTypeLabel`, `layerTypeIcon`, `layerChangeLabel`, `versionNumbers`,
  `versionLabel`).
- [`format.ts`](../src/lib/format.ts): timestamps.
- [`perf.ts`](../src/lib/perf.ts): the Performance tab's client-side timing (see
  [performance-report.md](performance-report.md)).
- [`tauri.ts`](../src/lib/tauri.ts): `inTauri()`, the shell check the browser-preview fallbacks
  branch on.
- [`mockRepo.ts`](../src/lib/mockRepo.ts): the dev-only `?mock` fixture described at the top.

All data flows through Tauri `invoke`, keyed by the selected painting's path. The component and prop
boundaries (`Repository`, `DiffEntry`, `Commit` including its `parents`, `Branch` including `tip`,
and `WorkingChange`) are the contract between `repoData.ts` and `repository.tsx` and the UI. The
context and type are still called `Repository` even though each one is a single painting; that name
is the contract with every panel, and renaming it would buy nothing the doc comments don't already
say.
