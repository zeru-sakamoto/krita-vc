# Release Notes

## Krita VC v2.0-beta

### Highlights

- **Color palette tracking.** `.gpl`, `.kpl`, `.aco`, and `.ase` palette files are now
  version-controlled alongside `.kra` documents. Diffs show named color swatches, and a
  recolor is tracked as a "modified" swatch rather than a remove+add — including palettes
  embedded inside `.kra` documents themselves.
- **Per-file staging.** Stage or unstage individual changed files before committing, with
  stage-all/unstage-all shortcuts. A commit now only captures what's staged — partial
  commits are a first-class workflow, not all-or-nothing.
- **Discard changes.** Revert uncommitted working-tree edits on a per-file or whole-project
  basis without creating a new commit.
- **Restore lineage in the history graph.** Restoring an older version now draws a dashed
  connector back to the version it was restored from, so the graph shows where a rollback
  came from at a glance.
- **Performance tab.** A new activity-bar tab shows what the delta store is saving you: a
  per-version storage-saved breakdown (already around 50% smaller than a full copy by your
  second save, and climbing from there) alongside save/compare timing for every version.

### Fixes

- Restoring to the current/latest commit no longer silently does nothing when there were
  uncommitted changes.
- The Inspector panel now shows the correct info when browsing the Changes panel.
- New app icon.

### Also since v1.0-beta

- Theme selector in Settings.
- Security and performance hardening across the storage engine.

## Krita VC v1.0-beta

### Highlights

- **Local-only version control for Krita art files.** Commit history, branches,
  and working-tree changes for `.kra` files — no remotes, no cloud sync.
- **Visual diffing.** See exactly what changed between versions: composite
  and per-layer image diffs, with a changed-pixel highlight overlay and
  side-by-side/slider compare views.
- **Real branching & merging.** Create, switch, and merge local branches;
  conflicts are flagged rather than silently resolved.
- **Krita plugin docker.** Commit, checkpoint, and switch/create branches
  without leaving Krita.
- **Custom title bar.** Optional in-app window chrome (minimize/maximize/close),
  toggle in Settings — falls back to the OS-native frame anytime.
- **Settings.** Artist Mode toggle, author name, title bar preference, raster
  cache size, and a one-click "Clean up storage" action to reclaim unreachable
  history and stale cache.
- **Artist Mode.** Friendly, plain-language labels throughout the UI by default,
  with technical detail (hashes, file paths, raw diffs) available when turned off.

### Performance & storage

- Content-addressed, sharded storage for commit history and `.kra` composites
  keeps repos compact even with long edit histories.
- Streamed, staged diff loading — the diff panel appears immediately, then
  layers stream in as they finish rendering.

See [`docs/README.md`](docs/README.md) for architecture details and
[`docs/performance.md`](docs/performance.md) for the performance design.
