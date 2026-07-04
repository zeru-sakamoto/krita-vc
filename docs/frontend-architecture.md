# Frontend Architecture

The frontend is a Vite + React 19 + TypeScript app rendered in the Tauri webview. In the desktop
shell it drives the real Rust backend through Tauri `invoke` (commit history, branches,
working-tree scan, per-commit visual diffs, repository lifecycle — see
[version-control.md](version-control.md)). There is **no mock data**: in a plain browser
(`npm run dev`, no backend) the data hooks return empty results, repository actions are no-ops,
and the status bar shows a "Browser preview" badge — the browser build is for UI work only.

## Styling

- **Tailwind CSS v4**, configured via `@theme` in [`src/styles/global.css`](../src/styles/global.css).
  Design tokens from [`DESIGN.md`](../DESIGN.md) are mapped to CSS variables and surface as utilities
  (`bg-bg`, `bg-surface-2`, `text-text-muted`, `text-accent`, `rounded-panel`, `font-mono`, …).
- Non-utility tokens (easing curves, durations, z-index scale) live in `:root` and are referenced
  as `z-(--z-sticky)`, `duration-(--dur-normal)`, etc.
- Fonts (Inter, JetBrains Mono) are self-hosted via `@fontsource` for offline use.

## App shell — the four zones

[`AppShell`](../src/components/shell/AppShell.tsx) splits on the selected repository: with none
selected (fresh install) it renders a welcome state pointing at the top-bar switcher; otherwise
`RepoShell` owns layout and view state and wires the zones:

```
┌─────────────────────────────────────────────────────────────────────────┐
│ TopBar (36px) — repository switcher                                       │
├──────────┬──────────────────────┬──────────────────────┬───────────────┤
│ Activity │ Sidebar              │ Main Panel           │ Inspector     │
│  48px    │  240–320px resizable │  flex: 1             │  280px toggle │
│  fixed   │  changes/history/    │  diff viewer         │  commit meta  │
│          │  branches            │                      │               │
└──────────┴──────────────────────┴──────────────────────┴───────────────┘
                        StatusBar (24px, fixed bottom)
```

| Zone | Component | Responsibility |
|------|-----------|----------------|
| Top bar | [`TopBar`](../src/components/shell/TopBar.tsx) | Repository switcher (folder the user designated); local-only — no remote affordances. Also hosts "Clean up storage…" (`CleanupModal`: dry-run preview on open, then a confirmed `cleanup_repository` pass). |
| Activity bar | [`ActivityBar`](../src/components/shell/ActivityBar.tsx) | Icon strip; emits the active view (`changes` \| `history` \| `branches`). |
| Sidebar | [`Sidebar`](../src/components/shell/Sidebar.tsx) | Resizable; its content **switches on the active view** (see below). |
| Main panel | [`MainPanel`](../src/components/MainPanel.tsx) → [`DiffView`](../src/components/vcs/DiffView.tsx) | Renders the selected commit's diff (art-diff canvas height is drag-resizable), or an empty state. |
| Inspector | [`Inspector`](../src/components/shell/Inspector.tsx) | Toggleable; selected commit's version/hash, author, date, message, changed files. |
| Status bar | [`StatusBar`](../src/components/shell/StatusBar.tsx) | Active file, branch, commit/version count. |

The center toolbar (in `AppShell`) also holds the **Artist view** toggle (paintbrush) and the
inspector show/hide button. See [Artist Mode](#artist-mode).

[`BusyOverlay`](../src/components/shell/BusyOverlay.tsx) renders alongside `RepoShell`/
`WelcomeShell` (a sibling in `AppShell`, not inside either zone): a full-screen,
non-dismissible block shown whenever `busyMessage` on the repository context is set — every
write op (commit, branch create/switch/merge/delete, rollback, undo, cleanup) sets a
human-readable label before the call and clears it in a `finally`. Renders nothing when idle.

[`DockerPanel`](../src/components/shell/DockerPanel.tsx) is the reusable panel container (24px title
bar + scroll area) used by the Sidebar and Inspector.

## State ownership

State lives in `RepoShell` and flows down via props:

| State | Drives |
|-------|--------|
| `activeView` | Which sidebar panel renders; the active activity-bar icon. |
| `selectedId` | Selected commit → main-panel diff + inspector. |
| `inspectorOpen` | Inspector visibility. |

Data comes from the hooks in [`src/lib/repoData.ts`](../src/lib/repoData.ts) — `useCommits`
(branch-scoped history), `useBranches` (local branches + current + tips), `useWorkingDiff`
(working-tree visual diffs), `useArtLayers` (streamed per-layer rasters) — all keyed by the
selected repository path and the shared `refreshNonce`. `useCommitDiff` (committed visual
diffs) keys only on path + commit id: a commit's diff is immutable once made, so it never needs
a nonce-driven refetch — only `useWorkingDiff`/the working side of `useArtLayers` do, since the
working copy genuinely changes. Derived per render: `currentBranch` (from `useBranches`),
`selectedCommit`, and `diff`.

Two pieces of state live **outside** `AppShell`, each in a React context so any component can read
them without prop-drilling: the global Artist Mode flag
([`src/lib/artistMode.tsx`](../src/lib/artistMode.tsx), see [Artist Mode](#artist-mode)) and the
selected repository ([`src/lib/repository.tsx`](../src/lib/repository.tsx) — list + `currentId`,
persisted to `localStorage`; the `TopBar` switcher reads it). The repository context also owns
`refreshNonce`/`refresh` (force a scan/history refetch) and the shared `saving` / `busyMessage` /
`scanning` busy flags — `saving` locks staging and drives the `StatusBar` progress bar during a
commit, `busyMessage` (a human-readable label, or `null` when idle) drives the full-screen
`BusyOverlay` during any write op, `scanning` spins the Changes refresh button. Both providers
are mounted in [`App.tsx`](../src/App.tsx).

Local, self-contained UI state stays in the leaf components — e.g. the sidebar width
(`Sidebar`), the art-diff canvas height (`ArtDiffView`), per-file staging toggles (`ChangesPanel`),
modal open/close state (`BranchesPanel`, `Sidebar`), and the diff view/compare/highlight controls
(`ArtDiffView`).
Both drag-resizable dimensions use the shared [`useResize`](../src/lib/useResize.ts) hook
(pointer-capture drag, clamped, persisted under a `krita-vc:` key).

## Sidebar views

`Sidebar` is a thin router on `view` (keeping the resizable shell + `DockerPanel` wrapper):

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
  on their commit card. Selection drives the main panel.
- **`changes`** — [`ChangesPanel`](../src/components/vcs/ChangesPanel.tsx): a "Saving to
  `<BranchBadge>`" header (the current branch — a commit always lands on it), then working-tree
  changes (from `scan_repository`) grouped Staged / Unstaged, with per-file
  and **Stage all / Unstage all** toggles. Staging is cosmetic — `commit_snapshot` captures the
  whole working tree. While a commit is in flight the staging controls lock, the commit button
  shows a spinner, the `StatusBar` shows an indeterminate progress bar (shared `saving` flag),
  and `BusyOverlay` blocks the app (`busyMessage`).
- **`branches`** — [`BranchesPanel`](../src/components/vcs/BranchesPanel.tsx): the local branch
  list with **real actions** — click a branch to switch, hover (or keyboard-focus) a row for
  "Merge into current" and "Delete" (both behind plain-language confirm modals), "New branch"
  opens the create modal. The create modal's base-branch picker (a plain `<select>`, only shown
  when more than one branch exists) defaults to the current branch — picking another one is passed
  through as `createBranch(name, base)`, which materializes that branch's tree before recording the
  new branch (refused, with a friendly prompt, on unsaved changes). Shared dialogs live in
  [`BranchDialogs.tsx`](../src/components/vcs/BranchDialogs.tsx); the backend's dirty-tree error
  (stable `"unsaved changes"` prefix) becomes a friendly save-first prompt with a jump to the
  Changes view. This is a local-only VCS — there are no remotes.

## Diff viewer

`DiffView` partitions entries by `kind` and routes each group independently:

- `kind: "art"` (`.kra`) → [`ArtDiffView`](../src/components/vcs/ArtDiffView.tsx): a visual layer
  diff. The layers + before/after canvas sit in a **drag-resizable region** (handle along its bottom
  edge, height clamped and persisted via `useResize`); when shrunk the layer list and canvas scroll
  internally, so the sections below stay reachable instead of being pushed off-screen. The first
  `palette` entry (if any) is embedded in `ArtDiffView`'s `LayerStackPanel` navigator. Documented
  in [visual-diff-viewer.md](visual-diff-viewer.md).
- `kind: "palette"` (`.gpl`) → [`PaletteDiffView`](../src/components/vcs/PaletteDiffView.tsx):
  always renders **color swatches** grouped by change (Modified / Added / Removed), each swatch
  showing before/after colors with hex codes. **Not gated by Artist Mode.** The first palette
  attaches to the art diff's navigator; extra palettes and palette-only diffs get a standalone
  panel (`StandalonePaletteDiff`, defined inline in `DiffView.tsx`).
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
read it with `useArtistMode()`. Label helpers live in
[`src/lib/friendly.ts`](../src/lib/friendly.ts).

| Surface | Artist Mode on | Artist Mode off |
|---------|----------------|-----------------|
| Non-art diff | Color-swatch / one-line summary (`FriendlyFileDiff`) | Code-style line diff (`DiffFileBlock`) |
| Commit hash (cards, toolbar, Inspector) | `Version N` (`versionLabel`) | Short hash |
| File paths (Inspector, status bar, art header) | Asset name (`assetName`, no folder/extension) | Full path |
| Status code (`FileStatusChip`) | Icon + word ("Updated") | Single letter (`M`) |
| Status-bar count | "N versions" | "N commits" |

Layer opacity/blend mode in `LayerStackPanel` are kept as-is in both modes — they're genuine art
concepts, not jargon.

## Component map

```
AppShell (→ WelcomeShell with no repository, else RepoShell)
├─ TopBar ─ Menu (repository switcher) + CleanupModal ("Clean up storage…")
├─ ActivityBar
├─ Sidebar ─ DockerPanel ─┬─ history  → Menu (branch switcher) + CommitGraph ─ CommitGraphRail + CommitCard (+ tip BranchBadge)
│                         ├─ changes  → ChangesPanel ─ FileStatusChip
│                         └─ branches → BranchesPanel ─ BranchBadge + BranchDialogs (create/save-first modals)
├─ MainPanel ─ DiffView ──┬─ art     → ArtDiffView ─┬─ LayerStackPanel ─ FileStatusChip
│                         │          (+ 1st palette)  ├─ ArtCanvas        (side-by-side)
│                         │                           └─ CompareSlider ─ ArtCanvas (swipe)
│                         ├─ palette → PaletteDiffView (standalone or via LayerStackPanel)
│                         └─ text  ──┬─ FriendlyFileDiff (Artist Mode on)
│                                    └─ DiffFileBlock     (Artist Mode off)
├─ Inspector ─ DockerPanel ─ FileStatusChip
└─ StatusBar

BusyOverlay (sibling of the above, not nested — renders when `busyMessage` is set)
```

The whole tree is wrapped in `RepositoryProvider` → `ArtistModeProvider` (both mounted in
[`App.tsx`](../src/App.tsx)).

Shared primitives: [`IconButton`](../src/components/ui/IconButton.tsx) (flat Krita-style),
[`Button`](../src/components/ui/Button.tsx), [`Menu`](../src/components/ui/Menu.tsx) (dropdown:
outside-click + Esc to close), [`FileStatusChip`](../src/components/vcs/FileStatusChip.tsx),
[`BranchBadge`](../src/components/vcs/BranchBadge.tsx).

Cross-cutting libs: [`src/lib/artistMode.tsx`](../src/lib/artistMode.tsx) (the toggle context),
[`src/lib/repository.tsx`](../src/lib/repository.tsx) (selected-repository context + all
mutating actions: commit/rollback/undo, branch create/switch/merge/delete),
[`src/lib/repoData.ts`](../src/lib/repoData.ts) (data hooks: commits, branches, diffs, layers),
[`src/lib/useResize.ts`](../src/lib/useResize.ts) (shared drag-resize hook),
[`src/lib/graph.ts`](../src/lib/graph.ts) (history-graph lane layout + `branchColorMap`),
[`src/lib/svgArt.ts`](../src/lib/svgArt.ts) (SVG layer compositing for the diff canvas),
[`src/lib/friendly.ts`](../src/lib/friendly.ts) (label helpers — `assetName`, `assetKind`,
`statusVerb`, `parsePaletteDiff`, `rgbToHex`, `versionNumbers`/`versionLabel`),
[`src/lib/format.ts`](../src/lib/format.ts) (timestamps).
