# Krita VCS

A desktop version control app for Krita paintings, built with Tauri 2, React 19 and TypeScript.
Instead of code-style text patches it shows artists what actually changed: the layer stack and a
visual diff of each version (side by side or with a swipe slider, with changed pixels highlighted and
zoom and pan kept in sync).

> Status: working end to end, at v2.1.0. The Rust backend is a custom local version control system
> with its own store (not git) and a `.kra` tile-delta engine, and the React frontend drives it over
> Tauri IPC. In a plain browser the UI renders but every action does nothing; that mode is for UI
> work only.

## What it is, and what it isn't

Krita VCS is local-only. There's deliberately no remote, push, pull or cloud sync: no accounts, no
server, and nothing leaves your machine. The unit it versions is one painting. You choose a `.kra`
file to track, and its history lives in a hidden `.kvc` folder beside it, with a store of its own
inside, so a folder of several tracked paintings still has one hidden folder. The UI only offers
local operations: the painting's versions, its unsaved changes, and local branches.

It doesn't use git. The backend is a purpose-built store for large binary `.kra` files, which git
stores poorly: a small brush stroke changes compressed bytes all through the archive, so git's delta
compression finds little to share between versions.

## Features

- **The Version Map**, the default view: a painting's versions laid out left to right on a pannable,
  zoomable canvas, each with its thumbnail, its note and the layers it changed. Click a version to
  open its visual diff in place, with an Inspector for its details and a Restore action. Turn on "All
  lines" to see every branch on its own lane, or start a new line from any past version. A dashed
  card marks unsaved changes at the end of the current line. The older History graph and Branches
  list are one setting away (Settings → Appearance → Legacy version history).
- **Visual layer diffs** for `.kra` files: a Krita-style layer panel beside a before and after
  canvas.
  - Side-by-side and swipe-slider modes.
  - Shared zoom and pan: the wheel zooms toward the cursor and a left or middle drag pans, the same
    in both modes, so before and after, and the slider's divider, stay aligned.
  - Changed-pixel highlighting (a tint, a hatch and a dashed outline that follows the changed
    pixels), with coarse region boxes as a fallback. The highlight is per layer: focus a layer and it
    shows only that layer's changes. Its color always follows the theme's accent.
  - Click a layer for its type, visibility, opacity, blend mode and painted area, or the composite for
    the canvas size, resolution and color space. Palettes embedded in the `.kra` get a swatch-by-swatch
    diff with hex values.
  - Each version is compared with the version before it, and your unsaved changes with the last
    version.
- **Saving versions**: save the whole painting, or tick only the layers you want in the version. The
  layers you leave out stay changed, ready for a later version, and everything starts ticked, so
  saving everything is still one click. Canvas size, animation and document settings always come
  along.
- **Going back**: restore any older version as a new version (nothing after it is lost), undo the
  last version (its changes come back as unsaved work), or undo everything since your last version.
  Restoring the version you're already on just discards unsaved changes, with no new history entry.
- **Set aside** (stash): park unsaved work to the side of history and bring it back later, without
  making a version. If the painting changed in the meantime, the set-aside layers are merged back in
  on top instead of overwriting anything. A switch or merge blocked by unsaved work offers to set it
  aside as the way through, and Settings lists everything on the shelf with its origin branch and
  age.
- **Branching and merging**: create (from the current version or any past one), switch, merge
  (fast-forward or two-parent) and delete local branches, with real tree materialization. When both
  sides changed the painting, the incoming version wins and is flagged for review; if one side
  deleted a file and the other edited it, the edit wins, so a merge never quietly loses work.
- **Backup and restore**: back up several paintings at once into one archive, checked right after
  it's written. Restoring puts each painting back in its original folder when it still exists, skips
  paintings already there unless you choose Replace, and can compare both histories side by side
  first.
- **Storage care**: "Clean up storage" reclaims history nothing can reach any more (mark and sweep,
  with a dry run first and a 14-day trash folder), "Check for problems" verifies the whole history
  without changing it, and the preview cache has a size budget with LRU pruning.
- **Settings** (the gear in the activity bar), in four tabs:
  - Appearance: Artist view, the custom title bar (the app's own frame and window controls, on by
    default; switch back to the OS frame any time, no restart), Legacy version history, your name
    for the versions you save, eight color themes (six dark, two light), and buttons to replay the
    tour and the welcome.
  - Performance: how much of the CPU background work may use (Gentle, Balanced or Full speed), and
    low-memory diffs for large paintings.
  - Storage: where version history is kept, the preview cache size, compact storage for heavily
    reworked paintings, Clean up storage, Check for problems, and when you last made a backup.
  - Set-Aside: the shelf.
- **Artist Mode**, on by default, swaps git and code jargon for plain language ("Version 3" instead
  of a hash, the painting's name instead of a file path, "Updated" instead of `M`).
- **A first-launch welcome and tour**: a two-step welcome (your name, then a theme picked from preview
  cards), then a one-time spotlight tour of the shell. Replay either from Settings → Appearance.
- A dark, Krita-inspired UI built against [`DESIGN.md`](DESIGN.md), with light themes too.

## How it works

- **One store per painting.** Each tracked `.kra` gets a self-contained store in the hidden `.kvc/`
  container beside it (or under a folder you choose in Settings). Stores share nothing. Inside a
  store, chains are sharded per tracked file and loaded lazily, loose objects are sharded 256 ways,
  and a commit with many new objects writes one pack file instead of many loose files (creating files
  one by one dominated large commits on Windows).
- **Only `.kra` files are tracked.** Nothing else in the folder is touched, and Krita's autosave
  (`*-autosave.kra`) and backup (`*.kra~`) files are never picked up. The autosave one matters,
  because it ends in `.kra` and a naive extension check would version your scratch state as a real
  document.
- **A `.kra` tile-delta engine.** `.kra` files are zip archives of per-layer tiles, and the engine
  diffs and stores them tile by tile, so a small edit to one layer stores a small delta, not a whole
  new file.
- **Two-stage visual diffs.** `commit_diff` returns the capped composite and the layer metadata
  quickly, then per-layer rasters stream in over a Tauri channel as each one finishes. Rasters are
  cached content-addressed in the store's `cache/` and served to the webview as browser-cacheable
  `kvcimg://` URLs. See [`docs/visual-diff-viewer.md`](docs/visual-diff-viewer.md) and
  [`docs/performance.md`](docs/performance.md).

## Getting started

The package manager is npm.

```bash
npm install          # install the JS dependencies
npm run tauri dev    # run the full desktop app (Vite dev server plus the Tauri webview)
```

Then use the switcher at the top: choose "Track an artwork…" and pick a `.kra` file. Save a version
in the Changes panel, edit the painting in Krita, save it, and save another version to see a visual
diff.

Frontend only (in a browser, with no Tauri shell and no backend; for UI work):

```bash
npm run dev          # Vite dev server at http://localhost:1420 (add ?mock for a demo history)
```

Build and package:

```bash
npm run build        # formats the source (prettier, cargo fmt), type-checks, checks the Version Map
                     # layout, and builds the frontend to dist/
npm run tauri build  # production desktop bundle (frontend build, Rust binaries and installers)
npx tsc --noEmit     # a type-check that doesn't reformat anything
```

`npm run build` rewrites source files, so don't run it with changes you aren't ready to have
reformatted.

Rust side (from `src-tauri/`):

```bash
cargo check          # compile the backend
cargo test           # integration tests in tests/ plus unit tests
cargo test --release --test bench -- --ignored --nocapture   # performance baseline
cargo build --release --bin kvc                              # the headless CLI the Krita plugin uses
```

## Project layout

```
src/
├─ components/
│  ├─ shell/   AppShell, TopBar and SwitchArtworkModal (the artwork switcher), ActivityBar,
│  │           Sidebar, Inspector, StatusBar, SettingsModal, BackupModal, RestoreModal,
│  │           RestoreCompareModal, BusyOverlay, OnboardingOverlay, TourOverlay, DockerPanel
│  ├─ vcs/     the Version Map (VersionMapPanel, VersionNode), the diff viewer (DiffView,
│  │           ArtDiffView, ArtCanvas, CompareSlider, LayerStackPanel, PaletteDiffView), the
│  │           Changes, Performance and legacy History and Branches panels, the commit graph,
│  │           branch and set-aside dialogs, useBranchActions
│  ├─ ui/      Button, IconButton, Switch, Slider, Checkbox, Radio, Menu (and Select), Modal, Tooltip
│  └─ MainPanel.tsx
├─ lib/        data hooks and Tauri calls (repoData.ts), the repository, Artist Mode, legacy
│              history, author name, theme, window chrome, CPU budget, welcome and tour contexts,
│              the Version Map layout, SVG compositing, zoom, pan and resize hooks
├─ styles/     global.css (Tailwind v4 @theme tokens from DESIGN.md)
└─ types.ts    domain types (the frontend and backend contract)

src-tauri/src/   the Rust backend (crate krita_vc_lib)
├─ repo, scan, commit, stage, delta, branch, stash, merge, gc, check   the local VCS engine
├─ kra, tiles, raster          .kra parsing, the tile store, rasters and diff imaging
├─ palette                     .gpl, .kpl, .aco and .ase parsing and the swatch diff
├─ cpu, diskspace, ops_log     CPU budget, free-space preflight, audit log
├─ commands.rs                 the Tauri #[command] IPC surface
├─ bin/kvc.rs                  the headless CLI over the same engine (for the Krita plugin)
└─ lib.rs, main.rs             the Tauri builder and entry point

docs/          developer documentation
content/       source copy for the marketing site, and the release notes (gitignored, local only)
DESIGN.md      the visual and interaction spec
krita-plugin/  the optional Krita docker plugin (see below)
```

## Krita plugin

A companion "Version Control" docker for Krita itself: save a version, discard changes, set work
aside and bring it back, and switch or create branches, without switching to this app. It saves your
painting for you (when you click into the panel, when you press refresh, and before every commit), so
a version can't miss work still sitting in Krita's memory, and it reopens the painting after any
action that rewrites it on disk.

It's a small Python (PyKrita) plugin that runs `kvc`, a headless CLI built from the same Rust engine
(`src-tauri/src/bin/kvc.rs`, no Tauri dependency), so the plugin and the desktop app go through the
same code against the same store. Every installer puts `kvc` next to the app. Starting to track a
painting, browsing and restoring history, undo, picking layers, merging and backups stay in the
desktop app. See [`krita-plugin/README.md`](krita-plugin/README.md) for installing, building and
troubleshooting.

## Documentation

- [`docs/`](docs/README.md): the frontend and backend architecture, how each feature works
  (version control, layer staging, setting work aside, backup and restore, the Version Map, the
  visual diff viewer, the welcome and tour), data integrity, performance, and the project's history.
- [`krita-plugin/README.md`](krita-plugin/README.md): the in-Krita docker, from install to
  troubleshooting.
- [`DESIGN.md`](DESIGN.md): design tokens, components and the interaction spec.
- [Releases on GitHub](https://github.com/zeru-sakamoto/krita-vc/releases): the changelog, with
  installers and the plugin zip for every version.
- [`CLAUDE.md`](CLAUDE.md): repository guidance and commands for Claude Code.

## License

GNU General Public License v3.0. See [`LICENSE`](LICENSE).

## Recommended IDE setup

[VS Code](https://code.visualstudio.com/) with the
[Tauri](https://marketplace.visualstudio.com/items?itemName=tauri-apps.tauri-vscode) and
[rust-analyzer](https://marketplace.visualstudio.com/items?itemName=rust-lang.rust-analyzer)
extensions.
