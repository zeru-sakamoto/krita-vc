# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

Tauri 2 + React 19 + TypeScript desktop app — a version-control client for Krita art files.
This is a **local-only VCS**: there is intentionally **no remote/push/pull/sync** — no remotes,
no fetch, no cloud sync. The UI exposes only local operations (commit history, local branches,
working-tree changes). Don't add remote-facing affordances unless the project scope changes.
The Rust side is a **working custom local VCS** — its own `.kvc/` store (not git), with a `.kra`
tile-delta engine (`src-tauri/src/`: `repo`, `scan`, `commit`, `delta`, `kra`, `tiles`, `branch`,
`gc`, `palette`, `stash`, `merge`; commands in `commands.rs`). **Tracking guardrail**: the scanner only newly tracks
*supported* file types — `.kra` documents and the palette formats (`.gpl`/`.kpl`/`.aco`/`.ase`,
`scan::is_supported`); every other file in the project folder is ignored (never staged, hashed, or
committed). Already-tracked files stay tracked, so pre-guardrail repos aren't pruned. `is_supported`
is a **suffix match on the whole relpath**, not an extension parse, so Krita's autosave artifact
(`foo.kra-autosave.kra`, dot-prefixed on Linux/macOS) ends in `.kra` and would be tracked as a real
document — it's rejected explicitly, and that rejection lives in `is_supported` (gating *new*
tracking only) rather than in the scan walk, so a repo that already committed one isn't pruned.
Krita's backup file (`*.kra~`) is skipped separately, in `scan_detailed`'s walk. Storage layout: chains are **sharded per tracked file**
(`.kvc/chains/`, lazy-loaded, `KVCC2`-tagged bincode — pre-KVCC2 shards and legacy monolithic
`chains.bin`/`chains.json` migrate transparently), the commit log is **append-only JSON-lines**
(`.kvc/commits.log`; a commit appends one line, legacy `commits.json` migrates on first save),
stashes live in `.kvc/stashes.json` (absent = empty shelf, which is the whole migration),
loose objects are sharded 256-way (`objects/<xx>/`, flat legacy stays readable), and a
commit with ≥32 new objects writes **one pack file** (`objects/pack/*.pack`) instead of loose
files — per-file creates dominated large commits on Windows. **Stashing** ("Set aside" in Artist
Mode) parks working-tree changes off to the side of history and reverts the files on disk
(`stash.rs`, records in `.kvc/stashes.json` — deliberately *not* `commits.log`, which would put
spurious version rows in the Performance tab and block undo). Stash content reuses the commit
path's relpath-keyed streams via the shared `commit::store_change`, so a stashed `.kra` dedups
its tiles against history for free; three orderings are load-bearing and each has a test —
a stash must **not** write `repo.index` (else the revert scans clean and silently keeps the
tree dirty), `create` must save **before** reverting (else a crash erases the work with no
record), and `pop` must write files **before** dropping the record (and computes every file's
bytes **before** the first write, so a failed merge leaves the tree + shelf untouched). On a pop
**conflict** (a stashed path edited since), a conflicting **`.kra` is merged** — only the layers the
set-aside version actually **added or modified** are folded into the working file (`merge_layers`
takes the committed **ancestor** and skips incoming top-level layers unchanged since, matched by
uuid then compared on **content** — each `layers/layerN` data file canonicalized to its tiles
**sorted by position** (Krita's tile *order* isn't stable across saves, so equal tiles reconstruct
to different bytes; `canon_entry`), collected filename-independently (Krita renumbers `layerN`) —
plus a small curated metadata set [`name`/`opacity`/`compositeop`/`visible`/`x`/`y`]. Deliberately
**not** raw `<layer>` XML or whole-blob bytes: Krita rewrites volatile attrs like `selected` and
reshuffles tile order every save, so either would fold *every* layer in — the bug this had. `None`
ancestor folds all; an obscure metadata attr off the list is at worst not folded, never spuriously
duplicated), clashing top-level layer names suffixed ` [2]`, folded data
files + uuids remapped to fresh ids (`merge.rs`, `merge::merge_layers`, dep-free `roxmltree`-range +
string-token `.kra` surgery) — so the artist reconciles by hand in Krita; it refuses (`MergeFailed`,
nothing written) on a different color space, or when the set-aside change is only outside the layer
stack (nothing to fold). **Any other conflict** still
hard-refuses the whole pop with `StashConflict` (prefix `"stash conflict"`, distinct from the
`"unsaved changes"` one) — a non-`.kra` file or a stashed *deletion* onto edited work. No
frontend/CLI change: a merged pop returns normally, so the existing "brought back" path applies.
See [`docs/version-control.md`](docs/version-control.md#stashes--setting-work-aside). The `.kra` composite
(`mergedimage.png`) is stored as **content-addressed 256px pixel blocks**
(`KraEntry::CompositePng`) instead of a full PNG per commit — the store's former dominant cost;
restores re-encode a valid PNG (pixels exact, bytes not Krita's original; ineligible PNGs stay
byte-exact `Raw`). Restored `.kra` files write tile entries **deflate-fast** (Stored left them
several× larger on disk), and restores are memory-bounded (64 MB build chunks). An opt-in
`tilePixelDeltas` config flag (off by default) stores decoded tile pixels that bsdiff
across versions — mixed histories are safe via a per-ref `raw` flag. A user-facing **"Clean up
storage"** action (`cleanup_repository`, mark-and-sweep in `gc.rs`, dry-run powered confirm
modal in the **Settings modal**) reclaims history unreachable from any branch tip **or stash**
(stashes are GC roots — nothing in `commits.log` references them) **and** prunes the raster
cache (reported separately as `cacheBytesReclaimed`), sweeps stale `*.tmp` files, gates pack
rewrites on >25% dead, and consolidates small packs; the raster cache (`.kvc/cache/`) is
size-budgeted (`Config.cacheMaxBytes`, default 256 MB) with LRU pruning. **Settings** (activity-bar
gear → `SettingsModal`) is the single home for user prefs, organized into three left-hand category
tabs (a static list regardless of whether a repository is selected — a tab whose settings need one
shows a plain "Open a repository…" fallback rather than disappearing, so the tab set never jumps
around as you switch repos): **Appearance** (Artist-view toggle, a **custom title bar** toggle
(`windowChrome.tsx`, default **on** — the window boots with no OS-native chrome; `TopBar` doubles
as the draggable title bar with its own minimize/maximize/close controls via `@tauri-apps/api/window`,
and the preference is applied live through `setDecorations`, no restart needed — see the Shell
section below), an **author name** (`authorName.tsx`, persisted to `localStorage`, sent as the
`author` on new commits/merges/rollbacks, falling back to `"You"`), and the theme picker), **Set-
Aside** (the shelf: every stash with its origin branch + age; per-row remove and remove-all,
confirms rendered as *sibling* modals per the `CleanupModal` pattern — `Modal` has no portal), and
**Storage** (per-repo `cacheMaxBytes` + `tilePixelDeltas` knobs — `get_repo_config`/`set_repo_config`
→ `Repo::save_config`, a config-only write — plus "Clean up storage"). Backing up a repository
(`backupRepository` in `repository.tsx`, zips the project via `export_repository_zip`) is **not**
in Settings — it's its own one-click zip-icon `IconButton` in `ActivityBar.tsx`, directly above
the Settings gear, wired to a small global toast (`lib/toast.tsx`, `ToastProvider`/`useToast`,
single-slot, auto-dismissing, bottom-right, reusing the `--z-toast` token) for the "Saved to …"/
error result, since the busy overlay covers only the in-flight zip itself. **Branching is real**:
`.kvc/branches.json` maps branch name → tip
commit id (+ the current branch); create is O(1) (an optional base branch materializes that
branch's tree first), switch rewrites only files that differ between branch trees, merge fast-forwards or builds a two-parent merge commit (conflicts take the source
version, flagged `"C"`). Trees fold along the **first-parent chain** (`tree_at_commit`) — every
commit's `files` is by invariant the diff vs its first parent. `list_commits` is scoped to
commits reachable from the current branch tip. The frontend drives it via Tauri `invoke` in the
desktop shell (history, scan, commit, repo lifecycle, rollback/undo, branch create/switch/merge/
delete, stash create/pop/drop, and per-commit visual diffs). **There is no mock data**: in a plain browser
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
control (the backend), the visual diff viewer, [performance](docs/performance.md) (why the
control (the backend), the visual diff viewer, [performance](docs/performance.md) (why the
`.kra` diff path is fast: staged/streamed loading, rayon parallelism, the `.kvc/cache/` raster
cache, raster downscaling, and the dev/release build profile), and the
[performance report](docs/performance-report.md) (the **Performance** tab: client-side operation
timing + per-version storage-saved-vs-full-copy metrics).

## Conventions

Deliberate simplifications/shortcuts (duplicated data that can't be shared across a build
boundary, a narrower fix than the "proper" one, etc.) get a plain comment at the point of the
shortcut explaining what and why — no `ponytail:`-style tags, not a prose explanation elsewhere.

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
- **`kvc` CLI** (`src-tauri/src/bin/kvc.rs`): a second, Tauri-free binary target over the same `krita_vc_lib` engine (the crate builds `rlib` for exactly this). Nine subcommands (`status`, `commit`, `branches`, `switch`, `create-branch`, `discard`, `stash`, `stash-pop`, `stash-list`) taking `--repo <path>` plus scalars, each printing one JSON object to stdout (or `{"error": "..."}` to stderr, non-zero exit). The optional file-subset flag (`--paths` on `commit`/`discard`/`stash`) is a **JSON array** — the hand-rolled parser is a map, so a repeated flag would overwrite, and paths can contain commas; omitting it means "everything". Every mutating subcommand takes a real OS-level advisory lock (`.kvc/kvc.lock`, `File::try_lock` — `LockFileEx`/`flock`, released automatically by the OS when the process's handle closes, even on a crash — tagged via a `kvc.lock.info` sidecar with a present-participle label like `"switching branches"` so a caller blocked by `KvcError::Locked` sees what's holding it and for how long) so it can't race a concurrent desktop-app write — the engine itself has no locking; reads (`status`, `branches`, `stash-list`) take none, so the plugin's 1.5s poll never contends. `status` carries a `stashes` count so that poll needn't spawn a third process. The **no-args usage line is load-bearing**: the plugin's "Locate kvc…" picker identifies the binary by its literal `"usage: kvc"` prefix, so widen the command list freely but never change that prefix. `stash-list` reuses `commands::stash_dtos` for its **newest-first** order, which "bring back latest" depends on. Contract tests: `src-tauri/tests/kvc_cli.rs` (spawns the real binary). Two `[[bin]]` targets means bare `cargo run` is ambiguous without `Cargo.toml`'s `default-run = "krita-vc"`.
- **Krita plugin** (`krita-plugin/`, kept out of the npm/Cargo build): a PyKrita "Version Control" docker — commit (with per-file ticks), one-tap checkpoint, discard, set-aside/bring-back, save-and-rescan (⟳), and branch switch/create from inside Krita, via `kvc_client.py` shelling out to the `kvc` CLI above. Deliberately does not do repo init, history browsing/restore, undo, branch merge/delete, or anything remote — those stay desktop-app-only. The engine only sees the disk, Krita's canvas only memory, so the docker moves both ways and **both directions are load-bearing**:
  - **memory → disk** (`_save_tracked`, all *modified* `.kra` under the repo root — `.kra` only, since Krita may raise an export dialog on a `.png` and hang the UI thread it's saving on). Driven by focus entering the docker (`QApplication.focusChanged` — not an event filter; focus lands on child widgets and `FocusIn` won't reach the dock), the ⟳ button, and `_commit_with_message`. Two traps: commit **must `refresh()` between the save and `_selected_paths()`** or it skips the very work just written (a doc clean *before* the save isn't in `_shown_paths`/`checked`); and `_save_tracked` sets `busy` because `doc.save()` spins the event loop, which would let the 1.5s poll `kvc status` a half-written `.kra`.
  - **disk → memory** (`_rebuild_docs`, wrapping switch/discard/stash/pop). Refuses while any open doc is unsaved, then **closes and reopens** each doc whose file changed (mtime/size snapshot — `switch` doesn't report what it rewrote). Drop the reopen and Krita keeps serving the pre-op copy, so the next Ctrl+S silently reverts the operation; drop the refusal and that reopen eats real work — the engine's dirty-tree guard never sees Krita's memory.

  Consequence to preserve: auto-save makes that refusal rare, so **Discard's confirm is the only thing standing between the artist and losing saved-but-uncommitted work** — saving isn't committing, and the reopen takes the undo history too. Also: checkbox state lives in `VcDocker.checked`, **not** the widget (the poll rebuilds the list and would wipe a tick mid-edit; the rebuild is skipped when the path list is unchanged). `kvc_client.py` blocks the UI thread by design (see its header). See [`krita-plugin/README.md`](krita-plugin/README.md).
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
  (changes/history/branches/performance, plus a gear opening `SettingsModal`) | `Sidebar` (resizable, content switches on the active view) |
  (changes/history/branches/performance, plus a gear opening `SettingsModal`) | `Sidebar` (resizable, content switches on the active view) |
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
  staging determines what a commit actually captures: `commit_snapshot`'s optional `paths` arg
  (`commit::commit_selected` in Rust) restricts the commit to those relative paths, leaving the
  rest dirty. Hitting "Commit version" with nothing staged or with a partial selection shows a
  confirm `Modal` first (commit everything anyway / commit only the staged files) before calling
  through; while a commit is in flight
  the staging controls lock, the commit button spins, and the `StatusBar` shows a progress bar, via
  the shared `saving`/`scanning` flags on the repository context — `BranchesPanel` is local
  branches with **real actions**: click to switch, hover-row merge/delete with confirm modals
  (the delete affordance is hidden on `main` — the backend also refuses it with `DeleteMain`), a
  "New branch" modal; shared dialogs live in `BranchDialogs.tsx`, and the backend's dirty-tree
  error — matched on its stable `"unsaved changes"` prefix — becomes a friendly save-first prompt
  offering three ways out: save, **set it aside** (stashes everything, then retries the blocked
  switch/merge), or jump to Changes. **Set-aside actions** sit in the `Sidebar` panel-options
  `Menu` as **three divider-separated groups**: undo/discard, then set-aside, then bring-back.
  `Menu` still has no submenus, but gained a `MenuItem.separator` flag (a `border-t` above that
  row) since one `footer` group can only draw one rule and this needs two. Set-aside and
  bring-back (two rows each) are both **changes-view only**, since both act on the working tree —
  History's panel-options menu is just undo.
  Dialogs live in `StashDialogs.tsx` (`SetAsideModal` label prompt, `PickStashModal`,
  `StashConflictModal` + `isStashConflictError`), fed by `useStashes` via `list_stashes`.
  The History sidebar has a live branch-switcher `Menu` (with a
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
  pixels or it overflows past the canvas' bottom-right. Palette files (`.gpl`, `.kpl`, `.aco`,
  `.ase`) have `kind: "palette"` and always render
  as **color swatches** (`PaletteDiffView`) — the first palette is embedded in the art diff's
  `LayerStackPanel` navigator; standalone palettes get their own panel. This route is **not**
  Artist Mode gated. The swatch diff is computed **in the backend** (`src-tauri/src/palette.rs`):
  each format is parsed to a flat list of named sRGB swatches (`.gpl` text, `.kpl` = zip +
  `colorset.xml` via roxmltree, `.aco`/`.ase` = hand-rolled big-endian binary readers), then
  `palette::diff` matches swatches by name (recolor = "modified", not remove+add) and
  `commands::palette_dto` serializes it as the `Palette` `DiffEntryDto` variant from `commit_diff`/
  `working_diff`. A malformed palette degrades to a plain text entry. A `.kra` diff also emits a
  `Palette` entry per **embedded document palette** that changed — Krita stores document palettes
  as `.kpl` blobs under `<image>/palettes/` inside the archive; `commands::kra_palette_dtos`
  enumerates them via `KraSource::palette_entry_names`, skips unchanged ones by content hash, and
  runs them through the same `palette_dto` (so one `.kra` yields its `Art` entry plus zero-or-more
  `Palette` entries, keyed `<kra>::<palette-file>`). Generic text files (`kind: "text"`) depend on Artist Mode: `FriendlyFileDiff`
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
- **Application tour** — a first-launch, one-time spotlight walkthrough of the shell
  (`src/lib/tour.tsx` `TourProvider`/`useTour`, `src/components/shell/TourOverlay.tsx`), fired via
  `beginIfFirstTime()` (called once from `RepoShell` on mount) and gated on a `localStorage` flag
  (`krita-vc:tour-completed`) — same context-plus-flag pattern as Artist Mode and the custom title
  bar toggle. `TOUR_STEPS` is a flat, linear array (`{tourId, title, body, view?}`); a step with a
  `view` drives `setActiveView` as a side effect so the tour can walk through Changes, History,
  Branches, and Performance without the user switching tabs. Spotlight targets are plain
  `data-tour-id` attributes (`IconButton`/`MenuItem` both take an optional `tourId` prop; a few
  other targets carry `data-tour-id` directly) — no ref plumbing. The dim-with-a-hole effect is
  four opaque `fixed` bands tiling the viewport around the target rect plus a fifth transparent
  non-interactive div over the hole itself — deliberately not a box-shadow spread or an SVG mask,
  both of which silently failed to paint in this WebView build. Steps that spotlight a row inside
  the panel-options `Menu` force it open via a new `Menu.forceOpen` prop (ORed with the normal
  click-toggled state so it never fights outside-click/Escape handling), since the overlay blocks
  the real click that would otherwise open it. Replay anytime via Settings → Appearance →
  "Replay tour" (`restart()`). See
  [`docs/frontend-architecture.md`](docs/frontend-architecture.md#application-tour).

All data flows through Tauri `invoke` keyed by the selected repository path; the component/prop
boundaries (`Repository`, `DiffEntry`, `Commit` — incl. `parents` lineage — `Branch` incl. `tip`,
`WorkingChange`) are the contract between `src/lib/repoData.ts`/`repository.tsx` and the UI.
