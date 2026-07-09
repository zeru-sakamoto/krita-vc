# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

Tauri 2 + React 19 + TypeScript desktop app — a version-control client for Krita art files.
This is a **local-only VCS**: there is intentionally **no remote/push/pull/sync** — no remotes,
no fetch, no cloud sync. The UI exposes only local operations (commit history, local branches,
working-tree changes). Don't add remote-facing affordances unless the project scope changes.
The Rust side is a **working custom local VCS** — its own `.kvc/` store (not git), with a `.kra`
tile-delta engine (`src-tauri/src/`: `repo`, `scan`, `commit`, `delta`, `kra`, `tiles`, `branch`,
`gc`; commands in `commands.rs`). Storage layout: chains are **sharded per tracked file**
(`.kvc/chains/`, lazy-loaded, `KVCC2`-tagged bincode — pre-KVCC2 shards and legacy monolithic
`chains.bin`/`chains.json` migrate transparently), the commit log is **append-only JSON-lines**
(`.kvc/commits.log`; a commit appends one line, legacy `commits.json` migrates on first save),
loose objects are sharded 256-way (`objects/<xx>/`, flat legacy stays readable), and a
commit with ≥32 new objects writes **one pack file** (`objects/pack/*.pack`) instead of loose
files — per-file creates dominated large commits on Windows. The `.kra` composite
(`mergedimage.png`) is stored as **content-addressed 256px pixel blocks**
(`KraEntry::CompositePng`) instead of a full PNG per commit — the store's former dominant cost;
restores re-encode a valid PNG (pixels exact, bytes not Krita's original; ineligible PNGs stay
byte-exact `Raw`). Restored `.kra` files write tile entries **deflate-fast** (Stored left them
several× larger on disk), and restores are memory-bounded (64 MB build chunks). An opt-in
`tilePixelDeltas` config flag (off by default) stores decoded tile pixels that bsdiff
across versions — mixed histories are safe via a per-ref `raw` flag. A user-facing **"Clean up
storage"** action (`cleanup_repository`, mark-and-sweep in `gc.rs`, dry-run powered confirm
modal in the **Settings modal**) reclaims history unreachable from any branch tip **and** prunes the raster
cache (reported separately as `cacheBytesReclaimed`), sweeps stale `*.tmp` files, gates pack
rewrites on >25% dead, and consolidates small packs; the raster cache (`.kvc/cache/`) is
size-budgeted (`Config.cacheMaxBytes`, default 256 MB) with LRU pruning. **Settings** (activity-bar
gear → `SettingsModal`) is the single home for user prefs: Artist-view toggle, a **custom title
bar** toggle (`windowChrome.tsx`, default **on** — the window boots with no OS-native chrome;
`TopBar` doubles as the draggable title bar with its own minimize/maximize/close controls via
`@tauri-apps/api/window`, and the preference is applied live through `setDecorations`, no
restart needed — see the Shell section below), an **author name**
(`authorName.tsx`, persisted to `localStorage`, sent as the `author` on new commits/merges/
rollbacks, falling back to `"You"`), and per-repo `cacheMaxBytes` + `tilePixelDeltas` knobs
(`get_repo_config`/`set_repo_config` → `Repo::save_config`, a config-only write) plus "Clean up
storage". **Branching is real**: `.kvc/branches.json` maps branch name → tip
commit id (+ the current branch); create is O(1) (an optional base branch materializes that
branch's tree first), switch rewrites only files that differ between branch trees, merge fast-forwards or builds a two-parent merge commit (conflicts take the source
version, flagged `"C"`). Trees fold along the **first-parent chain** (`tree_at_commit`) — every
commit's `files` is by invariant the diff vs its first parent. `list_commits` is scoped to
commits reachable from the current branch tip. The frontend drives it via Tauri `invoke` in the
desktop shell (history, scan, commit, repo lifecycle, rollback/undo, branch create/switch/merge/
delete, and per-commit visual diffs). **There is no mock data**: in a plain browser
(`npm run dev`, no backend) the data hooks return empty results, repository/branch actions are
no-ops, and the status bar shows a "Browser preview" badge — browser mode is for UI work only.
`.kra` diffs are real and load in two
stages: `commit_diff` returns the capped composite + layer metadata fast, then `commit_layers`/
`working_layers` **stream** per-layer rasters over a Tauri `Channel` as each finishes, with
capped PNGs persisted in a content-addressed `.kvc/cache/` (see the diff viewer section).
In the desktop shell rasters ship as **`kvcimg://` URLs** served straight from that cache
(registered in `lib.rs`, handler `commands::serve_raster` — no base64, browser-cacheable);
outside the shell or on a cache-write failure they fall back to base64 data URLs.
Non-`.kra` diffs are still minimal. Rust tests live in `src-tauri/tests/`; the frontend has no test
runner yet — if you add one, update this file.

Deeper docs live in [`docs/`](docs/README.md): frontend architecture, file tracking & version
control (the backend), the visual diff viewer, and [performance](docs/performance.md) (why the
`.kra` diff path is fast: staged/streamed loading, rayon parallelism, the `.kvc/cache/` raster
cache, raster downscaling, and the dev/release build profile).

## Commands

Package manager is npm (`package-lock.json` is present).

- `npm install` — install JS dependencies
- `npm run dev` — start the Vite dev server only (frontend in browser, no Tauri shell)
- `npm run build` — type-check (`tsc`) then build the frontend bundle to `dist/`
- `npm run preview` — preview the built frontend
- `npm run tauri dev` — run the full desktop app (spawns the Vite dev server per `beforeDevCommand`, then opens the Tauri/webview window); this is the normal way to run the app end-to-end
- `npm run tauri build` — produce a production desktop bundle (runs `npm run build` first per `beforeBuildCommand`, then compiles the Rust binary and packages installers)

Rust side (run from `src-tauri/`):
- `cargo check` / `cargo build` — compile the Rust backend without going through the Tauri CLI
- `cargo test` — run the Rust tests (engine integration tests in `src-tauri/tests/`)
- `cargo test --release --test bench -- --ignored --nocapture` — performance baseline
  (`tests/bench.rs`, `#[ignore]`d by default): synthesizes a Krita-scale document and times
  commit/switch/rollback/diff against the <10s target
- `cargo build --release --bin kvc` — build the headless `kvc` companion CLI (below); use
  `--bin krita-vc` (or no flag, since `default-run = "krita-vc"`) for the desktop app itself

## Architecture

This is a Tauri 2 app: a React/TypeScript frontend rendered in a native webview, paired with a Rust backend process.

- **Frontend** (`src/`): standard Vite + React 19 + TypeScript app. Entry point `src/main.tsx` mounts `App.tsx` into `index.html`. Built output goes to `dist/`, which `src-tauri/tauri.conf.json` (`build.frontendDist`) points at for packaged builds.
- **Backend** (`src-tauri/`): Rust crate `krita_vc_lib`. `src-tauri/src/main.rs` is the binary entry point and just calls `krita_vc_lib::run()` defined in `src-tauri/src/lib.rs`, where the `tauri::Builder` is configured, plugins are registered, and Tauri commands are wired up via `invoke_handler(tauri::generate_handler![...])`.
- **`kvc` CLI** (`src-tauri/src/bin/kvc.rs`): a second, Tauri-free binary target over the same `krita_vc_lib` engine (the crate builds `rlib` for exactly this). Five subcommands (`status`, `commit`, `branches`, `switch`, `create-branch`) taking `--repo <path>` plus scalars, each printing one JSON object to stdout (or `{"error": "..."}` to stderr, non-zero exit). Commit/switch/create-branch take an advisory `.kvc/kvc.lock` (create-exclusive, released on drop) so it can't race a concurrent desktop-app write — the engine itself has no locking. This is what the Krita plugin (below) shells out to. Two `[[bin]]` targets means bare `cargo run` is ambiguous without `Cargo.toml`'s `default-run = "krita-vc"`.
- **Krita plugin** (`krita-plugin/`, kept out of the npm/Cargo build): a PyKrita "Version Control" docker — commit, one-tap checkpoint, and branch switch/create from inside Krita, via `kvc_client.py` shelling out to the `kvc` CLI above. Deliberately does not do repo init, history browsing/restore, or anything remote — those stay desktop-app-only. See [`krita-plugin/README.md`](krita-plugin/README.md).
- **Frontend ↔ backend IPC**: Rust functions annotated `#[tauri::command]` (e.g. `greet` in `lib.rs`) are exposed to the frontend and called via `invoke("command_name", { args })` from `@tauri-apps/api/core`. New backend functionality should be added as a `#[tauri::command]` in `lib.rs` (or a module it includes) and registered in `generate_handler!`.
- **Permissions/capabilities**: `src-tauri/capabilities/default.json` declares which Tauri permissions (e.g. `core:default`, `opener:default`) the main window is allowed to use. Any new Tauri plugin or privileged API needs its permission added here or the call will be rejected at runtime.
- **Dev server coupling**: `vite.config.ts` hardcodes port `1420` (`strictPort: true`) and `src-tauri/tauri.conf.json`'s `build.devUrl` points at `http://localhost:1420`. These must stay in sync — Tauri's dev shell loads the app from that fixed URL. `src-tauri/` is excluded from Vite's file watcher.
- **App identity/config**: window size, app identifier (`com.zeru-sakamoto.krita-vc`), and bundle/icon settings live in `src-tauri/tauri.conf.json`.

Recommended editor setup (from README): VS Code with the Tauri and rust-analyzer extensions (already listed in `.vscode/extensions.json`).

## Frontend architecture (`src/`)

Backend-driven UI; design is specified in `DESIGN.md` and tokens are mapped into Tailwind v4
`@theme` in `src/styles/global.css` (utilities like `bg-surface-2`, `text-text-muted`,
`rounded-panel`). Domain types live in `src/types.ts`; data hooks in `src/lib/repoData.ts`
(commits, branches, diffs, streamed layers — keyed by repo path + `refreshNonce`); cross-cutting
presentation helpers in `src/lib/` (`format.ts` timestamps, `friendly.ts` artist-friendly labels,
`artistMode.tsx` the global toggle context, `repository.tsx` the selected-repository context,
`useResize.ts` the shared drag-resize hook, `graph.ts` history-graph lane layout,
`svgArt.ts` SVG layer compositing).

- **Shell** (`src/components/shell/`): `AppShell.tsx` splits on the selected repository — a
  welcome state when none is selected (fresh install), else `RepoShell` owns layout + view state
  and wires a top bar plus four zones — `TopBar` (repository switcher) above `ActivityBar`
  (changes/history/branches, plus a gear opening `SettingsModal`) | `Sidebar` (resizable, content switches on the active view) |
  `MainPanel` (diff) | `Inspector` (commit metadata) — plus `StatusBar`. `BusyOverlay.tsx` is a
  full-screen, non-dismissible block rendered by `AppShell` alongside the shell (not inside it)
  during any write op (commit, branch switch/merge/create/delete, rollback, undo, cleanup),
  driven by `busyMessage` on the repository context — stops a stray click racing a file rewrite.
  `TopBar` doubles as a **custom title bar** (`src-tauri/tauri.conf.json`'s window has
  `decorations: false` by default): when the Settings "Custom title bar" toggle
  (`lib/windowChrome.tsx`, `WindowChromeProvider`/`useWindowChrome`, default on) is active and
  the app is running in the Tauri shell, `TopBar` carries `data-tauri-drag-region` and
  right-aligned minimize/maximize/close buttons (`@tauri-apps/api/window`'s `getCurrentWindow()`);
  toggling the preference calls `setDecorations` live, so switching back to the OS-native frame
  needs no restart. Off, or in browser preview, `TopBar` renders exactly as before.
- **Repositories** (`src/lib/repository.tsx`): a local repository is a folder the user designates
  (local-only — no remotes). The `TopBar` switcher selects among them; the list + selected id
  persist to `localStorage` (`current` is null until the user adds one). In the desktop shell,
  Create/Browse open a native folder picker (`tauri-plugin-dialog`) and init a `.kvc/` store
  (`init_repository`); commits/history/changes come from the backend keyed by the selected path.
  In a plain browser there is no picker and repository actions are no-ops.
- **UI primitives** (`src/components/ui/`): `Button.tsx`, `IconButton.tsx` (flat Krita-style, no
  background until hover), `Menu.tsx` (dropdown with outside-click + Esc to close). Shared across
  shell and VCS components.
- **VCS components** (`src/components/vcs/`): commit cards, the git-style history graph
  (`CommitGraph` + `CommitGraphRail`, lane layout from `lib/graph.ts`; lane colors are a deliberate
  functional exception to the single-accent rule), branch badge, file-status chip, the sidebar
  panels (`ChangesPanel` — working-tree changes with per-file + stage-all/unstage-all toggles;
  staging is cosmetic since `commit_snapshot` captures the whole tree; while a commit is in flight
  the staging controls lock, the commit button spins, and the `StatusBar` shows a progress bar, via
  the shared `saving`/`scanning` flags on the repository context — `BranchesPanel` is local
  branches with **real actions**: click to switch, hover-row merge/delete with confirm modals
  (the delete affordance is hidden on `main` — the backend also refuses it with `DeleteMain`), a
  "New branch" modal; shared dialogs live in `BranchDialogs.tsx`, and the backend's dirty-tree
  error — matched on its stable `"unsaved changes"` prefix — becomes a friendly save-first prompt
  with a jump to the Changes view. The History sidebar has a live branch-switcher `Menu` (with a
  "New branch…" footer), the graph colors nodes per branch (`branchColorMap` in `lib/graph.ts`,
  current branch = accent) and badges branch tips on their commit cards, and `useBranches` in
  `lib/repoData.ts` feeds it all via `list_branches`), and the diff viewer
  (`DiffView`, `ArtDiffView`, `PaletteDiffView`, `LayerStackPanel`, `ArtCanvas`, `CompareSlider`).
- **Main panel** (`src/components/MainPanel.tsx`): thin wrapper between `AppShell` and `DiffView`;
  handles the empty-state when no commit is selected, and shows an "Analyzing changes…" spinner
  while the diff loads (the `loading` flag from `useCommitDiff`/`useWorkingDiff`).
- **Diff viewer** — `DiffView` routes each `DiffEntry` by `kind`: art (`.kra`) files render as a
  **visual layer diff** (`ArtDiffView` → `LayerStackPanel` + `ArtCanvas`/`CompareSlider`) inside a
  **drag-resizable region** (vertical handle on its bottom edge; height persisted via `useResize`,
  content scrolls when shrunk). Real `.kra` diffs load in two stages so the panel appears
  immediately: `commit_diff` (`useCommitDiff`) supplies the capped composite + layer metadata,
  then the heavy per-layer rasters stream in via `useArtLayers` → `commit_layers`/
  `working_layers`, one `Channel` message per finished layer (merged into `effectiveDiff` by id
  as each lands; pending layers show spinner thumbs plus the "Loading layers…" indicator). Each
  layer's raster comes as SVG `<image>` markup so the SVG-compositing viewer is unchanged, and
  the Composite view uses the `.kra`'s `mergedimage.png` (downscaled to the raster cap via an
  area-average box filter — `raster::box_downscale`, premultiplied-alpha; sharper than the old
  nearest-neighbour under the viewer's zoom). Capped PNGs are cached content-addressed in
  `.kvc/cache/` (keys carry a `box1` filter-version token), so repeat views skip rasterization.
  The viewer has **shared zoom/pan** (`useZoomPan`, wheel-to-cursor zoom + space/middle-mouse pan)
  applied identically to both side-by-side panes and the swipe slider so before/after and the
  slider divider stay pixel-aligned; zoom/pan and the slider drag are rAF-coalesced (one state
  flush per frame), the canvases and `LayerStackPanel`'s per-layer rows are `React.memo`'d with
  per-layer `compositeSvg` memoization (rebuilding a thumb re-serializes its raster markup), the
  canvas transform wrapper carries `will-change: transform`, and streamed-layer Channel messages
  batch into one state flush per frame — together these keep interaction off the multi-MB
  SVG-string rebuild path. The **change highlight** defaults to a true **changed-pixel** overlay — an accent
  mask (`ArtDiff.diffImage`) plus a hatch pattern and a **dashed outline that hugs the changed
  pixels' silhouette** (`ArtDiff.diffOutline`, a vector path traced by `raster::diff_overlay`/
  `outline_from_grid`), computed in Rust off the before/after composites so it ships with the first
  `commit_diff`; a coarse tile-bbox **region-box** mode (with corner brackets) remains as a fallback.
  The highlight is **per-layer**: the composite fields drive the Composite view, and each **modified**
  layer carries its *own* `diffImage`/`diffOutline`/`regions` on `LayerDto`/`ArtLayer` (Rust
  `commands::layer_diff_overlay` → `raster::diff_overlay_full`, one changed-pixel grid → mask +
  outline + normalized bbox, diffed from the before/after rasters the layer stream already decoded;
  mask cached by both layer raster keys). `ArtDiffView` picks the overlay source from the selection
  and passes `diffImage`/`diffOutline`/`regions` as explicit props into `ArtCanvas`/`CompareSlider`
  (never read off `diff`), so a focused layer shows only its own change and unchanged/added/removed
  layers show none. **Region boxes are normalized 0..1** of the viewBox (composite tile-bbox and
  per-layer alike) — `boxOverlay` scales by width/height, so a region must not be pre-scaled to
  pixels or it overflows past the canvas' bottom-right. Palette (`.gpl`) files have
  `kind: "palette"` and always render
  as **color swatches** (`PaletteDiffView`) — the first palette is embedded in the art diff's
  `LayerStackPanel` navigator; standalone palettes get their own panel. This route is **not**
  Artist Mode gated. Generic text files (`kind: "text"`) depend on Artist Mode: `FriendlyFileDiff`
  (one-line summary) on, `DiffFileBlock` (raw line diff with +/− and line numbers) off. Layer
  imagery is composited from **inline SVG markup strings** (`src/lib/svgArt.ts` — `layersBody`/
  `wrapSvg`/`compositeSvg`), which is how the backend's base64-PNG rasters render with no raster
  pipeline in the viewer. See [`docs/visual-diff-viewer.md`](docs/visual-diff-viewer.md).
- **Artist Mode** — a global toggle (default on) that swaps technical strings for plain-language
  labels app-wide: friendly diffs, `Version N` instead of hashes, asset names instead of file
  paths, words+icons instead of `M/A/D`. State + persistence in `src/lib/artistMode.tsx`
  (`useArtistMode()`); label helpers in `src/lib/friendly.ts`. The audience is artists, so prefer
  friendly labels over git/code jargon in new UI, and gate any unavoidable technical detail behind
  Artist Mode being off. See [`docs/frontend-architecture.md`](docs/frontend-architecture.md#artist-mode).

All data flows through Tauri `invoke` keyed by the selected repository path; the component/prop
boundaries (`Repository`, `DiffEntry`, `Commit` — incl. `parents` lineage — `Branch` incl. `tip`,
`WorkingChange`) are the contract between `src/lib/repoData.ts`/`repository.tsx` and the UI.
