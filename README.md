# Krita VCS

A desktop **version-control client for Krita art files**, built with Tauri 2 + React 19 +
TypeScript. Instead of code-style text patches, it shows artists what actually changed: the
**layer stack** and a **visual diff** (before/after, swipe slider, changed-pixel highlighting, and
synced zoom/pan) of each `.kra` file.

> **Status:** fully working end to end. The Rust backend is a **real, custom local VCS** with its
> own `.kvc/` store (not git) and a `.kra` tile-delta engine; the React frontend drives it over
> Tauri IPC. There is **no mock data**: in a plain browser the UI renders, but all repository
> actions are no-ops (UI-development mode only).

## What it is (and isn't)

This is a **local-only** version-control system. There is intentionally **no remote, push, pull, or
cloud sync**: no accounts, no server, nothing leaves your machine. A "repository" is just a folder
you designate; the app creates a `.kvc/` store inside it and tracks the art files there. The UI
exposes only local operations: commit history, working-tree changes, and local branches.

It does **not** use git. The backend is a purpose-built store optimized for large binary `.kra`
files, where git's text-oriented delta model performs poorly.

## Features

- **Visual layer diffs** for `.kra` files: a Krita-style layer panel beside a before/after canvas.
  - **Side-by-side** and **swipe slider** compare modes.
  - **Synced zoom & pan**: wheel to zoom toward the cursor, space- or middle-mouse-drag to pan,
    applied identically across both modes so before/after and the slider divider stay aligned.
  - **Changed-pixel highlighting**: a true per-pixel diff (toggle on/off), with a coarse
    region-box mode as a fallback. The highlight is **per-layer**: focus a layer and it outlines
    only *that* layer's changed pixels, not the whole-file silhouette. Its color always matches
    the active **theme's accent** (see Settings below).
  - Click a layer to focus its diff, or view the composited artwork; color palettes (`.gpl`,
    `.kpl`, `.aco`, `.ase`) — standalone or embedded inside a `.kra` — render as a color-by-color
    swatch diff (added / removed / recolored, with hex values). When a version touches several
    files, the inspector's file list picks which one the main panel shows, and it shows the
    selected file's or layer's details (type, visibility, opacity, blend, painted bounds) or the
    composite's size, resolution, and color space.
- **Real local version control**: stage exactly the files you want (or commit everything, with a
  confirm prompt if some changes aren't staged), browse history as a branch-aware graph, and roll
  back / undo commits. Rolling back to the version you're already on just discards unsaved changes
  in place (no new history entry); rolling back to an older one records a new commit, linked back
  to it in the graph by a dashed connector. Changed your mind mid-edit? Discard a single file's
  unsaved changes, or every unstaged file at once, without touching what's staged.
- **Set aside** (stash): park working-tree changes off to the side — staged files or everything —
  and bring them back later, without leaving a version in your history. A switch or merge blocked
  by unsaved changes offers setting them aside as a one-click way through, and a Settings-modal
  shelf lists every set-aside item with its origin branch and age.
- **Branching & merging**: create, switch, merge (fast-forward or two-parent), and delete local
  branches, all backed by real tree materialization. Conflicting edits are flagged for review; if
  one side edited a file and the other deleted it, the edit wins, so a merge never quietly loses
  work.
- **Storage housekeeping**: a "Clean up storage" action reclaims history unreachable from any
  branch tip or set-aside stash (mark-and-sweep GC), and the raster preview cache is
  size-budgeted with LRU pruning.
- **Settings** (activity-bar gear). One place for user preferences: the Artist Mode toggle, a
  **custom title bar** toggle (a frameless window with its own draggable title bar and window
  controls, on by default; switch back to your OS's native frame any time, no restart needed), a
  **theme selector** (8 color themes, including a true-black option, applied instantly via CSS,
  with the visual-diff highlight color following the chosen theme's accent), the **author name**
  signed on your versions, the **set-aside shelf** (every stash, with per-item remove and
  remove-all), and per-repository **preview-cache size**, **compact-storage**, and
  **low-memory diffs** options, plus "Clean up storage".
- **Artist Mode**: a global toggle (default on) that swaps git/code jargon for plain language
  (`Version 3` instead of a hash, asset names instead of file paths, friendly file summaries).
- A dark, Krita-inspired UI built against [`DESIGN.md`](DESIGN.md).

## How it works

- **Custom `.kvc/` store**: chains are sharded per tracked file (lazy-loaded), loose objects are
  sharded 256-way, and a commit with many new objects writes a single pack file instead of many
  loose files (per-file creates dominated large commits on Windows).
- **Tracked file types**: the scanner only tracks files Krita VCS understands: `.kra` documents
  and the color-palette formats (`.gpl`, `.kpl`, `.aco`, `.ase`). Anything else in the folder is
  left untouched and is never staged or committed.
- **`.kra` tile-delta engine**: `.kra` files are ZIP archives of per-layer tiles; the engine diffs
  and stores them at the tile level, so a small edit to one layer stores a small delta, not a whole
  new file.
- **Two-stage visual diffs**: `commit_diff` returns the capped composite + layer metadata fast,
  then per-layer rasters **stream** in over a Tauri channel as each finishes. Rasters are cached
  content-addressed in `.kvc/cache/` and served to the webview as browser-cacheable `kvcimg://`
  URLs. See [`docs/visual-diff-viewer.md`](docs/visual-diff-viewer.md) and
  [`docs/performance.md`](docs/performance.md).

## Getting started

Package manager is **npm**.

```bash
npm install          # install JS dependencies
npm run tauri dev    # run the full desktop app (Vite dev server + Tauri webview)
```

Then use the top-bar repository switcher to **Create** or **Browse** to a folder; that becomes a
local repository (a `.kvc/` store is initialized inside it). Drop a `.kra` file in, commit, edit it
in Krita, and commit again to see a visual diff.

Frontend-only (in a browser, no Tauri shell; UI development only, no backend):

```bash
npm run dev          # Vite dev server at http://localhost:1420
```

Build / package:

```bash
npm run build        # type-check (tsc) + build the frontend bundle to dist/
npm run tauri build  # production desktop bundle (frontend build + Rust binary + installers)
```

Rust side (from `src-tauri/`):

```bash
cargo check          # compile the backend
cargo test           # engine integration tests (tests/engine.rs) + unit tests
cargo test --release --test bench -- --ignored --nocapture   # performance baseline
```

## Project layout

```
src/
├─ components/
│  ├─ shell/   — AppShell, TopBar (repository switcher), ActivityBar, SettingsModal,
│  │            Sidebar, Inspector, StatusBar, BusyOverlay
│  ├─ vcs/     — diff viewer (DiffView, ArtDiffView, ArtCanvas, CompareSlider,
│  │            LayerStackPanel, PaletteDiffView), commit graph, branch/changes panels,
│  │            StashDialogs (set-aside prompts)
│  ├─ ui/      — IconButton, Button, Menu, Modal
│  └─ MainPanel.tsx
├─ lib/        — data hooks + Tauri invoke calls (repoData.ts), repository + artist-mode +
│  │            author-name contexts, shell detection (tauri.ts), SVG compositing, zoom/pan + resize hooks
├─ styles/     — global.css (Tailwind v4 @theme tokens from DESIGN.md)
└─ types.ts    — domain types (the frontend ↔ backend contract)

src-tauri/src/ — Rust backend (crate krita_vc_lib)
├─ repo, scan, commit, delta, branch, gc, stash   — the local VCS engine
├─ kra, tiles, raster                        — .kra parsing, tile store, raster/diff imaging
├─ palette                                   — .gpl/.kpl/.aco/.ase parsing + swatch diffing
├─ commands.rs                               — Tauri #[command] IPC surface
└─ lib.rs / main.rs                          — Tauri builder + entry point

docs/          — developer documentation
DESIGN.md      — visual + interaction spec
krita-plugin/  — optional Krita docker plugin (see below)
```

## Krita plugin

A companion "Version Control" docker for Krita itself: commit, quick-checkpoint, and
branch-switch without alt-tabbing to this app. It's a small Python (PyKrita) plugin that
shells out to `kvc`, a headless CLI built from the same Rust engine
(`src-tauri/src/bin/kvc.rs`, no Tauri dependency), so the plugin and the desktop app
always go through identical commit/branch code against the same `.kvc` store. See
[`krita-plugin/README.md`](krita-plugin/README.md) for build + install steps.

## Documentation

- [`docs/`](docs/README.md): frontend architecture, the file-tracking / version-control backend,
  the visual diff viewer, and performance.
- [`krita-plugin/README.md`](krita-plugin/README.md): the in-Krita commit docker: install,
  usage, and troubleshooting.
- [`DESIGN.md`](DESIGN.md): design tokens, components, and interaction spec.
- [`CLAUDE.md`](CLAUDE.md): repo guidance and commands.

## Recommended IDE setup

[VS Code](https://code.visualstudio.com/) +
[Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) +
[rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer).
