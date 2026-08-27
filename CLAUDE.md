# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project status

Tauri 2 + React 19 + TypeScript desktop app — a version-control client for Krita art files.
This is a **local-only VCS**: there is intentionally **no remote/push/pull/sync** — no remotes,
no fetch, no cloud sync. The UI exposes only local operations (commit history, local branches,
working-tree changes). Don't add remote-facing affordances unless the project scope changes.
The Rust side is a **working custom local VCS** — its own store (not git), with a `.kra`
tile-delta engine (`src-tauri/src/`: `repo`, `scan`, `commit`, `delta`, `kra`, `tiles`, `branch`,
`gc`, `palette`, `stash`, `merge`; commands in `commands.rs`).

**One `.kra` document = one history** (v2.0.0; see
[`docs/per-document-tracking.md`](docs/per-document-tracking.md)). The unit the app versions is a
single artwork, not a folder of them — artists work per painting, and a folder-wide history with a
"which files go in this version" step was the biggest conceptual tax the app charged. Each
document's store is **entirely self-contained** (its own `objects/`, `chains/`, `cache/`,
`commits.log`, `branches.json`, `stashes.json`, `index.json`, `config.json`) and they sit side by
side in one hidden `.kvc/` container beside the artwork:
`artfolder/.kvc/<slug>/`. The container is what keeps a folder of seven tracked paintings from
sprouting seven folders; **nothing is shared between the stores inside it**, and that is
load-bearing — sharing `objects/` is what would force a `Project`/`Document` split, a GC root
union, and a sweep able to delete another artwork's blobs. Cross-document tile dedup is the only
loss and it's worth ~nothing (dedup pays off *within* one painting's history, which is untouched).
`Repo` therefore carries **two paths**: `root` (the folder holding the document — what working-tree
writes `safe_join` onto) and `store` (history). `store_dir_for` is the single place that decides,
honouring an app-global custom store root read from
`%LOCALAPPDATA%/com.zeru-sakamoto.krita-vc/storeRoot.json` — deliberately *not* per-repo `Config`,
since the `kvc` CLI never sees the app's settings and must resolve the same store; it's cached in
a `RwLock` because `store_dir_for` now runs on every command. `doc.json` records
`{relpath, displayName, createdAt}` and `Repo::doc` carries it, so the tracked document is known
before the first commit (the index only knows *committed* files). Three failure states, and
conflating two of them loses data: opens / `NotARepo` (never versioned) / **`StoreUnreachable`**
(history on a drive that isn't mounted) — answering the third like the second would mint an empty
store and orphan every saved version, so `locate_failure` is a pure function precisely so the rule
is testable without touching the process-global root. `Repo::delete` removes the **store**, never
the artwork (the folder model deleted the art too), and takes the container with it when empty.
**There is no migration** — v1 folder repositories are simply not readable, which was free because
the app had no users.

**Tracking**: only `.kra` (`scan::is_supported`, a **suffix match on the whole relpath** so
Krita's autosave artifact `foo.kra-autosave.kra` — which ends in `.kra` — is rejected explicitly).
Standalone palettes (`.gpl`/`.kpl`/`.aco`/`.ase`) are **not** tracked; a `.kra`'s *embedded*
document palettes are still parsed and diffed off the `.kra` itself, which is where the value was
and the observation that motivated the whole change. `is_supported` no longer gates a walk —
**there is no walk**: a store tracks one designated document, so `scan_detailed` stats one path
(keeping the racy-clean guard against the index's own mtime). Scanning an art folder holding fifty
400 MB `.kra` files costs one `stat`.

Storage layout inside one store: chains are **sharded per tracked file**
(`chains/`, lazy-loaded, `KVCC2`-tagged bincode — pre-KVCC2 shards and legacy monolithic
`chains.bin`/`chains.json` migrate transparently), the commit log is **append-only JSON-lines**
(`commits.log`; a commit appends one line), stashes live in `stashes.json` (absent = empty shelf),
loose objects are sharded 256-way (`objects/<xx>/`, flat legacy stays readable), and a
commit with ≥32 new objects writes **one pack file** (`objects/pack/*.pack`) instead of loose
files — per-file creates dominated large commits on Windows. **Stashing** ("Set aside" in Artist
Mode) parks working-tree changes off to the side of history and reverts the files on disk
(`stash.rs`, records in `stashes.json` — deliberately *not* `commits.log`, which would put
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
rewrites on >25% dead, and consolidates small packs; the raster cache (`cache/`) is
size-budgeted (`Config.cacheMaxBytes`, default 256 MB) with LRU pruning. **Data integrity**: every
working-tree write (switch/rollback/discard/stash-pop/restore-file) and every loose-object write
goes temp-then-rename — `repo::write_file_atomic` appends a `.kvctmp` suffix rather than
substituting the extension (`with_extension` would collapse `a.kra` and `a.gpl` onto one temp path)
— and state-file writes plus the `commits.log` append are **fsynced** before the rename, without
which "tips go last" isn't an ordering under power loss; object/pack payloads deliberately are not
(commit hot path). The same restore paths set `Repo::verify_reads`, which makes `reconstruct`
re-hash every object it rebuilds and refuse with `KvcError::Corrupt` — deliberately **off** for
diffs/previews, the hot loop, and a test pins both halves. A read-only `check_repository`
(`check.rs`, reusing the extracted `gc::mark_live`; also `kvc check` and Settings → Storage →
"Check for problems…") reports missing objects, broken chains, dangling tips, undecodable
commit-log lines and unreadable packs; findings come back as a *successful* run, since
`{"error":…}` means the check itself failed. See
[`docs/data-integrity.md`](docs/data-integrity.md). **Settings** (activity-bar
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
**Storage** (per-artwork `cacheMaxBytes` + `tilePixelDeltas` knobs — `get_repo_config`/`set_repo_config`
→ `Repo::save_config`, a config-only write — plus "Clean up storage", plus two **app-global**
settings that render *outside* the tab's artwork gate so they're reachable with nothing open:
**"Where version history is kept"** (`get_store_root`/`set_store_root`; default is the hidden
`.kvc/` container beside each artwork, and changing it moves nothing — it only decides where the
*next* artwork's store is created, which the copy says plainly rather than implying a migration)
and **"Background CPU use"** — see the CPU headroom note below). Backing up an artwork
(`backupRepository` in `repository.tsx`, zips the `.kra` **and its store** via
`export_repository_zip`, rebased under `.kvc/<slug>/` so extracting the archive anywhere gives a
tracked document there) is **not** in Settings — it's its own one-click zip-icon `IconButton` in `ActivityBar.tsx`, directly above
the Settings gear, wired to a small global toast (`lib/toast.tsx`, `ToastProvider`/`useToast`,
single-slot, auto-dismissing, bottom-right, reusing the `--z-toast` token) for the "Saved to …"/
error result, since the busy overlay covers only the in-flight zip itself. **Branching is real**:
`branches.json` maps branch name → tip
commit id (+ the current branch); create is O(1) (an optional base branch materializes that
branch's tree first, and `branch::create_branch_at` starts one at an **arbitrary commit** —
"go back to version 5 and try a different direction" — via the same `tree_at_commit` +
`materialize_tree` path, exposed as `create_branch`'s `commit:` arg, mutually exclusive with
`base:`; deliberately **not** in the `kvc` CLI, since the Krita plugin has no version picker to
call it from), switch rewrites only files that differ between branch trees, merge fast-forwards or builds a two-parent merge commit (conflicts take the source
version, flagged `"C"`). Trees fold along the **first-parent chain** (`tree_at_commit`) — every
commit's `files` is by invariant the diff vs its first parent. `list_commits` is scoped to
commits reachable from the current branch tip unless its `allBranches` flag is set (default
false, so every existing caller is unchanged), which unions the reachable set over *every*
branch tip — the Version Map's "show all lines" mode. The frontend drives it via Tauri `invoke` in the
desktop shell (history, scan, commit, repo lifecycle, rollback/undo, branch create/switch/merge/
delete, stash create/pop/drop, and per-commit visual diffs). **There is no mock data by default**: in a plain browser
(`npm run dev`, no backend) the data hooks return empty results, repository/branch actions are
no-ops, and the status bar shows a "Browser preview" badge — browser mode is for UI work only.
The one exception is an explicitly opt-in dev fixture (`src/lib/mockRepo.ts`): loading
`http://localhost:1420/?mock` makes `useCommits`/`useBranches`/`useCommitDiff` return a
hand-written 12-version history with synthetic composites, so canvas/layout work (the Version
Map) can be seen without the desktop shell. It is gated on `import.meta.env.DEV` **and** the
query flag, so it is stripped from production builds and never fires by accident.
`.kra` diffs are real and load in two
stages: `commit_diff` returns the capped composite + layer metadata fast, then `commit_layers`/
`working_layers` **stream** per-layer rasters over a Tauri `Channel` as each finishes, with
capped PNGs persisted in a content-addressed `cache/` (see the diff viewer section).
In the desktop shell rasters ship as **`kvcimg://` URLs** served straight from that cache
(registered in `lib.rs`, handler `commands::serve_raster` — no base64, browser-cacheable);
outside the shell or on a cache-write failure they fall back to base64 data URLs.
Non-`.kra` diffs are still minimal. Rust tests live in `src-tauri/tests/`; the frontend has no test
runner yet — if you add one, update this file.

**CPU headroom** (`cpu.rs`): the engine deliberately does **not** take the whole machine. Rayon's
global pool would size to `num_cpus`, and with 17 `par_iter` sites nested three deep on both hot
paths (diff: layers → tiles → downscale rows; commit: entries → tiles → bsdiff/zstd/blake3) a
commit pinned every core at normal priority — starving Krita, which is often the very thing that
triggered the commit via the plugin. So we build **our own pool**: workers are born at
below-normal priority (`start_handler` → `SetThreadPriority`, Windows-only via `windows-sys`, no-op
elsewhere — this is the part that actually keeps the desktop responsive), sized to a user-set
percentage of logical cores (default 75%, min 1). The integration is **one line** —
`commands::run` is the single funnel for every command and nested `par_iter`s inherit the
installing pool, so wrapping that closure in `cpu::install` covers the entire engine; a unit test
pins that inheritance because everything depends on it. The budget is app-global (the pool is
process-wide), so it lives in `localStorage` via `src/lib/cpuBudget.tsx` → `set_cpu_budget`, **not**
in the per-repo `Config`. Changing it swaps in a fresh pool with no restart. `cpu_budget_sweep` in
`tests/bench.rs` measures what the cap costs. Three things ride along with it:
the **`kvc` CLI** calls `cpu::lower_process_priority` (`SetPriorityClass`) and installs the pool
too — it needs it *more* than the app, since the plugin spawns it inside Krita's process tree
mid-paint; **concurrent heavy ops are capped at two** (`cpu::heavy_permit`, a
`tokio::sync::Semaphore`, taken by `commands::run_heavy` — cheap reads keep plain `run` so they
never queue behind a diff), because cancelling a diff in the UI does *not* cancel the backend and
rapid history clicking otherwise stacked unbounded 64 MB decode buffers; and the **plugin poll
spawns one process per tick, not two** — `kvc status` now also emits the branch list, free because
`open_light` already parsed `branches.json` (shared `branch_list` helper with `run_branches`,
pinned equal by `kvc_cli.rs`), and `refresh()` returns early when the docker isn't visible.
On the frontend, three memo dep arrays are deliberately *narrower* than the values they close over
(`ArtDiffView`'s composite layer, `ArtCanvas`'s `compositeSvg`, `LayerStackPanel`'s
`compositeThumb`) — streamed layers reallocate `diff`/`diff.layers` every arrival while the fields
those memos actually read never change, and depending on the whole object rebuilt multi-MB SVG
strings per layer. Don't "fix" them back to exhaustive deps. See
[`docs/performance.md`](docs/performance.md).

Deeper docs live in [`docs/`](docs/README.md): frontend architecture, file tracking & version
control (the backend), the visual diff viewer, [performance](docs/performance.md) (why the
`.kra` diff path is fast: staged/streamed loading, rayon parallelism, CPU headroom, the
`cache/` raster cache, raster downscaling, and the dev/release build profile), and the
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
- **`kvc` CLI** (`src-tauri/src/bin/kvc.rs`): a second, Tauri-free binary target over the same `krita_vc_lib` engine (the crate builds `rlib` for exactly this). Ten subcommands (`status`, `commit`, `branches`, `switch`, `create-branch`, `discard`, `stash`, `stash-pop`, `stash-list`, `check`) taking `--repo <path to a .kra>` plus scalars (the flag name is unchanged — the plugin passes whatever it has, and the engine resolves the store from the document path), each printing one JSON object to stdout (or `{"error": "..."}` to stderr, non-zero exit — a panic is caught in `main` (`catch_unwind` + a silenced panic hook) and reported as `{"error":...}` JSON too, since the plugin parses stdout/stderr as JSON and a bare Rust backtrace would break it). The optional file-subset flag (`--paths` on `commit`/`discard`/`stash`) is a **JSON array** — the hand-rolled parser is a map, so a repeated flag would overwrite, and paths can contain commas; omitting it means "everything". Every mutating subcommand takes a real OS-level advisory lock (`<store>/kvc.lock`, `File::try_lock` — `LockFileEx`/`flock`, released automatically by the OS when the process's handle closes, even on a crash — tagged via a `kvc.lock.info` sidecar with a present-participle label like `"switching branches"` so a caller blocked by `KvcError::Locked` sees what's holding it and for how long) so it can't race a concurrent desktop-app write — the engine itself has no locking; reads (`status`, `branches`, `stash-list`, `check`) take none, so the plugin's 1.5s poll never contends. `status` carries a `stashes` count so that poll needn't spawn a third process, plus the tracked `document`. The **no-args usage line is load-bearing**: the plugin's "Locate kvc…" picker identifies the binary by its literal `"usage: kvc"` prefix, so widen the command list freely but never change that prefix. `stash-list` reuses `commands::stash_dtos` for its **newest-first** order, which "bring back latest" depends on. Contract tests: `src-tauri/tests/kvc_cli.rs` (spawns the real binary). Two `[[bin]]` targets means bare `cargo run` is ambiguous without `Cargo.toml`'s `default-run = "krita-vc"`.
- **Krita plugin** (`krita-plugin/`, kept out of the npm/Cargo build): a PyKrita "Version Control" docker — commit, one-tap checkpoint, discard, set-aside/bring-back, save-and-rescan (⟳), and branch switch/create from inside Krita, via `kvc_client.py` shelling out to the `kvc` CLI above. It scopes to the **active document**: `find_doc` replaced the old `find_repo` (which walked up looking for a `.kvc/` directory) and `is_tracked_document` replaced `in_repo` (a folder-prefix test that would have said yes to a *neighbouring* artwork — a different history entirely). Deliberately does not do tracking setup, history browsing/restore, undo, branch merge/delete, or anything remote — those stay desktop-app-only. The engine only sees the disk, Krita's canvas only memory, so the docker moves both ways and **both directions are load-bearing**:
  - **memory → disk** (`_save_tracked`, the tracked `.kra` when modified — `.kra` only, since Krita may raise an export dialog on a `.png` and hang the UI thread it's saving on). Driven by focus entering the docker (`QApplication.focusChanged` — not an event filter; focus lands on child widgets and `FocusIn` won't reach the dock), the ⟳ button, and `_commit_with_message`. Two traps: commit **must `refresh()` between the save and `_selected_paths()`** or it skips the very work just written (a doc clean *before* the save isn't in `_shown_paths`/`checked`); and `_save_tracked` sets `busy` because `doc.save()` spins the event loop, which would let the 1.5s poll `kvc status` a half-written `.kra`.
  - **disk → memory** (`_rebuild_docs`, wrapping switch/discard/stash/pop). Refuses while any open doc is unsaved, then **closes and reopens** each doc whose file changed (mtime/size snapshot — `switch` doesn't report what it rewrote). Drop the reopen and Krita keeps serving the pre-op copy, so the next Ctrl+S silently reverts the operation; drop the refusal and that reopen eats real work — the engine's dirty-tree guard never sees Krita's memory.

  Consequence to preserve: auto-save makes that refusal rare, so **Discard's confirm is the only thing standing between the artist and losing saved-but-uncommitted work** — saving isn't committing, and the reopen takes the undo history too. Also: checkbox state lives in `VcDocker.checked`, **not** the widget (the poll rebuilds the list and would wipe a tick mid-edit; the rebuild is skipped when the path list is unchanged). `kvc_client.py` blocks the UI thread by design (see its header). See [`krita-plugin/README.md`](krita-plugin/README.md).
- **Frontend ↔ backend IPC**: Rust functions annotated `#[tauri::command]` (e.g. `greet` in `lib.rs`) are exposed to the frontend and called via `invoke("command_name", { args })` from `@tauri-apps/api/core`. New backend functionality should be added as a `#[tauri::command]` in `lib.rs` (or a module it includes) and registered in `generate_handler!`.
- **Permissions/capabilities**: `src-tauri/capabilities/default.json` declares which Tauri permissions (e.g. `core:default`, `dialog:default`) the main window is allowed to use. Any new Tauri plugin or privileged API needs its permission added here or the call will be rejected at runtime.
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
- **Artworks** (`src/lib/repository.tsx`): the selected unit is **one `.kra` the user chose to
  track** (local-only — no remotes). The `TopBar` switcher selects among them; the list + selected
  id persist to `localStorage` (`current` is null until the user adds one), and `Repository.id` is
  the document's path — which is why the ~30 Tauri commands took a per-document model with **zero
  signature changes**. In the desktop shell "Track an artwork…" opens a native *file* picker
  filtered to `.kra` (`tauri-plugin-dialog`) and creates its store (`init_repository`). There is no
  "create repository" flow: you can't create an artwork from the VCS, only start tracking one
  Krita already made. `removeRepository`'s destructive arm deletes the **history**, never the
  `.kra`. The context and type are still named `Repository` — that name is the app-wide contract
  with `repoData.ts` and every panel, and renaming it buys nothing the doc comments don't.
  `isStoreUnreachableError` matches the backend's stable `"history isn't reachable"` prefix, the
  one error that must never be answered with "start tracking?". In a plain browser there is no
  picker and these actions are no-ops.
- **UI primitives** (`src/components/ui/`): `Button.tsx`, `IconButton.tsx` (flat Krita-style, no
  background until hover), `Menu.tsx` (dropdown with outside-click + Esc to close). Shared across
  shell and VCS components.
- **VCS components** (`src/components/vcs/`): commit cards, the git-style history graph
  (`CommitGraph` + `CommitGraphRail`, lane layout from `lib/graph.ts`; lane colors are a deliberate
  functional exception to the single-accent rule), branch badge, file-status chip, the sidebar
  panels (`ChangesPanel` — **the layers that changed** since the last version, not a file list.
  A store tracks one `.kra`, so a file list would always be one row and staging a subset of a
  one-file working tree means nothing; what the artist wants is what moved in the painting. The
  rows come from `useWorkingDiff`'s existing per-layer `change` — **no new backend command** —
  rolled up so a changed group reads as one row with a "+N inside" count (the backend enumerates
  layers via `.descendants()`, so children arrive as siblings of their group with no parent link;
  the rollup is approximate for that reason and says so). The rows are **read-only**: layer-level
  staging needs a write path that synthesizes a `.kra` holding only the ticked layers —
  `merge::merge_layers` is most of it, and **top-level** is the grain it natively speaks
  (`layers_node` is the `<layers>` directly under `<IMAGE>`), which also keeps the unit whole so
  you can never emit XML referencing a data file you didn't copy. Until that exists a version
  captures the whole artwork; checkboxes that don't bind would be worse than none.
  `commit_snapshot`'s `paths` arg (`commit::commit_selected`) survives for the CLI and for that
  future. While a commit is in flight the commit button spins and the `StatusBar` shows a progress
  bar, via the shared `saving`/`scanning` flags on the repository context — `BranchesPanel` is local
  branches with **real actions**: click to switch, hover-row merge/delete with confirm modals
  (the delete affordance is hidden on `main` — the backend also refuses it with `DeleteMain`), a
  "New branch" modal; shared dialogs live in `BranchDialogs.tsx`, and the backend's dirty-tree
  error — matched on its stable `"unsaved changes"` prefix — becomes a friendly save-first prompt
  offering three ways out: save, **set it aside** (stashes everything, then retries the blocked
  switch/merge), or jump to Changes. **Set-aside actions** sit in the `Sidebar` panel-options
  `Menu` as **three divider-separated groups**: undo/discard, then set-aside, then bring-back.
  `Menu` still has no submenus, but gained a `MenuItem.separator` flag (a `border-t` above that
  row) since one `footer` group can only draw one rule and this needs two. Set-aside (one row now
  — with one tracked artwork "staged" and "everything" became the same action) and bring-back are
  both **changes-view only**, since both act on the working tree — History's panel-options menu is
  just undo. `StashScope` kept its parameter but lost its `"staged"` arm, so layer-scoped
  set-aside has somewhere to land.
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
  `cache/` (keys carry a `box1` filter-version token), so repeat views skip rasterization.
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
- **Version Map** (`src/components/vcs/VersionMapPanel.tsx` + `VersionNode.tsx`) — the **default**
  view, and the visual replacement for the History graph. This branch's line of versions on a
  **pannable, zoomable canvas**, laid out **left→right oldest first** along a spine, one node per
  commit: the version's **after-composite** on top, a connector dot the spine runs through, then
  the caption and a two-column grid of **chips for the layers that changed** (layer-type icon from
  the new `friendly.ts` `layerTypeIcon()` + an A/M/D glyph in `FileStatusChip`'s icon-and-color
  vocabulary). On the Map tab it owns the whole well — **no Sidebar, no Inspector**; the node *is*
  the metadata. (The Performance tab is the one place it shares the well with a Sidebar — see
  below.) Clicking one opens the full `MainPanel`/`DiffView` in place (back button; a `Menu`
  file-picker in the header for a multi-file version), alongside its own toggleable **Inspector**
  (open by default, same restore action and "Selected" section as the legacy view — hidden/shown
  with the same `SidebarSimple` icon-button pattern `AppShell` uses, but tracked in
  `CommitDrilldown`'s own local state rather than shared with the legacy layout's). The map's
  "no Inspector" only describes the un-opened canvas.
  By default only the current branch is drawn, which is free: `list_commits` is already scoped to
  the current branch tip. A header toggle (`GitBranch` icon, shown only when another branch
  exists) switches to **all branches on their own lanes**; it defaults **off**, persists to
  `localStorage` (`krita-vc:map-show-all`), and is deliberately plain component state rather than
  another app-wide context — it is map-local, not a global preference. On, the panel makes its
  *own* `useCommits(repoPath, nonce, true)` call for the wider `allBranches` scope; off, the path
  it passes is `""`, which `useCommits` short-circuits, so the off state costs nothing and draws
  exactly the commits the shell already loaded. **The map draws branches; it does not act on
  them** — create/switch/merge/delete still live only in the legacy Branches panel, pending a
  floating action bar.
  Built on **React Flow** (`@xyflow/react`), the one framework-scale frontend dependency in the
  app — chosen over the in-repo `useZoomPan` because the branch phase needs edge routing, a
  minimap and fit-view over a real graph, which is most of what React Flow is. It costs ~60 KB gz.
  **The version is pinned exactly (`12.10.2`, no caret)**: `12.11.4` ships a broken pairing —
  `@xyflow/react` imports `handleAttributionWarning` from `@xyflow/system@0.0.80`, which doesn't
  export it, and Vite's dep optimizer dies on it. Re-test before widening the range.
  Load-bearing details:
  - Node positions are **computed** from the commit graph and `nodesDraggable={false}` — history
    is not a mood board, so nothing is persisted and a new commit can never leave the layout stale.
    `NODE_PITCH`/`LANE_PITCH` are the layout constants. The lane/column assignment itself is a
    pure function in **`src/lib/versionMap.ts`** (`buildVersionMap`), deliberately *not* in
    `lib/graph.ts` — `buildGraph` lays a DAG out vertically for the legacy rail, where a lane is
    an x column and lane 0 means "the mainline"; here a lane is a y offset and lane 0 means "the
    branch you're standing on". Three rules carry it: **lane 0 is the current branch's
    first-parent spine** walked back from its tip (*not* the commits stamped with its name — after
    a merge the folded-in commits still carry *their* branch and belong on a side lane, and
    standing on a side branch its shared ancestors are stamped `main` and would jog your own line
    down a lane); everything else groups by `commit.branch` into lanes 1.. in order of first
    appearance; and **column = generation depth** (`1 + max(depth(parents))`), so parallel work on
    two branches lines up in the same column instead of leaving chronological gaps and a merge
    lands one column past the deeper parent. That same depth is the node's **"Version N"** — so
    shared ancestors read the same on every lane, and for a linear history it is identical to
    `friendly.ts`'s positional `versionNumbers()` (which the legacy graph and Inspector still
    use). Two lanes can therefore both show "Version 5"; the lane color and the branch name in the
    caption disambiguate.
  - Edges are derived from `parents`, not from list adjacency, so a merge commit's second parent
    draws its own line.
  - **The spine is one drawing system: SVG edges, dot to dot.** Both of a node's handles sit on
    its **connector dot** (node center, `SPINE_TOP`) rather than on its left/right edges, so a
    single edge path spans the source dot, the gutter and the target dot; React Flow draws edges
    beneath nodes and the dot is opaque, so the line reads as passing through it. This works
    because `@xyflow/system`'s `getHandlePosition` uses the handle's own measured x/y and does
    **not** snap to the node box. It replaced half-width CSS bars drawn inside each node: a
    1.5px box shifted `-translate-y-1/2` lands on a half pixel while an SVG stroke centers on its
    path, so the in-node and gutter runs stepped ~1px at every node edge and could disagree on
    color. Don't reintroduce an in-node segment.
  - Line color is **opaque** — `color-mix(…, var(--color-bg))`, not `transparent`. Mixing toward
    transparent let two crossing lines composite into a brighter, two-tone band that read as a
    doubled line.
  - A lane-crossing connector carries per-edge `pathOptions: { offset: NODE_PITCH / 2,
    stepPosition: fork ? 0 : 1, borderRadius: 16 }`. `offset` half a column puts the bend in the
    **middle of the gutter**, clear of the node's caption and chips (the default 20 descends
    straight through them); `stepPosition` 0 bends right after the source, 1 right before the
    target — always next to the end on the shallower lane, so a branch drops out of the spine at
    the version it started from and climbs back in at the version it merges into. The default 0.5
    ran it alongside the spine for half a column, which is what doubled the line.
  - That connector's stroke is a **gradient** between the two lanes' colors, defined by
    `LaneGradients` in its own zero-size `<svg>` (a `url(#…)` paint reference resolves
    document-wide). It must be `gradientUnits="userSpaceOnUse"` running **purely vertically**
    between the two lanes' spine y values — user space here is React Flow's flow coordinates. That
    confines the transition to the descent, so each horizontal run is exactly its own lane's
    color, which is also what hides the short stretch the connector shares with the spine before
    the bend. An `objectBoundingBox` gradient smears the transition across the whole path and
    tints that shared stretch.
  - Nodes must carry **explicit `width`/`height`** (`NODE_W`/`NODE_H`). React Flow's MiniMap sizes
    from the *user* node object (`getNodeDimensions` reads it, not the measured box), so without
    them it renders completely empty. Real node height varies with the chip count; `NODE_H` is
    nominal and only feeds culling + the minimap.
  - The minimap's **mask and viewport frame are drawn by `MinimapViewport`**, not React Flow,
    which paints both as one evenodd path — so `maskStrokeColor` also strokes the mask's outer
    rectangle (half of it inside the viewBox: a stray accent line down the edge) and the hole's
    corners can't be rounded. The `MiniMap` gets `maskColor="transparent"`/`maskStrokeWidth={0}`
    and the overlay draws an evenodd dim path plus a stroke-only `<rect rx>`. It re-derives React
    Flow's (unexported) minimap geometry, so `MINIMAP_W`/`MINIMAP_H`/`MINIMAP_OFFSET_SCALE` must
    stay exactly what the `MiniMap` is passed, and it's a `Panel` so it lands on the minimap
    pixel-exactly.
  - Wheel **zooms toward the cursor** and drag pans (`zoomOnScroll`, `panOnScroll={false}`) —
    deliberately the same gesture pair as the diff viewer's `useZoomPan`, so the app's two
    canvases don't disagree about what the wheel does.
  - **Branch color** is a small local palette (`BRANCH_LANE_COLORS` in `VersionMapPanel.tsx` —
    `info-fg`, `success-fg`, `warning-fg`, `accent`, cycled by `laneColor(lane)`), deliberately
    separate from `graph.ts`'s `LANE_COLORS` (whose lane-0-is-accent is a fixed convention for the
    *legacy* History graph and stays untouched). A lane's color paints every node's connector dot
    on it and, mixed 55% toward transparent via `color-mix`, the spine between them; **every**
    branch tip's thumbnail additionally gets a **detached** `outline`/`outline-offset` ring in its
    lane color, distinct from the flush `ring-accent` used for whichever node is open, plus a
    branch-name chip under its caption. A branch created but not yet committed on shares its
    parent branch's tip node, so it shows up as a second chip there for free.
  - The canvas background is React Flow's `Lines` variant, colored by a `--color-grid` token
    (`src/styles/global.css`) derived from each theme's own `--color-bg` via
    `color-mix(in srgb, var(--color-bg) 75%, black)`, so dark themes render a near-black, barely
    visible grid with no per-theme literals; the two light themes override it back to
    `--color-border` (the darkening is dark-theme-only).
  - Opening a version unmounts the *canvas* (not the panel — see "mounted once" below), which
    would come back at the origin (i.e. scrolled to the *oldest* version). The viewport is stashed
    in a ref on the way out and handed back as `defaultViewport` on the way in.
  - **Zoom LOD**: below `LOD_ZOOM` the caption and chips are dropped. The `useStore` selector
    returns a *boolean*, so a node re-renders only when the threshold is crossed, not per frame.
  - **Mounted once, for the shell's lifetime.** `AppShell` renders exactly one
    `VersionMapPanel` and toggles a `hidden` class on its wrapper rather than conditionally
    mounting it per view — it used to have two separate JSX call sites (one for the Map tab, one
    for Performance), each remounting the whole `ReactFlowProvider` on every switch away. The
    wrapper's `showMap` flag covers both the Map tab and Performance-without-Legacy (below);
    everywhere else the panel is present but `display: none`. Losing the mount would silently
    reset both the panned/zoomed viewport and the open-drilldown `openId` on every tab switch,
    since neither lives in anything React Flow itself persists — both are local `VersionMap`
    state.
  **It adds no backend command.** A node calls the same `useCommitDiff` → `commit_diff` the diff
  viewer does, which already returns exactly what a node needs (`afterImage` = the capped,
  content-addressed `mergedimage.png` as a `kvcimg://` URL, plus `layers[]` with `change`/
  `layerType`) and *not* the expensive per-layer rasters (`with_rasters = false`). So opening a
  node's drilldown is a `diffCache` hit, not a second round trip. `commit_diff` is `run_heavy`
  (2 concurrent) and also builds a changed-pixel mask the node discards, so the count of heavy
  calls is bounded by `onlyRenderVisibleElements` — an off-viewport node isn't mounted, so it
  never fetches. If that stops being enough the upgrade is a dedicated
  `commit_thumbnails(path, ids)`; don't reach for it before then.
  The old **History** and **Branches** tabs are still there but hidden behind Settings →
  Appearance → **"Legacy version history"** (`lib/legacyHistory.tsx`, same context-plus-
  `localStorage` shape as Artist Mode, default **off**). `ActivityBar` filters those two items on
  it and `RepoShell` snaps back to the map if the toggle goes off while you're standing on one, so
  you can't be stranded on a view with no icon. `CommitGraph`/`BranchesPanel`/`lib/graph.ts` are
  untouched — branch create/switch/merge/delete still live only in the Branches panel. The
  **Performance** tab (`PerformancePanel`, always visible — not legacy-gated) rides the same
  toggle: with Legacy off there's no commit selection left to drive a diff viewer, so `AppShell`'s
  `perfShowsMap` flag (`activeView === "performance" && !legacy`) shows the map beside the stats
  sidebar instead of `MainPanel`+`Inspector`, reusing the same persistent `VersionMapPanel`
  instance (see "mounted once" above). Legacy on restores the old diff-viewer layout there,
  unchanged.
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
