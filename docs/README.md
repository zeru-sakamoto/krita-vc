# Krita VCS documentation

Developer documentation for the Krita VCS desktop app (Tauri 2, React 19 and TypeScript) and the
Krita plugin that shares its engine.

The Rust backend is a working local version control system with its own store (not git) and a
`.kra` tile-delta engine. One `.kra` document is one history, and branches can be created, switched,
merged and deleted locally. The frontend drives it through Tauri commands in the desktop shell. In a
plain browser (`npm run dev`) the UI renders with empty data and actions do nothing, apart from an
opt-in `?mock` fixture for layout work. Visual diffs of `.kra` files are real and load in two stages:
the composite and metadata first, then per-layer rasters streamed in.

## Architecture

- [Frontend architecture](frontend-architecture.md): the app shell and its zones, who owns which
  state, the sidebar views, Settings, the diff viewer's routing, Artist Mode, the custom title bar,
  the theme selector, the component map and the shared UI primitives.
- [Backend architecture](backend-architecture.md): the Rust crate's module map, how a request gets
  from a Tauri command or the `kvc` CLI into the engine and back, the concurrency model, the two
  binaries that share one crate, a reference for every Tauri command and CLI subcommand, and the
  third-party crates with their licenses.

## How the features work

- [File tracking and version control](version-control.md): the store layout, locking, the scanner,
  commits, the delta chains, the `.kra` tile engine, restoring, rollback and undo, branches, and
  palette diffs.
- [Per-document tracking](per-document-tracking.md): why one painting is one history, why the shipped
  design (one hidden container, many self-contained stores) isn't the shared-object-store split the
  original proposal specified, and what's still not done.
- [Layer-subset staging](layer-staging.md): saving only the ticked layers, how the partial version is
  synthesized, and why the painting keeps showing as changed afterwards.
- [Setting work aside](stashes.md): stashes in the engine, merging set-aside layers back onto an
  edited painting, and where the actions live in the app and the Krita docker.
- [Backup and restore](backup-and-restore.md): the multi-painting archive, how a backup is verified,
  restoring to this machine's history location, and the version comparison shown before Replace.
- [The Version Map](version-map.md): the default view. Branch lanes and colors, how the line is
  drawn, the minimap, branch actions and pick-a-version branching, the pending-version preview, and
  the Legacy version history toggle.
- [The visual diff viewer](visual-diff-viewer.md): how a `.kra` renders as layer images, SVG
  compositing, the highlight and compare modes, and where the data comes from.
- [First-launch welcome and tour](onboarding-and-tour.md): the two-step welcome, and the spotlight
  tour, including how a step gates itself on what the shell is showing.

## Quality

- [Data integrity](data-integrity.md): every measure the engine takes to avoid losing an artist's
  work, from the lock and atomic, fsynced writes to verified reads, the read-only check and its
  optional scrub, cleanup's trash folder, verified backups, the stash ordering rules and input
  validation.
- [Performance](performance.md): why the `.kra` diff path is fast (two-stage and streamed loading,
  parallelism, skipping work, caching, downscaling, storage formats) and the build profile behind it.
- [CPU headroom](cpu-headroom.md): how the engine leaves room for Krita, with its own lower-priority
  worker pool, a user-set CPU budget and a cap on concurrent heavy work.
- [Performance report](performance-report.md): the Performance tab, its client-side timing and its
  storage-saved figures, and how each is measured.

## History

- [Project history](history/README.md): how the current architecture came to be, from the abandoned
  `git2` prototype and the custom tile-delta engine through branching, staging, stashing, CPU
  headroom, data-integrity hardening, the Bento redesign, the Version Map, the per-document rewrite,
  backup and restore, layer-subset staging, and v2.1.0, one era per file, with a table mapping every
  release tag to its commit.

## See also

- [`../krita-plugin/README.md`](../krita-plugin/README.md): the in-Krita "Version Control" docker
  (save a version, discard, set aside and switch branches without leaving Krita; it saves your
  painting for you, because the engine only ever sees the disk), built on the headless `kvc` CLI
  (`src-tauri/src/bin/kvc.rs`).
- [`../DESIGN.md`](../DESIGN.md): the visual and interaction spec the UI is built against.
- [`../CLAUDE.md`](../CLAUDE.md): repository guidance, commands and architecture notes for Claude
  Code.
- `content/`: the source copy for the marketing site, the release notes and the performance audit.
  It's gitignored, so it only exists in local checkouts; the published release notes are on
  [GitHub Releases](https://github.com/zeru-sakamoto/krita-vc/releases).
