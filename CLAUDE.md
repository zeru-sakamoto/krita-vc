# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

Tauri 2 + React 19 + TypeScript desktop app — a version-control client for Krita art files.
This is a **local-only VCS**: there is intentionally **no remote/push/pull/sync** — no remotes,
no fetch, no cloud sync. The UI exposes only local operations (commit history, local branches,
working-tree changes). Don't add remote-facing affordances unless the project scope changes.
The Rust side is a **working custom local VCS** — its own `.kvc/` store (not git), with a `.kra`
tile-delta engine (`src-tauri/src/`: `repo`, `scan`, `commit`, `delta`, `kra`, `tiles`; commands
in `commands.rs`). The frontend drives it via Tauri `invoke` in the desktop shell (history, scan,
commit, repo lifecycle, rollback/undo, and per-commit visual diffs) and **falls back to mock
data** (`src/data/`) in a plain browser (`npm run dev`). `.kra` diffs are real and load in two
stages: `commit_diff` returns the capped composite + layer metadata fast, then `commit_layers`/
`working_layers` **stream** per-layer PNG rasters over a Tauri `Channel` as each finishes, with
capped PNGs persisted in a content-addressed `.kvc/cache/` (see the diff viewer section);
non-`.kra` diffs are still minimal/mock. Rust tests live in `src-tauri/tests/`; the frontend has no test
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

## Architecture

This is a Tauri 2 app: a React/TypeScript frontend rendered in a native webview, paired with a Rust backend process.

- **Frontend** (`src/`): standard Vite + React 19 + TypeScript app. Entry point `src/main.tsx` mounts `App.tsx` into `index.html`. Built output goes to `dist/`, which `src-tauri/tauri.conf.json` (`build.frontendDist`) points at for packaged builds.
- **Backend** (`src-tauri/`): Rust crate `krita_vc_lib`. `src-tauri/src/main.rs` is the binary entry point and just calls `krita_vc_lib::run()` defined in `src-tauri/src/lib.rs`, where the `tauri::Builder` is configured, plugins are registered, and Tauri commands are wired up via `invoke_handler(tauri::generate_handler![...])`.
- **Frontend ↔ backend IPC**: Rust functions annotated `#[tauri::command]` (e.g. `greet` in `lib.rs`) are exposed to the frontend and called via `invoke("command_name", { args })` from `@tauri-apps/api/core`. New backend functionality should be added as a `#[tauri::command]` in `lib.rs` (or a module it includes) and registered in `generate_handler!`.
- **Permissions/capabilities**: `src-tauri/capabilities/default.json` declares which Tauri permissions (e.g. `core:default`, `opener:default`) the main window is allowed to use. Any new Tauri plugin or privileged API needs its permission added here or the call will be rejected at runtime.
- **Dev server coupling**: `vite.config.ts` hardcodes port `1420` (`strictPort: true`) and `src-tauri/tauri.conf.json`'s `build.devUrl` points at `http://localhost:1420`. These must stay in sync — Tauri's dev shell loads the app from that fixed URL. `src-tauri/` is excluded from Vite's file watcher.
- **App identity/config**: window size, app identifier (`com.zeru-sakamoto.krita-vc`), and bundle/icon settings live in `src-tauri/tauri.conf.json`.

Recommended editor setup (from README): VS Code with the Tauri and rust-analyzer extensions (already listed in `.vscode/extensions.json`).

## Frontend architecture (`src/`)

Mock-data-driven UI; design is specified in `DESIGN.md` and tokens are mapped into Tailwind v4
`@theme` in `src/styles/global.css` (utilities like `bg-surface-2`, `text-text-muted`,
`rounded-panel`). Domain types live in `src/types.ts`; mock data in `src/data/`; cross-cutting
presentation helpers in `src/lib/` (`format.ts` timestamps, `friendly.ts` artist-friendly labels,
`artistMode.tsx` the global toggle context, `repository.tsx` the selected-repository context,
`useResize.ts` the shared drag-resize hook, `graph.ts` history-graph lane layout).

- **Shell** (`src/components/shell/`): `AppShell.tsx` owns layout + view state and wires a top bar
  plus four zones — `TopBar` (repository switcher) above `ActivityBar` (changes/history/branches) |
  `Sidebar` (resizable, content switches on the active view) | `MainPanel` (diff) | `Inspector`
  (commit metadata) — plus `StatusBar`.
- **Repositories** (`src/lib/repository.tsx`): a local repository is a folder the user designates
  (local-only — no remotes). The `TopBar` switcher selects among them; the list + selected id
  persist to `localStorage`. In the desktop shell, Create/Browse open a native folder picker
  (`tauri-plugin-dialog`) and init a `.kvc/` store (`init_repository`); commits/history/changes
  come from the backend keyed by the selected path. In a plain browser there is no picker —
  `MOCK_REPOSITORIES` seeds the list and "Add repository…" appends a placeholder.
- **UI primitives** (`src/components/ui/`): `Button.tsx`, `IconButton.tsx` (flat Krita-style, no
  background until hover), `Menu.tsx` (dropdown with outside-click + Esc to close). Shared across
  shell and VCS components.
- **VCS components** (`src/components/vcs/`): commit cards, the git-style history graph
  (`CommitGraph` + `CommitGraphRail`, lane layout from `lib/graph.ts`; lane colors are a deliberate
  functional exception to the single-accent rule), branch badge, file-status chip, the sidebar
  panels (`ChangesPanel` — working-tree changes with per-file + stage-all/unstage-all toggles;
  staging is cosmetic since `commit_snapshot` captures the whole tree; while a commit is in flight
  the staging controls lock, the commit button spins, and the `StatusBar` shows a progress bar, via
  the shared `saving`/`scanning` flags on the repository context — `BranchesPanel` is local branches
  only), and the diff viewer
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
  the Composite view uses the `.kra`'s `mergedimage.png` (downscaled to the raster cap). Capped
  PNGs are cached content-addressed in `.kvc/cache/`, so repeat views skip rasterization. Palette (`.gpl`) files have
  `kind: "palette"` and always render
  as **color swatches** (`PaletteDiffView`) — the first palette is embedded in the art diff's
  `LayerStackPanel` navigator; standalone palettes get their own panel. This route is **not**
  Artist Mode gated. Generic text files (`kind: "text"`) depend on Artist Mode: `FriendlyFileDiff`
  (one-line summary) on, `DiffFileBlock` (raw line diff with +/− and line numbers) off. Mock layer
  imagery is **generated inline SVG** (`src/data/mockArt.ts`) — no assets, no deps, composited
  offline. Palette mock data lives in `src/data/mockPalette.ts`. See
  [`docs/visual-diff-viewer.md`](docs/visual-diff-viewer.md).
- **Artist Mode** — a global toggle (default on) that swaps technical strings for plain-language
  labels app-wide: friendly diffs, `Version N` instead of hashes, asset names instead of file
  paths, words+icons instead of `M/A/D`. State + persistence in `src/lib/artistMode.tsx`
  (`useArtistMode()`); label helpers in `src/lib/friendly.ts`. The audience is artists, so prefer
  friendly labels over git/code jargon in new UI, and gate any unavoidable technical detail behind
  Artist Mode being off. See [`docs/frontend-architecture.md`](docs/frontend-architecture.md#artist-mode).

When the backend lands, replace the `src/data/` mock modules with data fetched via Tauri `invoke`
(keyed by the selected repository path; a real folder picker needs `tauri-plugin-dialog`); the
component/prop boundaries (`Repository`, `DiffEntry`, `Commit` — incl. `parents` lineage — `Branch`,
`WorkingChange`) are designed to be the swap point.
