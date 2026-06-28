# Krita VCS — Documentation

Developer documentation for the Krita VCS desktop app (Tauri 2 + React 19 + TypeScript).

> **Status:** the Rust backend is a working custom local VCS (the `.kvc/` store — see below); the
> frontend calls it through Tauri commands in the desktop shell and falls back to mock data
> (`src/data/`) in a plain browser. Diff rendering is still mock until per-commit diffs are wired up.

## Contents

- [**Frontend architecture**](frontend-architecture.md) — app shell, the four zones, state
  ownership, the component map, and **Artist Mode** (the global friendly-labels toggle).
- [**File tracking & version control**](version-control.md) — the Rust backend: the `.kvc/` store,
  the scanner, commits, delta-chain storage, the `.kra` tile engine, and the Tauri commands.
- [**Visual diff viewer**](visual-diff-viewer.md) — how art (`.kra`) files render as layer images
  and visual diffs: the data model, the generated-SVG mock art, and the highlight/compare modes.

## See also

- [`../DESIGN.md`](../DESIGN.md) — the visual + interaction spec the UI is built against.
- [`../CLAUDE.md`](../CLAUDE.md) — repo guidance, commands, and Tauri architecture.
