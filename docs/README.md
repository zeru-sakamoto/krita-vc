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
  ownership, the component map, **Artist Mode** (the global friendly-labels toggle), and the
  **theme selector** (color themes + the theme-reactive diff highlight).
- [**File tracking & version control**](version-control.md) — the Rust backend: the `.kvc/` store,
  the scanner, commits, branches (create/switch/merge), delta-chain storage, the `.kra` tile
  engine, and the Tauri commands.
- [**Visual diff viewer**](visual-diff-viewer.md) — how art (`.kra`) files render as layer images
  and visual diffs: the data model, SVG compositing, and the highlight/compare modes.
- [**Performance**](performance.md) — why the `.kra` diff path is fast: two-stage/streamed loading,
  parallelism, caching, downscaling, and the build profile tuning behind each.

## See also

- [`../krita-plugin/README.md`](../krita-plugin/README.md) — the in-Krita "Version Control"
  docker (commit/checkpoint/branch-switch without leaving Krita), built on the headless `kvc`
  CLI (`src-tauri/src/bin/kvc.rs`) that reuses this engine with no Tauri dependency.
- [`../DESIGN.md`](../DESIGN.md) — the visual + interaction spec the UI is built against.
- [`../CLAUDE.md`](../CLAUDE.md) — repo guidance, commands, and Tauri architecture.
