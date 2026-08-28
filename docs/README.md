# Krita VCS — Documentation

Developer documentation for the Krita VCS desktop app (Tauri 2 + React 19 + TypeScript).

> **Status:** the Rust backend is a working custom local VCS (the `.kvc/` store — see below) with
> full local branching (create / fast switch / merge); the frontend drives it through Tauri
> commands in the desktop shell. There is no mock data — in a plain browser (`npm run dev`) the
> UI renders with empty data and no-op actions. `.kra` diffs are real, loading in two stages
> (fast composite + metadata, then lazily-streamed per-layer rasters); non-`.kra` diffs are
> still minimal.

## Contents

- [**Frontend architecture**](frontend-architecture.md) — app shell, the four zones, state
  ownership, the component map, the **Version Map** (the default view — a pannable/zoomable
  canvas of versions, replacing the History graph; branch lanes, branch color, the floating
  branch action bar and pick-a-version branching, the pending-version preview node, the grid
  background, and the Legacy version history toggle that brings the old History/Branches tabs
  back), **backup & restore** (the multi-artwork archive, and the version comparison a restore
  offers when the destination is already tracked), **Artist Mode** (the global friendly-labels
  toggle), the **theme selector** (color themes
  + the theme-reactive diff highlight), and the **application tour** (the first-launch spotlight
  walkthrough — including how a step gates itself on the shell state its target needs).
- [**Backend architecture**](backend-architecture.md) — the Rust crate's structural map: module
  layout, the request flow from a Tauri command or the `kvc` CLI into the engine and back, the
  concurrency model (`RepoLock` + the CPU-budgeted pool), the two binaries that share one crate,
  and the third-party crates in use with their licenses.
- [**File tracking & version control**](version-control.md) — the Rust backend's *feature*
  behavior: the `.kvc/` store, the scanner, commits, branches (create/switch/merge), stashes
  (setting work aside), delta-chain storage, the `.kra` tile engine, and the Tauri commands.
- [**Per-document tracking**](per-document-tracking.md) — *design note, not implemented.* Moving
  from one history per folder to one history per `.kra` document: the `Project`/`Document` split,
  a shared object store with per-document history, migration, and the blast radius.
- [**Data integrity**](data-integrity.md) — the measures the engine already applies to avoid
  losing an artist's work: the cross-process repo lock, a `branches.json` generation counter that
  lets user-facing reads detect a stale snapshot, atomic + fsynced writes and save ordering (plus
  a previous-generation `.bak` for the small state files), write-time patch verification, verified
  reads on the restore paths, the read-only repository check (with an opt-in full-store bit-rot
  scrub), the GC safety model (quarantine-to-trash instead of outright delete), verified
  self-describing backups (and the side-by-side version comparison shown before a restore is
  allowed to replace an existing history), stash ordering invariants, and the input-validation
  caps at the trust boundaries.
- [**Visual diff viewer**](visual-diff-viewer.md) — how art (`.kra`) files render as layer images
  and visual diffs: the data model, SVG compositing, and the highlight/compare modes.
- [**Performance**](performance.md) — why the `.kra` diff path is fast: two-stage/streamed loading,
  parallelism, caching, downscaling, and the build profile tuning behind each.
- [**Performance report**](performance-report.md) — the **Performance** tab: client-side operation
  timing (localStorage) and the storage saved vs. full-copy-per-version metric, and how each is
  measured.
- [**Performance audit**](performance-audit.md) — measured speed and storage on **real** Krita
  documents: the commit/reconstruct/rollback baseline, the ~2.9x store-vs-naive ratio, the
  settled tile-delta decision, what was fixed, and what was measured and deliberately left alone.

## See also

- [`../krita-plugin/README.md`](../krita-plugin/README.md) — the in-Krita "Version Control"
  docker (commit, discard, set-aside, branch-switch without leaving Krita; it saves
  your documents for you, since the engine only ever sees the disk), built on the headless `kvc`
  CLI (`src-tauri/src/bin/kvc.rs`) that reuses this engine with no Tauri dependency.
- [`../DESIGN.md`](../DESIGN.md) — the visual + interaction spec the UI is built against.
- [`../REDESIGN.md`](../REDESIGN.md) — the screen-by-screen redesign tracker (status + scope).
- [`../CLAUDE.md`](../CLAUDE.md) — repo guidance, commands, and Tauri architecture.
